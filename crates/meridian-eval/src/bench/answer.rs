//! Suite 18 — `answer`: best-passage replay for answer mode (Phase 10,
//! ADR-29 / 04-bench-plan §7).
//!
//! Extends the suite-15 replay shape with answer structure: per query, 20
//! candidates — 3 decisive originals (ONE of which carries THE answer
//! passage), 2 syndicated copies each (truncation/paraphrase degrades the
//! passage — a copy is a worse place to read the answer than its original),
//! 11 chaff. Snippet-level signals are deliberately ambiguous: the no-fetch
//! baseline must sometimes pick a doc whose full text does NOT contain the
//! best passage — that gap is what fetching buys.
//!
//! Unit system (ADR-29): answer mode runs `pandora_walk` in PASSAGE-CE
//! units — gain = novelty·1.0 (the upside of reading a fresh doc is a
//! passage in [0,1]), realized value = the fetched doc's best passage CE,
//! initial_best = 0 (no passage exists before the first fetch), and the
//! per-fetch cost `ANSWER_FETCH_COST` is swept HERE on the tuning seed and
//! frozen for the hold-out (risk-#21 protocol). The page-mode constants
//! (β0, β2 for p) are NOT re-swept — they are suite-15 property.
//!
//! Harness-construct note (v0 documented, not hidden): the first run gave
//! the SELECTOR only the raw retrieval score while the baseline read the
//! snippet-CE — but production runs the fetch phase AFTER the deep snippet
//! rerank, so the selector's score signal includes snippet-CE. The replay
//! now mirrors that framing (selector z over snippet-CE, additive headroom
//! over the CE-reranked order); v0 measured the selectors at 0.10 hit vs
//! the baseline's 0.54 purely through that starvation.
//!
//! Gates (04-bench-plan §7, hold-out, production budget cap 2):
//! - hit-rate(answer mode) ≥ hit-rate(snippet-head baseline) + 10pp;
//! - pandora hit-rate ≥ additive's AND pandora mean fetches ≤ additive's —
//!   the single-best regime claim of ADR-26, measured, not assumed.

use super::{BenchConfig, SuiteResult};
use crate::stats::Rng;
use meridian_fetch::voi::{Candidate, DEFAULT_FETCH_COST, additive_walk, pandora_walk};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

const CANDIDATES: usize = 20;
const DECISIVE: usize = 3;
const COPIES_PER: usize = 2;
const QUERIES: usize = 1000;
/// Production default `search.deep_fetch_max` — the budget the gate judges.
const BUDGET: usize = 2;

const COST_SWEEP: [f64; 4] = [0.1, 0.2, 0.3, 0.4];

struct Cand {
    cluster: usize,
    /// The full text's best (query, passage) CE score — revealed on fetch.
    passage_ce: f64,
    /// What snippet-level CE sees without fetching (ambiguous).
    snippet_ce: f64,
    is_answer: bool,
}

struct Query {
    cands: Vec<Cand>,
}

fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

fn generate(rng: &mut Rng, queries: usize) -> Vec<Query> {
    (0..queries)
        .map(|_| {
            let answer_at = rng.below(DECISIVE);
            let mut cands = Vec::with_capacity(CANDIDATES);
            for d in 0..DECISIVE {
                let is_answer = d == answer_at;
                let passage = if is_answer {
                    clamp01(0.85 + 0.05 * rng.next_gaussian() as f64)
                } else {
                    clamp01(0.55 + 0.10 * rng.next_gaussian() as f64)
                };
                cands.push(Cand {
                    cluster: d,
                    passage_ce: passage,
                    snippet_ce: clamp01(passage - 0.25 + 0.18 * rng.next_gaussian() as f64),
                    is_answer,
                });
                for _ in 0..COPIES_PER {
                    // Syndication truncates/paraphrases: the copy's best
                    // passage is materially worse than its original's.
                    let copy_passage = clamp01(passage - 0.25 + 0.05 * rng.next_gaussian() as f64);
                    cands.push(Cand {
                        cluster: d,
                        passage_ce: copy_passage,
                        snippet_ce: clamp01(
                            copy_passage - 0.25 + 0.18 * rng.next_gaussian() as f64,
                        ),
                        is_answer: false,
                    });
                }
            }
            while cands.len() < CANDIDATES {
                let c = cands.len();
                let passage = clamp01(0.15 + 0.10 * rng.next_gaussian() as f64);
                cands.push(Cand {
                    cluster: 100 + c,
                    passage_ce: passage,
                    snippet_ce: clamp01(passage - 0.05 + 0.18 * rng.next_gaussian() as f64),
                    is_answer: false,
                });
            }
            Query { cands }
        })
        .collect()
}

struct Outcome {
    hit: bool,
    fetches: usize,
}

/// Standardized POST-RERANK scores → p: the planner runs the fetch phase
/// after the deep snippet rerank, so the selector's score signal is the
/// snippet-CE, not the raw retrieval score (harness-construct fix, v0 note
/// above).
fn score_stats(q: &Query) -> (f64, f64) {
    let n = q.cands.len() as f64;
    let mean = q.cands.iter().map(|c| c.snippet_ce).sum::<f64>() / n;
    let sd = (q
        .cands
        .iter()
        .map(|c| (c.snippet_ce - mean).powi(2))
        .sum::<f64>()
        / n)
        .sqrt()
        .max(1e-9);
    (mean, sd)
}

fn p_of(q: &Query, i: usize, mean: f64, sd: f64) -> f64 {
    let z = ((q.cands[i].snippet_ce - mean) / sd).clamp(-3.0, 3.0);
    (meridian_fetch::voi::P_BETA0 + meridian_fetch::voi::P_BETA2 * z).clamp(0.05, 0.95)
}

/// Answer mode: pandora_walk in passage-CE units, re-posed between opens
/// (novelty shifts when a cluster is read), hit = the best fetched passage
/// belongs to the answer original.
fn run_answer(q: &Query, cost: f64, budget: usize) -> Outcome {
    let (mean, sd) = score_stats(q);
    let mut fetched: HashSet<usize> = HashSet::new();
    let mut read_clusters: HashSet<usize> = HashSet::new();
    let mut best_passage: Option<(usize, f64)> = None;
    let mut best_value = 0.0f64; // no passage in hand before the first fetch

    loop {
        if fetched.len() >= budget {
            break;
        }
        let cands: Vec<Candidate> = (0..q.cands.len())
            .filter(|i| !fetched.contains(i))
            .map(|i| {
                let novelty = if read_clusters.contains(&q.cands[i].cluster) {
                    0.05
                } else {
                    1.0
                };
                Candidate {
                    id: i as u64,
                    p: p_of(q, i, mean, sd),
                    gain: novelty, // passage-CE upside of a fresh doc
                    cost,
                }
            })
            .collect();
        let before = fetched.len();
        let r = pandora_walk(&cands, best_value, 1, |id| {
            let i = id as usize;
            fetched.insert(i);
            read_clusters.insert(q.cands[i].cluster);
            let ce = q.cands[i].passage_ce;
            if best_passage.map(|(_, b)| ce > b).unwrap_or(true) {
                best_passage = Some((i, ce));
            }
            ce
        });
        for id in r.opened {
            best_value = best_value.max(q.cands[id as usize].passage_ce);
        }
        if fetched.len() == before {
            break; // the incumbent beat the next reservation index — done
        }
    }
    Outcome {
        hit: best_passage
            .map(|(i, _)| q.cands[i].is_answer)
            .unwrap_or(false),
        fetches: fetched.len(),
    }
}

/// The v0.4.0 page selector at the same budget — fetches for PAGE value
/// (novelty·headroom), then the best passage among what it happened to read.
fn run_additive(q: &Query, budget: usize) -> Outcome {
    let (mean, sd) = score_stats(q);
    let mut order: Vec<usize> = (0..q.cands.len()).collect();
    order.sort_by(|&a, &b| {
        q.cands[b]
            .snippet_ce
            .partial_cmp(&q.cands[a].snippet_ce)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let rank_of: HashMap<usize, usize> = order.iter().enumerate().map(|(r, &i)| (i, r)).collect();
    let headroom = |rank: usize| {
        let disc = |r: usize| 1.0 / ((r as f64 + 2.0).log2());
        7.0 * (disc(0) - disc(rank))
    };

    let mut fetched: HashSet<usize> = HashSet::new();
    let mut read_clusters: HashSet<usize> = HashSet::new();
    loop {
        if fetched.len() >= budget {
            break;
        }
        let cands: Vec<Candidate> = (0..q.cands.len())
            .filter(|i| !fetched.contains(i))
            .map(|i| {
                let novelty = if read_clusters.contains(&q.cands[i].cluster) {
                    0.05
                } else {
                    1.0
                };
                Candidate {
                    id: i as u64,
                    p: p_of(q, i, mean, sd),
                    gain: novelty * headroom(rank_of[&i]),
                    cost: DEFAULT_FETCH_COST,
                }
            })
            .collect();
        let before = fetched.len();
        let r = additive_walk(&cands, 1, |_| {});
        for id in r.opened {
            let i = id as usize;
            fetched.insert(i);
            read_clusters.insert(q.cands[i].cluster);
        }
        if fetched.len() == before {
            break;
        }
    }
    let best = fetched
        .iter()
        .max_by(|&&a, &&b| {
            q.cands[a]
                .passage_ce
                .partial_cmp(&q.cands[b].passage_ce)
                .unwrap()
        })
        .copied();
    Outcome {
        hit: best.map(|i| q.cands[i].is_answer).unwrap_or(false),
        fetches: fetched.len(),
    }
}

/// No-fetch baseline: CE over snippets picks the best-looking doc directly.
fn run_snippet_head(q: &Query) -> Outcome {
    let best = (0..q.cands.len())
        .max_by(|&a, &b| {
            q.cands[a]
                .snippet_ce
                .partial_cmp(&q.cands[b].snippet_ce)
                .unwrap()
        })
        .unwrap();
    Outcome {
        hit: q.cands[best].is_answer,
        fetches: 0,
    }
}

struct Agg {
    hit_rate: f64,
    mean_fetches: f64,
}

fn agg(outcomes: &[Outcome]) -> Agg {
    let n = outcomes.len().max(1) as f64;
    Agg {
        hit_rate: outcomes.iter().filter(|o| o.hit).count() as f64 / n,
        mean_fetches: outcomes.iter().map(|o| o.fetches as f64).sum::<f64>() / n,
    }
}

pub fn run(_cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("answer");
    let start = Instant::now();
    result.metric("queries", QUERIES);
    result.metric("budget", BUDGET);

    // Sweep ANSWER_FETCH_COST on the tuning seed (a-priori rule: max
    // hit-rate, fewer fetches as tie-break), freeze, judge on the hold-out.
    let mut tune_rng = Rng::new(0x0A45_2026);
    let tuning = generate(&mut tune_rng, QUERIES);
    let mut chosen = (f64::NAN, -1.0f64, f64::MAX);
    for cost in COST_SWEEP {
        let a = agg(&tuning
            .iter()
            .map(|q| run_answer(q, cost, BUDGET))
            .collect::<Vec<_>>());
        result.note(format!(
            "sweep cost={cost}: tuning hit_rate={:.3} mean_fetches={:.2}",
            a.hit_rate, a.mean_fetches
        ));
        if a.hit_rate > chosen.1 + 1e-12
            || (a.hit_rate >= chosen.1 - 1e-12 && a.mean_fetches < chosen.2)
        {
            chosen = (cost, a.hit_rate, a.mean_fetches);
        }
    }
    let cost = chosen.0;
    result.metric("chosen_answer_fetch_cost", cost);

    let mut hold_rng = Rng::new(0x1A45_2026);
    let holdout = generate(&mut hold_rng, QUERIES);
    let answer = agg(&holdout
        .iter()
        .map(|q| run_answer(q, cost, BUDGET))
        .collect::<Vec<_>>());
    let additive = agg(&holdout
        .iter()
        .map(|q| run_additive(q, BUDGET))
        .collect::<Vec<_>>());
    let baseline = agg(&holdout.iter().map(run_snippet_head).collect::<Vec<_>>());

    result.metric("holdout_answer_hit_rate", round3(answer.hit_rate));
    result.metric("holdout_answer_mean_fetches", round3(answer.mean_fetches));
    result.metric("holdout_additive_hit_rate", round3(additive.hit_rate));
    result.metric(
        "holdout_additive_mean_fetches",
        round3(additive.mean_fetches),
    );
    result.metric(
        "holdout_snippet_baseline_hit_rate",
        round3(baseline.hit_rate),
    );
    result.note(format!(
        "answer mode = pandora_walk in passage-CE units (gain = novelty, realized = best \
         fetched passage CE, initial_best = 0, cost frozen on tuning); additive comparator = \
         the v0.4.0 page selector at the same budget; baseline = no-fetch CE over snippets; \
         budget = production deep_fetch_max ({BUDGET})"
    ));

    let g_lift = answer.hit_rate >= baseline.hit_rate + 0.10;
    let g_regime = answer.hit_rate >= additive.hit_rate - 1e-12
        && answer.mean_fetches <= additive.mean_fetches + 1e-12;
    result.metric("gate_hit_lift_10pp", u8::from(g_lift));
    result.metric("gate_pandora_beats_additive_regime", u8::from(g_regime));
    result.gate(
        "hold-out: answer hit-rate ≥ snippet baseline + 10pp AND pandora ≥ additive on hit-rate \
         at ≤ its fetches (the ADR-26 single-best regime, measured)",
        g_lift && g_regime,
    );
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_plants_one_answer_per_query() {
        let mut rng = Rng::new(9);
        let qs = generate(&mut rng, 10);
        for q in &qs {
            assert_eq!(q.cands.len(), CANDIDATES);
            assert_eq!(q.cands.iter().filter(|c| c.is_answer).count(), 1);
            let ans = q.cands.iter().find(|c| c.is_answer).unwrap();
            assert!(ans.passage_ce > 0.6, "answer passage must be strong");
        }
    }

    #[test]
    fn fetching_the_answer_beats_not_fetching_it() {
        let mut rng = Rng::new(11);
        let qs = generate(&mut rng, 50);
        let with_fetch: f64 = qs
            .iter()
            .map(|q| run_answer(q, 0.2, BUDGET))
            .filter(|o| o.hit)
            .count() as f64;
        let without: f64 = qs.iter().map(run_snippet_head).filter(|o| o.hit).count() as f64;
        assert!(
            with_fetch >= without,
            "fetching must not LOSE hits: {with_fetch} vs {without}"
        );
    }
}
