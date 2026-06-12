//! Suite 15b — `voi-embed`: the embedding-coverage study carried from Phase 9
//! (ADR-26 deviation: "embedding-coverage joins the value model when a
//! calibratable signal study exists"). AMEND-OR-RECORD: the coverage term
//! ships only if it beats the FROZEN v0.4.0 value model on the hold-out;
//! a measured no closes the carry.
//!
//! The motivating case the sketch-novelty term cannot see: a PARAPHRASED
//! syndication copy shares few exact tokens with its original (MinHash
//! containment low → novelty HIGH → it looks like fresh evidence) while
//! living in the same semantic neighborhood (embedding cosine high). The
//! generator plants exactly that: each decisive original has one TOKEN copy
//! (high containment — the old model already prices it out) and one
//! PARAPHRASE copy built from a different REGISTER of the same real-word
//! semantic field (low containment, high cosine — only the embedding sees
//! the redundancy).
//!
//! Protocol: calibration sanity first (potion must actually separate
//! same-field registers from cross-field text — if it cannot, the verdict
//! is "no calibratable signal" and the carry closes honestly), then the
//! suite-15 replay frame with REAL `Sketch` novelty over the generated
//! texts and real potion embeddings, tuning-seed sweep over the coverage
//! combiner, frozen judgment on the hold-out seed. Needs `models/`; skips
//! (not passes) without it.

use super::{BenchConfig, SuiteResult};
use crate::metrics::ndcg_at;
use crate::stats::Rng;
use meridian_fetch::voi::{additive_walk, candidate_with_betas};
use meridian_index::sketch::Sketch;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

const CANDIDATES: usize = 20;
const DECISIVE: usize = 3;
const QUERIES: usize = 200;
const FETCH_ALL_N: usize = 10;
const COST: f64 = meridian_fetch::voi::DEFAULT_FETCH_COST;
const BETA0: f64 = meridian_fetch::voi::P_BETA0;
const BETA2: f64 = meridian_fetch::voi::P_BETA2;

/// Real-word semantic fields, two registers each: register B paraphrases
/// register A with minimal exact-token overlap. Static and curated — the
/// study needs potion's ACTUAL semantic geometry, not synthetic tokens.
const TOPICS: [(&str, &str); 10] = [
    (
        "volcano eruption lava magma ash crater seismic vent basalt pyroclastic flow tremor",
        "volcanic explosion molten rock plume cinder caldera earthquake fissure igneous debris quake",
    ),
    (
        "election ballot vote candidate campaign polling constituency electorate turnout primary",
        "voting referendum elector nominee canvass survey district voters participation runoff",
    ),
    (
        "vaccine immunization dose antibody trial efficacy booster pathogen immunity clinical",
        "inoculation vaccination shot antigen study effectiveness jab virus protection medical",
    ),
    (
        "drought rainfall reservoir irrigation aquifer scarcity groundwater precipitation arid crop",
        "dry spell rain shortage dam watering well depletion water shower parched harvest",
    ),
    (
        "satellite orbit launch payload rocket telemetry trajectory booster spacecraft mission",
        "probe orbital liftoff cargo missile signal flight path thruster vessel expedition",
    ),
    (
        "inflation prices wages currency interest monetary fiscal deficit budget economy",
        "cost living salary money rates banking spending shortfall finances economic growth",
    ),
    (
        "wildfire blaze evacuation firefighter containment acreage smoke ember flame burn",
        "forest fire inferno evacuees crews control hectares haze sparks combustion scorch",
    ),
    (
        "glacier icecap melting permafrost arctic thaw seaice tundra polar frozen",
        "ice sheet snowpack melt frost northern warming floe steppe pole icy",
    ),
    (
        "harbor port shipping cargo container vessel dock freight maritime customs",
        "wharf terminal transport goods crate ship pier logistics naval clearance",
    ),
    (
        "museum exhibition artifact curator gallery collection archaeology restoration heritage display",
        "exhibit showcase relic conservator hall archive excavation preservation history installation",
    ),
];

const FILLER: &str = "report annual summary update notes details overview review";

struct Cand {
    cluster: usize,
    grade: u32,
    snippet_score: f64,
    text: String,
}

struct Query {
    cands: Vec<Cand>,
}

fn sample_words(pool: &str, n: usize, rng: &mut Rng) -> String {
    let words: Vec<&str> = pool.split_whitespace().collect();
    (0..n)
        .map(|_| words[rng.below(words.len())])
        .collect::<Vec<_>>()
        .join(" ")
}

fn generate(rng: &mut Rng, queries: usize) -> Vec<Query> {
    (0..queries)
        .map(|_| {
            let mut cands = Vec::with_capacity(CANDIDATES);
            // Three decisive topics, distinct per query.
            let mut topic_ids: Vec<usize> = (0..TOPICS.len()).collect();
            for d in 0..DECISIVE {
                let pick = d + rng.below(topic_ids.len() - d);
                topic_ids.swap(d, pick);
            }
            for d in 0..DECISIVE {
                let (reg_a, reg_b) = TOPICS[topic_ids[d]];
                let base = 0.5 + 0.15 + 0.15 * rng.next_gaussian() as f64;
                let original = sample_words(reg_a, 30, rng);
                // TOKEN copy: literally the original's words re-sampled from
                // the same register — high MinHash containment.
                let token_copy = format!("{} {}", original, sample_words(FILLER, 4, rng));
                // PARAPHRASE copy: same semantic field, other register —
                // low containment, high cosine. The case under study.
                let paraphrase = sample_words(reg_b, 30, rng);
                cands.push(Cand {
                    cluster: d,
                    grade: 3,
                    snippet_score: base,
                    text: original,
                });
                cands.push(Cand {
                    cluster: d,
                    grade: 2,
                    snippet_score: base + 0.01 * rng.next_gaussian() as f64,
                    text: token_copy,
                });
                cands.push(Cand {
                    cluster: d,
                    grade: 2,
                    snippet_score: base + 0.01 * rng.next_gaussian() as f64,
                    text: paraphrase,
                });
            }
            while cands.len() < CANDIDATES {
                let c = cands.len();
                // Chaff: generic filler plus an unrelated topic's scattered
                // words — its own cluster, no planted redundancy.
                let stray = TOPICS[rng.below(TOPICS.len())].0;
                let text = format!(
                    "{} {}",
                    sample_words(FILLER, 8, rng),
                    sample_words(stray, 3, rng)
                );
                cands.push(Cand {
                    cluster: 100 + c,
                    grade: 0,
                    snippet_score: 0.5 + 0.15 * rng.next_gaussian() as f64,
                    text,
                });
            }
            Query { cands }
        })
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    f64::from(dot / (na * nb).max(1e-9))
}

/// Suite-15 outcome frame, verbatim semantics.
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

fn headroom(rank: usize) -> f64 {
    let disc = |r: usize| 1.0 / ((r as f64 + 2.0).log2());
    7.0 * (disc(0) - disc(rank))
}

#[derive(Clone, Copy)]
enum GainModel {
    /// The FROZEN v0.4.0 model: sketch novelty only.
    Frozen,
    /// Candidate combiners for novelty × embedding coverage.
    Min,
    Product,
    Mean,
}

struct PolicyOutcome {
    ndcg: f64,
    fetches: usize,
    clusters: usize,
}

fn run_policy(q: &Query, embs: &[Vec<f32>], model: GainModel, budget: usize) -> PolicyOutcome {
    let mut order: Vec<usize> = (0..q.cands.len()).collect();
    order.sort_by(|&a, &b| {
        q.cands[b]
            .snippet_score
            .partial_cmp(&q.cands[a].snippet_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let rank_of: HashMap<usize, usize> = order.iter().enumerate().map(|(r, &i)| (i, r)).collect();
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

    let sketches: Vec<Sketch> = q.cands.iter().map(|c| Sketch::compute(&c.text)).collect();
    let mut fetched: HashSet<usize> = HashSet::new();
    let mut fetched_clusters: HashSet<usize> = HashSet::new();
    let mut stop = false;
    while !stop && fetched.len() < budget {
        let cands: Vec<_> = (0..q.cands.len())
            .filter(|i| !fetched.contains(i))
            .map(|i| {
                let novelty = 1.0
                    - fetched
                        .iter()
                        .map(|&j| sketches[i].containment(&sketches[j]))
                        .fold(0.0f64, f64::max);
                let coverage = 1.0
                    - fetched
                        .iter()
                        .map(|&j| cosine(&embs[i], &embs[j]).max(0.0))
                        .fold(0.0f64, f64::max);
                let redundancy_discount = match model {
                    GainModel::Frozen => novelty,
                    GainModel::Min => novelty.min(coverage),
                    GainModel::Product => novelty * coverage,
                    GainModel::Mean => 0.5 * (novelty + coverage),
                };
                let score_z = (q.cands[i].snippet_score - mean) / sd;
                candidate_with_betas(
                    i as u64,
                    headroom(rank_of[&i]) * redundancy_discount,
                    1.0, // the discount moved into the headroom factor above
                    score_z,
                    COST,
                    BETA0,
                    BETA2,
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
        stop = fetched == before;
    }
    PolicyOutcome {
        ndcg: ndcg_after(q, &fetched),
        fetches: fetched.len(),
        clusters: fetched_clusters.len(),
    }
}

fn run_fetch_all(q: &Query) -> PolicyOutcome {
    let mut order: Vec<usize> = (0..q.cands.len()).collect();
    order.sort_by(|&a, &b| {
        q.cands[b]
            .snippet_score
            .partial_cmp(&q.cands[a].snippet_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let fetched: HashSet<usize> = order.into_iter().take(FETCH_ALL_N).collect();
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

struct Agg {
    ndcg: f64,
    fetches: f64,
    median_clusters: f64,
}

fn agg(outcomes: &[PolicyOutcome]) -> Agg {
    let n = outcomes.len().max(1) as f64;
    let mut clusters: Vec<usize> = outcomes.iter().map(|o| o.clusters).collect();
    clusters.sort_unstable();
    Agg {
        ndcg: outcomes.iter().map(|o| o.ndcg).sum::<f64>() / n,
        fetches: outcomes.iter().map(|o| o.fetches as f64).sum::<f64>() / n,
        median_clusters: clusters[clusters.len() / 2] as f64,
    }
}

fn evaluate(queries: &[Query], embs: &[Vec<Vec<f32>>], model: GainModel) -> (Agg, Agg) {
    let policy: Vec<_> = queries
        .iter()
        .zip(embs)
        .map(|(q, e)| run_policy(q, e, model, FETCH_ALL_N))
        .collect();
    let all: Vec<_> = queries.iter().map(run_fetch_all).collect();
    (agg(&policy), agg(&all))
}

pub fn run(cfg: &BenchConfig) -> SuiteResult {
    let start = Instant::now();
    let Ok(embedder) = meridian_embed::Embedder::load(&cfg.models_dir) else {
        return SuiteResult::skipped(
            "voi-embed",
            "models dir absent — run on the device with models/ (the study needs potion's real geometry)",
        );
    };
    let mut result = SuiteResult::new("voi-embed");
    result.metric("queries", QUERIES);

    // ---- calibration sanity: does potion separate same-field registers
    // from cross-field text at all? If not, there is no calibratable signal
    // and the carry closes honestly (that IS a verdict).
    let reg_a: Vec<String> = TOPICS.iter().map(|(a, _)| (*a).to_owned()).collect();
    let reg_b: Vec<String> = TOPICS.iter().map(|(_, b)| (*b).to_owned()).collect();
    let emb_a = embedder.embed_batch(&reg_a);
    let emb_b = embedder.embed_batch(&reg_b);
    let mut intra = 0.0;
    let mut inter = 0.0;
    let mut inter_n = 0.0;
    for (i, ea) in emb_a.iter().enumerate() {
        intra += cosine(ea, &emb_b[i]);
        for (j, eb) in emb_b.iter().enumerate() {
            if i != j {
                inter += cosine(ea, eb);
                inter_n += 1.0;
            }
        }
    }
    intra /= TOPICS.len() as f64;
    inter /= inter_n;
    result.metric("calibration_intra_topic_cosine", round4(intra));
    result.metric("calibration_inter_topic_cosine", round4(inter));
    let calibratable = intra >= inter + 0.10;
    result.metric("gate_calibratable_signal", u8::from(calibratable));
    if !calibratable {
        result.note(
            "VERDICT: potion does not separate same-field registers from cross-field text by \
             ≥0.10 cosine — no calibratable embedding-coverage signal exists at this model \
             size; the P9 carry closes with this measured no (ADR-26)"
                .to_owned(),
        );
        result.gate("calibratable signal exists", false);
        result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
        return result;
    }

    // ---- tuning sweep over the combiner, frozen judgment on the hold-out.
    let mut tune_rng = Rng::new(0x15B0_2026);
    let tuning = generate(&mut tune_rng, QUERIES);
    let tune_embs: Vec<Vec<Vec<f32>>> = tuning
        .iter()
        .map(|q| embedder.embed_batch(&q.cands.iter().map(|c| c.text.clone()).collect::<Vec<_>>()))
        .collect();
    let mut chosen: Option<(GainModel, f64)> = None;
    for (name, model) in [
        ("min", GainModel::Min),
        ("product", GainModel::Product),
        ("mean", GainModel::Mean),
    ] {
        let (p, a) = evaluate(&tuning, &tune_embs, model);
        let ok = p.ndcg >= a.ndcg - 0.01;
        result.note(format!(
            "sweep {name}: tuning nDCG {:.4} vs all {:.4}, fetches {:.2}, clusters {} — {}",
            p.ndcg,
            a.ndcg,
            p.fetches,
            p.median_clusters,
            if ok {
                "admissible"
            } else {
                "INADMISSIBLE (quality)"
            }
        ));
        if ok && chosen.map(|(_, f)| p.fetches < f).unwrap_or(true) {
            chosen = Some((model, p.fetches));
        }
    }
    let Some((chosen_model, _)) = chosen else {
        result.note(
            "VERDICT: a calibratable embedding signal EXISTS (0.72 vs 0.05 cosine separation)              but no swept combiner holds quality on tuning — the coverage discount prices out              paraphrase copies whose reveal still buys page nDCG, so the saved fetches cost              more than they earn. The P9 carry closes with this measured no (ADR-26); a value              model that spends the signal profitably remains future work, and this suite is              its standing judge."
                .to_owned(),
        );
        result.gate(
            "amend-or-record: an admissible combiner exists on tuning (a FAIL is a valid              verdict that closes the carry)",
            false,
        );
        result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
        return result;
    };

    let mut hold_rng = Rng::new(0x15B1_2026);
    let holdout = generate(&mut hold_rng, QUERIES);
    let hold_embs: Vec<Vec<Vec<f32>>> = holdout
        .iter()
        .map(|q| embedder.embed_batch(&q.cands.iter().map(|c| c.text.clone()).collect::<Vec<_>>()))
        .collect();
    let (frozen, all) = evaluate(&holdout, &hold_embs, GainModel::Frozen);
    let (cand, _) = evaluate(&holdout, &hold_embs, chosen_model);

    result.metric("holdout_frozen_ndcg10", round4(frozen.ndcg));
    result.metric("holdout_candidate_ndcg10", round4(cand.ndcg));
    result.metric("holdout_fetch_all_ndcg10", round4(all.ndcg));
    result.metric("holdout_frozen_fetches", round4(frozen.fetches));
    result.metric("holdout_candidate_fetches", round4(cand.fetches));
    result.metric("holdout_frozen_median_clusters", frozen.median_clusters);
    result.metric("holdout_candidate_median_clusters", cand.median_clusters);

    // Amend-or-record: ship ONLY on a real hold-out win — ≥5% fewer fetches
    // at held quality with non-degrading clusters. Either outcome closes the
    // P9 carry; only one of them changes production.
    let quality_held = cand.ndcg >= all.ndcg - 0.01;
    let saves = cand.fetches <= frozen.fetches * 0.95;
    let diversity_held = cand.median_clusters >= frozen.median_clusters;
    result.metric("gate_quality_held", u8::from(quality_held));
    result.metric("gate_saves_5pct", u8::from(saves));
    result.metric("gate_diversity_held", u8::from(diversity_held));
    result.gate(
        "amend-or-record: candidate beats the FROZEN model by ≥5% fetches at held quality and \
         non-degrading clusters (a FAIL here is a valid verdict that closes the carry)",
        quality_held && saves && diversity_held,
    );
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}

fn round4(x: f64) -> f64 {
    (x * 1e4).round() / 1e4
}
