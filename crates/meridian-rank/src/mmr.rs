//! MMR diversity rerank (Phase 8, WBS 8.7): `diversity=mmr`, OFF by default.
//!
//! Maximal Marginal Relevance over the final ranked list: each next slot picks
//! the candidate maximizing λ·relevance − (1−λ)·max-similarity-to-selected.
//! Similarity here is term-set Jaccard over title+snippet — a deliberate
//! substitution for the embedding similarity the WBS sketched: web results
//! carry no stored vectors (only ingested docs do), so token similarity is the
//! one signal available uniformly across lanes, and the suite-gate target
//! (alpha-nDCG improvement on duplicate-heavy sets, ≤1% nDCG cost) is about
//! near-duplicate demotion, which token overlap captures directly. Recorded
//! as a deviation in the WBS; embedding MMR remains open if the suite says
//! token MMR underperforms.
//!
//! O(k²) over the RESPONSE list (≤50) — microseconds; no allocation beyond
//! the term sets.

use std::collections::HashSet;

/// Conventional balance: mostly relevance, enough redundancy penalty to
/// demote near-duplicates (Carbonell & Goldstein).
pub const MMR_LAMBDA: f32 = 0.7;

/// One rerank candidate: its relevance score and the text its similarity is
/// judged on.
pub struct MmrDoc<'a> {
    pub score: f32,
    pub text: &'a str,
}

/// Returns the MMR ordering as indices into `docs`. Scores are shifted
/// non-negative and divided by the max so λ means the same thing regardless
/// of the score scale that reached this stage (LTR or CE). Deliberately NOT
/// min-max: the response head's score band is compressed, and stretching it
/// to [0,1] would make near-equal relevances look decisive — the redundancy
/// penalty could then never demote a near-duplicate.
pub fn mmr_order(docs: &[MmrDoc<'_>], lambda: f32) -> Vec<usize> {
    let n = docs.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let min = docs.iter().fold(f32::MAX, |lo, d| lo.min(d.score)).min(0.0);
    let max = docs.iter().fold(f32::MIN, |hi, d| hi.max(d.score));
    let scale = (max - min).max(1e-6);
    let rel: Vec<f32> = docs.iter().map(|d| (d.score - min) / scale).collect();
    let terms: Vec<HashSet<&str>> = docs.iter().map(|d| term_set(d.text)).collect();

    let mut selected: Vec<usize> = Vec::with_capacity(n);
    let mut remaining: Vec<usize> = (0..n).collect();
    while !remaining.is_empty() {
        let (pos, &best) = remaining
            .iter()
            .enumerate()
            .max_by(|&(_, &a), &(_, &b)| {
                let ma = lambda * rel[a] - (1.0 - lambda) * max_sim(a, &selected, &terms);
                let mb = lambda * rel[b] - (1.0 - lambda) * max_sim(b, &selected, &terms);
                ma.partial_cmp(&mb)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    // Deterministic tie-break: keep the incoming order.
                    .then(b.cmp(&a))
            })
            .expect("remaining is non-empty");
        selected.push(best);
        remaining.remove(pos);
    }
    selected
}

fn max_sim(candidate: usize, selected: &[usize], terms: &[HashSet<&str>]) -> f32 {
    selected
        .iter()
        .map(|&s| jaccard(&terms[candidate], &terms[s]))
        .fold(0.0, f32::max)
}

fn jaccard(a: &HashSet<&str>, b: &HashSet<&str>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    inter / ((a.len() + b.len()) as f32 - inter)
}

fn term_set(text: &str) -> HashSet<&str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_duplicate_is_demoted_below_a_diverse_result() {
        let docs = vec![
            MmrDoc {
                score: 1.0,
                text: "desalination plant approval coastal council review",
            },
            MmrDoc {
                score: 0.95,
                text: "desalination plant approval coastal council review extra",
            },
            MmrDoc {
                score: 0.9,
                text: "completely different renewable wind farm subsidy story",
            },
        ];
        let order = mmr_order(&docs, MMR_LAMBDA);
        assert_eq!(order[0], 0, "top relevance leads");
        assert_eq!(
            order[1], 2,
            "diverse doc must outrank the near-duplicate: {order:?}"
        );
        assert_eq!(order[2], 1);
    }

    #[test]
    fn lambda_one_preserves_relevance_order() {
        let docs = vec![
            MmrDoc {
                score: 0.5,
                text: "same words here",
            },
            MmrDoc {
                score: 0.9,
                text: "same words here",
            },
            MmrDoc {
                score: 0.7,
                text: "same words here",
            },
        ];
        let order = mmr_order(&docs, 1.0);
        assert_eq!(order, vec![1, 2, 0]);
    }

    #[test]
    fn tiny_lists_pass_through() {
        let docs = vec![MmrDoc {
            score: 1.0,
            text: "a",
        }];
        assert_eq!(mmr_order(&docs, MMR_LAMBDA), vec![0]);
    }
}
