//! Suite 20 — `answer_trust`: the answer-mode trust layer judge (roadmap
//! §8.2–8.3 / 06-roadmap-completion.md bet 1). Two trust signals over a planted
//! corpus:
//!
//! - **C1 corroboration** — does the winning passage's claim appear in passages
//!   from *distinct* ADR-18 evidence clusters? `independent_clusters =
//!   |{ cluster(d) : d CE-supports the claim, cluster(d) != C(w) }|`. Same-cluster
//!   syndicated copies must **never** count (a false "k independent sources agree"
//!   badge is worse than none — the conformal lesson at claim level). This arm
//!   validates the **production support signal** (the real ort cross-encoder over
//!   real sketch clusters), not a synthetic stand-in:
//!     - **Arm A (hermetic, CI, no model):** plant claim text — a claim, near-dup
//!       *syndicated copies* (high lexical overlap → same cluster, the trap),
//!       *independent restatements* (same entities, reordered → distinct cluster),
//!       and *chaff* (off-topic + a hard "adjacent" fraction sharing one entity).
//!       Run the SHIPPED `meridian_index::sketch` + `evidence::cluster_tags` and
//!       assert every copy stays in the claim's cluster (zero same-cluster
//!       leakage by construction) while restatements split off. Build-profile
//!       independent.
//!     - **Arm B (gnu-only, `bench-ce-real`, needs `models/`):** for each distinct
//!       non-claim cluster, score `Reranker::rerank(claim_text, [canonical
//!       snippet])` with the real INT8 ms-marco CE and count it iff `ce ≥ τ_corr`.
//!       Sweep `τ_corr` on the tuning seed (max recall s.t. precision ≥0.9),
//!       FREEZE, judge on a disjoint hold-out AND a query-STYLE shift. Reports
//!       cluster-level precision/recall + a same-cluster-leakage probe.
//! - **H3 abstention** — from the per-query (winner ce_score, hit) pairs, publish
//!   the coverage-vs-selective-hit curve, choose `τ* = max{τ : abstain ≤ 20%}`
//!   maximising selective hit, and check it DOMINATES always-show (≥+5pp) on the
//!   hold-out AND holds within 10pp on a query-STYLE shift (the suite-16
//!   no-collapse falsifier). Selective prediction WITHOUT a coverage guarantee.
//!
//! Support rule (deviation from the §8.2 sketch, documented): the conservative
//! bar is the ABSOLUTE relevance `ce(claim, snippet) ≥ τ_corr`. The illustrative
//! `ce ≥ best_passage.ce_score − δ` term mixed two incomparable CE pairings
//! (query↔passage vs passage↔snippet), so the implementation uses the absolute,
//! swept-and-frozen bar — which is what the precision gate actually validates.
//!
//! Protocol (risk #21): thresholds chosen on a tuning seed, judged FROZEN on a
//! disjoint hold-out AND a style-shift variant. MDE pre-registered (J3).
//!
//! Gate (combined, one slot): C1 cluster-precision ≥0.9 (tuning + style) AND
//! cluster-recall ≥0.6 (hold-out) AND zero same-cluster leakage (Arm A, all
//! variants); H3 selective hit ≥ baseline+5pp at ≤20% abstention (hold-out) AND
//! no >10pp collapse on the style variant. When `bench-ce-real` is off, the CE
//! precision/recall half is DEFERRED to the gnu run and the gate covers the
//! hermetic half (H3 + zero-leakage); a clean NO on any arm is recorded honestly.

use super::{BenchConfig, SuiteResult};
use crate::stats::Rng;
use meridian_fetch::voi::{Candidate, pandora_walk};
use meridian_index::sketch::Sketch;
use meridian_query::evidence::cluster_tags;
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;

const QUERIES: usize = 1000; // H3 arm (synthetic, model-free, fast)
const CE_QUERIES: usize = 360; // C1 claim corpus (model-bound in arm B)
const CANDIDATES: usize = 20; // H3 candidate set
const COPIES_PER: usize = 2; // syndicated copies per decisive original
const BUDGET: usize = 2; // production deep_fetch_max
const ABSTAIN_CAP: f64 = 0.20; // H3: at most 20% abstention
const H3_MARGIN: f64 = 0.05; // selective hit must beat always-show by ≥5pp
const NO_COLLAPSE: f64 = 0.10; // style-shift selective hit within 10pp

/// τ_corr sweep grid over ms-marco-MiniLM-L6 relevance logits (≈ −11 dis-relevant
/// … +11 strongly relevant, boundary ~0). The sweep picks the smallest τ (max
/// recall) that still clears precision ≥0.9 on the TUNING seed, then freezes it.
/// (CE-arm constants — only referenced when the real reranker is compiled in.)
#[cfg(feature = "bench-ce-real")]
const TAU_CORR_GRID: &[f64] = &[
    -4.0, -3.0, -2.0, -1.0, 0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 6.0, 7.0, 8.0,
    9.0, 10.0,
];
#[cfg(feature = "bench-ce-real")]
const C1_PRECISION_BAR: f64 = 0.9;
#[cfg(feature = "bench-ce-real")]
const C1_RECALL_BAR: f64 = 0.6;

fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

// ===========================================================================
// H3 abstention arm (synthetic, model-free — unchanged framing from suite 18)
// ===========================================================================

struct Cand {
    cluster: usize,
    /// Realised on fetch: the doc's best (query, passage) CE.
    passage_ce: f64,
    /// Pre-fetch snippet-level CE (ambiguous).
    snippet_ce: f64,
    is_answer: bool,
}

struct Query {
    cands: Vec<Cand>,
}

/// `style` shifts the score distributions to emulate a query-style change
/// (suite-16 falsifier): the answer/non-answer CE separation is compressed and
/// the whole snippet signal shifted down, so a frozen τ behaves differently.
fn generate(rng: &mut Rng, n: usize, style: bool) -> Vec<Query> {
    let sep = if style { 0.18 } else { 0.30 };
    let snip_shift = if style { -0.12 } else { 0.0 };
    (0..n)
        .map(|_| {
            let has_independent = rng.next_f32() < 0.5;
            let syndication_only = !has_independent && rng.next_f32() < 0.5;
            let mut cands = Vec::with_capacity(CANDIDATES);

            let ans_ce = clamp01(0.55 + sep + 0.05 * rng.next_gaussian() as f64);
            cands.push(Cand {
                cluster: 0,
                passage_ce: ans_ce,
                snippet_ce: clamp01(ans_ce - 0.25 + snip_shift + 0.18 * rng.next_gaussian() as f64),
                is_answer: true,
            });
            if has_independent || syndication_only {
                for _ in 0..COPIES_PER {
                    let copy = clamp01(ans_ce - 0.25 + 0.05 * rng.next_gaussian() as f64);
                    cands.push(Cand {
                        cluster: 0,
                        passage_ce: copy,
                        snippet_ce: clamp01(
                            copy - 0.25 + snip_shift + 0.18 * rng.next_gaussian() as f64,
                        ),
                        is_answer: false,
                    });
                }
            }
            if has_independent {
                let k = 1 + rng.below(2);
                for j in 0..k {
                    let ce = clamp01(0.55 + 0.10 * rng.next_gaussian() as f64);
                    cands.push(Cand {
                        cluster: 10 + j,
                        passage_ce: ce,
                        snippet_ce: clamp01(
                            ce - 0.25 + snip_shift + 0.18 * rng.next_gaussian() as f64,
                        ),
                        is_answer: false,
                    });
                }
            }
            while cands.len() < CANDIDATES {
                let c = cands.len();
                let ce = clamp01(0.15 + 0.10 * rng.next_gaussian() as f64);
                cands.push(Cand {
                    cluster: 100 + c,
                    passage_ce: ce,
                    snippet_ce: clamp01(ce - 0.05 + snip_shift + 0.18 * rng.next_gaussian() as f64),
                    is_answer: false,
                });
            }
            Query { cands }
        })
        .collect()
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

fn p_of(snippet_ce: f64, mean: f64, sd: f64) -> f64 {
    let z = ((snippet_ce - mean) / sd).clamp(-3.0, 3.0);
    (meridian_fetch::voi::P_BETA0 + meridian_fetch::voi::P_BETA2 * z).clamp(0.05, 0.95)
}

struct AnswerOutcome {
    hit: bool,
    winner_ce: f64,
}

/// Answer-mode pandora_walk (passage-CE units), budget 2 — identical framing to
/// suite 18. Returns the winning passage's CE for the trust layer.
fn run_answer(q: &Query) -> AnswerOutcome {
    let (mean, sd) = score_stats(q);
    let mut fetched: HashSet<usize> = HashSet::new();
    let mut read_clusters: HashSet<usize> = HashSet::new();
    let mut best: Option<(usize, f64)> = None;
    let mut best_value = 0.0f64;
    loop {
        if fetched.len() >= BUDGET {
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
                    p: p_of(q.cands[i].snippet_ce, mean, sd),
                    gain: novelty,
                    cost: 0.2,
                }
            })
            .collect();
        let before = fetched.len();
        let r = pandora_walk(&cands, best_value, 1, |id| {
            let i = id as usize;
            fetched.insert(i);
            read_clusters.insert(q.cands[i].cluster);
            let ce = q.cands[i].passage_ce;
            if best.map(|(_, b)| ce > b).unwrap_or(true) {
                best = Some((i, ce));
            }
            ce
        });
        for id in r.opened {
            best_value = best_value.max(q.cands[id as usize].passage_ce);
        }
        if fetched.len() == before {
            break;
        }
    }
    match best {
        Some((i, ce)) => AnswerOutcome {
            hit: q.cands[i].is_answer,
            winner_ce: ce,
        },
        None => AnswerOutcome {
            hit: false,
            winner_ce: 0.0,
        },
    }
}

struct H3Curve {
    baseline_hit: f64,
    points: Vec<(f64, f64, f64)>, // (tau, coverage, selective_hit)
}

fn eval_h3(queries: &[Query]) -> H3Curve {
    let outs: Vec<AnswerOutcome> = queries.iter().map(run_answer).collect();
    let n = outs.len().max(1) as f64;
    let baseline_hit = outs.iter().filter(|o| o.hit).count() as f64 / n;
    let mut points = Vec::new();
    let mut tau = 0.0;
    while tau <= 0.95 {
        let shown: Vec<&AnswerOutcome> = outs.iter().filter(|o| o.winner_ce >= tau).collect();
        let coverage = shown.len() as f64 / n;
        let selective_hit = if shown.is_empty() {
            1.0
        } else {
            shown.iter().filter(|o| o.hit).count() as f64 / shown.len() as f64
        };
        points.push((tau, coverage, selective_hit));
        tau += 0.05;
    }
    H3Curve {
        baseline_hit,
        points,
    }
}

fn choose_tau(curve: &H3Curve) -> (f64, f64, f64) {
    curve
        .points
        .iter()
        .filter(|(_, cov, _)| *cov >= 1.0 - ABSTAIN_CAP)
        .copied()
        .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap())
        .unwrap_or((0.0, 1.0, curve.baseline_hit))
}

fn selective_at(curve: &H3Curve, tau: f64) -> (f64, f64) {
    let mut best = (1.0, curve.baseline_hit);
    for (t, cov, sh) in &curve.points {
        if *t >= tau - 1e-9 {
            best = (*cov, *sh);
            break;
        }
    }
    best
}

// ===========================================================================
// C1 claim corpus (natural language — the real CE must score it)
// ===========================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Claim,
    Copy,
    Independent,
    Chaff,
}

struct ClaimCand {
    text: String,
    kind: Kind,
}

struct ClaimQuery {
    cands: Vec<ClaimCand>, // index 0 is always the Claim
    /// ground truth: this query carries ONLY same-cluster syndicated copies (no
    /// independent supporter) — the precision trap (must never flag corroborated).
    syndication_only: bool,
}

/// Multiword entities kept ≤3 words so no 4-word shingle lies wholly inside one
/// entity — entities anchor CE relevance without manufacturing shared shingles
/// between a claim and its independent restatement.
const SUBJECTS: &[&str] = &[
    "the Harwick reservoir",
    "the Calbourne viaduct",
    "the Aldous observatory",
    "the Pendleton mill",
    "the Tamsin barrage",
    "the Sefton lighthouse",
    "the Marlow foundry",
    "the Renton aqueduct",
    "the Wexford granary",
    "the Dunmore colliery",
    "the Aldridge cannery",
    "the Bromley signal box",
    "the Castleton weir",
    "the Errol distillery",
    "the Fenwick boatyard",
    "the Garrow smelter",
    "the Holloway tannery",
    "the Inglewood kiln",
    "the Jarrow drydock",
    "the Kelmscott pumphouse",
    "the Lyndon brewery",
    "the Morley depot",
    "the Norbury sawmill",
    "the Oakvale creamery",
    "the Padstow quarry",
    "the Quenby forge",
    "the Risley pottery",
    "the Saxby windmill",
];

const PLACES: &[&str] = &[
    "Brentmoor county",
    "the Halsford district",
    "the Tamsin valley",
    "the northern moors",
    "Aldermere town",
    "the Welby lowlands",
    "Carrick parish",
    "the Denholm basin",
    "Ensley borough",
    "the Foxton fells",
    "Granby village",
    "the Hartfield plain",
    "Ilkeston ward",
    "the Jedburgh glen",
    "Kessock harbour",
    "the Langton heath",
    "Marsden hollow",
    "the Newport reach",
    "Otterburn ridge",
    "the Pelham marshes",
    "Quarrend hamlet",
    "the Rookhope dale",
    "Stanmore green",
    "the Thirlby downs",
    "Underwood common",
    "the Vance estuary",
    "Wharram cliffs",
    "the Yardley wolds",
];

const YEARS: &[&str] = &[
    "1887", "1902", "1913", "1921", "1934", "1948", "1956", "1963", "1971", "1984", "1992", "2001",
    "2009", "2016",
];

struct Template {
    /// [canonical, restatement-1, restatement-2] — all keep {S} and {P}.
    forms: [&'static str; 3],
    /// terse restatement used by the style-shift variant.
    style: &'static str,
    /// shares {S} only — a different claim (hard "adjacent" chaff).
    adjacent: &'static str,
}

// Restatement discipline: each restatement shares with the claim ONLY the
// entities {S}/{P} and isolated topical words — never a 4-word run (so the
// SHIPPED k=4 sketch keeps them in DISTINCT clusters) while the cross-encoder
// still sees the same entities + topic and scores them as supporting.
const TEMPLATES: &[Template] = &[
    Template {
        forms: [
            "{S} supplies fresh drinking water to {P} each dry season",
            "{P} relies on {S} for water once the wells run low",
            "water reaches homes in {P} only because of {S}",
        ],
        style: "{P} is watered by {S}",
        adjacent: "{S} was repainted by local volunteers over the summer",
    },
    Template {
        forms: [
            "{S} carries freight between {P} and the busy river port",
            "{P} ships its produce outward using {S}",
            "freight bound for markets leaves {P} aboard {S}",
        ],
        style: "{P} moves freight via {S}",
        adjacent: "{S} stays closed to all traffic on public holidays",
    },
    Template {
        forms: [
            "{S} first opened its doors to visitors near {P} in {Y}",
            "{P} gained {S} as a public attraction back in {Y}",
            "visitors have explored {S} beside {P} ever since {Y}",
        ],
        style: "{S} opened by {P} in {Y}",
        adjacent: "{S} lost much of its roof in a winter storm",
    },
    Template {
        forms: [
            "{S} employs most of the skilled workers living in {P}",
            "{P} sends nearly all its tradespeople to jobs at {S}",
            "skilled employment in {P} centres almost wholly on {S}",
        ],
        style: "{P} mostly works at {S}",
        adjacent: "{S} recently replaced its ageing boiler house",
    },
    Template {
        forms: [
            "{S} generates electricity for the homes scattered across {P}",
            "{P} keeps its lights on with power drawn from {S}",
            "electric power throughout {P} originates at {S}",
        ],
        style: "{P} is powered by {S}",
        adjacent: "{S} was surveyed by inspectors earlier this year",
    },
    Template {
        forms: [
            "{S} protects {P} from flooding during the high spring tides",
            "{P} stays dry each spring thanks entirely to {S}",
            "flood water would swamp {P} were it not for {S}",
        ],
        style: "{S} shields {P} from floods",
        adjacent: "{S} was photographed for a national exhibition",
    },
    Template {
        forms: [
            "{S} was rebuilt after a great fire swept through {P} in {Y}",
            "{P} watched {S} rise again once the {Y} blaze had passed",
            "fire damaged {S} so badly that {P} saw it restored after {Y}",
        ],
        style: "{S} rebuilt for {P} after {Y}",
        adjacent: "{S} now offers guided tours on weekday mornings",
    },
    Template {
        forms: [
            "{S} processes the grain harvested across {P} each autumn",
            "{P} sends its autumn harvest of grain to {S}",
            "milled grain from fields around {P} all passes through {S}",
        ],
        style: "{S} mills grain for {P}",
        adjacent: "{S} was listed as a protected heritage structure",
    },
    Template {
        forms: [
            "{S} draws thousands of summer tourists toward {P}",
            "{P} fills with holiday visitors who come to see {S}",
            "tourism around {P} depends heavily on {S}",
        ],
        style: "{S} brings tourists to {P}",
        adjacent: "{S} closed its eastern wing for repairs",
    },
    Template {
        forms: [
            "{S} links {P} to the distant coastal railway by one track",
            "{P} reaches the seaside line only by way of {S}",
            "the coast becomes accessible from {P} thanks to {S}",
        ],
        style: "{S} connects {P} seaward",
        adjacent: "{S} was repainted in its original livery last year",
    },
];

fn fill(t: &str, s: &str, p: &str, y: &str) -> String {
    t.replace("{S}", s).replace("{P}", p).replace("{Y}", y)
}

const BOILERPLATE: &[&str] = &[
    "according to the county gazette",
    "the regional ledger reports that",
    "in a bulletin this week officials said",
    "as the local archive records",
];

/// A syndicated copy: the claim verbatim wrapped in outlet boilerplate
/// (prefix and/or suffix). The edit is ADDITIVE — the claim text stays
/// contiguous, so every one of its 4-word shingles survives and
/// `containment(claim, copy) = 1.0 ≥ τ`: the copy ALWAYS lands in the claim's
/// cluster, exactly as a real near-verbatim repost of a full document does
/// (ADR-18/suite-9: document-level syndication clusters robustly). This is the
/// hardest trap — its CE against the claim is near-maximal — and the C1
/// structural exclusion (c ≠ c_w) must drop it regardless. A shingle-fragmenting
/// synonym swap is deliberately AVOIDED: on a sentence-length claim it can break
/// a large fraction of the few shingles and force an unrepresentative false-split
/// that real document-length copies never exhibit.
fn make_copy(rng: &mut Rng, claim: &str) -> String {
    let pre = BOILERPLATE[rng.below(BOILERPLATE.len())];
    let suf = BOILERPLATE[rng.below(BOILERPLATE.len())];
    match rng.below(3) {
        0 => format!("{pre} {claim}"),
        1 => format!("{claim} {suf}"),
        _ => format!("{pre} {claim} {suf}"),
    }
}

fn pick_distinct(rng: &mut Rng, n: usize, avoid: usize) -> usize {
    let mut x = rng.below(n);
    if x == avoid {
        x = (x + 1) % n;
    }
    x
}

/// Generate a claim corpus. `style` selects terse restatements and a heavier
/// fraction of hard "adjacent" chaff — the no-collapse falsifier for τ_corr.
fn gen_claims(rng: &mut Rng, n: usize, style: bool) -> Vec<ClaimQuery> {
    let total_cands = 8usize;
    let adj_p: f32 = if style { 0.6 } else { 0.3 };
    (0..n)
        .map(|_| {
            let has_independent = rng.next_f32() < 0.5;
            let syndication_only = !has_independent && rng.next_f32() < 0.5;

            let ti = rng.below(TEMPLATES.len());
            let t = &TEMPLATES[ti];
            let si = rng.below(SUBJECTS.len());
            let pi = rng.below(PLACES.len());
            let yi = rng.below(YEARS.len());
            let (s, p, y) = (SUBJECTS[si], PLACES[pi], YEARS[yi]);

            let claim = fill(t.forms[0], s, p, y);
            let mut cands = vec![ClaimCand {
                text: claim.clone(),
                kind: Kind::Claim,
            }];

            if has_independent || syndication_only {
                for _ in 0..COPIES_PER {
                    cands.push(ClaimCand {
                        text: make_copy(rng, &claim),
                        kind: Kind::Copy,
                    });
                }
            }
            if has_independent {
                let k = 1 + rng.below(2);
                for j in 0..k {
                    // restatement forms 1..; style uses the terse form.
                    let form = if style { t.style } else { t.forms[1 + (j % 2)] };
                    cands.push(ClaimCand {
                        text: fill(form, s, p, y),
                        kind: Kind::Independent,
                    });
                }
            }
            // Chaff fills the rest: mostly off-topic (fresh entities + template),
            // a controlled fraction "adjacent" (same {S}, different claim).
            while cands.len() < total_cands {
                let text = if rng.next_f32() < adj_p {
                    let at = &TEMPLATES[pick_distinct(rng, TEMPLATES.len(), ti)];
                    fill(
                        at.adjacent,
                        s,
                        PLACES[pick_distinct(rng, PLACES.len(), pi)],
                        y,
                    )
                } else {
                    let ct = &TEMPLATES[rng.below(TEMPLATES.len())];
                    let cs = SUBJECTS[pick_distinct(rng, SUBJECTS.len(), si)];
                    let cp = PLACES[pick_distinct(rng, PLACES.len(), pi)];
                    let cy = YEARS[rng.below(YEARS.len())];
                    fill(ct.forms[0], cs, cp, cy)
                };
                cands.push(ClaimCand {
                    text,
                    kind: Kind::Chaff,
                });
            }
            ClaimQuery {
                cands,
                syndication_only,
            }
        })
        .collect()
}

/// The clustered view of a query: cluster id per candidate (production
/// `cluster_tags`), the claim's cluster `c_w`, and per distinct cluster != c_w
/// its canonical member (the C1 feature scores exactly this member).
struct Clustered {
    /// (cluster_id, canonical candidate index, that candidate's kind)
    other_clusters: Vec<(u32, usize, Kind)>,
    /// copies that landed OUTSIDE the claim's cluster (sketch false-split) — the
    /// only way a same-cluster copy could ever be counted. Must be 0.
    copy_split: usize,
    /// independent restatements that split into a distinct cluster (the recall
    /// ceiling: an independent merged into c_w can never be corroboration).
    indep_distinct: usize,
    indep_total: usize,
}

fn cluster_query(q: &ClaimQuery) -> Clustered {
    let keys: Vec<u64> = (0..q.cands.len() as u64).collect();
    let mut sketches: HashMap<u64, Sketch> = HashMap::with_capacity(q.cands.len());
    for (i, c) in q.cands.iter().enumerate() {
        sketches.insert(i as u64, Sketch::compute(&c.text));
    }
    let tags = cluster_tags(&keys, &sketches, meridian_index::sketch::CONTAINMENT_TAU);
    let c_w = tags[0].expect("claim always sketched").id;

    let mut other_clusters = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut copy_split = 0usize;
    let (mut indep_distinct, mut indep_total) = (0usize, 0usize);
    for (i, tag) in tags.iter().enumerate() {
        let Some(tag) = tag else { continue };
        let kind = q.cands[i].kind;
        if kind == Kind::Independent {
            indep_total += 1;
            if tag.id != c_w {
                indep_distinct += 1;
            }
        }
        if kind == Kind::Copy && tag.id != c_w {
            copy_split += 1;
        }
        if tag.id != c_w && tag.canonical && seen.insert(tag.id) {
            other_clusters.push((tag.id, i, kind));
        }
    }
    Clustered {
        other_clusters,
        copy_split,
        indep_distinct,
        indep_total,
    }
}

/// Arm A — hermetic leakage gate (no model): aggregate the clustering over a
/// corpus. `copy_split` summed must be 0 (zero same-cluster leakage); the
/// independent-distinct rate is the achievable recall ceiling.
struct LeakReport {
    copy_split: usize,
    indep_distinct_rate: f64,
    /// mean distinct non-claim clusters per query (the CE-batch fan-out).
    mean_other_clusters: f64,
}

fn eval_leakage(queries: &[ClaimQuery]) -> LeakReport {
    let mut copy_split = 0usize;
    let (mut id, mut it) = (0usize, 0usize);
    let mut others = 0usize;
    for q in queries {
        let c = cluster_query(q);
        copy_split += c.copy_split;
        id += c.indep_distinct;
        it += c.indep_total;
        others += c.other_clusters.len();
    }
    LeakReport {
        copy_split,
        indep_distinct_rate: id as f64 / it.max(1) as f64,
        mean_other_clusters: others as f64 / queries.len().max(1) as f64,
    }
}

// ---- Arm B (real CE) ----------------------------------------------------
// Only compiled with `bench-ce-real` (ort/gnu). Produces per-query CE scores for
// each distinct non-claim cluster; the τ_corr sweep then runs over stored scores
// (the model runs once per pair).

#[cfg(feature = "bench-ce-real")]
struct ScoredQuery {
    /// (kind, ce_score) per distinct cluster != c_w.
    clusters: Vec<(Kind, f32)>,
}

/// Returns the per-query CE scores AND the wall-clock of each non-empty CE batch
/// (ms) — the latter IS the production corroboration batch (same model, same
/// claim↔cluster-snippet pairing, mean ~5 pairs), so its p50 is the on-device
/// answer-p50 budget delta (no separate meridiand build needed).
#[cfg(feature = "bench-ce-real")]
fn ce_score_corpus(
    reranker: &meridian_rerank::Reranker,
    queries: &[ClaimQuery],
) -> (Vec<ScoredQuery>, Vec<f64>) {
    use meridian_rerank::Pair;
    use std::time::{Duration, Instant};
    let mut batch_ms: Vec<f64> = Vec::new();
    let out = queries
        .iter()
        .map(|q| {
            let cl = cluster_query(q);
            let claim = &q.cands[0].text;
            let pairs: Vec<Pair> = cl
                .other_clusters
                .iter()
                .map(|&(cid, idx, _)| Pair {
                    doc_key: cid as u64,
                    title: String::new(),
                    snippet: q.cands[idx].text.clone(),
                })
                .collect();
            let t = Instant::now();
            let scored = reranker
                .rerank(claim, &pairs, Duration::from_secs(20))
                .expect("ort rerank");
            if !pairs.is_empty() {
                batch_ms.push(t.elapsed().as_secs_f64() * 1e3);
            }
            let by_key: HashMap<u64, f32> =
                scored.iter().map(|s| (s.doc_key, s.ce_score)).collect();
            let clusters = cl
                .other_clusters
                .iter()
                .map(|&(cid, _, kind)| {
                    (kind, by_key.get(&(cid as u64)).copied().unwrap_or(f32::MIN))
                })
                .collect();
            ScoredQuery { clusters }
        })
        .collect();
    (out, batch_ms)
}

/// Cluster-level corroboration scores at a frozen τ_corr — the "k independent
/// sources agree" badge must be RIGHT, so precision/recall are counted per
/// counted cluster, not per query.
#[cfg(feature = "bench-ce-real")]
struct C1Score {
    precision: f64,
    recall: f64,
    /// counted clusters whose member is a same-cluster copy (false-split that
    /// cleared τ) — the binding leakage probe, must be 0.
    copy_counted: usize,
    /// counted chaff clusters — false badges that are not copies.
    chaff_counted: usize,
}

#[cfg(feature = "bench-ce-real")]
fn eval_c1_at(scored: &[ScoredQuery], tau: f64) -> C1Score {
    let (mut tp, mut fp, mut fne) = (0u64, 0u64, 0u64);
    let (mut copy_counted, mut chaff_counted) = (0usize, 0usize);
    for sq in scored {
        for &(kind, ce) in &sq.clusters {
            let counted = f64::from(ce) >= tau;
            match (kind, counted) {
                (Kind::Independent, true) => tp += 1,
                (Kind::Independent, false) => fne += 1,
                (Kind::Chaff, true) => {
                    fp += 1;
                    chaff_counted += 1;
                }
                (Kind::Copy, true) => {
                    fp += 1;
                    copy_counted += 1;
                }
                _ => {}
            }
        }
    }
    C1Score {
        precision: tp as f64 / (tp + fp).max(1) as f64,
        recall: tp as f64 / (tp + fne).max(1) as f64,
        copy_counted,
        chaff_counted,
    }
}

/// Sweep τ_corr on tuning: pick the smallest τ (⇒ max recall) whose precision
/// clears the bar; tie-break on higher precision. Returns the frozen τ.
#[cfg(feature = "bench-ce-real")]
fn choose_tau_corr(tuning: &[ScoredQuery]) -> f64 {
    let mut best: Option<(f64, f64, f64)> = None; // (tau, recall, precision)
    for &tau in TAU_CORR_GRID {
        let s = eval_c1_at(tuning, tau);
        if s.precision + 1e-12 < C1_PRECISION_BAR {
            continue;
        }
        let cand = (tau, s.recall, s.precision);
        best = Some(match best {
            None => cand,
            Some(b) => {
                if cand.1 > b.1 + 1e-12 || (cand.1 >= b.1 - 1e-12 && cand.2 > b.2) {
                    cand
                } else {
                    b
                }
            }
        });
    }
    // If nothing clears precision on tuning, freeze the most precise τ so the
    // hold-out still reports an honest (failing) number rather than panicking.
    best.map(|b| b.0).unwrap_or_else(|| {
        TAU_CORR_GRID
            .iter()
            .copied()
            .max_by(|a, c| {
                eval_c1_at(tuning, *a)
                    .precision
                    .partial_cmp(&eval_c1_at(tuning, *c).precision)
                    .unwrap()
            })
            .unwrap_or(5.0)
    })
}

// ===========================================================================
// Suite entry
// ===========================================================================

pub fn run(cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("answer_trust");
    let start = Instant::now();
    result.metric("queries", QUERIES);
    result.metric("ce_queries", CE_QUERIES);
    result.metric("budget", BUDGET);

    // ---- H3 abstention (synthetic, model-free) ----
    let h3_tuning = generate(&mut Rng::new(0xA117_2026), QUERIES, false);
    let h3_holdout = generate(&mut Rng::new(0x1A11_7026), QUERIES, false);
    let h3_styled = generate(&mut Rng::new(0x5717_1E26), QUERIES, true);

    let h3_t = eval_h3(&h3_tuning);
    let (tau_star, cov_t, sel_t) = choose_tau(&h3_t);
    let h3_h = eval_h3(&h3_holdout);
    let (cov_h, sel_h) = selective_at(&h3_h, tau_star);
    let h3_s = eval_h3(&h3_styled);
    let (cov_s, sel_s) = selective_at(&h3_s, tau_star);
    result.metric("h3_tau_star", round3(tau_star));
    result.metric("h3_baseline_hit_holdout", round3(h3_h.baseline_hit));
    result.metric("h3_coverage_tuning", round3(cov_t));
    result.metric("h3_selective_hit_tuning", round3(sel_t));
    result.metric("h3_coverage_holdout", round3(cov_h));
    result.metric("h3_selective_hit_holdout", round3(sel_h));
    result.metric("h3_coverage_styled", round3(cov_s));
    result.metric("h3_selective_hit_styled", round3(sel_s));
    for (t, cov, sh) in &h3_h.points {
        result.note(format!(
            "h3 curve holdout: τ={:.2} coverage={:.3} selective_hit={:.3}",
            t, cov, sh
        ));
    }
    let g_h3_dominates = sel_h >= h3_h.baseline_hit + H3_MARGIN && cov_h >= 1.0 - ABSTAIN_CAP;
    let g_h3_no_collapse = (sel_h - sel_s).abs() <= NO_COLLAPSE;

    // ---- C1 Arm A: hermetic leakage gate (always; no model) ----
    let c1_tuning = gen_claims(&mut Rng::new(0xC100_2026), CE_QUERIES, false);
    let c1_holdout = gen_claims(&mut Rng::new(0xC101_7026), CE_QUERIES, false);
    let c1_styled = gen_claims(&mut Rng::new(0xC15E_1E26), CE_QUERIES, true);
    result.metric(
        "c1_syndication_trap_queries_tuning",
        c1_tuning.iter().filter(|q| q.syndication_only).count() as u64,
    );
    let leak_t = eval_leakage(&c1_tuning);
    let leak_h = eval_leakage(&c1_holdout);
    let leak_s = eval_leakage(&c1_styled);
    let total_copy_split = leak_t.copy_split + leak_h.copy_split + leak_s.copy_split;
    for (tag, l) in [
        ("tuning", &leak_t),
        ("holdout", &leak_h),
        ("styled", &leak_s),
    ] {
        result.metric(format!("c1_copy_split_{tag}").as_str(), l.copy_split as u64);
        result.metric(
            format!("c1_indep_distinct_rate_{tag}").as_str(),
            round3(l.indep_distinct_rate),
        );
        result.metric(
            format!("c1_mean_other_clusters_{tag}").as_str(),
            round3(l.mean_other_clusters),
        );
    }
    let g_c1_no_leak = total_copy_split == 0;
    if !g_c1_no_leak {
        result.note(
            "KILL: a syndicated copy false-split out of the claim's cluster — it could be counted \
             as an independent source. A false 'k independent sources agree' badge is worse than \
             none. Do NOT ship C1."
                .to_owned(),
        );
    }

    // ---- C1 Arm B: real production CE precision/recall (gnu/ort only) ----
    #[allow(unused_mut)]
    let mut g_c1_ce_available = false;
    #[allow(unused_mut)]
    let mut g_c1_precision = true; // vacuously true until the CE arm runs
    #[allow(unused_mut)]
    let mut g_c1_recall = true;
    #[allow(unused_mut)]
    let mut g_c1_ce_no_leak = true;

    #[cfg(feature = "bench-ce-real")]
    {
        let models_dir = cfg.models_dir.as_path();
        match meridian_rerank::Reranker::load(models_dir) {
            Ok(reranker) if reranker.available() => {
                g_c1_ce_available = true;
                let (st, lat_t) = ce_score_corpus(&reranker, &c1_tuning);
                let (sh, lat_h) = ce_score_corpus(&reranker, &c1_holdout);
                let (ss, lat_s) = ce_score_corpus(&reranker, &c1_styled);
                // on-device corroboration-batch latency = the answer-p50 budget
                // delta (the feature adds exactly one such batch per answer).
                let mut batch_ms: Vec<f64> = lat_t.into_iter().chain(lat_h).chain(lat_s).collect();
                let batch_p50 = crate::stats::percentile_ms(&mut batch_ms, 50.0);
                let batch_p99 = crate::stats::percentile_ms(&mut batch_ms, 99.0);
                result.metric("corroboration_batch_p50_ms", round3(batch_p50));
                result.metric("corroboration_batch_p99_ms", round3(batch_p99));
                let tau_corr = choose_tau_corr(&st);
                result.metric("c1_tau_corr", round3(tau_corr));

                let s_t = eval_c1_at(&st, tau_corr);
                let s_h = eval_c1_at(&sh, tau_corr);
                let s_s = eval_c1_at(&ss, tau_corr);
                for (tag, s) in [("tuning", &s_t), ("holdout", &s_h), ("styled", &s_s)] {
                    result.metric(
                        format!("c1_ce_precision_{tag}").as_str(),
                        round3(s.precision),
                    );
                    result.metric(format!("c1_ce_recall_{tag}").as_str(), round3(s.recall));
                    result.metric(
                        format!("c1_ce_copy_counted_{tag}").as_str(),
                        s.copy_counted as u64,
                    );
                    result.metric(
                        format!("c1_ce_chaff_counted_{tag}").as_str(),
                        s.chaff_counted as u64,
                    );
                }
                // Pre-registered MDE80 for the cluster-precision proportion.
                let counted_t = st
                    .iter()
                    .flat_map(|q| q.clusters.iter())
                    .filter(|(_, ce)| f64::from(*ce) >= tau_corr)
                    .count()
                    .max(1) as f64;
                let p = s_t.precision.clamp(1e-6, 1.0 - 1e-6);
                let mde = 2.8 * (2.0 * p * (1.0 - p) / counted_t).sqrt();
                result.metric("mde80_c1_precision", round3(mde));

                g_c1_precision =
                    s_t.precision >= C1_PRECISION_BAR && s_s.precision >= C1_PRECISION_BAR;
                g_c1_recall = s_h.recall >= C1_RECALL_BAR;
                g_c1_ce_no_leak =
                    s_t.copy_counted == 0 && s_h.copy_counted == 0 && s_s.copy_counted == 0;
                result.note(format!(
                    "C1 production CE (ort INT8 ms-marco): τ_corr={:.2} frozen on tuning; \
                     precision tuning={:.3}/holdout={:.3}/styled={:.3}; recall holdout={:.3}; \
                     copy_counted(all)={}; MDE80(precision)≈{:.3}",
                    tau_corr,
                    s_t.precision,
                    s_h.precision,
                    s_s.precision,
                    s_h.recall,
                    s_t.copy_counted + s_h.copy_counted + s_s.copy_counted,
                    mde
                ));
            }
            _ => {
                result.note(
                    "C1 arm B: reranker model unavailable (need models/ms-marco-minilm-l6-v2) — \
                     CE precision/recall DEFERRED; hermetic leakage half still gated."
                        .to_owned(),
                );
            }
        }
    }
    #[cfg(not(feature = "bench-ce-real"))]
    {
        let _ = cfg;
        result.note(
            "C1 arm B (real ort CE) not compiled (build --features bench-ce-real on gnu) — CE \
             precision/recall DEFERRED to the on-device gnu run; hermetic leakage half gated here."
                .to_owned(),
        );
    }

    // ---- combined gate (one slot) ----
    for (k, v) in [
        ("gate_h3_dominates_5pp_at_20pct", g_h3_dominates),
        ("gate_h3_no_collapse_10pp_styled", g_h3_no_collapse),
        ("gate_c1_zero_same_cluster_leakage", g_c1_no_leak),
        ("gate_c1_ce_available", g_c1_ce_available),
        ("gate_c1_precision_ge_0_9_tuning_styled", g_c1_precision),
        ("gate_c1_recall_ge_0_6_holdout", g_c1_recall),
        ("gate_c1_ce_zero_leakage", g_c1_ce_no_leak),
    ] {
        result.metric(k, u8::from(v));
    }
    let passed = g_h3_dominates
        && g_h3_no_collapse
        && g_c1_no_leak
        && g_c1_precision
        && g_c1_recall
        && g_c1_ce_no_leak;
    let gate_desc = if g_c1_ce_available {
        "C1 cluster-precision ≥0.9 (tuning+styled) AND cluster-recall ≥0.6 (holdout) AND zero \
         same-cluster leakage (sketch + CE); H3 selective hit ≥ baseline+5pp at ≤20% abstention \
         (holdout) AND ≤10pp style collapse"
    } else {
        "[hermetic half — CE precision/recall DEFERRED to gnu] zero same-cluster leakage; H3 \
         selective hit ≥ baseline+5pp at ≤20% abstention (holdout) AND ≤10pp style collapse"
    };
    result.gate(gate_desc, passed);
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syndicated_copies_stay_in_the_claim_cluster() {
        // The binding leakage guard on REAL text + the SHIPPED sketch: every
        // copy must land in the claim's cluster (so c != c_w excludes it).
        let qs = gen_claims(&mut Rng::new(3), 300, false);
        let report = eval_leakage(&qs);
        assert_eq!(
            report.copy_split, 0,
            "a syndicated copy false-split out of the claim cluster — leakage risk"
        );
    }

    #[test]
    fn independent_restatements_split_into_distinct_clusters() {
        // Recall ceiling: restatements must NOT merge into the claim's cluster,
        // or they can never be counted as independent corroboration.
        let qs = gen_claims(&mut Rng::new(5), 300, false);
        let report = eval_leakage(&qs);
        assert!(
            report.indep_distinct_rate >= 0.9,
            "independent restatements merged into the claim cluster: rate={}",
            report.indep_distinct_rate
        );
    }

    #[test]
    fn generator_plants_the_trap() {
        let qs = gen_claims(&mut Rng::new(9), 200, false);
        assert!(qs.iter().any(|q| q.syndication_only));
        assert!(
            qs.iter()
                .any(|q| q.cands.iter().any(|c| c.kind == Kind::Independent))
        );
        // syndication_only queries carry copies but no independent.
        for q in qs.iter().filter(|q| q.syndication_only) {
            assert!(q.cands.iter().any(|c| c.kind == Kind::Copy));
            assert!(!q.cands.iter().any(|c| c.kind == Kind::Independent));
        }
    }
}
