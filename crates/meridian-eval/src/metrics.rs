//! Relevance metrics (SPEC §15): nDCG@k, MRR@k, Recall@k over graded qrels.

use std::collections::HashMap;

/// Graded judgments for one query: doc-id (URL) → grade (0–3).
pub type Judgments = HashMap<String, u32>;

/// nDCG@k with the standard log2 discount and graded gains (2^g - 1).
pub fn ndcg_at(k: usize, ranked: &[String], judgments: &Judgments) -> f64 {
    let dcg: f64 = ranked
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, doc)| {
            let g = *judgments.get(doc).unwrap_or(&0) as f64;
            ((2f64.powf(g)) - 1.0) / ((i as f64 + 2.0).log2())
        })
        .sum();
    let mut ideal: Vec<u32> = judgments.values().copied().filter(|&g| g > 0).collect();
    ideal.sort_unstable_by(|a, b| b.cmp(a));
    let idcg: f64 = ideal
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &g)| ((2f64.powf(g as f64)) - 1.0) / ((i as f64 + 2.0).log2()))
        .sum();
    if idcg > 0.0 { dcg / idcg } else { 0.0 }
}

/// Reciprocal rank of the first relevant doc within k.
pub fn mrr_at(k: usize, ranked: &[String], judgments: &Judgments) -> f64 {
    ranked
        .iter()
        .take(k)
        .position(|doc| judgments.get(doc).is_some_and(|&g| g > 0))
        .map(|i| 1.0 / (i as f64 + 1.0))
        .unwrap_or(0.0)
}

/// Fraction of relevant docs retrieved within k.
pub fn recall_at(k: usize, ranked: &[String], judgments: &Judgments) -> f64 {
    let relevant: usize = judgments.values().filter(|&&g| g > 0).count();
    if relevant == 0 {
        return 0.0;
    }
    let hit = ranked
        .iter()
        .take(k)
        .filter(|doc| judgments.get(*doc).is_some_and(|&g| g > 0))
        .count();
    hit as f64 / relevant as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j(pairs: &[(&str, u32)]) -> Judgments {
        pairs.iter().map(|(d, g)| (d.to_string(), *g)).collect()
    }

    #[test]
    fn known_item_metrics() {
        let judgments = j(&[("target", 3)]);
        let first = vec!["target".to_string(), "x".to_string()];
        let third = vec!["a".to_string(), "b".to_string(), "target".to_string()];
        let absent = vec!["a".to_string(), "b".to_string()];

        assert!((ndcg_at(10, &first, &judgments) - 1.0).abs() < 1e-9);
        assert!(ndcg_at(10, &third, &judgments) < 1.0);
        assert!(ndcg_at(10, &third, &judgments) > 0.0);
        assert_eq!(ndcg_at(10, &absent, &judgments), 0.0);

        assert_eq!(mrr_at(10, &first, &judgments), 1.0);
        assert!((mrr_at(10, &third, &judgments) - (1.0 / 3.0)).abs() < 1e-9);
        assert_eq!(recall_at(2, &third, &judgments), 0.0);
        assert_eq!(recall_at(3, &third, &judgments), 1.0);
    }
}
