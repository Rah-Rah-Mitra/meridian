//! Ranking models via `ort` (SPEC §11): intent GBDT classifier, LightGBM LTR model
//! (offline-trained → ONNX), feature extraction from tantivy fast fields. Cold-start
//! is hand-tuned linear weights behind the same ONNX interface (single Gemm).
//!
//! Status: Phase-0 scaffold — lands in Phase 3.
