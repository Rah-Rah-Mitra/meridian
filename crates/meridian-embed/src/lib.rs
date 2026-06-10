//! Dense embeddings (SPEC §3): model2vec static embeddings (`potion-base-8M`,
//! 256-dim) — pure Rust (safetensors + tokenizers + ndarray), no ONNX. Measured
//! on-device: 42.9k docs/s at batch 32 (bench 2026-06-10) — never the ingest
//! bottleneck.

use model2vec_rs::model::StaticModel;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
#[error("embedder: {0}")]
pub struct EmbedError(String);

enum Backend {
    Model(Box<StaticModel>),
    /// Hermetic test stub: deterministic hash-derived unit vectors. Never
    /// constructed by production code paths.
    TestStub,
}

pub struct Embedder {
    backend: Backend,
    dims: usize,
}

impl Embedder {
    /// Load `potion-base-8M` from `<models_dir>/potion-base-8M` (baked into the
    /// image; the device never downloads models — SPEC §9.6).
    pub fn load(models_dir: &Path) -> Result<Self, EmbedError> {
        let path = models_dir.join("potion-base-8M");
        if !path.join("model.safetensors").exists() {
            return Err(EmbedError(format!(
                "model not found at {} (image build runs deploy/fetch-models.sh)",
                path.display()
            )));
        }
        let model = StaticModel::from_pretrained(path.to_string_lossy().as_ref(), None, None, None)
            .map_err(|e| EmbedError(e.to_string()))?;
        let dims = model.encode_single("probe").len();
        if dims == 0 {
            return Err(EmbedError("model produced empty embedding".to_owned()));
        }
        Ok(Self {
            backend: Backend::Model(Box::new(model)),
            dims,
        })
    }

    /// Deterministic stub for tests in dependent crates (no model files in CI).
    #[doc(hidden)]
    pub fn test_stub(dims: usize) -> Self {
        Self {
            backend: Backend::TestStub,
            dims,
        }
    }

    pub fn dims(&self) -> usize {
        self.dims
    }

    /// Batch embed, L2-normalized (cosine via dot — SPEC §11). CPU-bound:
    /// callers on the async path go through the rayon bridge.
    pub fn embed_batch(&self, texts: &[String]) -> Vec<Vec<f32>> {
        match &self.backend {
            Backend::Model(model) => {
                let mut out = model.encode_with_args(texts, Some(512), 256);
                for v in &mut out {
                    l2_normalize(v);
                }
                out
            }
            Backend::TestStub => texts.iter().map(|t| self.stub_vector(t)).collect(),
        }
    }

    pub fn embed_query(&self, q: &str) -> Vec<f32> {
        match &self.backend {
            Backend::Model(model) => {
                let mut v = model.encode_single(q);
                l2_normalize(&mut v);
                v
            }
            Backend::TestStub => self.stub_vector(q),
        }
    }

    fn stub_vector(&self, text: &str) -> Vec<f32> {
        // Token-bag hashing: shared words → similar vectors, so relevance-ish
        // ordering survives in tests.
        let mut v = vec![0.0f32; self.dims];
        for word in text.split_whitespace() {
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            for b in word.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x1000_0000_01b3);
            }
            v[(h as usize) % self.dims] += 1.0;
        }
        l2_normalize(&mut v);
        v
    }
}

fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

#[cfg(test)]
mod tests {
    use super::l2_normalize;

    #[test]
    fn normalize_produces_unit_vectors_and_tolerates_zero() {
        let mut v = vec![3.0, 4.0];
        l2_normalize(&mut v);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6);

        let mut z = vec![0.0, 0.0];
        l2_normalize(&mut z); // must not produce NaN
        assert!(z.iter().all(|x| x.is_finite()));
    }
}
