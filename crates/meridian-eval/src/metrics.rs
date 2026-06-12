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

/// alpha-nDCG@k (Clarke et al., SIGIR 2008) over subtopic judgments:
/// `judgments[doc] = (grade, subtopic)`. The gain of a doc covering subtopic a
/// at rank i is grade·(1−α)^(prior hits on a) — redundant coverage decays.
/// α = 0.5 (conventional). The ideal ranking is computed GREEDILY (the exact
/// ideal gain vector is NP-complete; greedy is the standard approximation and
/// is exact for the small per-query sets used here — recorded in
/// 04-bench-plan §6).
pub fn alpha_ndcg_at(
    k: usize,
    ranked: &[String],
    judgments: &std::collections::HashMap<String, (u32, u32)>,
) -> f64 {
    const ALPHA: f64 = 0.5;
    fn dcg(gains: &[f64]) -> f64 {
        gains
            .iter()
            .enumerate()
            .map(|(i, g)| g / ((i + 2) as f64).log2())
            .sum()
    }
    let mut seen: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let gains: Vec<f64> = ranked
        .iter()
        .take(k)
        .map(|doc| match judgments.get(doc) {
            Some(&(grade, sub)) if grade > 0 => {
                let prior = seen.entry(sub).or_insert(0);
                let gain = f64::from(grade) * (1.0 - ALPHA).powi(*prior as i32);
                *prior += 1;
                gain
            }
            _ => 0.0,
        })
        .collect();

    // Greedy ideal: repeatedly take the doc with the highest marginal gain.
    let mut pool: Vec<(u32, u32)> = judgments
        .values()
        .copied()
        .filter(|(g, _)| *g > 0)
        .collect();
    let mut ideal_seen: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut ideal_gains: Vec<f64> = Vec::new();
    for _ in 0..k.min(pool.len()) {
        let (best_idx, best_gain) = pool
            .iter()
            .enumerate()
            .map(|(i, &(g, sub))| {
                let prior = ideal_seen.get(&sub).copied().unwrap_or(0);
                (i, f64::from(g) * (1.0 - ALPHA).powi(prior as i32))
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap();
        let (_, sub) = pool.swap_remove(best_idx);
        *ideal_seen.entry(sub).or_insert(0) += 1;
        ideal_gains.push(best_gain);
    }
    let ideal = dcg(&ideal_gains);
    if ideal <= 0.0 {
        return 0.0;
    }
    (dcg(&gains) / ideal).clamp(0.0, 1.0)
}

#[cfg(test)]
mod alpha_ndcg_tests {
    use super::*;
    use std::collections::HashMap;

    fn sj(items: &[(&str, u32, u32)]) -> HashMap<String, (u32, u32)> {
        items
            .iter()
            .map(|(d, g, s)| (d.to_string(), (*g, *s)))
            .collect()
    }

    #[test]
    fn diverse_beats_redundant() {
        // Three subtopics, one doc each + a duplicate of subtopic 0.
        let j = sj(&[("a", 3, 0), ("a2", 3, 0), ("b", 3, 1), ("c", 3, 2)]);
        let diverse = vec!["a".into(), "b".into(), "c".into(), "a2".into()];
        let redundant = vec!["a".into(), "a2".into(), "b".into(), "c".into()];
        let d = alpha_ndcg_at(4, &diverse, &j);
        let r = alpha_ndcg_at(4, &redundant, &j);
        assert!(d > r, "diverse {d} must beat redundant {r}");
        assert!(
            (d - 1.0).abs() < 1e-9,
            "diverse-first IS the greedy ideal: {d}"
        );
    }

    #[test]
    fn irrelevant_and_empty() {
        let j = sj(&[("a", 3, 0)]);
        assert_eq!(alpha_ndcg_at(10, &["x".into(), "y".into()], &j), 0.0);
        let empty: HashMap<String, (u32, u32)> = HashMap::new();
        assert_eq!(alpha_ndcg_at(10, &["x".into()], &empty), 0.0);
    }
}
