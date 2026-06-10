//! The `direct` lane (SPEC §12.2): plain reqwest + rustls, shared connection
//! pool, honest User-Agent with operator contact, minimal lane-consistent
//! headers (SPEC §13.4): no Referer, no cookies, fixed generic Accept-Language.
//! The only lane that may hedge, and the only lane enabled out of the box.

use crate::EgressError;
use meridian_common::config::DirectLaneConfig;
use std::net::SocketAddr;
use std::time::Duration;

pub struct DirectLane {
    /// Shared pooled client for trusted internal endpoints (SearXNG) and lane
    /// health checks. SSRF-guarded fetches use [`DirectLane::pinned_client`].
    client: reqwest::Client,
    cfg: DirectLaneConfig,
    user_agent: String,
}

impl DirectLane {
    pub fn new(cfg: &DirectLaneConfig) -> Result<Self, EgressError> {
        // SPEC §13.2: honest UA with contact URL on direct/region lanes.
        let user_agent = format!(
            "MeridianSearch/{} (+{})",
            env!("CARGO_PKG_VERSION"),
            cfg.contact_url
        );
        let client = Self::base_builder(cfg, &user_agent)
            .build()
            .map_err(|e| EgressError::NotReady(format!("direct client: {e}")))?;
        Ok(Self {
            client,
            cfg: cfg.clone(),
            user_agent,
        })
    }

    /// Every direct-lane client shares these invariants — pooled or pinned.
    fn base_builder(cfg: &DirectLaneConfig, user_agent: &str) -> reqwest::ClientBuilder {
        let mut headers = reqwest::header::HeaderMap::new();
        // Fixed, generic value so requests in a lane are mutually
        // indistinguishable (SPEC §13.4 header minimization).
        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            reqwest::header::HeaderValue::from_static("en"),
        );
        // TLS backend: rustls — the only one compiled in (workspace features).
        reqwest::Client::builder()
            .user_agent(user_agent)
            .default_headers(headers)
            .referer(false)
            // No cookie store is configured — reqwest's default; stated here as
            // a guardrail against someone "helpfully" enabling it (SPEC §13.4).
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_millis(cfg.connect_timeout_ms))
            .timeout(Duration::from_millis(cfg.total_timeout_ms))
    }

    /// Shared pooled client (internal endpoints; not for untrusted-URL fetches).
    pub fn pooled(&self) -> reqwest::Client {
        self.client.clone()
    }

    /// Client whose DNS for `host` is pinned to a pre-validated address — the
    /// fetch ladder's no-TOCTOU connect path (SPEC §13.1). Single-use, no pool
    /// reuse across requests by construction.
    pub fn pinned_client(
        &self,
        host: &str,
        addr: SocketAddr,
    ) -> Result<reqwest::Client, EgressError> {
        Self::base_builder(&self.cfg, &self.user_agent)
            .resolve(host, addr)
            .build()
            .map_err(|e| EgressError::NotReady(format!("pinned client: {e}")))
    }

    /// Hedge delay for metasearch duplicates (SPEC §11: direct lane only;
    /// `None` = hedging disabled).
    pub fn hedge_after(&self) -> Option<Duration> {
        (self.cfg.hedge_after_ms > 0).then(|| Duration::from_millis(self.cfg.hedge_after_ms))
    }
}
