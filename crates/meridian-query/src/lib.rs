//! Query planner & orchestrator (SPEC §3 pipeline): intent + language, budget and
//! egress-lane selection, local/metasearch fan-out with deadlines (hedging on the
//! direct lane ONLY), RRF fusion (k=60), MMR-lite domain diversity.
//!
//! Status: Phase 1 — lexical planner + ingest pipeline live; ANN joins the
//! fusion in Phase 2, LTR/rerank in Phase 3.

pub mod ingest;
pub mod intent;
pub mod planner;
pub mod rrf;

// Re-exports so meridian-api consumes lane/fetch types through its sanctioned
// dependency (api → query) instead of growing direct edges (SPEC §5 rules).
pub use meridian_egress::{Lane, LaneRegistry, LaneStatus};
pub use meridian_fetch::{FetchError, Fetcher};

/// Search execution mode (SPEC §10). `Deep` adds the INT8 cross-encoder rerank stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    #[default]
    Fast,
    Deep,
}

/// Which result sources a query consults (SPEC §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    Local,
    Web,
    #[default]
    Both,
}
