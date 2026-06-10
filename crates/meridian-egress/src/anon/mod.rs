//! The anon lane (SPEC §12.4): Tor via embedded Arti, fail-closed always.
//!
//! Topology (ADR-04, amended Phase 4): ONE egress path — the in-process SOCKS5
//! listener ([`socks`]) backed by [`arti::ArtiDialer`]. `searxng-anon` uses it
//! as its sole upstream proxy, and Meridian's own anon fetches go through it
//! too (a fresh RFC1929 username per logical request → per-request
//! `IsolationToken`), rather than maintaining a second raw
//! `connect_with_prefs` HTTP stack. One code path, uniformly policy-checked.
//!
//! Fail-closed is enforced structurally, not by discipline:
//! - the reqwest client handed out by [`AnonLane::client_for_request`] is built
//!   with `Proxy::all(socks5h://…loopback listener…)` and is the ONLY transport
//!   the caller receives — reqwest never bypasses an explicit proxy;
//! - the listener's [`socks::Dialer`] has exactly one production
//!   implementation, Arti — there is no TCP fallback to fall back TO;
//! - while Arti is not `ready_for_traffic`, the lane refuses to build clients
//!   AND the listener refuses CONNECTs. Three layers, zero direct egress.

pub mod arti;
pub mod socks;

use crate::{EgressError, Lane, LaneClient, LaneStatus};
use arti_client::TorClient;
use meridian_common::config::AnonLaneConfig;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

/// Per-request username nonce. Predictability is fine (loopback isolation key,
/// not a secret); uniqueness per logical request is what matters.
static REQ_SEQ: AtomicU64 = AtomicU64::new(1);

struct Started {
    client: std::sync::Arc<TorClient<tor_rtcompat::PreferredRuntime>>,
    /// Actual bound address (config may say port 0 in tests).
    socks_addr: SocketAddr,
}

pub struct AnonLane {
    cfg: AnonLaneConfig,
    allow_onion: bool,
    state_root: PathBuf,
    started: OnceLock<Started>,
    /// Set by the bootstrap driver on failure (status surface only; the traffic
    /// gate is always `ready_for_traffic`).
    failure: RwLock<Option<String>>,
}

impl AnonLane {
    pub fn new(cfg: &AnonLaneConfig, allow_onion: bool, data_dir: &Path) -> Self {
        let state_root = cfg
            .state_dir
            .clone()
            .unwrap_or_else(|| data_dir.join("arti"));
        Self {
            cfg: cfg.clone(),
            allow_onion,
            state_root,
            started: OnceLock::new(),
            failure: RwLock::new(None),
        }
    }

    /// Bring the lane up: create the Tor client, bind the SOCKS listener, and
    /// drive bootstrap in the background. Idempotent; must run on the runtime.
    pub async fn start(self: &Arc<Self>) -> Result<(), EgressError> {
        if self.started.get().is_some() {
            return Ok(());
        }
        let client = arti::build_client(&self.state_root).map_err(EgressError::NotReady)?;

        let listen: SocketAddr = self
            .cfg
            .socks_listen
            .parse()
            .map_err(|_| EgressError::NotReady("bad anon.socks_listen address".into()))?;
        let listener = tokio::net::TcpListener::bind(listen)
            .await
            .map_err(|e| EgressError::NotReady(format!("anon socks bind: {e}")))?;
        let socks_addr = listener
            .local_addr()
            .map_err(|e| EgressError::NotReady(format!("anon socks addr: {e}")))?;

        let server = socks::Socks5Server::new(
            Arc::new(arti::ArtiDialer::new(Arc::clone(&client))),
            socks::SocksPolicy {
                allow_onion: self.allow_onion,
                max_streams: self.cfg.max_circuits,
                dial_timeout: Duration::from_millis(self.cfg.connect_timeout_ms),
            },
        );
        tokio::spawn(server.run(listener));

        // Bootstrap driver: retry with backoff; status is observable throughout.
        let lane = Arc::clone(self);
        let boot_client = Arc::clone(&client);
        tokio::spawn(async move {
            loop {
                match boot_client.bootstrap().await {
                    Ok(()) => {
                        *lane.failure.write().expect("anon failure lock") = None;
                        tracing::info!("anon lane bootstrapped");
                        break;
                    }
                    Err(e) => {
                        // Bootstrap errors carry no user data (directory fetch
                        // failures), but keep the surfaced status generic.
                        tracing::warn!(error = %e, "anon bootstrap failed; retrying in 30s");
                        *lane.failure.write().expect("anon failure lock") =
                            Some("bootstrap failing; retrying".to_owned());
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                }
            }
        });

        self.started
            .set(Started { client, socks_addr })
            .map_err(|_| EgressError::NotReady("anon lane started twice".into()))?;
        Ok(())
    }

    pub fn status(&self) -> LaneStatus {
        let Some(s) = self.started.get() else {
            return LaneStatus::Down;
        };
        if let Some(reason) = self.failure.read().expect("anon failure lock").clone() {
            return LaneStatus::Degraded(reason);
        }
        let bs = s.client.bootstrap_status();
        if bs.ready_for_traffic() {
            LaneStatus::Up
        } else {
            LaneStatus::Bootstrapping((bs.as_frac() * 100.0).clamp(0.0, 99.0) as u8)
        }
    }

    /// The SOCKS listener address `searxng-anon` should be pointed at (compose
    /// wires the hostname; this is for diagnostics/tests).
    pub fn socks_addr(&self) -> Option<SocketAddr> {
        self.started.get().map(|s| s.socks_addr)
    }

    /// A lane-bound client for ONE logical request (one isolation username =
    /// one Tor circuit family). Fails closed unless Arti is ready for traffic.
    pub fn client_for_request(&self) -> Result<LaneClient, EgressError> {
        let Some(s) = self.started.get() else {
            return Err(EgressError::NotReady("anon lane is not started".into()));
        };
        if !s.client.bootstrap_status().ready_for_traffic() {
            return Err(EgressError::NotReady(
                "anon lane is bootstrapping or down".into(),
            ));
        }
        let user = format!("iso-{}", REQ_SEQ.fetch_add(1, Ordering::Relaxed));
        // The listener may be bound wildcard (compose: reachable from the back
        // network); OUR hop to it is always loopback — name it explicitly.
        let mut proxy_addr = s.socks_addr;
        if proxy_addr.ip().is_unspecified() {
            proxy_addr.set_ip(match proxy_addr.ip() {
                std::net::IpAddr::V4(_) => std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                std::net::IpAddr::V6(_) => std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
            });
        }
        let proxy_url = format!("socks5h://{user}:x@{proxy_addr}");
        let proxy = reqwest::Proxy::all(&proxy_url)
            .map_err(|e| EgressError::NotReady(format!("anon proxy: {e}")))?;

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            reqwest::header::HeaderValue::from_static("en"),
        );
        // Generic UA on anon (SPEC §13.2): no operator-identifying fingerprint.
        let http = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (compatible)")
            .default_headers(headers)
            .referer(false)
            .redirect(reqwest::redirect::Policy::none())
            .proxy(proxy)
            .connect_timeout(Duration::from_millis(self.cfg.connect_timeout_ms))
            .timeout(Duration::from_millis(self.cfg.total_timeout_ms))
            .build()
            .map_err(|e| EgressError::NotReady(format!("anon client: {e}")))?;
        Ok(LaneClient {
            lane: Lane::Anon,
            http,
        })
    }
}
