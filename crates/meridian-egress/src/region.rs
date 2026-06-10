//! `region:<id>` lanes (SPEC §12.3): lawful region-vantaged retrieval through
//! operator-managed WireGuard interfaces. Meridian's entire involvement is
//! binding outbound sockets to the interface's source IP (`local_address`);
//! policy routing — applied by the operator from `deploy/wg-lane-templates/`,
//! never auto-mutated — steers the packets. Keys never reach Meridian.
//!
//! Verification (the routing-leak tripwire): on bring-up and every 30 minutes,
//! fetch the configured IP-echo endpoint through the lane and compare the
//! observed egress IP against `expected_ip`. Anything but a clean match leaves
//! the lane refusing traffic — a region lane that egresses from the wrong
//! address is a leak, not a degraded-but-usable lane.

use crate::EgressError;
use meridian_common::config::{DirectLaneConfig, RegionConfig};
use std::net::SocketAddr;
use std::sync::RwLock;
use std::time::Duration;

/// Verified / Unverified / Failed(reason) — mapped to LaneStatus in lib.rs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyState {
    /// Not yet verified since process start (lane refuses traffic).
    Pending,
    /// Echo fetch succeeded and the egress IP matched expectation.
    Verified,
    /// Echo fetch failed or the egress IP mismatched (lane refuses traffic).
    Failed(String),
}

pub struct RegionLane {
    id: String,
    cfg: RegionConfig,
    base: DirectLaneConfig,
    user_agent: String,
    /// Shared pooled client, source-IP bound. SSRF-pinned fetches use
    /// [`RegionLane::pinned_client`] (same invariants + `resolve`).
    client: reqwest::Client,
    verify: RwLock<VerifyState>,
}

impl RegionLane {
    pub fn new(id: &str, cfg: &RegionConfig, base: &DirectLaneConfig) -> Result<Self, EgressError> {
        // Honest UA with operator contact on region lanes too (SPEC §13.2).
        let user_agent = format!(
            "MeridianSearch/{} (+{})",
            env!("CARGO_PKG_VERSION"),
            base.contact_url
        );
        let client = Self::base_builder(cfg, base, &user_agent)
            .build()
            .map_err(|e| EgressError::NotReady(format!("region client: {e}")))?;
        Ok(Self {
            id: id.to_owned(),
            cfg: cfg.clone(),
            base: base.clone(),
            user_agent,
            client,
            verify: RwLock::new(VerifyState::Pending),
        })
    }

    /// Same header/redirect/timeout invariants as the direct lane, plus the
    /// source-IP bind that makes it a region lane.
    fn base_builder(
        cfg: &RegionConfig,
        base: &DirectLaneConfig,
        user_agent: &str,
    ) -> reqwest::ClientBuilder {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            reqwest::header::HeaderValue::from_static("en"),
        );
        reqwest::Client::builder()
            .user_agent(user_agent)
            .default_headers(headers)
            .referer(false)
            .redirect(reqwest::redirect::Policy::none())
            .local_address(cfg.source_ip)
            .connect_timeout(Duration::from_millis(base.connect_timeout_ms))
            .timeout(Duration::from_millis(base.total_timeout_ms))
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn verify_state(&self) -> VerifyState {
        self.verify.read().expect("region verify lock").clone()
    }

    /// Pooled client, ONLY once verification has passed (fail-closed).
    pub fn pooled(&self) -> Result<reqwest::Client, EgressError> {
        self.require_verified()?;
        Ok(self.client.clone())
    }

    /// Pinned single-use client for SSRF-vetted fetches (SPEC §13.1), with the
    /// region source-IP bind. Fail-closed on unverified lanes.
    pub fn pinned_client(
        &self,
        host: &str,
        addr: SocketAddr,
    ) -> Result<reqwest::Client, EgressError> {
        self.require_verified()?;
        Self::base_builder(&self.cfg, &self.base, &self.user_agent)
            .resolve(host, addr)
            .build()
            .map_err(|e| EgressError::NotReady(format!("region pinned client: {e}")))
    }

    fn require_verified(&self) -> Result<(), EgressError> {
        match self.verify_state() {
            VerifyState::Verified => Ok(()),
            VerifyState::Pending => Err(EgressError::NotReady(
                "region lane awaiting egress-IP verification".into(),
            )),
            VerifyState::Failed(reason) => Err(EgressError::NotReady(format!(
                "region lane failed verification: {reason}"
            ))),
        }
    }

    /// One verification round (SPEC §12.3): fetch the echo endpoint THROUGH the
    /// lane, compare the observed egress IP. Sets the gate accordingly.
    pub async fn verify_once(&self) {
        let state = match self.fetch_egress_ip().await {
            Err(reason) => VerifyState::Failed(reason),
            Ok(observed) => match &self.cfg.expected_ip {
                None => VerifyState::Verified,
                Some(expected) => {
                    let prefix = expected.ends_with('.') || expected.ends_with(':');
                    let matches = if prefix {
                        observed.starts_with(expected.as_str())
                    } else {
                        observed == *expected
                    };
                    if matches {
                        VerifyState::Verified
                    } else {
                        // The observed IP is the operator's own egress address —
                        // fine to surface to the operator's status endpoint.
                        VerifyState::Failed(format!("egress ip mismatch (observed {observed})"))
                    }
                }
            },
        };
        let verified = state == VerifyState::Verified;
        *self.verify.write().expect("region verify lock") = state;
        tracing::info!(region = %self.id, verified, "region lane verification");
    }

    async fn fetch_egress_ip(&self) -> Result<String, String> {
        // Deliberately self.client (the unverified pooled client): verification
        // must exercise the exact transport that will carry traffic.
        let response = self
            .client
            .get(&self.cfg.verify_url)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    "echo fetch connect error".to_owned()
                } else if e.is_timeout() {
                    "echo fetch timeout".to_owned()
                } else {
                    "echo fetch transport error".to_owned()
                }
            })?;
        if !response.status().is_success() {
            return Err(format!("echo fetch status {}", response.status().as_u16()));
        }
        let body = response
            .text()
            .await
            .map_err(|_| "echo fetch body error".to_owned())?;
        let observed = body.trim();
        // Echo endpoints return a bare IP; refuse anything else.
        observed
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.to_string())
            .map_err(|_| "echo endpoint returned a non-IP body".to_owned())
    }
}
