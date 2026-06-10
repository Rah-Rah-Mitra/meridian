//! Deep-mode cross-encoder rerank (SPEC §11). `mode=deep` only; the largest
//! quality jump, too slow for the default path on the A76.
//!
//! ADR-02: runs on `ort` (ONNX Runtime) behind the `ort-backend` feature, which
//! needs glibc ≥2.39 → the gnu image variant. The default scratch/musl image
//! builds WITHOUT this feature, and the planner degrades `mode=deep` to LTR order
//! with `degraded:["rerank_unavailable"]`. tract is excluded (can't load the INT8
//! export, can't cross-compile under musl).
//!
//! The crate always compiles; the [`Reranker`] type is a no-op stub without the
//! feature so the planner can hold one unconditionally.

use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum RerankError {
    #[error("rerank model load failed: {0}")]
    Load(String),
    #[error("rerank inference failed: {0}")]
    Inference(String),
    #[error("rerank not compiled in (build with --features ort-backend on a gnu target)")]
    NotCompiled,
}

/// One (query, candidate) pair to score.
#[derive(Debug, Clone)]
pub struct Pair {
    /// Stable identity used as the cache key alongside the query hash.
    pub doc_key: u64,
    pub title: String,
    pub snippet: String,
}

/// A reranked result: the pair's doc_key with its cross-encoder relevance score.
#[derive(Debug, Clone, Copy)]
pub struct Scored {
    pub doc_key: u64,
    pub ce_score: f32,
}

/// Deep reranker. Construction succeeds (and `available()` is true) only with the
/// `ort-backend` feature on a supported target; otherwise it is an inert stub.
pub struct Reranker {
    #[cfg(feature = "ort-backend")]
    inner: backend::OrtReranker,
    #[cfg(not(feature = "ort-backend"))]
    _private: (),
}

impl Reranker {
    /// Load the INT8 cross-encoder from `<models_dir>/ms-marco-minilm-l6-v2/`.
    /// Returns `Err(NotCompiled)` when the feature is off — the planner treats
    /// that as "degrade deep mode", not a fatal error.
    pub fn load(models_dir: &std::path::Path) -> Result<Self, RerankError> {
        #[cfg(feature = "ort-backend")]
        {
            Ok(Self {
                inner: backend::OrtReranker::Loaded(Box::new(backend::Inner::load(models_dir)?)),
            })
        }
        #[cfg(not(feature = "ort-backend"))]
        {
            let _ = models_dir;
            Err(RerankError::NotCompiled)
        }
    }

    /// An always-inert reranker: `available()` is false, `rerank()` errors. The
    /// planner uses this when the model is missing or the feature is off, so
    /// `mode=deep` degrades cleanly instead of failing startup.
    pub fn unavailable() -> Self {
        #[cfg(feature = "ort-backend")]
        {
            Self {
                inner: backend::OrtReranker::Inert,
            }
        }
        #[cfg(not(feature = "ort-backend"))]
        {
            Self { _private: () }
        }
    }

    pub fn available(&self) -> bool {
        #[cfg(feature = "ort-backend")]
        {
            matches!(self.inner, backend::OrtReranker::Loaded(_))
        }
        #[cfg(not(feature = "ort-backend"))]
        {
            false
        }
    }

    /// Score `pairs` against `query`, returning them sorted by CE score desc.
    /// Honors `deadline` (SPEC §11: 1.5s stage budget) — on overrun, returns
    /// what completed so the planner can fall back to LTR order for the rest.
    pub fn rerank(
        &self,
        query: &str,
        pairs: &[Pair],
        deadline: Duration,
    ) -> Result<Vec<Scored>, RerankError> {
        #[cfg(feature = "ort-backend")]
        {
            match &self.inner {
                backend::OrtReranker::Loaded(inner) => inner.rerank(query, pairs, deadline),
                backend::OrtReranker::Inert => Err(RerankError::NotCompiled),
            }
        }
        #[cfg(not(feature = "ort-backend"))]
        {
            let _ = (query, pairs, deadline);
            Err(RerankError::NotCompiled)
        }
    }
}

#[cfg(feature = "ort-backend")]
mod backend {
    use super::{Pair, RerankError, Scored};
    use ort::session::Session;
    use ort::session::builder::GraphOptimizationLevel;
    use ort::value::Tensor;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

    const SEQ_LEN: usize = 256;
    const BATCH: usize = 4;

    pub enum OrtReranker {
        Inert,
        Loaded(Box<Inner>),
    }

    pub struct Inner {
        // ort's Session::run is &mut self; serialize behind a Mutex. The rerank
        // stage already runs on the rayon pool, one query at a time.
        session: Mutex<Session>,
        tokenizer: Tokenizer,
        n_inputs: usize,
    }

    /// (input_ids, attention_mask, token_type_ids), each flat [batch*SEQ_LEN].
    type EncodedBatch = (Vec<i64>, Vec<i64>, Vec<i64>);

    impl Inner {
        pub fn load(models_dir: &std::path::Path) -> Result<Self, RerankError> {
            let dir = models_dir.join("ms-marco-minilm-l6-v2");
            let model_path = dir.join("model_qint8_arm64.onnx");
            let tok_path = dir.join("tokenizer.json");
            let session = Session::builder()
                .map_err(|e| RerankError::Load(e.to_string()))?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(|e| RerankError::Load(e.to_string()))?
                .with_intra_threads(4)
                .map_err(|e| RerankError::Load(e.to_string()))?
                .with_inter_threads(1)
                .map_err(|e| RerankError::Load(e.to_string()))?
                .commit_from_file(&model_path)
                .map_err(|e| RerankError::Load(e.to_string()))?;
            let n_inputs = session.inputs().len();

            let mut tokenizer =
                Tokenizer::from_file(&tok_path).map_err(|e| RerankError::Load(e.to_string()))?;
            tokenizer
                .with_truncation(Some(TruncationParams {
                    max_length: SEQ_LEN,
                    ..Default::default()
                }))
                .map_err(|e| RerankError::Load(e.to_string()))?;
            tokenizer.with_padding(Some(PaddingParams {
                strategy: PaddingStrategy::Fixed(SEQ_LEN),
                ..Default::default()
            }));

            Ok(Self {
                session: Mutex::new(session),
                tokenizer,
                n_inputs,
            })
        }

        pub fn rerank(
            &self,
            query: &str,
            pairs: &[Pair],
            deadline: Duration,
        ) -> Result<Vec<Scored>, RerankError> {
            let started = Instant::now();
            let mut out = Vec::with_capacity(pairs.len());
            let mut session = self.session.lock().expect("rerank session");

            for chunk in pairs.chunks(BATCH) {
                if started.elapsed() >= deadline {
                    break; // partial — planner keeps LTR order for the rest
                }
                let batch = chunk.len();
                let (ids, mask, types) = self.encode_batch(query, chunk)?;

                let shape = [batch as i64, SEQ_LEN as i64];
                let t_ids = Tensor::from_array((shape, ids))
                    .map_err(|e| RerankError::Inference(e.to_string()))?;
                let t_mask = Tensor::from_array((shape, mask))
                    .map_err(|e| RerankError::Inference(e.to_string()))?;

                let outputs = if self.n_inputs >= 3 {
                    let t_types = Tensor::from_array((shape, types))
                        .map_err(|e| RerankError::Inference(e.to_string()))?;
                    session.run(ort::inputs! {
                        "input_ids" => t_ids,
                        "attention_mask" => t_mask,
                        "token_type_ids" => t_types,
                    })
                } else {
                    session.run(ort::inputs! {
                        "input_ids" => t_ids,
                        "attention_mask" => t_mask,
                    })
                }
                .map_err(|e| RerankError::Inference(e.to_string()))?;

                let (out_shape, logits) = outputs[0]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| RerankError::Inference(e.to_string()))?;
                // Cross-encoder: [batch, 1] relevance score, or [batch, 2] logits
                // (take class-1). Derive width from the output shape.
                let width = (logits.len() / batch).max(1);
                let _ = out_shape;
                for (i, pair) in chunk.iter().enumerate() {
                    let score = if width >= 2 {
                        logits[i * width + 1]
                    } else {
                        logits[i * width]
                    };
                    out.push(Scored {
                        doc_key: pair.doc_key,
                        ce_score: score,
                    });
                }
            }
            out.sort_unstable_by(|a, b| b.ce_score.partial_cmp(&a.ce_score).unwrap());
            Ok(out)
        }

        fn encode_batch(&self, query: &str, pairs: &[Pair]) -> Result<EncodedBatch, RerankError> {
            let mut ids = Vec::with_capacity(pairs.len() * SEQ_LEN);
            let mut mask = Vec::with_capacity(pairs.len() * SEQ_LEN);
            let mut types = Vec::with_capacity(pairs.len() * SEQ_LEN);
            for pair in pairs {
                let passage = format!("{} {}", pair.title, pair.snippet);
                let enc = self
                    .tokenizer
                    .encode((query, passage.as_str()), true)
                    .map_err(|e| RerankError::Inference(e.to_string()))?;
                let take = |v: &[u32]| -> Vec<i64> {
                    let mut out: Vec<i64> = v.iter().take(SEQ_LEN).map(|&x| x as i64).collect();
                    out.resize(SEQ_LEN, 0);
                    out
                };
                ids.extend(take(enc.get_ids()));
                mask.extend(take(enc.get_attention_mask()));
                types.extend(take(enc.get_type_ids()));
            }
            Ok((ids, mask, types))
        }
    }
}

#[cfg(all(test, feature = "ort-backend"))]
mod ort_tests {
    use super::*;

    #[test]
    fn loads_and_scores_the_int8_ce() {
        let models = std::path::Path::new("../../models");
        let Ok(r) = Reranker::load(models) else {
            eprintln!("model not present; skipping on-device CE smoke");
            return;
        };
        let pairs = vec![
            Pair {
                doc_key: 1,
                title: "Rust borrow checker".into(),
                snippet: "ownership and borrowing prevent data races at compile time".into(),
            },
            Pair {
                doc_key: 2,
                title: "Gardening basics".into(),
                snippet: "how to plant tomatoes in a raised bed".into(),
            },
        ];
        let scored = r
            .rerank(
                "how does the rust borrow checker work",
                &pairs,
                std::time::Duration::from_secs(5),
            )
            .expect("rerank");
        assert_eq!(scored.len(), 2);
        // The relevant doc must outscore the irrelevant one.
        assert_eq!(
            scored[0].doc_key, 1,
            "CE should rank the rust doc first: {scored:?}"
        );
    }
}
