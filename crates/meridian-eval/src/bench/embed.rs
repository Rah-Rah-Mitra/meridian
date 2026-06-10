//! Suite 1 — model2vec embedding throughput (SPEC §15.1).
//! Gate: >2k docs/s (batch-32 figure is the headline; 1/32/256 all reported).

use super::{BenchConfig, SuiteResult};
use crate::stats::Rng;
use model2vec_rs::model::StaticModel;
use std::path::Path;
use std::time::Instant;

const SENTENCES: usize = 10_000;

pub(crate) fn load(models_dir: &Path) -> Result<StaticModel, String> {
    let path = models_dir.join("potion-base-8M");
    if !path.join("model.safetensors").exists() {
        return Err(format!(
            "model not found at {} (run deploy/fetch-models.sh)",
            path.display()
        ));
    }
    StaticModel::from_pretrained(path.to_string_lossy().as_ref(), None, None, None)
        .map_err(|e| format!("model load failed: {e}"))
}

pub fn run(cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("embed");
    let start = Instant::now();

    let model = match load(&cfg.models_dir) {
        Ok(m) => m,
        Err(e) => {
            result.note(e);
            result.gate("embed throughput > 2000 docs/s", false);
            return result;
        }
    };

    let mut rng = Rng::new(0xE3BED);
    let sentences = super::synthetic_sentences(SENTENCES, &mut rng);

    // Warm-up (tokenizer caches, allocator).
    let warm = model.encode(&sentences[..256.min(sentences.len())]);
    let dims = warm.first().map(Vec::len).unwrap_or(0);
    result.metric("dimensions", dims);

    let mut headline = 0.0;
    for batch in [1usize, 32, 256] {
        let t = Instant::now();
        let mut produced = 0usize;
        for chunk in sentences.chunks(batch) {
            produced += model.encode(chunk).len();
        }
        let secs = t.elapsed().as_secs_f64();
        let rate = produced as f64 / secs;
        result.metric(&format!("docs_per_sec_batch_{batch}"), rate.round());
        if batch == 32 {
            headline = rate;
        }
    }

    result.gate(
        "embed throughput > 2000 docs/s (batch 32)",
        headline > 2000.0,
    );
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}
