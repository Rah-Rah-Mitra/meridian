//! Suite 9 — `synfarm`: syndication-farm sketch-clustering parameter sweep
//! (Phase 7 entry experiment, ADR-18 / 04-bench-plan §6).
//!
//! Generates synthetic syndication farms (1 origin article + N lightly-rewritten
//! copies across distinct outlets) mixed with genuinely independent articles on
//! the same topics, then sweeps shingle size × MinHash permutations × cluster
//! threshold and scores the resulting derivation clusters against ground truth.
//! A second, deliberately different generator ("held-out variant", risk #21)
//! guards against the detector overfitting the generator it was tuned on.
//!
//! Gate: some swept parameter combination reaches pairwise F1 > 0.8 AND
//! false-merge (1 − pairwise precision) < 5% on BOTH variants. The winning
//! constants are what ADR-18 adopts; they are printed in the report.
//!
//! Statistical outputs are build-profile-independent (no timing gates here).

use super::{BenchConfig, SuiteResult};
use crate::stats::Rng;
use std::time::Instant;

const FARMS: usize = 24;
const DERIVED_PER_FARM: usize = 8;
const INDEPENDENTS_PER_TOPIC: usize = 3;
const SIMHASH_BITS: usize = 64;

const SHINGLE_SIZES: &[usize] = &[4, 5, 7, 9];
const PERM_COUNTS: &[usize] = &[64, 128];

/// Pair-similarity measure for the derivation edge. Containment
/// (|A∩B| / min(|A|,|B|), estimated from the MinHash Jaccard + set sizes) is
/// robust to the truncation + boilerplate asymmetry of real syndication;
/// raw Jaccard punishes exactly those edits. The sweep decides empirically.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Measure {
    Jaccard,
    Containment,
}

const MEASURES: &[Measure] = &[Measure::Jaccard, Measure::Containment];
const JACCARD_THRESHOLDS: &[f64] = &[0.15, 0.25, 0.35, 0.5];
const CONTAINMENT_THRESHOLDS: &[f64] = &[0.3, 0.45, 0.6, 0.75];

/// One synthetic article: its ground-truth origin, a synthetic outlet domain,
/// and the word stream the sketches are computed over.
struct Doc {
    origin: u32,
    domain: u32,
    words: Vec<String>,
}

pub fn run(_cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("synfarm");
    let start = Instant::now();

    let primary = generate(0x5F4A_2026, Variant::Primary);
    let held_out = generate(0xBEEF_2026, Variant::HeldOut);
    result.metric("docs_primary", primary.len());
    result.metric("docs_held_out", held_out.len());

    // Domain-dedup baseline (what v0.1.0 could do): cluster = registered domain.
    let base_p = score_domain_baseline(&primary);
    let base_h = score_domain_baseline(&held_out);
    result.metric("baseline_domain_f1_primary", round3(base_p));
    result.metric("baseline_domain_f1_held_out", round3(base_h));

    // Sweep both variants over the same combo grid. The winner must satisfy the
    // gate on BOTH; among those, highest mean F1 wins.
    let swept_p = sweep(&primary);
    let swept_h = sweep(&held_out);
    let mut best: Option<(Combo, Scores, Scores)> = None;
    for ((combo, sp), (_, sh)) in swept_p.into_iter().zip(swept_h) {
        let better = match &best {
            None => true,
            Some((_, bp, bh)) => {
                let cur_ok = gate_ok(&sp) && gate_ok(&sh);
                let best_ok = gate_ok(bp) && gate_ok(bh);
                let cur_score = (sp.f1 + sh.f1) / 2.0;
                let best_score = (bp.f1 + bh.f1) / 2.0;
                (cur_ok && !best_ok) || (cur_ok == best_ok && cur_score > best_score)
            }
        };
        if better {
            best = Some((combo, sp, sh));
        }
    }

    let (combo, sp, sh) = best.expect("sweep grid is non-empty");
    result.metric("recommended_shingle_k", combo.k);
    result.metric("recommended_minhash_perms", combo.perms);
    result.metric(
        "recommended_measure",
        match combo.measure {
            Measure::Jaccard => "jaccard",
            Measure::Containment => "containment",
        },
    );
    result.metric("recommended_threshold", combo.tau);
    result.metric("f1_primary", round3(sp.f1));
    result.metric("false_merge_primary", round3(sp.false_merge));
    result.metric("f1_held_out", round3(sh.f1));
    result.metric("false_merge_held_out", round3(sh.false_merge));
    result.metric("f1_variant_drop", round3((sp.f1 - sh.f1).abs()));

    // SimHash gate recommendation: the Hamming radius that keeps ≥99% of true
    // same-origin pairs at the winning shingle size (the cheap prefilter in
    // front of the MinHash verify, per ADR-18).
    let gate_radius = simhash_gate_radius(&primary, combo.k, 0.99);
    result.metric("recommended_simhash_hamming_gate", gate_radius);

    // Production-format validation: the shipped `meridian_index::sketch::Sketch`
    // (densified one-permutation b=8 MinHash, 60 bins, containment τ) is a
    // DIFFERENT estimator from the full-u64 sweep above — quantization and OPH
    // densification change the variance, so it gets its own gate on both
    // variants before ADR-18 may rely on it.
    let prod_p = score_production(&primary);
    let prod_h = score_production(&held_out);
    result.metric("production_f1_primary", round3(prod_p.f1));
    result.metric("production_false_merge_primary", round3(prod_p.false_merge));
    result.metric("production_f1_held_out", round3(prod_h.f1));
    result.metric(
        "production_false_merge_held_out",
        round3(prod_h.false_merge),
    );
    result.note(format!(
        "production format = meridian_index::sketch (OPH b=8, {} bins, k={}, containment τ={})",
        meridian_index::sketch::BINS,
        meridian_index::sketch::SHINGLE_K,
        meridian_index::sketch::CONTAINMENT_TAU
    ));

    result.note(format!(
        "winner: k={} perms={} measure={} tau={} — adopt these as the ADR-18 constants",
        combo.k,
        combo.perms,
        match combo.measure {
            Measure::Jaccard => "jaccard",
            Measure::Containment => "containment",
        },
        combo.tau
    ));
    result.note(
        "false_merge = 1 − pairwise precision (fraction of merged pairs that are \
         cross-origin); risk #21 hold-out variant uses block shuffles, lead rewrites \
         and heavier word edits"
            .to_owned(),
    );
    if (sp.f1 - sh.f1).abs() > 0.15 {
        result.note("WARNING: F1 drop between generator variants exceeds 0.15 (risk #21 tripwire)");
    }

    result.gate(
        "pairwise F1 > 0.8 AND false-merge < 5% on BOTH variants — for the swept \
         winner AND the production sketch format",
        gate_ok(&sp)
            && gate_ok(&sh)
            && (sp.f1 - sh.f1).abs() <= 0.15
            && gate_ok(&prod_p)
            && gate_ok(&prod_h),
    );
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}

/// Cluster a corpus with the SHIPPED sketch implementation and score it.
fn score_production(docs: &[Doc]) -> Scores {
    use meridian_index::sketch::{CONTAINMENT_TAU, Sketch};
    let sketches: Vec<Sketch> = docs
        .iter()
        .map(|d| Sketch::compute(&d.words.join(" ")))
        .collect();
    let n = docs.len();
    let mut uf = UnionFind::new(n);
    for i in 0..n {
        for j in (i + 1)..n {
            if sketches[i].containment(&sketches[j]) >= CONTAINMENT_TAU {
                uf.union(i, j);
            }
        }
    }
    let labels: Vec<usize> = (0..n).map(|i| uf.find(i)).collect();
    pairwise_scores(docs, &labels)
}

#[derive(Clone, Copy)]
struct Combo {
    k: usize,
    perms: usize,
    measure: Measure,
    tau: f64,
}

struct Scores {
    f1: f64,
    false_merge: f64,
}

fn gate_ok(s: &Scores) -> bool {
    s.f1 > 0.8 && s.false_merge < 0.05
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

enum Variant {
    /// Light syndication edits: word swaps, sentence drops, outlet boilerplate,
    /// tail truncation.
    Primary,
    /// Heavier rewriting: everything above plus sentence-block shuffles and a
    /// fully rewritten lead — a different parameterization so the sweep cannot
    /// overfit one generator (risk #21).
    HeldOut,
}

fn generate(seed: u64, variant: Variant) -> Vec<Doc> {
    let mut rng = Rng::new(seed);
    let general: Vec<String> = (0..400).map(|i| format!("w{i:03}")).collect();
    let mut docs = Vec::new();
    let mut next_domain = 0u32;

    for farm in 0..FARMS as u32 {
        // Topic vocabulary shared by the farm AND its independents: same story,
        // different newsrooms.
        let topic: Vec<String> = (0..40).map(|i| format!("t{farm:02}x{i:02}")).collect();

        // Origin article.
        let origin_sentences = make_sentences(&mut rng, &topic, &general, 14);
        let origin_domain = next_domain;
        next_domain += 1;
        docs.push(Doc {
            origin: farm * 100,
            domain: origin_domain,
            words: origin_sentences.concat(),
        });

        // Derived copies: distinct outlets (the cross-domain syndication that
        // domain dedup structurally misses); one in five repost on the origin
        // domain (which domain dedup does catch).
        for copy in 0..DERIVED_PER_FARM {
            let mut sentences = origin_sentences.clone();
            match variant {
                // Real syndication is near-verbatim: trims + boilerplate + light
                // copyediting (a few percent of words), not paragraph rewrites.
                Variant::Primary => light_edit(&mut rng, &mut sentences, &general, 0.04),
                Variant::HeldOut => {
                    light_edit(&mut rng, &mut sentences, &general, 0.12);
                    if !sentences.is_empty() {
                        sentences[0] = make_sentences(&mut rng, &topic, &general, 1).remove(0);
                    }
                    block_shuffle(&mut rng, &mut sentences);
                }
            }
            add_boilerplate(
                &mut rng,
                &mut sentences,
                &general,
                matches!(variant, Variant::HeldOut),
            );
            let domain = if copy % 5 == 4 {
                origin_domain
            } else {
                next_domain += 1;
                next_domain - 1
            };
            docs.push(Doc {
                origin: farm * 100,
                domain,
                words: sentences.concat(),
            });
        }

        // Independent coverage: same topic pool, freshly generated sentences —
        // moderate word overlap, near-zero shingle overlap. Each is its own
        // ground-truth origin.
        for ind in 0..INDEPENDENTS_PER_TOPIC as u32 {
            let sentences = make_sentences(&mut rng, &topic, &general, 12);
            next_domain += 1;
            docs.push(Doc {
                origin: farm * 100 + 1 + ind,
                domain: next_domain - 1,
                words: sentences.concat(),
            });
        }
    }
    docs
}

fn make_sentences(
    rng: &mut Rng,
    topic: &[String],
    general: &[String],
    n: usize,
) -> Vec<Vec<String>> {
    (0..n)
        .map(|_| {
            let len = 7 + rng.below(8);
            (0..len)
                .map(|_| {
                    if rng.next_f32() < 0.45 {
                        topic[rng.below(topic.len())].clone()
                    } else {
                        general[rng.below(general.len())].clone()
                    }
                })
                .collect()
        })
        .collect()
}

fn light_edit(rng: &mut Rng, sentences: &mut Vec<Vec<String>>, general: &[String], word_p: f32) {
    // Word-level synonym swaps.
    for s in sentences.iter_mut() {
        for w in s.iter_mut() {
            if rng.next_f32() < word_p {
                *w = general[rng.below(general.len())].clone();
            }
        }
    }
    // Drop ~1 in 7 sentences, keep at least 4.
    let mut i = 0;
    while i < sentences.len() && sentences.len() > 4 {
        if rng.next_f32() < 0.14 {
            sentences.remove(i);
        } else {
            i += 1;
        }
    }
    // Tail truncation: keep the first 60–95%.
    let keep = ((sentences.len() as f32) * (0.6 + 0.35 * rng.next_f32())).ceil() as usize;
    sentences.truncate(keep.max(4));
}

fn block_shuffle(rng: &mut Rng, sentences: &mut [Vec<String>]) {
    // Swap two random 2-sentence blocks (newsroom restructuring).
    if sentences.len() >= 6 {
        let a = rng.below(sentences.len() - 1);
        let b = rng.below(sentences.len() - 1);
        if a.abs_diff(b) >= 2 {
            sentences.swap(a, b);
            sentences.swap(a + 1, b + 1);
        }
    }
}

fn add_boilerplate(
    rng: &mut Rng,
    sentences: &mut Vec<Vec<String>>,
    general: &[String],
    heavy: bool,
) {
    let outlet = rng.below(1000);
    let tail: Vec<String> = (0..if heavy { 12 } else { 6 })
        .map(|i| format!("bp{outlet:03}n{i}"))
        .collect();
    sentences.push(tail);
    if heavy {
        let lead: Vec<String> = (0..5)
            .map(|_| general[rng.below(general.len())].clone())
            .collect();
        sentences.insert(0, lead);
    }
}

// ---------------------------------------------------------------------------
// Sketches
// ---------------------------------------------------------------------------

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn shingle_hashes(words: &[String], k: usize) -> Vec<u64> {
    if words.len() < k {
        return vec![fnv1a(words.join(" ").as_bytes())];
    }
    words
        .windows(k)
        .map(|w| fnv1a(w.join(" ").as_bytes()))
        .collect()
}

fn minhash(shingles: &[u64], perms: usize) -> Vec<u64> {
    (0..perms as u64)
        .map(|p| {
            shingles
                .iter()
                .map(|&s| splitmix64(s ^ splitmix64(p.wrapping_mul(0xA5A5_5A5A_DEAD_BEEF))))
                .min()
                .unwrap_or(u64::MAX)
        })
        .collect()
}

fn simhash(shingles: &[u64]) -> u64 {
    let mut acc = [0i32; SIMHASH_BITS];
    for &s in shingles {
        for (b, slot) in acc.iter_mut().enumerate() {
            if s >> b & 1 == 1 {
                *slot += 1;
            } else {
                *slot -= 1;
            }
        }
    }
    acc.iter()
        .enumerate()
        .fold(0u64, |h, (b, &v)| if v > 0 { h | 1 << b } else { h })
}

fn est_jaccard(a: &[u64], b: &[u64]) -> f64 {
    let matches = a.iter().zip(b.iter()).filter(|(x, y)| x == y).count();
    matches as f64 / a.len() as f64
}

// ---------------------------------------------------------------------------
// Clustering + scoring
// ---------------------------------------------------------------------------

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn new(n: usize) -> Self {
        Self((0..n).collect())
    }
    fn find(&mut self, x: usize) -> usize {
        if self.0[x] != x {
            let root = self.find(self.0[x]);
            self.0[x] = root;
        }
        self.0[x]
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra] = rb;
        }
    }
}

/// Score every combo on one corpus, sharing work across the grid: shingle hashes
/// are computed once per shingle size, the 64-perm signature is the prefix of the
/// 128-perm one (same per-perm seeds), and the pairwise similarity matrices are
/// threshold-independent. Keeps the suite fast even in a debug build on the A76.
fn sweep(docs: &[Doc]) -> Vec<(Combo, Scores)> {
    let max_perms = *PERM_COUNTS.iter().max().expect("non-empty");
    let n = docs.len();
    let mut out = Vec::new();
    for &k in SHINGLE_SIZES {
        let shingles: Vec<Vec<u64>> = docs.iter().map(|d| shingle_hashes(&d.words, k)).collect();
        let set_sizes: Vec<f64> = shingles
            .iter()
            .map(|s| {
                let mut uniq = s.clone();
                uniq.sort_unstable();
                uniq.dedup();
                uniq.len() as f64
            })
            .collect();
        let sigs: Vec<Vec<u64>> = shingles.iter().map(|s| minhash(s, max_perms)).collect();
        for &perms in PERM_COUNTS {
            // Upper-triangle similarity estimates over the signature prefix:
            // Jaccard Ĵ directly from MinHash; containment derived from Ĵ and
            // the exact set sizes via |A∩B| = Ĵ·(|A|+|B|)/(1+Ĵ).
            let mut est_j = vec![0.0f64; n * n];
            let mut est_c = vec![0.0f64; n * n];
            for i in 0..n {
                for j in (i + 1)..n {
                    let jac = est_jaccard(&sigs[i][..perms], &sigs[j][..perms]);
                    let inter = jac * (set_sizes[i] + set_sizes[j]) / (1.0 + jac);
                    est_j[i * n + j] = jac;
                    est_c[i * n + j] = (inter / set_sizes[i].min(set_sizes[j])).min(1.0);
                }
            }
            for &measure in MEASURES {
                let (est, thresholds) = match measure {
                    Measure::Jaccard => (&est_j, JACCARD_THRESHOLDS),
                    Measure::Containment => (&est_c, CONTAINMENT_THRESHOLDS),
                };
                for &tau in thresholds {
                    let mut uf = UnionFind::new(n);
                    for i in 0..n {
                        for j in (i + 1)..n {
                            if est[i * n + j] >= tau {
                                uf.union(i, j);
                            }
                        }
                    }
                    let labels: Vec<usize> = (0..n).map(|i| uf.find(i)).collect();
                    out.push((
                        Combo {
                            k,
                            perms,
                            measure,
                            tau,
                        },
                        pairwise_scores(docs, &labels),
                    ));
                }
            }
        }
    }
    out
}

fn score_domain_baseline(docs: &[Doc]) -> f64 {
    let mut uf = UnionFind::new(docs.len());
    let mut by_domain: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (i, d) in docs.iter().enumerate() {
        if let Some(&first) = by_domain.get(&d.domain) {
            uf.union(i, first);
        } else {
            by_domain.insert(d.domain, i);
        }
    }
    let labels: Vec<usize> = (0..docs.len()).map(|i| uf.find(i)).collect();
    pairwise_scores(docs, &labels).f1
}

fn pairwise_scores(docs: &[Doc], labels: &[usize]) -> Scores {
    let (mut tp, mut fp, mut fne) = (0u64, 0u64, 0u64);
    for i in 0..docs.len() {
        for j in (i + 1)..docs.len() {
            let same_truth = docs[i].origin == docs[j].origin;
            let same_pred = labels[i] == labels[j];
            match (same_truth, same_pred) {
                (true, true) => tp += 1,
                (false, true) => fp += 1,
                (true, false) => fne += 1,
                (false, false) => {}
            }
        }
    }
    let precision = if tp + fp > 0 {
        tp as f64 / (tp + fp) as f64
    } else {
        1.0
    };
    let recall = if tp + fne > 0 {
        tp as f64 / (tp + fne) as f64
    } else {
        1.0
    };
    let f1 = if precision + recall > 0.0 {
        2.0 * precision * recall / (precision + recall)
    } else {
        0.0
    };
    Scores {
        f1,
        false_merge: 1.0 - precision,
    }
}

/// Smallest Hamming radius that retains ≥ `coverage` of true same-origin pairs.
fn simhash_gate_radius(docs: &[Doc], k: usize, coverage: f64) -> u32 {
    let hashes: Vec<u64> = docs
        .iter()
        .map(|d| simhash(&shingle_hashes(&d.words, k)))
        .collect();
    let mut dists: Vec<u32> = Vec::new();
    for i in 0..docs.len() {
        for j in (i + 1)..docs.len() {
            if docs[i].origin == docs[j].origin {
                dists.push((hashes[i] ^ hashes[j]).count_ones());
            }
        }
    }
    dists.sort_unstable();
    if dists.is_empty() {
        return SIMHASH_BITS as u32;
    }
    let idx = ((dists.len() as f64 * coverage).ceil() as usize).min(dists.len()) - 1;
    dists[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_is_deterministic() {
        let a = generate(42, Variant::Primary);
        let b = generate(42, Variant::Primary);
        assert_eq!(a.len(), b.len());
        assert_eq!(a[0].words, b[0].words);
    }

    #[test]
    fn minhash_estimates_identity_and_disjoint() {
        let s1 = shingle_hashes(&["a", "b", "c", "d", "e", "f"].map(String::from), 3);
        let sig1 = minhash(&s1, 64);
        assert!((est_jaccard(&sig1, &sig1) - 1.0).abs() < 1e-9);
        let s2 = shingle_hashes(&["x", "y", "z", "u", "v", "w"].map(String::from), 3);
        let sig2 = minhash(&s2, 64);
        assert!(est_jaccard(&sig1, &sig2) < 0.2);
    }

    #[test]
    fn derived_copies_cluster_with_origin() {
        let docs = generate(7, Variant::Primary);
        let swept = sweep(&docs);
        let (_, scores) = swept
            .into_iter()
            .find(|(c, _)| {
                c.k == 5
                    && c.perms == 128
                    && c.measure == Measure::Containment
                    && (c.tau - 0.45).abs() < 1e-9
            })
            .expect("combo in grid");
        // Sanity: sketch clustering must beat coin-flip territory on the easy
        // variant; the real gate runs in the suite.
        assert!(scores.f1 > 0.5, "f1 = {}", scores.f1);
    }
}
