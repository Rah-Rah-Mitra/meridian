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
//!   future `AnonClient` simply owns no direct transport.
//!
//! Status: Phase 1 — `direct` is live; `region`/`anon` arrive in Phase 4, each
//! gated by the SPEC §12.5 invariant tests.

pub mod direct;

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

    #[error("egress layer not yet implemented (Phase 4)")]
    Unimplemented,
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
/// fail-closed semantics. Phase 1 = `direct` only; a requested `region`/`anon`
/// returns the error the API surfaces as a degraded `lane` block — NEVER a
/// silent direct-lane fallback (SPEC §12.1).
pub struct LaneRegistry {
    direct: direct::DirectLane,
    anon_enabled: bool,
    regions_enabled: bool,
}

impl LaneRegistry {
    pub fn new(cfg: &meridian_common::config::LanesConfig) -> Result<Self, EgressError> {
        Ok(Self {
            direct: direct::DirectLane::new(&cfg.direct)?,
            anon_enabled: cfg.anon_enabled,
            regions_enabled: cfg.regions_enabled,
        })
    }

    pub fn direct(&self) -> &direct::DirectLane {
        &self.direct
    }

    /// Resolve a lane request. The error text distinguishes "disabled by
    /// config" from "not built yet" for honest `/v1/lanes` reporting.
    pub fn resolve(&self, lane: &Lane) -> Result<LaneClient, EgressError> {
        match lane {
            Lane::Direct => Ok(LaneClient {
                lane: Lane::Direct,
                http: self.direct.pooled(),
            }),
            Lane::Anon if !self.anon_enabled => Err(EgressError::LaneDisabled("anon")),
            Lane::Region(_) if !self.regions_enabled => Err(EgressError::LaneDisabled("region")),
            Lane::Anon | Lane::Region(_) => Err(EgressError::Unimplemented),
        }
    }

    /// Pinned single-use client for SSRF-validated fetches (SPEC §13.1).
    /// Phase 1: direct lane only.
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
            _ => self
                .resolve(lane)
                .map(|_| unreachable!("non-direct resolve cannot succeed yet")),
        }
    }

    /// Lane statuses for `GET /v1/lanes` (SPEC §10).
    pub fn statuses(&self) -> Vec<(String, LaneStatus)> {
        let mut v = vec![("direct".to_owned(), LaneStatus::Up)];
        v.push((
            "anon".to_owned(),
            if self.anon_enabled {
                LaneStatus::Down // enabled but not built until Phase 4
            } else {
                LaneStatus::Disabled
            },
        ));
        v.push((
            "region".to_owned(),
            if self.regions_enabled {
                LaneStatus::Down
            } else {
                LaneStatus::Disabled
            },
        ));
        v
    }
}
