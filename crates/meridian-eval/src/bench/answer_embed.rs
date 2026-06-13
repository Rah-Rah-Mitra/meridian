//! Suite 18b — `answer_embed`: embedding-redundancy pruning for ANSWER mode
//! (roadmap B1, §8.5). The standing judge for the embedding-coverage carry is
//! suite-15b (`voi-embed`); that study closed a measured NO on the PAGE
//! objective because the paraphrase copies whose reveal the coverage discount
//! prices out still buy page nDCG mass. B1 asks the SAME signal a different
//! question, in the regime where that failure mode cannot exist.
//!
//! The case under study (the suite-15b motivating case, now in answer units):
//! a PARAPHRASE copy of an already-fetched original shares few exact tokens
//! with it — MinHash containment LOW → sketch-novelty HIGH → the shipped
//! single-best selector treats it as fresh evidence and spends a scarce
//! budget-2 fetch on it — while living in the same semantic neighbourhood
//! (embedding cosine HIGH → ~zero reveal reward, because syndication degrades
//! the passage: a copy is a strictly worse place to read the answer than its
//! original). In ANSWER mode the page-objective failure mode of 15b does NOT
//! exist: there is no additive rank mass for the paraphrase reveal to buy —
//! the only thing a fetch can earn is the single best passage. So a redundant
//! paraphrase fetch is pure waste, and the embedding term should reclaim it.
//!
//! Corpus (modelled on `voi_embed` for the texts/embeddings and on `answer`
//! for the outcome): per query 20 candidates — 3 decisive originals (ONE
//! carries THE answer passage), each with one TOKEN copy (high containment —
//! the shipped sketch term already prices it out) and one PARAPHRASE copy
//! (low containment, high cosine — only the embedding sees the redundancy),
//! plus chaff. Texts are sampled from the suite-15b two-register topic pool so
//! potion's REAL geometry decides the cosines; passage-CE follows the suite-18
//! answer semantics (a copy's best passage is materially worse than its
//! original's). Needs `models/`; skips (not passes) without it.
//!
//! Selectors (both `pandora_walk` in passage-CE units, budget 2, ADR-29):
//! - SHIPPED: gain = sketch_novelty = 1 − max sketch-containment to fetched;
//! - CANDIDATE: gain = sketch_novelty discounted by the embedding-novelty
//!   nu_emb = 1 − max cosine to already-fetched embeddings. Combiner swept:
//!   soft `min(sketch_novelty, nu_emb)` and a hard prune at
//!   tau ∈ {0.3,0.4,0.5,0.6} (a candidate with nu_emb < tau is pruned to ~0
//!   gain). Tuning seed picks the combiner; the hold-out judges it (risk-#21).
//!
//! GATE (amend-or-record, roadmap B1): on the paraphrase-heavy hold-out the
//! candidate must be hit-rate NON-DEGRADING AND (≥15% fewer fetches OR
//! hit-rate +2pp). KILL/record-NO if EVERY swept tau trades >1pp hit-rate for
//! its fetch saving (the exact suite-15b failure shape) — then this records
//! the NO and closes the 15b nomination for answer mode too. The suite PASSES
//! when it validly adjudicates (an AMEND or a decisive record-NO); it FAILS
//! only when inconclusive / underpowered (MDE pre-registered as a note).

use super::{BenchConfig, SuiteResult};
use crate::stats::Rng;
use meridian_fetch::voi::{Candidate, pandora_walk};
use meridian_index::sketch::Sketch;
use std::collections::HashSet;
use std::time::Instant;

const CANDIDATES: usize = 20;
const DECISIVE: usize = 3;
const QUERIES: usize = 1000;
/// Production default `search.deep_fetch_max` — the budget the gate judges.
const BUDGET: usize = 2;
/// The hard-prune thresholds swept on tuning (a candidate whose embedding
/// novelty falls below tau is treated as redundant and pruned to ~0 gain).
const TAU_SWEEP: [f64; 4] = [0.3, 0.4, 0.5, 0.6];
/// Answer-mode per-fetch cost — frozen from suite-18's tuning sweep (0.2),
/// NOT re-swept here (this suite tunes the embedding combiner only).
const ANSWER_FETCH_COST: f64 = 0.2;

/// Real-word semantic fields, two registers each (register B paraphrases
/// register A with minimal exact-token overlap). Verbatim from suite-15b — the
/// study needs potion's ACTUAL geometry, not synthetic tokens.
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
    /// The decisive/chaff cluster id — asserted by the generator tests; the
    /// selectors key redundancy off the texts (sketch + embedding), not this.
    #[allow(dead_code)]
    cluster: usize,
    /// The full text's best (query, passage) CE — revealed on fetch.
    passage_ce: f64,
    /// What snippet-level CE sees without fetching (ambiguous).
    snippet_ce: f64,
    is_answer: bool,
    /// Real text — drives the MinHash sketch and the potion embedding.
    text: String,
}

struct Query {
    cands: Vec<Cand>,
}

fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

fn sample_words(pool: &str, n: usize, rng: &mut Rng) -> String {
    let words: Vec<&str> = pool.split_whitespace().collect();
    (0..n)
        .map(|_| words[rng.below(words.len())])
        .collect::<Vec<_>>()
        .join(" ")
}

/// Paraphrase-heavy answer corpus: each decisive original (one carries the
/// answer) gets a TOKEN copy (high containment, already priced by sketch) and
/// a PARAPHRASE copy (low containment, high cosine — the case under study).
/// Copies degrade the passage exactly as in suite-18: a copy is a worse place
/// to read the answer than its original.
fn generate(rng: &mut Rng, queries: usize) -> Vec<Query> {
    (0..queries)
        .map(|_| {
            let answer_at = rng.below(DECISIVE);
            let mut cands = Vec::with_capacity(CANDIDATES);
            // Three decisive topics, distinct per query.
            let mut topic_ids: Vec<usize> = (0..TOPICS.len()).collect();
            for d in 0..DECISIVE {
                let pick = d + rng.below(topic_ids.len() - d);
                topic_ids.swap(d, pick);
            }
            for d in 0..DECISIVE {
                let (reg_a, reg_b) = TOPICS[topic_ids[d]];
                let is_answer = d == answer_at;
                let passage = if is_answer {
                    clamp01(0.85 + 0.05 * rng.next_gaussian() as f64)
                } else {
                    clamp01(0.55 + 0.10 * rng.next_gaussian() as f64)
                };
                let original = sample_words(reg_a, 30, rng);
                cands.push(Cand {
                    cluster: d,
                    passage_ce: passage,
                    snippet_ce: clamp01(passage - 0.25 + 0.18 * rng.next_gaussian() as f64),
                    is_answer,
                    text: original.clone(),
                });
                // TOKEN copy: the original's words re-sampled from the same
                // register plus filler — high MinHash containment, the shipped
                // sketch term already prices it out.
                let token_passage = clamp01(passage - 0.25 + 0.05 * rng.next_gaussian() as f64);
                let token_copy = format!("{} {}", original, sample_words(FILLER, 4, rng));
                cands.push(Cand {
                    cluster: d,
                    passage_ce: token_passage,
                    snippet_ce: clamp01(token_passage - 0.25 + 0.18 * rng.next_gaussian() as f64),
                    is_answer: false,
                    text: token_copy,
                });
                // PARAPHRASE copy: same semantic field, OTHER register — low
                // containment (sketch-novelty HIGH, looks fresh) but high
                // cosine (embedding-novelty LOW). The degraded passage means
                // fetching it is near-pure waste; only the embedding sees that.
                let para_passage = clamp01(passage - 0.25 + 0.05 * rng.next_gaussian() as f64);
                let paraphrase = sample_words(reg_b, 30, rng);
                cands.push(Cand {
                    cluster: d,
                    passage_ce: para_passage,
                    snippet_ce: clamp01(para_passage - 0.25 + 0.18 * rng.next_gaussian() as f64),
                    is_answer: false,
                    text: paraphrase,
                });
            }
            while cands.len() < CANDIDATES {
                let c = cands.len();
                // Chaff: generic filler plus an unrelated topic's scattered
                // words — its own cluster, no planted redundancy.
                let stray = TOPICS[rng.below(TOPICS.len())].0;
                let passage = clamp01(0.15 + 0.10 * rng.next_gaussian() as f64);
                let text = format!(
                    "{} {}",
                    sample_words(FILLER, 8, rng),
                    sample_words(stray, 3, rng)
                );
                cands.push(Cand {
                    cluster: 100 + c,
                    passage_ce: passage,
                    snippet_ce: clamp01(passage - 0.05 + 0.18 * rng.next_gaussian() as f64),
                    is_answer: false,
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

#[derive(Clone, Copy)]
enum Selector {
    /// SHIPPED: gain = sketch-novelty only (1 − max sketch-containment).
    Shipped,
    /// CANDIDATE, soft combiner: gain = min(sketch_novelty, nu_emb).
    SoftMin,
    /// CANDIDATE, hard prune: gain = sketch_novelty, but pruned to ~0 when
    /// nu_emb < tau (the paraphrase is embedding-redundant with what we hold).
    HardPrune(f64),
}

struct Outcome {
    hit: bool,
    fetches: usize,
}

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

/// Answer mode (suite-18 semantics), with the gain re-posed between opens over
/// REAL sketch-novelty and (for the candidate) embedding-novelty against the
/// docs actually fetched so far. hit = the best fetched passage belongs to the
/// answer original.
fn run_selector(
    q: &Query,
    sketches: &[Sketch],
    embs: &[Vec<f32>],
    sel: Selector,
    budget: usize,
) -> Outcome {
    let (mean, sd) = score_stats(q);
    let mut fetched: HashSet<usize> = HashSet::new();
    let mut best_passage: Option<(usize, f64)> = None;
    let mut best_value = 0.0f64; // no passage in hand before the first fetch

    loop {
        if fetched.len() >= budget {
            break;
        }
        let cands: Vec<Candidate> = (0..q.cands.len())
            .filter(|i| !fetched.contains(i))
            .map(|i| {
                let sketch_novelty = if fetched.is_empty() {
                    1.0
                } else {
                    1.0 - fetched
                        .iter()
                        .map(|&j| sketches[i].containment(&sketches[j]))
                        .fold(0.0f64, f64::max)
                };
                let nu_emb = if fetched.is_empty() {
                    1.0
                } else {
                    1.0 - fetched
                        .iter()
                        .map(|&j| cosine(&embs[i], &embs[j]).max(0.0))
                        .fold(0.0f64, f64::max)
                };
                let gain = match sel {
                    Selector::Shipped => sketch_novelty,
                    Selector::SoftMin => sketch_novelty.min(nu_emb),
                    Selector::HardPrune(tau) => {
                        if nu_emb < tau {
                            0.001 // embedding-redundant → priced out
                        } else {
                            sketch_novelty
                        }
                    }
                };
                Candidate {
                    id: i as u64,
                    p: p_of(q, i, mean, sd),
                    gain,
                    cost: ANSWER_FETCH_COST,
                }
            })
            .collect();
        let before = fetched.len();
        let r = pandora_walk(&cands, best_value, 1, |id| {
            let i = id as usize;
            fetched.insert(i);
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

/// Pre-compute the per-query sketches and potion embeddings once.
fn build_signals(
    embedder: &meridian_embed::Embedder,
    queries: &[Query],
) -> (Vec<Vec<Sketch>>, Vec<Vec<Vec<f32>>>) {
    let sketches: Vec<Vec<Sketch>> = queries
        .iter()
        .map(|q| q.cands.iter().map(|c| Sketch::compute(&c.text)).collect())
        .collect();
    let embs: Vec<Vec<Vec<f32>>> = queries
        .iter()
        .map(|q| embedder.embed_batch(&q.cands.iter().map(|c| c.text.clone()).collect::<Vec<_>>()))
        .collect();
    (sketches, embs)
}

fn eval(queries: &[Query], sketches: &[Vec<Sketch>], embs: &[Vec<Vec<f32>>], sel: Selector) -> Agg {
    let out: Vec<Outcome> = queries
        .iter()
        .zip(sketches)
        .zip(embs)
        .map(|((q, sk), em)| run_selector(q, sk, em, sel, BUDGET))
        .collect();
    agg(&out)
}

pub fn run(cfg: &BenchConfig) -> SuiteResult {
    let start = Instant::now();
    let Ok(embedder) = meridian_embed::Embedder::load(&cfg.models_dir) else {
        return SuiteResult::skipped(
            "answer_embed",
            "models dir absent — run on the device with models/ (the study needs potion's real geometry)",
        );
    };
    let mut result = SuiteResult::new("answer_embed");
    result.metric("queries", QUERIES);
    result.metric("budget", BUDGET);

    // ---- calibration sanity (same protocol as suite-15b): potion must
    // separate same-field registers from cross-field text, else there is no
    // signal to spend and the nomination closes honestly.
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
             ≥0.10 cosine — no embedding-novelty signal exists at this model size; B1 closes \
             with this measured no and the suite-15b nomination closes for answer mode too"
                .to_owned(),
        );
        // amend-or-record idiom: a decisive record-NO IS a valid adjudication.
        result.metric("verdict", "record_no_no_signal");
        result.gate(
            "amend-or-record: the suite validly adjudicates B1 (amend OR a decisive record-no)",
            true,
        );
        result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
        return result;
    }

    // Pre-register the MDE for the hit-rate proportion gate (risk-#21):
    // 2.8·sqrt(2·p·(1−p)/n) at the shipped baseline p, n = QUERIES.
    // (Filled with the actual shipped p below; noted for transparency.)

    // ---- tuning seed: sweep the combiner (soft-min + the four hard taus),
    // pick by the B1 rule (non-degrading hit-rate, then fewest fetches).
    let mut tune_rng = Rng::new(0x18B0_2026);
    let tuning = generate(&mut tune_rng, QUERIES);
    let (tune_sk, tune_em) = build_signals(&embedder, &tuning);
    let tune_shipped = eval(&tuning, &tune_sk, &tune_em, Selector::Shipped);
    result.metric("tuning_shipped_hit_rate", round3(tune_shipped.hit_rate));
    result.metric(
        "tuning_shipped_mean_fetches",
        round3(tune_shipped.mean_fetches),
    );

    // Pre-registered MDE on the tuning baseline proportion.
    let p_base = tune_shipped.hit_rate.clamp(1e-3, 1.0 - 1e-3);
    let mde = 2.8 * (2.0 * p_base * (1.0 - p_base) / QUERIES as f64).sqrt();
    result.metric("mde_hit_rate_pp", round4(mde * 100.0));
    result.note(format!(
        "pre-registered MDE (proportion gate, 2.8·sqrt(2·p·(1−p)/n), p={:.3}, n={QUERIES}) = \
         {:.2}pp; the +2pp hit-rate arm of the gate is {} at this n",
        p_base,
        mde * 100.0,
        if mde * 100.0 <= 2.0 {
            "powered"
        } else {
            "UNDERPOWERED"
        }
    ));

    let combiners: Vec<(&str, Selector)> = {
        let mut v: Vec<(&str, Selector)> = vec![("soft_min", Selector::SoftMin)];
        // hard-prune taus, labelled.
        v.push(("hard_tau_0.3", Selector::HardPrune(TAU_SWEEP[0])));
        v.push(("hard_tau_0.4", Selector::HardPrune(TAU_SWEEP[1])));
        v.push(("hard_tau_0.5", Selector::HardPrune(TAU_SWEEP[2])));
        v.push(("hard_tau_0.6", Selector::HardPrune(TAU_SWEEP[3])));
        v
    };

    // B1 selection rule on tuning: hit-rate non-degrading (≥ shipped − 1pp
    // slack), then maximise fetch savings; record every arm.
    let mut chosen: Option<(&str, Selector, f64)> = None; // (name, sel, fetches)
    // KILL-shape tracker: does EVERY tau trade >1pp hit-rate for its savings?
    let mut any_non_degrading_saver = false;
    for (name, sel) in &combiners {
        let a = eval(&tuning, &tune_sk, &tune_em, *sel);
        let drop_pp = (tune_shipped.hit_rate - a.hit_rate) * 100.0;
        let save_pct = if tune_shipped.mean_fetches > 0.0 {
            (1.0 - a.mean_fetches / tune_shipped.mean_fetches) * 100.0
        } else {
            0.0
        };
        let non_degrading = a.hit_rate >= tune_shipped.hit_rate - 0.01;
        let saves = save_pct > 0.0;
        if non_degrading && saves {
            any_non_degrading_saver = true;
        }
        result.note(format!(
            "sweep {name}: tuning hit_rate={:.3} (Δ={:+.2}pp) mean_fetches={:.2} (save {:+.1}%) — {}",
            a.hit_rate,
            -drop_pp,
            a.mean_fetches,
            save_pct,
            if non_degrading {
                "non-degrading"
            } else {
                "DEGRADES hit-rate"
            }
        ));
        if non_degrading && chosen.map(|(_, _, f)| a.mean_fetches < f).unwrap_or(true) {
            chosen = Some((name, *sel, a.mean_fetches));
        }
    }

    // ---- KILL/record-NO check (the suite-15b failure shape): if NO combiner
    // is a non-degrading saver on tuning, every arm trades hit-rate for its
    // fetches — record the NO and close the 15b nomination for answer mode.
    let Some((chosen_name, chosen_sel, _)) = chosen.filter(|_| any_non_degrading_saver) else {
        result.metric("verdict", "record_no_kill_shape");
        result.note(
            "VERDICT (record-NO): every swept combiner trades >1pp hit-rate for its fetch \
             saving on tuning — the exact suite-15b failure shape, now reproduced in answer \
             mode. The embedding-redundancy discount prices out paraphrase fetches that the \
             single-best regime still needed; B1 closes with this measured no and the 15b \
             embedding-coverage nomination closes for answer mode too."
                .to_owned(),
        );
        // amend-or-record: a decisive record-NO is a valid adjudication → PASS.
        result.gate(
            "amend-or-record: the suite validly adjudicates B1 (amend OR a decisive record-no)",
            true,
        );
        result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
        return result;
    };
    result.metric("chosen_combiner", chosen_name);

    // ---- FROZEN hold-out judgment (risk-#21): the tuning-chosen combiner vs
    // the shipped selector on an unseen seed.
    let mut hold_rng = Rng::new(0x18B1_2026);
    let holdout = generate(&mut hold_rng, QUERIES);
    let (hold_sk, hold_em) = build_signals(&embedder, &holdout);
    let shipped = eval(&holdout, &hold_sk, &hold_em, Selector::Shipped);
    let cand = eval(&holdout, &hold_sk, &hold_em, chosen_sel);

    result.metric("holdout_shipped_hit_rate", round3(shipped.hit_rate));
    result.metric("holdout_candidate_hit_rate", round3(cand.hit_rate));
    result.metric("holdout_shipped_mean_fetches", round3(shipped.mean_fetches));
    result.metric("holdout_candidate_mean_fetches", round3(cand.mean_fetches));

    let hit_delta_pp = (cand.hit_rate - shipped.hit_rate) * 100.0;
    let fetch_save_pct = if shipped.mean_fetches > 0.0 {
        (1.0 - cand.mean_fetches / shipped.mean_fetches) * 100.0
    } else {
        0.0
    };
    result.metric("holdout_hit_delta_pp", round3(hit_delta_pp));
    result.metric("holdout_fetch_save_pct", round3(fetch_save_pct));

    result.note(format!(
        "answer_embed = pandora_walk in passage-CE units (gain = sketch_novelty for SHIPPED, \
         discounted by nu_emb=1−max cosine for the CANDIDATE [{chosen_name}], cost {ANSWER_FETCH_COST} \
         frozen from suite-18, budget {BUDGET}); embeddings = potion-base-8M, sketches = real MinHash; \
         hold-out frozen from a distinct seed (risk-#21)"
    ));

    // ---- B1 amend gate: hit-rate non-degrading AND (≥15% fewer fetches OR
    // hit-rate +2pp).
    let non_degrading = cand.hit_rate >= shipped.hit_rate - 1e-12;
    let fewer_fetches = fetch_save_pct >= 15.0;
    let hit_lift = hit_delta_pp >= 2.0;
    let amend = non_degrading && (fewer_fetches || hit_lift);
    result.metric("gate_non_degrading", u8::from(non_degrading));
    result.metric("gate_fewer_fetches_15pct", u8::from(fewer_fetches));
    result.metric("gate_hit_lift_2pp", u8::from(hit_lift));

    if amend {
        result.metric("verdict", "amend_ship_embedding_prune");
        result.note(format!(
            "VERDICT (AMEND): the embedding-redundancy discount [{chosen_name}] holds hit-rate \
             ({:+.2}pp) and reclaims {:.1}% of the budget-2 fetches on paraphrase-heavy answer \
             traffic — the signal that closed a NO on the PAGE objective (suite-15b) pays off in \
             the single-best regime where redundant paraphrase reveals buy nothing. Recommend \
             wiring nu_emb into voi.rs (deferred to the orchestrator).",
            hit_delta_pp, fetch_save_pct
        ));
    } else {
        // Non-amend on the hold-out: a clean record-NO (still a valid
        // adjudication — the tuning combiner did not generalise / the win was
        // not decisive on the frozen seed).
        result.metric("verdict", "record_no_holdout");
        result.note(format!(
            "VERDICT (record-NO): the tuning-chosen combiner [{chosen_name}] did not clear the B1 \
             amend gate on the FROZEN hold-out (hit Δ={:+.2}pp, fetch save={:.1}%); non-degrading={}, \
             ≥15%-fewer={}, +2pp-hit={}. B1 closes with this measured no on the hold-out.",
            hit_delta_pp,
            fetch_save_pct,
            non_degrading,
            fewer_fetches,
            hit_lift
        ));
    }

    // amend-or-record idiom: the suite PASSES when it validly adjudicates
    // (AMEND or a decisive record-NO); the underpowered/inconclusive case is
    // the only FAIL. The MDE arm is checked: if the +2pp arm is underpowered
    // AND the fetch arm did not decide, the verdict is inconclusive.
    let powered = mde * 100.0 <= 2.0;
    let decisive = amend || fewer_fetches || hit_lift || !non_degrading || powered;
    // A clean record-NO on the hold-out (gate did not amend) is decisive when
    // the fetch arm is observable (always, n=QUERIES) — the only inconclusive
    // case is when neither arm moved AND the +2pp arm is underpowered.
    let inconclusive = !amend && !fewer_fetches && fetch_save_pct.abs() < 1e-9 && !powered;
    result.metric("verdict_powered", u8::from(powered));
    let valid_adjudication = decisive && !inconclusive;
    result.gate(
        "amend-or-record: the suite validly adjudicates B1 (amend OR a decisive record-no; \
         FAIL only if inconclusive/underpowered)",
        valid_adjudication,
    );
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn round4(x: f64) -> f64 {
    (x * 1e4).round() / 1e4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_plants_one_answer_per_query() {
        let mut rng = Rng::new(7);
        let qs = generate(&mut rng, 20);
        for q in &qs {
            assert_eq!(q.cands.len(), CANDIDATES);
            assert_eq!(q.cands.iter().filter(|c| c.is_answer).count(), 1);
            let ans = q.cands.iter().find(|c| c.is_answer).unwrap();
            assert!(ans.passage_ce > 0.6, "answer passage must be strong");
        }
    }

    #[test]
    fn each_decisive_has_a_paraphrase_and_token_copy() {
        let mut rng = Rng::new(9);
        let qs = generate(&mut rng, 10);
        for q in &qs {
            // 3 decisive clusters, each with exactly 3 members (orig+token+para).
            for d in 0..DECISIVE {
                let n = q.cands.iter().filter(|c| c.cluster == d).count();
                assert_eq!(n, 3, "cluster {d} must have original + token + paraphrase");
            }
        }
    }
}
