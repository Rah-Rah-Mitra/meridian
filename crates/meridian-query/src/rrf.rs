//! Reciprocal Rank Fusion (SPEC §11): `score(d) = Σ_i 1/(k + rank_i(d))` with k=60
//! (Cormack et al., SIGIR'09). Score-scale agnostic; no tuning.

/// The locked fusion constant (SPEC §3).
pub const RRF_K: u32 = 60;

/// Fuse ranked lists of document keys into a single ranking with RRF scores.
///
/// Each input list is ordered best-first; `rank` is the 0-based position (the
/// canonical formula's 1-based rank is `position + 1`). Returns `(key, score)`
/// pairs sorted by descending score, ties broken by key for determinism.
pub fn rrf_fuse(lists: &[Vec<u64>], k: u32) -> Vec<(u64, f32)> {
    let mut scores: std::collections::HashMap<u64, f32> = std::collections::HashMap::new();
    for list in lists {
        for (position, &key) in list.iter().enumerate() {
            let rank = position as u32 + 1;
            *scores.entry(key).or_insert(0.0) += 1.0 / (k + rank) as f32;
        }
    }
    let mut fused: Vec<(u64, f32)> = scores.into_iter().collect();
    fused.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
    fused
}

#[cfg(test)]
mod tests {
    use super::{RRF_K, rrf_fuse};

    #[test]
    fn doc_in_multiple_lists_outranks_single_list_winners() {
        // Doc 7 is mid-ranked in both lists; docs 1 and 2 top one list each.
        let bm25 = vec![1, 7, 3, 4];
        let ann = vec![2, 7, 5, 6];
        let fused = rrf_fuse(&[bm25, ann], RRF_K);
        assert_eq!(fused[0].0, 7, "consensus doc should win: {fused:?}");
        let expected = 1.0 / 62.0 + 1.0 / 62.0;
        assert!((fused[0].1 - expected).abs() < 1e-6);
    }

    #[test]
    fn deterministic_tie_break_and_empty_input() {
        assert!(rrf_fuse(&[], RRF_K).is_empty());
        // Two docs with identical contribution tie-break by key.
        let fused = rrf_fuse(&[vec![9], vec![4]], RRF_K);
        assert_eq!(fused.iter().map(|p| p.0).collect::<Vec<_>>(), vec![4, 9]);
    }
}
