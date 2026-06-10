//! Evaluation & benchmarking (SPEC §15): relevance eval harness (nDCG@10, MRR@10,
//! Recall@100 over trec-style qrels — Phase 2), the `meridian-bench` on-device suite
//! (implemented), and the privacy smoke test driver (Phase 1).
//!
//! The bench framework lives here so suites share report types, percentile math,
//! and the /proc-based resource probes.

pub mod bench;
pub mod probe;
pub mod stats;
