//! Evidence layer (Phase 7, ADR-18): derivation clusters over a result set.
//!
//! Results whose documents carry an ingest-time sketch are clustered by
//! pairwise containment (union-find over edges ≥ τ); each cluster is one
//! apparent ORIGIN. `independent_source_count` counts those clusters — the
//! suite-9 experiment showed domain dedup is structurally blind to
//! cross-domain syndication (baseline F1 0.054), which is exactly what this
//! layer surfaces.
//!
//! Honesty contract (ADR-18): results WITHOUT a sketch (web results never
//! fetched/ingested, docs from a pre-v0.2.0 volume) get `evidence: null` and
//! are excluded from the independence count — a snippet is too little text to
//! assert independence on. The block reports how many results were sketched so
//! the count's basis is visible.

use crate::planner::SearchResult;
use meridian_index::lexical::url_key;
use meridian_index::sketch::Sketch;
use std::collections::HashMap;

/// Per-result evidence annotation (additive, ADR-20).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResultEvidence {
    /// Derivation-cluster id, dense within this response.
    pub cluster: u32,
    /// This member has the most shingles in its cluster — the superset the
    /// other members derive from (v0.6.0; drives `diversity=evidence`).
    pub canonical: bool,
}

/// Response-level `evidence` block (additive, ADR-20; `schema` versions the
/// block shape independently of the API).
#[derive(Debug, Clone, serde::Serialize)]
pub struct EvidenceBlock {
    pub schema: u8,
    /// Number of derivation clusters among sketched results — apparent origins.
    pub independent_source_count: u32,
    /// Total results in the response (the naive source count).
    pub apparent_source_count: u32,
    /// How many results carried a sketch (the basis of the independence count).
    pub sketched_results: u32,
    pub clusters: Vec<ClusterSummary>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ClusterSummary {
    pub id: u32,
    pub members: u32,
    /// Distinct result domains inside the cluster — members > domains means
    /// cross-domain syndication, the case domain dedup cannot see.
    pub domains: u32,
}

/// Per-result cluster tag: dense id + whether this member is the cluster's
/// CANONICAL document — the one with the most shingles, i.e. the superset
/// the others derive from. Truncated/syndicated copies carry strictly fewer
/// shingles than their original, which is exactly why "best-ranked member"
/// is the wrong representative: shorter copies outscore originals in BM25
/// (the suite-13b MMR post-mortem), but they cannot out-SHINGLE them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClusterTag {
    pub id: u32,
    pub canonical: bool,
}

/// Per-key cluster tags by sketch containment, dense ids in first-appearance
/// order; None for keys without a sketch. The union-find core shared by
/// `annotate` (the response block) and the dup-eval diversity harness
/// (the `diversity=evidence` gate). O(s²) pairwise over sketched keys —
/// bounded by the response limit (≤50), microseconds in practice (suite 11).
/// `tau` is the containment merge threshold (config `evidence.containment_tau`,
/// default `sketch::CONTAINMENT_TAU`); callers pass the deployment/per-request
/// value so the threshold is never silently hard-coded here.
pub fn cluster_tags(
    keys: &[u64],
    sketches: &HashMap<u64, Sketch>,
    tau: f64,
) -> Vec<Option<ClusterTag>> {
    let keyed: Vec<(usize, u64)> = keys
        .iter()
        .enumerate()
        .filter_map(|(i, &key)| sketches.contains_key(&key).then_some((i, key)))
        .collect();

    let mut parent: Vec<usize> = (0..keyed.len()).collect();
    fn find(parent: &mut Vec<usize>, x: usize) -> usize {
        if parent[x] != x {
            let root = find(parent, parent[x]);
            parent[x] = root;
        }
        parent[x]
    }
    for a in 0..keyed.len() {
        for b in (a + 1)..keyed.len() {
            let (sa, sb) = (&sketches[&keyed[a].1], &sketches[&keyed[b].1]);
            if sa.containment(sb) >= tau {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
    }

    let mut out: Vec<Option<ClusterTag>> = vec![None; keys.len()];
    let mut id_by_root: HashMap<usize, u32> = HashMap::new();
    // (best shingle count, key index) per cluster id — the canonical member.
    let mut best: Vec<(u32, usize)> = Vec::new();
    for (slot, &(idx, key)) in keyed.iter().enumerate() {
        let root = find(&mut parent, slot);
        let next_id = id_by_root.len() as u32;
        let id = *id_by_root.entry(root).or_insert(next_id);
        if id as usize == best.len() {
            best.push((0, idx));
        }
        let shingles = sketches[&key].shingles;
        if shingles > best[id as usize].0 {
            best[id as usize] = (shingles, idx);
        }
        out[idx] = Some(ClusterTag {
            id,
            canonical: false,
        });
    }
    for &(_, idx) in &best {
        if let Some(tag) = &mut out[idx] {
            tag.canonical = true;
        }
    }
    out
}

/// Cluster `results` by sketch containment and annotate them in place.
/// Returns the response-level block. `tau` is the containment merge threshold
/// (config `evidence.containment_tau`, default `sketch::CONTAINMENT_TAU`).
pub fn annotate(
    results: &mut [SearchResult],
    sketches: &HashMap<u64, Sketch>,
    tau: f64,
) -> EvidenceBlock {
    let keys: Vec<u64> = results.iter().map(|r| url_key(&r.url)).collect();
    let tags = cluster_tags(&keys, sketches, tau);

    let mut members: Vec<u32> = Vec::new();
    let mut domains: Vec<std::collections::HashSet<String>> = Vec::new();
    let mut sketched = 0u32;
    for (result_idx, tag) in tags.iter().enumerate() {
        let Some(tag) = tag else { continue };
        sketched += 1;
        if tag.id as usize == members.len() {
            members.push(0);
            domains.push(std::collections::HashSet::new());
        }
        members[tag.id as usize] += 1;
        domains[tag.id as usize].insert(result_host(&results[result_idx].url));
        results[result_idx].evidence = Some(ResultEvidence {
            cluster: tag.id,
            canonical: tag.canonical,
        });
    }

    EvidenceBlock {
        // schema 2 (v0.6.0): per-result annotations gain `canonical`.
        schema: 2,
        independent_source_count: members.len() as u32,
        apparent_source_count: results.len() as u32,
        sketched_results: sketched,
        clusters: members
            .iter()
            .zip(domains.iter())
            .enumerate()
            .map(|(id, (m, d))| ClusterSummary {
                id: id as u32,
                members: *m,
                domains: d.len() as u32,
            })
            .collect(),
    }
}

fn result_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::RankSignals;
    use meridian_index::sketch::CONTAINMENT_TAU;

    fn result(url: &str) -> SearchResult {
        SearchResult {
            url: url.to_owned(),
            title: String::new(),
            snippet: String::new(),
            score: 0.0,
            rank_signals: RankSignals {
                rrf: 0.0,
                bm25: None,
                ann: None,
                searx_rank: None,
                ltr: 0.0,
                ce: None,
            },
            source: "local",
            h3: None,
            ts: None,
            evidence: None,
        }
    }

    #[test]
    fn config_defaults_track_promoted_consts() {
        // The promoted (C) constants must keep config defaults pinned to the
        // SPEC-locked / suite-derived values — drift here is a silent behavior
        // change, so the defaults are asserted against the canonical consts.
        use meridian_common::config::{EvidenceConfig, SearchConfig};
        assert_eq!(EvidenceConfig::default().containment_tau, CONTAINMENT_TAU);
        assert_eq!(SearchConfig::default().rrf_k, crate::rrf::RRF_K);
    }

    #[test]
    fn clusters_copies_and_leaves_web_results_null() {
        let origin = "the desalination plant approval covered by many outlets verbatim \
                      with identical wording across the syndication network today"
            .repeat(3);
        let independent = "completely different reporting angle with its own words and \
                           structure about water infrastructure policy and budgets"
            .repeat(3);
        let mut sketches = HashMap::new();
        sketches.insert(url_key("https://a.example/1"), Sketch::compute(&origin));
        sketches.insert(url_key("https://b.example/2"), Sketch::compute(&origin));
        sketches.insert(
            url_key("https://c.example/3"),
            Sketch::compute(&independent),
        );

        let mut results = vec![
            result("https://a.example/1"),
            result("https://b.example/2"),
            result("https://c.example/3"),
            result("https://unsketched.example/4"), // pure web result
        ];
        let block = annotate(&mut results, &sketches, CONTAINMENT_TAU);

        assert_eq!(block.schema, 2);
        assert_eq!(block.apparent_source_count, 4);
        assert_eq!(block.sketched_results, 3);
        // Verbatim copies have EQUAL shingle counts — the first one in
        // response order is the deterministic canonical; exactly one per
        // cluster either way.
        assert!(results[0].evidence.as_ref().unwrap().canonical);
        assert!(!results[1].evidence.as_ref().unwrap().canonical);
        assert!(results[2].evidence.as_ref().unwrap().canonical);
        assert_eq!(
            block.independent_source_count, 2,
            "two copies + one independent = two origins"
        );
        assert_eq!(
            results[0].evidence.as_ref().unwrap().cluster,
            results[1].evidence.as_ref().unwrap().cluster,
            "verbatim copies share a cluster"
        );
        assert_ne!(
            results[0].evidence.as_ref().unwrap().cluster,
            results[2].evidence.as_ref().unwrap().cluster
        );
        assert!(
            results[3].evidence.is_none(),
            "unsketched result must stay null, never guessed"
        );
        let c0 = &block.clusters[results[0].evidence.as_ref().unwrap().cluster as usize];
        assert_eq!(c0.members, 2);
        assert_eq!(c0.domains, 2, "cross-domain syndication visible");
    }
}
