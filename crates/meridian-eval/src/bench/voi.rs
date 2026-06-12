//! Suite 15 — `voi`: hermetic replay of VoI fetch selection & stopping
//! (Phase 9, ADR-26 / 07-voi-design.md §3).
//!
//! Simulates the deep-mode fetch decision exactly as the planner will frame
//! it, with corpus-revealed values instead of network fetches: per query, 20
//! candidates ranked by an ambiguous snippet score — 3 "decisive" originals
//! whose FULL text would prove relevance, each with 2 syndicated copies
//! (same evidence cluster: fetching a copy of something already read
//! realizes nothing new), and 11 chaff. Opening a candidate reveals its full
//! text: decisive docs jump to the top of the re-ranking, chaff drops
//! slightly. nDCG@10 is measured against the known grades (orig 3, copy 2).
//!
//! Three policies at the production default budget:
//! - **fetch-all-top-N** (N=10): the quality ceiling and the cost strawman;
//! - **rank-greedy** at VoI's realized budget: what naive top-k fetching
//!   buys — it burns budget on same-cluster copies ranked adjacently;
//! - **VoI** (the real `pandora_walk` + the real value model from
//!   `meridian-fetch::voi`).
//!
//! Gates (SPEC §16 P9): VoI nDCG@10 within ±1% of fetch-all using ≥25%
//! fewer fetches, AND VoI's median distinct-clusters-fetched ≥ rank-greedy's
//! at the same per-query budget (the diversity guard: a fetch policy that
//! optimizes relevance by starving dissenting sources is a regression,
//! risk #22). The (β0, β1) probability constants are swept on the tuning
//! seed and the FROZEN shipped constants are what the gate judges on the
//! hold-out seed (risk-#21 protocol).

use super::{BenchConfig, SuiteResult};
use crate::metrics::ndcg_at;
use crate::stats::Rng;
use meridian_fetch::voi::{P_BETA0, P_BETA2, additive_walk, candidate_with_betas};
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;

const CANDIDATES: usize = 20;
const DECISIVE: usize = 3;
const COPIES_PER: usize = 2;
const QUERIES: usize = 200;
const FETCH_ALL_N: usize = 10;
/// Per-fetch cost in DCG-gain units (calibrated so one decisive find at
/// rank ~10 clearly pays for itself and chaff clearly does not).
const COST: f64 = 0.6;

struct Cand {
    /// Evidence cluster (copies share their original's).
    cluster: usize,
    /// grade: 3 original decisive, 2 copy, 0 chaff.
    grade: u32,
    /// Ambiguous snippet score (what the shipped ranking sees).
    snippet_score: f64,
}

struct Query {
    cands: Vec<Cand>,
}

fn generate(rng: &mut Rng, queries: usize) -> Vec<Query> {
    (0..queries)
        .map(|_| {
            let mut cands = Vec::with_capacity(CANDIDATES);
            for d in 0..DECISIVE {
                // Snippets HINT at decisiveness without proving it: +0.15
                // bias under 0.15σ noise keeps the orderings honestly mixed.
                let base = 0.5 + 0.15 + 0.15 * rng.next_gaussian() as f64;
                cands.push(Cand {
                    cluster: d,
                    grade: 3,
                    snippet_score: base,
                });
                for _ in 0..COPIES_PER {
                    cands.push(Cand {
                        cluster: d,
                        grade: 2,
                        // Copies are near-dups: nearly the original's score.
                        snippet_score: base + 0.01 * rng.next_gaussian() as f64,
                    });
                }
            }
            while cands.len() < CANDIDATES {
                let c = cands.len();
                cands.push(Cand {
                    cluster: 100 + c, // chaff: every doc its own cluster
                    grade: 0,
                    snippet_score: 0.5 + 0.15 * rng.next_gaussian() as f64,
                });
            }
            Query { cands }
        })
        .collect()
}

/// Final ranking after a fetch set: revealed decisive docs outrank
/// everything unfetched (full-text evidence), revealed chaff sinks, the
/// rest keep snippet order. Returns nDCG@10 against the known grades.
fn ndcg_after(q: &Query, fetched: &HashSet<usize>) -> f64 {
    let mut order: Vec<usize> = (0..q.cands.len()).collect();
    let score = |i: usize| {
        let c = &q.cands[i];
        if fetched.contains(&i) {
            if c.grade > 0 {
                1.0 + c.snippet_score
            } else {
                c.snippet_score - 0.1
            }
        } else {
            c.snippet_score
        }
    };
    order.sort_by(|&a, &b| {
        score(b)
            .partial_cmp(&score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let ranked: Vec<String> = order.iter().map(|i| i.to_string()).collect();
    let qrels: HashMap<String, u32> = q
        .cands
        .iter()
        .enumerate()
        .map(|(i, c)| (i.to_string(), c.grade))
        .collect();
    ndcg_at(10, &ranked, &qrels)
}

/// DCG headroom of moving rank r (0-based) to the top of the prefix.
fn headroom(rank: usize) -> f64 {
    let disc = |r: usize| 1.0 / ((r as f64 + 2.0).log2());
    7.0 * (disc(0) - disc(rank))
}

struct PolicyOutcome {
    ndcg: f64,
    fetches: usize,
    clusters: usize,
}

/// Run VoI on one query with given betas; realized value of a fetch is its
/// actual nDCG contribution scaled into gain units (decisive → big, copy of
/// an already-fetched cluster → small, chaff → none).
fn run_voi(q: &Query, budget: usize, beta0: f64, beta2: f64) -> PolicyOutcome {
    let mut order: Vec<usize> = (0..q.cands.len()).collect();
    order.sort_by(|&a, &b| {
        q.cands[b]
            .snippet_score
            .partial_cmp(&q.cands[a].snippet_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let rank_of: HashMap<usize, usize> = order.iter().enumerate().map(|(r, &i)| (i, r)).collect();
    // Standardize snippet scores over the head — what the planner does with
    // its own retrieval scores.
    let n = q.cands.len() as f64;
    let mean = q.cands.iter().map(|c| c.snippet_score).sum::<f64>() / n;
    let sd = (q
        .cands
        .iter()
        .map(|c| (c.snippet_score - mean).powi(2))
        .sum::<f64>()
        / n)
        .sqrt()
        .max(1e-9);

    let mut fetched: HashSet<usize> = HashSet::new();
    let mut fetched_clusters: HashSet<usize> = HashSet::new();
    // Walk in rounds: novelty shifts after every open (the fetched doc's
    // cluster loses marginal value), so re-pose the candidate set each
    // round — the planner wiring does the same; cheap at k=20.
    let mut stop = false;
    while !stop && fetched.len() < budget {
        let cands: Vec<_> = (0..q.cands.len())
            .filter(|i| !fetched.contains(i))
            .map(|i| {
                let novelty = if fetched_clusters.contains(&q.cands[i].cluster) {
                    0.05
                } else {
                    1.0
                };
                let score_z = (q.cands[i].snippet_score - mean) / sd;
                candidate_with_betas(
                    i as u64,
                    headroom(rank_of[&i]),
                    novelty,
                    score_z,
                    COST,
                    beta0,
                    beta2,
                )
            })
            .collect();
        let before = fetched.clone();
        let r = additive_walk(&cands, 1, |_| {});
        for id in &r.opened {
            let i = *id as usize;
            fetched.insert(i);
            if q.cands[i].grade > 0 {
                fetched_clusters.insert(q.cands[i].cluster);
            }
        }
        // One round opened nothing → no remaining candidate's expected net
        // marginal value is positive; that is the stop.
        stop = fetched == before;
    }
    PolicyOutcome {
        ndcg: ndcg_after(q, &fetched),
        fetches: fetched.len(),
        clusters: fetched_clusters.len(),
    }
}

fn run_rank_greedy(q: &Query, budget: usize) -> PolicyOutcome {
    let mut order: Vec<usize> = (0..q.cands.len()).collect();
    order.sort_by(|&a, &b| {
        q.cands[b]
            .snippet_score
            .partial_cmp(&q.cands[a].snippet_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let fetched: HashSet<usize> = order.into_iter().take(budget).collect();
    let clusters: HashSet<usize> = fetched
        .iter()
        .filter(|&&i| q.cands[i].grade > 0)
        .map(|&i| q.cands[i].cluster)
        .collect();
    PolicyOutcome {
        ndcg: ndcg_after(q, &fetched),
        fetches: fetched.len(),
        clusters: clusters.len(),
    }
}

fn run_fetch_all(q: &Query) -> PolicyOutcome {
    run_rank_greedy(q, FETCH_ALL_N)
}

struct Aggregate {
    ndcg: f64,
    fetches: f64,
    median_clusters: f64,
}

fn aggregate(outcomes: &[PolicyOutcome]) -> Aggregate {
    let n = outcomes.len().max(1) as f64;
    let mut clusters: Vec<usize> = outcomes.iter().map(|o| o.clusters).collect();
    clusters.sort_unstable();
    Aggregate {
        ndcg: outcomes.iter().map(|o| o.ndcg).sum::<f64>() / n,
        fetches: outcomes.iter().map(|o| o.fetches as f64).sum::<f64>() / n,
        median_clusters: clusters[clusters.len() / 2] as f64,
    }
}

fn evaluate(queries: &[Query], beta0: f64, beta2: f64) -> (Aggregate, Aggregate, Aggregate) {
    let voi: Vec<_> = queries
        .iter()
        .map(|q| run_voi(q, FETCH_ALL_N, beta0, beta2))
        .collect();
    // Rank-greedy is judged at VoI's own realized budget per query — the
    // apples-to-apples diversity comparison.
    let greedy: Vec<_> = queries
        .iter()
        .zip(&voi)
        .map(|(q, v)| run_rank_greedy(q, v.fetches.max(1)))
        .collect();
    let all: Vec<_> = queries.iter().map(run_fetch_all).collect();
    (aggregate(&voi), aggregate(&greedy), aggregate(&all))
}

pub fn run(_cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("voi");
    let start = Instant::now();

    // Sweep on the tuning seed (reported); judge the FROZEN constants on the
    // hold-out seed (gated).
    let mut tune_rng = Rng::new(0x0501_2026);
    let tuning = generate(&mut tune_rng, QUERIES);
    let mut best_note = String::new();
    let mut best = (f64::MAX, 0.0, 0.0);
    for beta0 in [0.05, 0.1, 0.2] {
        for beta2 in [0.1, 0.2, 0.3] {
            let (v, _, a) = evaluate(&tuning, beta0, beta2);
            let ok = v.ndcg >= a.ndcg - 0.01;
            if ok && v.fetches < best.0 {
                best = (v.fetches, beta0, beta2);
            }
            best_note.push_str(&format!(
                "(β0={beta0},β2={beta2}: nDCG {:.4} vs all {:.4}, fetches {:.2}) ",
                v.ndcg, a.ndcg, v.fetches
            ));
        }
    }
    result.metric("sweep_best_beta0", best.1);
    result.metric("sweep_best_beta2", best.2);
    result.note(format!("tuning sweep: {best_note}"));
    result.note(format!(
        "shipped constants P_BETA0={P_BETA0}, P_BETA2={P_BETA2} — must match the sweep winner \
         or the divergence is recorded here"
    ));

    let mut hold_rng = Rng::new(0x1337_2026);
    let holdout = generate(&mut hold_rng, QUERIES);
    let (v, g, a) = evaluate(&holdout, P_BETA0, P_BETA2);

    result.metric("holdout_voi_ndcg10", round4(v.ndcg));
    result.metric("holdout_fetch_all_ndcg10", round4(a.ndcg));
    result.metric("holdout_rank_greedy_ndcg10", round4(g.ndcg));
    result.metric("holdout_voi_fetches_mean", round4(v.fetches));
    result.metric("holdout_fetch_all_fetches", round4(a.fetches));
    result.metric("holdout_voi_median_clusters", v.median_clusters);
    result.metric("holdout_greedy_median_clusters", g.median_clusters);

    let quality_held = v.ndcg >= a.ndcg - 0.01;
    let fetch_saving = 1.0 - v.fetches / a.fetches;
    result.metric("holdout_fetch_saving", round4(fetch_saving));
    let diversity_held = v.median_clusters >= g.median_clusters;
    result.gate(
        "VoI nDCG@10 within 0.01 of fetch-all with ≥25% fewer fetches AND median \
         fetched-cluster count ≥ rank-greedy at equal budget",
        quality_held && fetch_saving >= 0.25 && diversity_held,
    );
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}

fn round4(x: f64) -> f64 {
    (x * 1e4).round() / 1e4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suite_gate_passes_with_shipped_constants() {
        let r = run(&BenchConfig::default());
        assert_eq!(r.gate_passed, Some(true), "metrics: {:?}", r.metrics);
    }
}
