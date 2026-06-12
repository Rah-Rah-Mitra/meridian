//! Evidence-cluster diversity (the MMR replacement, carried from the P8
//! withdrawal). MMR died by its own gate because symmetric token similarity
//! demotes a cluster's CANONICAL original exactly like its syndicated copies
//! (suite 13b: alpha-nDCG gains only at 7–11% plain-nDCG cost). The first
//! cut of THIS diversifier repeated the trap from the other side ("the
//! best-ranked member keeps the slot"): truncated copies outscore their
//! originals in BM25, so the kept representative was usually a copy and the
//! original sank (dup harness: nDCG −13%). The shipped rule uses what the
//! engine actually KNOWS: the ADR-18 cluster's canonical member — the
//! superset document the copies derive from (most shingles).
//!
//! Rule — stable, knobless: walk the relevance-ranked head in order. At each
//! cluster's FIRST appearance, emit the cluster's CANONICAL member in that
//! slot (promoting the original over whichever copy outscored it); every
//! other member of the cluster defers to the back, preserving relative
//! order. Results without a cluster (web results, pre-v0.2.0 docs) are
//! singletons and never move — independence is never guessed.
//!
//! O(k) over the response head; no allocation beyond the output order.

use std::collections::{HashMap, HashSet};

/// Per-result cluster input: (cluster id, is the cluster's canonical member).
/// None = unsketched.
pub type ClusterOf = Option<(u32, bool)>;

/// Returns the diversified ordering as indices into `clusters`.
pub fn cluster_diversify(clusters: &[ClusterOf]) -> Vec<usize> {
    // The canonical member of each cluster (fallback: its first member —
    // possible when the head was truncated below the canonical's rank).
    let mut canonical_of: HashMap<u32, usize> = HashMap::new();
    for (i, c) in clusters.iter().enumerate() {
        if let Some((id, canonical)) = c {
            canonical_of
                .entry(*id)
                .and_modify(|slot| {
                    if *canonical {
                        *slot = i;
                    }
                })
                .or_insert(i);
        }
    }

    let mut emitted_clusters: HashSet<u32> = HashSet::new();
    let mut head: Vec<usize> = Vec::with_capacity(clusters.len());
    let mut deferred: Vec<usize> = Vec::new();
    for (i, c) in clusters.iter().enumerate() {
        match c {
            None => head.push(i),
            Some((id, _)) => {
                if emitted_clusters.insert(*id) {
                    // First appearance: the canonical takes this slot.
                    head.push(canonical_of[id]);
                }
                // Non-canonical members defer (the canonical was emitted at
                // the cluster's first slot, possibly above its own rank).
                if i != canonical_of[id] {
                    deferred.push(i);
                }
            }
        }
    }
    head.extend(deferred);
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_takes_the_cluster_slot_copies_sink() {
        // ranks: copy(c0) ORIG(c0) other(c1 canon) none ORIG-copy2(c0)
        // The cluster-0 slot (rank 0) goes to the canonical at rank 1.
        let clusters = [
            Some((0, false)),
            Some((0, true)),
            Some((1, true)),
            None,
            Some((0, false)),
        ];
        assert_eq!(cluster_diversify(&clusters), vec![1, 2, 3, 0, 4]);
    }

    #[test]
    fn canonical_already_first_stays_put() {
        let clusters = [Some((0, true)), Some((0, false)), Some((1, true)), None];
        assert_eq!(cluster_diversify(&clusters), vec![0, 2, 3, 1]);
    }

    #[test]
    fn no_clusters_means_no_movement() {
        assert_eq!(cluster_diversify(&[None, None, None]), vec![0, 1, 2]);
        let singles = [Some((0, true)), Some((1, true)), Some((2, true))];
        assert_eq!(cluster_diversify(&singles), vec![0, 1, 2]);
    }

    #[test]
    fn truncated_head_without_canonical_falls_back_to_first_member() {
        // Cluster 0 has no canonical in the head (it ranked below the cut):
        // its first member represents it rather than losing the cluster.
        let clusters = [Some((0, false)), Some((0, false)), None];
        assert_eq!(cluster_diversify(&clusters), vec![0, 2, 1]);
    }

    #[test]
    fn empty_input() {
        assert!(cluster_diversify(&[]).is_empty());
    }
}
