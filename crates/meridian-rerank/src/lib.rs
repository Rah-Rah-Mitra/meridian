//! Opt-in deep rerank (SPEC §11): `ms-marco-MiniLM-L-6-v2` INT8 cross-encoder via
//! `ort`, top-20 only, batch=4, Moka-cached by (query_hash, doc_hash), strict 1.5s
//! stage deadline — on overrun, return LTR order with `degraded:["rerank_timeout"]`.
//!
//! Status: Phase-0 scaffold — lands in Phase 3 behind the `rerank` cargo feature.
