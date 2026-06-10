//! Suite 4 — INT8 cross-encoder ms/pair (SPEC §15.4), run via tract (pure Rust —
//! the ADR-02 Plan-A runtime). Informational: the measurement sets the default
//! rerank depth/batch and feeds the Phase-3 tract-vs-ort decision rule; a model
//! that fails to load is itself ADR-02 data and is reported, not hidden.

use super::{BenchConfig, SuiteResult};
use crate::stats::Rng;
use std::time::Instant;
use tokenizers::Tokenizer;
use tract_onnx::prelude::*;

const SEQ_LEN: usize = 256;
const PAIRS_PER_BATCH_RUN: usize = 20;

type CePlan = Arc<RunnableModel<TypedFact, Box<dyn TypedOp>>>;

fn load_model(path: &std::path::Path, batch: usize) -> TractResult<(CePlan, usize)> {
    let fact = InferenceFact::dt_shape(i64::datum_type(), tvec!(batch, SEQ_LEN));
    let mut model = tract_onnx::onnx().model_for_path(path)?;
    let n_inputs = model.inputs.len();
    for i in 0..n_inputs {
        model = model.with_input_fact(i, fact.clone())?;
    }
    let plan = model.into_optimized()?.into_runnable()?;
    Ok((plan, n_inputs))
}

fn make_inputs(
    tokenizer: &Tokenizer,
    n_inputs: usize,
    batch: usize,
    rng: &mut Rng,
) -> Result<TVec<TValue>, String> {
    let queries = super::synthetic_sentences(batch, rng);
    let docs: Vec<String> = super::synthetic_sentences(batch, rng)
        .into_iter()
        .map(|s| format!("{s} {s} {s}"))
        .collect();

    let mut ids = Vec::with_capacity(batch * SEQ_LEN);
    let mut mask = Vec::with_capacity(batch * SEQ_LEN);
    let mut type_ids = Vec::with_capacity(batch * SEQ_LEN);
    for i in 0..batch {
        let enc = tokenizer
            .encode((queries[i].as_str(), docs[i].as_str()), true)
            .map_err(|e| format!("tokenize: {e}"))?;
        let take = |v: &[u32]| -> Vec<i64> {
            let mut out: Vec<i64> = v.iter().take(SEQ_LEN).map(|&x| x as i64).collect();
            out.resize(SEQ_LEN, 0);
            out
        };
        ids.extend(take(enc.get_ids()));
        mask.extend(take(enc.get_attention_mask()));
        type_ids.extend(take(enc.get_type_ids()));
    }
    let to_tvalue = |data: Vec<i64>| -> Result<TValue, String> {
        Tensor::from_shape(&[batch, SEQ_LEN], &data)
            .map(|t| t.into())
            .map_err(|e| format!("tensor: {e}"))
    };
    // Standard BERT input order: input_ids, attention_mask, token_type_ids.
    // Models exported without token_type_ids just take the first two.
    let all = [to_tvalue(ids)?, to_tvalue(mask)?, to_tvalue(type_ids)?];
    Ok(all.into_iter().take(n_inputs).collect())
}

pub fn run(cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("rerank");
    let start = Instant::now();

    let model_path = cfg
        .models_dir
        .join("ms-marco-minilm-l6-v2/model_qint8_arm64.onnx");
    let tok_path = cfg.models_dir.join("ms-marco-minilm-l6-v2/tokenizer.json");
    if !model_path.exists() || !tok_path.exists() {
        result.note("CE model/tokenizer missing (run deploy/fetch-models.sh)");
        return result;
    }
    let tokenizer = match Tokenizer::from_file(&tok_path) {
        Ok(t) => t,
        Err(e) => {
            result.note(format!("tokenizer load failed: {e}"));
            return result;
        }
    };
    let mut rng = Rng::new(0x4E4A);

    for batch in [1usize, 4, 8] {
        let (plan, n_inputs) = match load_model(&model_path, batch) {
            Ok(p) => p,
            Err(e) => {
                // A tract op-coverage gap on this INT8 graph is ADR-02 evidence.
                result.note(format!(
                    "tract failed to load/optimize INT8 CE at batch {batch}: {e}"
                ));
                continue;
            }
        };
        let inputs = match make_inputs(&tokenizer, n_inputs, batch, &mut rng) {
            Ok(i) => i,
            Err(e) => {
                result.note(e);
                continue;
            }
        };
        // Warm-up run (also sanity-checks the output is readable f32), then timed runs.
        match plan.run(inputs.clone()) {
            Ok(outputs) => {
                if let Ok(view) = outputs[0].to_plain_array_view::<f32>() {
                    std::hint::black_box(view.iter().next().copied());
                }
            }
            Err(e) => {
                result.note(format!("tract run failed at batch {batch}: {e}"));
                continue;
            }
        }
        let runs = PAIRS_PER_BATCH_RUN.div_ceil(batch);
        let t = Instant::now();
        for _ in 0..runs {
            if plan.run(inputs.clone()).is_err() {
                break;
            }
        }
        let total_ms = t.elapsed().as_secs_f64() * 1e3;
        let ms_per_pair = total_ms / (runs * batch) as f64;
        result.metric(
            &format!("ms_per_pair_batch_{batch}"),
            (ms_per_pair * 10.0).round() / 10.0,
        );
        if batch == 4 {
            let top20_ms = ms_per_pair * 20.0;
            result.metric("projected_top20_ms_batch_4", top20_ms.round());
            result.note(format!(
                "deep-mode stage budget is 1500ms — top-20 at batch 4 projects to {top20_ms:.0}ms"
            ));
        }
    }

    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}
