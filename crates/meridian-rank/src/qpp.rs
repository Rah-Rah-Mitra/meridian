//! Query-performance prediction (Phase 8, ADR-23): post-retrieval confidence
//! signals — cheap (O(k·terms), ≤1ms budget), explainable, and honest about
//! what they are: PREDICTORS with modest correlation (suite-13 gate: Spearman
//! ρ ≥ 0.25 vs per-query nDCG@10), not probabilities.
//!
//! - **NQC** (normalized query commitment): dispersion of the top-k fused
//!   retrieval scores against the candidate-pool mean. A confident head
//!   separates from the pool; a flat score curve predicts a weak result set.
//! - **Clarity-lite**: KL divergence between the top-k snippets' term
//!   distribution and the full candidate pool's — a focused result set uses
//!   distinctive vocabulary. ("Lite": the reference model is the candidate
//!   pool, not the whole collection — no index pass, bounded cost; recorded
//!   in ADR-23.)
//!
//! Calibration of the fused score into user-facing bands is a v0.3.0-exit
//! deliverable (suite 13 accumulates the eval set); until then the block
//! carries the RAW signals so nothing pretends to a precision it lacks.

use std::collections::HashMap;

/// Response-level `confidence` block (additive, ADR-20).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfidenceBlock {
    pub schema: u8,
    /// Normalized query commitment over the fused (RRF) scores.
    pub nqc: f32,
    /// Clarity-lite: KL(top-k ‖ pool) over snippet terms, nats, ≥ 0.
    pub clarity: f32,
    /// Uncalibrated blend in [0,1] — ranking-comparable across queries on one
    /// deployment, NOT a probability (calibration lands with suite 13).
    pub score: f32,
}

/// Compute the block from the fused candidate pool. `top_k` counts results
/// (the response head); `pool_*` cover the full scored candidate set.
/// Returns None when there is nothing to predict from (empty pool).
pub fn confidence(
    top_scores: &[f32],
    top_snippets: &[&str],
    pool_scores: &[f32],
    pool_snippets: &[&str],
) -> Option<ConfidenceBlock> {
    if top_scores.is_empty() || pool_scores.is_empty() {
        return None;
    }
    let nqc = nqc(top_scores, pool_scores);
    let clarity = clarity_lite(top_snippets, pool_snippets);
    // Squash each signal to [0,1] with fixed soft scales (NOT calibration —
    // just bounded blending; suite 13 owns the real mapping).
    let nqc_01 = 1.0 - (-nqc * 2.0).exp();
    let clarity_01 = 1.0 - (-clarity * 0.7).exp();
    Some(ConfidenceBlock {
        schema: 1,
        nqc,
        clarity,
        score: (0.5 * nqc_01 + 0.5 * clarity_01).clamp(0.0, 1.0),
    })
}

/// σ(top-k scores) / max(|pool mean|, ε) — the classic NQC shape over the
/// fused score list.
fn nqc(top_scores: &[f32], pool_scores: &[f32]) -> f32 {
    let k = top_scores.len() as f32;
    let mean_top = top_scores.iter().sum::<f32>() / k;
    let var = top_scores
        .iter()
        .map(|s| (s - mean_top).powi(2))
        .sum::<f32>()
        / k;
    let pool_mean = pool_scores.iter().sum::<f32>() / pool_scores.len() as f32;
    var.sqrt() / pool_mean.abs().max(1e-6)
}

/// KL(P_topk ‖ P_pool) over lowercase alphanumeric snippet terms, with add-ε
/// smoothing on the pool side. 0 when top-k vocabulary mirrors the pool.
fn clarity_lite(top_snippets: &[&str], pool_snippets: &[&str]) -> f32 {
    let top = term_distribution(top_snippets);
    let pool = term_distribution(pool_snippets);
    if top.1 == 0.0 || pool.1 == 0.0 {
        return 0.0;
    }
    let vocab = pool.0.len() as f32;
    let mut kl = 0.0f32;
    for (term, &tf) in &top.0 {
        let p = tf / top.1;
        // Add-one smoothing keeps unseen-in-pool terms finite (they are the
        // most clarity-bearing signal, not an error).
        let q = (pool.0.get(term).copied().unwrap_or(0.0) + 1.0) / (pool.1 + vocab.max(1.0));
        kl += p * (p / q).ln();
    }
    kl.max(0.0)
}

fn term_distribution(snippets: &[&str]) -> (HashMap<String, f32>, f32) {
    let mut counts: HashMap<String, f32> = HashMap::new();
    let mut total = 0.0f32;
    for s in snippets {
        for term in s
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() > 2)
        {
            *counts.entry(term.to_lowercase()).or_insert(0.0) += 1.0;
            total += 1.0;
        }
    }
    (counts, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_head_scores_higher_than_flat() {
        // Confident: head separates from pool, distinctive vocabulary.
        let focused = confidence(
            &[0.9, 0.85, 0.2, 0.1],
            &[
                "desalination membrane osmosis plant intake",
                "desalination brine outfall permit megalitre",
            ],
            &[0.9, 0.85, 0.2, 0.1, 0.05, 0.04, 0.03, 0.02],
            &[
                "desalination membrane osmosis plant intake",
                "desalination brine outfall permit megalitre",
                "generic words about many different topics here",
                "another unrelated snippet with common words",
                "more filler text covering separate subjects",
                "completely different theme entirely",
                "random web result about something else",
                "yet another broad snippet",
            ],
        )
        .unwrap();

        // Weak: flat scores, head vocabulary = pool vocabulary.
        let pool: Vec<&str> = vec!["the same generic words repeated everywhere"; 8];
        let flat = confidence(
            &[0.31, 0.30, 0.30, 0.29],
            &pool[..2],
            &[0.31, 0.30, 0.30, 0.29, 0.29, 0.28, 0.28, 0.28],
            &pool,
        )
        .unwrap();

        assert!(
            focused.score > flat.score,
            "focused {} must beat flat {}",
            focused.score,
            flat.score
        );
        assert!(focused.nqc > flat.nqc);
        assert!(focused.clarity > flat.clarity);
        assert!((0.0..=1.0).contains(&focused.score));
    }

    #[test]
    fn empty_inputs_yield_none() {
        assert!(confidence(&[], &[], &[], &[]).is_none());
    }
}
