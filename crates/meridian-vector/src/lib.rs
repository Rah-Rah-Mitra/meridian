//! ANN vector index (SPEC §5): USearch wrapper (default feature) with hnsw_rs as the
//! pure-Rust fallback; int8 scalar quantizer (per-dim scale stored once); persistence
//! and RAM accounting; binary-quantization + rescore path (config, default off).
//!
//! HNSW parameters per SPEC §8.2: M=16, ef_construction=128, ef_search=64.
//!
//! Status: Phase-0 scaffold — implementation lands in Phase 2, parameterized by the
//! Phase-0 bench results.
