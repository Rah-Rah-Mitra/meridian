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

/// Spearman rank correlation (average ranks for ties). NaN when either side
/// is constant — a constant predictor predicts nothing.
pub fn spearman(xs: &[f64], ys: &[f64]) -> f64 {
    assert_eq!(xs.len(), ys.len());
    let n = xs.len();
    if n < 3 {
        return f64::NAN;
    }
    let rx = average_ranks(xs);
    let ry = average_ranks(ys);
    let mx = rx.iter().sum::<f64>() / n as f64;
    let my = ry.iter().sum::<f64>() / n as f64;
    let mut num = 0.0;
    let mut dx = 0.0;
    let mut dy = 0.0;
    for i in 0..n {
        let (a, b) = (rx[i] - mx, ry[i] - my);
        num += a * b;
        dx += a * a;
        dy += b * b;
    }
    if dx <= 0.0 || dy <= 0.0 {
        return f64::NAN;
    }
    num / (dx.sqrt() * dy.sqrt())
}

fn average_ranks(xs: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..xs.len()).collect();
    order.sort_by(|&a, &b| xs[a].partial_cmp(&xs[b]).unwrap());
    let mut ranks = vec![0.0; xs.len()];
    let mut i = 0;
    while i < order.len() {
        let mut j = i;
        while j + 1 < order.len() && xs[order[j + 1]] == xs[order[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for &idx in &order[i..=j] {
            ranks[idx] = avg;
        }
        i = j + 1;
    }
    ranks
}

/// Expected calibration error of a [0,1] predictor against a [0,1] outcome:
/// 10 equal-width predictor bins, Σ wᵢ·|mean predictorᵢ − mean outcomeᵢ|.
/// For QPP this treats per-query nDCG as the outcome the score claims to
/// track — a calibration DIAGNOSTIC, not a probability statement (ADR-23).
pub fn ece_10(predictor: &[f64], outcome: &[f64]) -> f64 {
    assert_eq!(predictor.len(), outcome.len());
    let n = predictor.len();
    if n == 0 {
        return f64::NAN;
    }
    let mut sums = [(0.0f64, 0.0f64, 0usize); 10];
    for i in 0..n {
        let b = ((predictor[i] * 10.0) as usize).min(9);
        sums[b].0 += predictor[i];
        sums[b].1 += outcome[i];
        sums[b].2 += 1;
    }
    sums.iter()
        .filter(|(_, _, c)| *c > 0)
        .map(|(p, o, c)| {
            let cf = *c as f64;
            (cf / n as f64) * ((p / cf) - (o / cf)).abs()
        })
        .sum()
}

#[cfg(test)]
mod qpp_metric_tests {
    use super::*;

    #[test]
    fn spearman_signs_and_ties() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0];
        let y = [2.0, 4.0, 6.0, 8.0, 10.0];
        assert!(
            (spearman(&x, &y) - 1.0).abs() < 1e-12,
            "perfect monotone = 1"
        );
        let yr: Vec<f64> = y.iter().rev().copied().collect();
        assert!((spearman(&x, &yr) + 1.0).abs() < 1e-12, "reversed = -1");
        let constant = [3.0; 5];
        assert!(spearman(&x, &constant).is_nan(), "constant predictor = NaN");
        let tied = [1.0, 1.0, 2.0, 2.0, 3.0];
        let r = spearman(&tied, &x);
        assert!(r > 0.9, "ties handled via average ranks: {r}");
    }

    #[test]
    fn ece_perfect_and_offset() {
        let p = [0.05, 0.15, 0.25, 0.35, 0.45, 0.55, 0.65, 0.75, 0.85, 0.95];
        assert!(ece_10(&p, &p) < 1e-12, "self-calibrated = 0");
        let shifted: Vec<f64> = p.iter().map(|v| (v + 0.2).min(1.0)).collect();
        let e = ece_10(&shifted, &p);
        assert!(e > 0.15 && e <= 0.21, "uniform +0.2 offset ≈ 0.2 ECE: {e}");
    }
}
