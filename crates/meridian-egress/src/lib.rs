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
//! Status: Phase-0 scaffold. The trait and lane model are the contract the rest of
//! the workspace builds against; transports arrive in Phase 1 (direct) and Phase 4
//! (anon, region), each gated by the SPEC §12.5 invariant tests.

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

    #[error("egress layer not yet implemented (Phase 1/4)")]
    Unimplemented,
}

/// Opaque, lane-bound HTTP client handle.
///
/// Phase 1 wraps a `reqwest::Client` set; the type stays opaque here so callers can
/// never extract a transport and smuggle a request onto a different lane.
pub struct LaneClient {
    _private: (),
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
