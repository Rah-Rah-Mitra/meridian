//! Query planner & orchestrator (SPEC §3 pipeline): intent + language, budget and
//! egress-lane selection, local/metasearch fan-out with deadlines (hedging on the
//! direct lane ONLY), RRF fusion (k=60), MMR-lite domain diversity.
//!
//! Status: Phase 1 — lexical planner + ingest pipeline live; ANN joins the
//! fusion in Phase 2, LTR/rerank in Phase 3.

pub mod ingest;
pub mod planner;
pub mod rrf;

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
