//! Dense embeddings (SPEC §3): model2vec static embeddings (`potion-base-8M`,
//! 256-dim) via `model2vec-rs` — mean-pooled static vectors, L2-norm, int8 output.
//! Pure Rust (safetensors + tokenizers + ndarray); no ONNX in this path.
//!
//! Status: Phase-0 scaffold — runtime lands in Phase 2.
