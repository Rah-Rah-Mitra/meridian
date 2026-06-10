//! Nightly PageRank over the domain co-occurrence graph → `domain_prior`
//! (SPEC §9.4, §11). ADR amendment: GDELT carries no hyperlink graph, so the
//! substrate is domain co-occurrence in (slice, event-root) buckets — domains
//! that repeatedly report the same kinds of events together rank together.
//! Bounded, recomputed from scratch nightly, and observable via /metrics.

use petgraph::graph::UnGraph;
use std::collections::HashMap;

/// Edge weights below this are noise (a single co-occurrence ever).
const MIN_WEIGHT: u32 = 2;
/// PageRank damping (the standard value) and iteration budget.
const DAMPING: f32 = 0.85;
const ITERATIONS: usize = 30;

/// (domain_hash, prior in [0,1]) — max-normalized PageRank.
pub fn compute_priors(edges: &[(u64, u64, u32)]) -> Vec<(u64, f32)> {
    let mut nodes: HashMap<u64, petgraph::graph::NodeIndex> = HashMap::new();
    let mut graph: UnGraph<u64, f32> = UnGraph::new_undirected();
    for &(a, b, w) in edges {
        if w < MIN_WEIGHT || a == b {
            continue;
        }
        let na = *nodes.entry(a).or_insert_with(|| graph.add_node(a));
        let nb = *nodes.entry(b).or_insert_with(|| graph.add_node(b));
        graph.add_edge(na, nb, w as f32);
    }
    if graph.node_count() == 0 {
        return Vec::new();
    }
    let ranks = petgraph::algo::page_rank(&graph, DAMPING, ITERATIONS);
    let max = ranks.iter().copied().fold(f32::MIN, f32::max).max(1e-12);
    graph
        .node_indices()
        .map(|ix| (graph[ix], ranks[ix.index()] / max))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_outranks_leaves_and_noise_is_dropped() {
        // Star graph: hub (1) connects to 2,3,4; plus a weight-1 noise edge.
        let edges = vec![
            (1u64, 2u64, 5u32),
            (1, 3, 5),
            (1, 4, 5),
            (2, 3, 2),
            (8, 9, 1), // below MIN_WEIGHT — never enters the graph
        ];
        let priors: HashMap<u64, f32> = compute_priors(&edges).into_iter().collect();
        assert!(priors.contains_key(&1) && priors.contains_key(&4));
        assert!(!priors.contains_key(&8), "noise edge dropped");
        assert!(priors[&1] > priors[&4], "hub must outrank leaf: {priors:?}");
        assert_eq!(priors[&1], 1.0, "max-normalized");
        for p in priors.values() {
            assert!((0.0..=1.0).contains(p));
        }
    }

    #[test]
    fn empty_graph_is_fine() {
        assert!(compute_priors(&[]).is_empty());
        assert!(compute_priors(&[(1, 2, 1)]).is_empty(), "all noise");
    }
}
