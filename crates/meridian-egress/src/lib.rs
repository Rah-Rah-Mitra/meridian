//! Egress lane layer (SPEC §12) — the most security-sensitive subsystem.
//!
//! Lanes govern OUTBOUND NETWORK ONLY; local index/vector/rank stages are
//! lane-independent. Three lanes:
//!
//! - `direct` — plain Reqwest+Rustls, the only lane that may hedge, the only lane
//!   enabled out of the box.
//! - `region:<id>` — bind outbound source IP to an operator-managed WireGuard
//!   interface (host-network compose profile + documented `ip rule` policy routing).
//! - `anon` — embedded Arti (Tor). Fail-closed: if Arti cannot serve the request the
//!   lane errors; it never falls back to direct. Enforced at the type level — the
//!   anon client's only transport is the in-process SOCKS front-end, whose only
//!   dialer is Arti (see `anon::socks::Dialer`).
//!
//! Status: Phase 4 — all three lanes live, gated by the SPEC §12.5 invariant tests
//! in `tests/invariants.rs`.

pub mod anon;
pub mod direct;
pub mod region;

use meridian_common::ids::RegionId;

/// Which egress lane a request asked for (SPEC §12.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Lane {
    Direct,
    Region(RegionId),
    Anon,
}

/// Observable lane health, surfaced via `GET /v1/lanes` (SPEC §10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneStatus {
    Up,
    /// Bootstrap progress percentage (Arti directory download, WG verification).
    Bootstrapping(u8),
    /// Lane is up but impaired (e.g. anon error-rate > 30%). Surfaced to the caller;
    /// NEVER auto-falls back to another lane (SPEC §8.6).
    Degraded(String),
    Down,
    /// Disabled in configuration (the shipped default for anon/region).
    Disabled,
}

/// Per-request lane requirements resolved by the query planner.
#[derive(Debug, Clone)]
pub struct LaneSpec {
    pub lane: Lane,
    /// Total deadline for network work on this lane (anon gets the generous one).
    pub deadline: std::time::Duration,
}

/// Errors from lane resolution or transport construction.
///
/// There is deliberately no "fell back to another lane" variant: a lane that cannot
/// serve a request fails, visibly (SPEC §12.4 kill-switch).
#[derive(Debug, thiserror::Error)]
pub enum EgressError {
    #[error("lane is not enabled in configuration: {0}")]
    LaneDisabled(&'static str),

    #[error("lane is not ready: {0}")]
    NotReady(String),

    #[error("no such region lane configured: {0}")]
    UnknownRegion(String),
}

/// Opaque, lane-bound HTTP client handle.
///
/// The transport never leaves this type: callers get a `RequestBuilder` already
/// bound to the lane's client, so a request cannot be smuggled onto a different
/// lane (the SPEC §12.4 type-level guard, applied to every lane from day one).
#[derive(Clone)]
pub struct LaneClient {
    lane: Lane,
    http: reqwest::Client,
}

impl LaneClient {
    pub fn lane(&self) -> &Lane {
        &self.lane
    }

    /// Start a request on this lane. The builder is bound to the lane's
    /// transport; there is deliberately no accessor for the inner client.
    pub fn request(&self, method: reqwest::Method, url: reqwest::Url) -> reqwest::RequestBuilder {
        self.http.request(method, url)
    }

    pub fn get(&self, url: reqwest::Url) -> reqwest::RequestBuilder {
        self.http.get(url)
    }
}

/// The lane abstraction every outbound subsystem (`fetch`, `searx`) consumes.
///
/// `async fn` in a public trait: acceptable here because the trait is internal to the
/// workspace (not a public library API); revisit Send bounds when the first real
/// implementation lands in Phase 1.
#[allow(async_fn_in_trait)]
pub trait Egress {
    /// Returns a configured client (or per-request builder) bound to this lane.
    async fn client(&self, spec: &LaneSpec) -> Result<LaneClient, EgressError>;

    /// Current lane health for observability and planner decisions.
    fn status(&self) -> LaneStatus;
}

/// Process-wide lane registry: resolves a requested lane to a transport with
/// fail-closed semantics. A requested `region`/`anon` that cannot be satisfied
/// returns the error the API surfaces as a degraded `lane` block — NEVER a
/// silent direct-lane fallback (SPEC §12.1).
pub struct LaneRegistry {
    direct: direct::DirectLane,
    anon: Option<std::sync::Arc<anon::AnonLane>>,
    regions: std::collections::BTreeMap<String, std::sync::Arc<region::RegionLane>>,
    regions_enabled: bool,
}

impl LaneRegistry {
    pub fn new(
        cfg: &meridian_common::config::LanesConfig,
        data_dir: &std::path::Path,
    ) -> Result<Self, EgressError> {
        let anon = cfg.anon_enabled.then(|| {
            std::sync::Arc::new(anon::AnonLane::new(&cfg.anon, cfg.allow_onion, data_dir))
        });
        let mut regions = std::collections::BTreeMap::new();
        if cfg.regions_enabled {
            for (id, rc) in &cfg.regions {
                regions.insert(
                    id.clone(),
                    std::sync::Arc::new(region::RegionLane::new(id, rc, &cfg.direct)?),
                );
            }
        }
        Ok(Self {
            direct: direct::DirectLane::new(&cfg.direct)?,
            anon,
            regions,
            regions_enabled: cfg.regions_enabled,
        })
    }

    pub fn direct(&self) -> &direct::DirectLane {
        &self.direct
    }

    /// Bring up the network-touching lanes: Arti bootstrap + SOCKS listener,
    /// region verification loops. Must run on the Tokio runtime; cheap no-op
    /// when only `direct` is enabled.
    pub async fn start(&self) -> Result<(), EgressError> {
        if let Some(anon) = &self.anon {
            anon.start().await?;
        }
        for lane in self.regions.values() {
            let lane = std::sync::Arc::clone(lane);
            tokio::spawn(async move {
                // Bring-up verification, then periodic re-verification — a
                // routing change mid-flight must trip the gate too (SPEC §12.3).
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(30 * 60));
                loop {
                    tick.tick().await;
                    lane.verify_once().await;
                }
            });
        }
        Ok(())
    }

    /// Resolve a lane request to a transport for ONE logical request.
    /// Fail-closed everywhere: disabled, unknown, bootstrapping, unverified and
    /// verification-failed lanes all error; nothing ever falls back to direct.
    pub fn resolve(&self, lane: &Lane) -> Result<LaneClient, EgressError> {
        match lane {
            Lane::Direct => Ok(LaneClient {
                lane: Lane::Direct,
                http: self.direct.pooled(),
            }),
            Lane::Anon => self
                .anon
                .as_ref()
                .ok_or(EgressError::LaneDisabled("anon"))?
                .client_for_request(),
            Lane::Region(id) => {
                if !self.regions_enabled {
                    return Err(EgressError::LaneDisabled("region"));
                }
                let region = self
                    .regions
                    .get(&id.0)
                    .ok_or_else(|| EgressError::UnknownRegion(id.0.clone()))?;
                Ok(LaneClient {
                    lane: lane.clone(),
                    http: region.pooled()?,
                })
            }
        }
    }

    /// Pinned single-use client for SSRF-validated fetches (SPEC §13.1) on the
    /// resolve-then-pin lanes (direct/region). The anon lane has no pin step —
    /// resolution happens inside Tor — so the fetch ladder uses
    /// [`LaneRegistry::resolve`] + the static destination policy there instead.
    pub fn pinned(
        &self,
        lane: &Lane,
        host: &str,
        addr: std::net::SocketAddr,
    ) -> Result<LaneClient, EgressError> {
        match lane {
            Lane::Direct => Ok(LaneClient {
                lane: Lane::Direct,
                http: self.direct.pinned_client(host, addr)?,
            }),
            Lane::Region(id) => {
                if !self.regions_enabled {
                    return Err(EgressError::LaneDisabled("region"));
                }
                let region = self
                    .regions
                    .get(&id.0)
                    .ok_or_else(|| EgressError::UnknownRegion(id.0.clone()))?;
                Ok(LaneClient {
                    lane: lane.clone(),
                    http: region.pinned_client(host, addr)?,
                })
            }
            Lane::Anon => Err(EgressError::NotReady(
                "anon lane does not pin addresses; use resolve()".into(),
            )),
        }
    }

    /// Client for a back-network sidecar hop carried out ON BEHALF of `lane`
    /// (the planner → `searxng-anon` HTTP call). The transport is the internal
    /// pool — the sidecar is on the isolated `back` network and the actual
    /// EGRESS happens at the sidecar through this process's Tor SOCKS listener —
    /// but the returned handle is labeled with the logical lane so
    /// `effective_lane` reporting stays truthful (§12.5 invariant 5).
    pub fn sidecar(&self, lane: Lane) -> LaneClient {
        LaneClient {
            lane,
            http: self.direct.pooled(),
        }
    }

    /// The anon lane handle (status checks, tests). `None` when disabled.
    pub fn anon(&self) -> Option<&std::sync::Arc<anon::AnonLane>> {
        self.anon.as_ref()
    }

    /// Lane statuses for `GET /v1/lanes` (SPEC §10).
    pub fn statuses(&self) -> Vec<(String, LaneStatus)> {
        let mut v = vec![("direct".to_owned(), LaneStatus::Up)];
        v.push((
            "anon".to_owned(),
            match &self.anon {
                Some(lane) => lane.status(),
                None => LaneStatus::Disabled,
            },
        ));
        if self.regions_enabled && !self.regions.is_empty() {
            for (id, lane) in &self.regions {
                let status = match lane.verify_state() {
                    region::VerifyState::Verified => LaneStatus::Up,
                    region::VerifyState::Pending => LaneStatus::Bootstrapping(0),
                    region::VerifyState::Failed(reason) => LaneStatus::Degraded(reason),
                };
                v.push((format!("region:{id}"), status));
            }
        } else {
            v.push(("region".to_owned(), LaneStatus::Disabled));
        }
        v
    }
}
