//! Value-of-information fetch selection & stopping (Phase 9, ADR-26 /
//! 07-voi-design.md): Pandora's-box reservation indices over fetch
//! candidates. Pure decision logic — no I/O, no clock; the caller realizes
//! values (by actually fetching) and enforces wall-clock deadlines. This is
//! deliberate: the core is CI-gatable and the suite-15 replay drives it with
//! corpus-revealed values instead of network fetches.
//!
//! Model (two-point value): opening candidate i costs `cost_i` and realizes
//! value `gain_i` with probability `p_i`, else 0. Weitzman's reservation
//! index is the z solving `cost = p·(gain − z)`:
//!
//! ```text
//! z_i = gain_i − cost_i / p_i
//! ```
//!
//! The optimal policy opens candidates in decreasing z and stops as soon as
//! the best value already in hand meets the next z — provably optimal for
//! independent boxes, and the honesty payload falls out for free: when the
//! walk stops, `max(z_next − best, 0)` IS the estimated marginal gain left
//! on the table (`analysis.estimated_marginal_gain_remaining`).

/// One fetch candidate. Units are the caller's, but `gain`/`cost` must share
/// them (07-voi-design.md uses DCG mass; suite 15 calibrates the mapping).
#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    pub id: u64,
    /// Probability the fetch realizes its gain (clamped away from 0).
    pub p: f64,
    /// Value realized on success.
    pub gain: f64,
    /// Cost of opening, in gain units.
    pub cost: f64,
}

/// Weitzman reservation index. p ≤ 0 candidates price themselves out
/// (−∞-ish) rather than dividing by zero.
pub fn reservation_index(c: &Candidate) -> f64 {
    c.gain - c.cost / c.p.max(1e-9)
}

/// v1 value model (07-voi-design.md §2.4, twice corrected by its own suite):
/// - suite-15 v0 proved novelty alone is no value model — deep-ranked chaff
///   is as "novel" as deep-ranked decisive docs, and the walk opened chaff;
/// - suite-15 v1 proved the PANDORA stopping rule answers the wrong
///   question for ranking: Weitzman optimizes the single best find, but
///   nDCG over the page is ADDITIVE — after the first decisive find the
///   walk (correctly, for its objective) stopped and starved the other
///   subtopics.
///
/// Shipped mapping: `p = clamp(β0 + β2·score_z)` (relevance likelihood from
/// the retrieval score the head already owns) and `gain = novelty ·
/// dcg_headroom` (a near-copy of something already fetched has ~no MARGINAL
/// ranking value, however big its headroom). Selection uses
/// [`additive_walk`]; [`pandora_walk`] stays for the single-best regime
/// (answer mode, a Phase-10 candidate). Constants FIXED by the suite-15
/// sweep (tuning seed, judged frozen on hold-out) — recorded in the bench
/// report, not hand-picked. Embedding `coverage` joins when the planner
/// wiring lands with real embeddings (recorded deviation).
pub const P_BETA0: f64 = 0.2;
pub const P_BETA2: f64 = 0.3;

/// Map observable signals to a candidate. `dcg_headroom` = the DCG mass the
/// document would gain by moving to the top of the evaluated prefix (the
/// most the fetch could matter); `novelty` ∈ [0,1] = 1 − max
/// sketch-containment vs already-fetched docs; `score_z` = the candidate's
/// retrieval score standardized over the head (z-score, clamped ±3);
/// `cost` in gain units.
pub fn candidate_from_signals(
    id: u64,
    dcg_headroom: f64,
    novelty: f64,
    score_z: f64,
    cost: f64,
) -> Candidate {
    candidate_with_betas(id, dcg_headroom, novelty, score_z, cost, P_BETA0, P_BETA2)
}

/// Sweep-able variant — the suite explores the βs on the tuning seed and
/// the frozen winner becomes [`P_BETA0`]/[`P_BETA2`].
pub fn candidate_with_betas(
    id: u64,
    dcg_headroom: f64,
    novelty: f64,
    score_z: f64,
    cost: f64,
    beta0: f64,
    beta2: f64,
) -> Candidate {
    Candidate {
        id,
        p: (beta0 + beta2 * score_z.clamp(-3.0, 3.0)).clamp(0.05, 0.95),
        gain: novelty.clamp(0.0, 1.0) * dcg_headroom,
        cost,
    }
}

/// Additive-objective selector (the shipped deep-mode rule): open candidates
/// in decreasing EXPECTED net marginal value `p·gain − cost` while it is
/// positive and budget remains. Greedy is near-optimal for the submodular
/// page-level objective (novelty-discounted gains are diminishing by
/// construction). The caller re-poses between opens when novelty shifts;
/// `est_gain_remaining` on a budget stop is the best expected net value
/// left unopened.
pub fn additive_walk(
    candidates: &[Candidate],
    budget: usize,
    mut open: impl FnMut(u64),
) -> WalkResult {
    let mut ranked: Vec<(f64, Candidate)> = candidates
        .iter()
        .map(|c| (c.p * c.gain - c.cost, *c))
        .collect();
    ranked.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.id.cmp(&b.1.id))
    });

    let mut opened = Vec::new();
    for (net, c) in ranked {
        if net <= 0.0 {
            return WalkResult {
                opened,
                stopped_because: StopReason::ValueBelowReservation,
                est_gain_remaining: 0.0,
            };
        }
        if opened.len() >= budget {
            return WalkResult {
                opened,
                stopped_because: StopReason::BudgetExhausted,
                est_gain_remaining: net,
            };
        }
        opened.push(c.id);
        open(c.id);
    }
    WalkResult {
        opened,
        stopped_because: StopReason::Exhausted,
        est_gain_remaining: 0.0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The best value in hand met the next reservation index — the optimal
    /// stop. Reading more is expected to be a net loss.
    ValueBelowReservation,
    /// The caller's open budget ran out first.
    BudgetExhausted,
    /// Every candidate was opened.
    Exhausted,
    /// The caller's wall-clock deadline fired between opens (the walk itself
    /// has no clock — the planner's fetch phase sets this).
    Deadline,
}

/// Per-fetch cost in DCG-gain units — CALIBRATED BY SUITE 15 together with
/// the β constants (they are a unit system, not independent knobs; re-run
/// the suite if any of them moves).
pub const DEFAULT_FETCH_COST: f64 = 0.6;

#[derive(Debug, Clone)]
pub struct WalkResult {
    /// Candidate ids in the order they were opened.
    pub opened: Vec<u64>,
    pub stopped_because: StopReason,
    /// max(z of best unopened − best value in hand, 0): what stopping left
    /// on the table, in gain units. 0 when the stop was optimal/exhaustive.
    pub est_gain_remaining: f64,
}

/// Run the walk. `initial_best` is the value already in hand WITHOUT any
/// fetch (the head's current top score contribution) — Pandora's outside
/// option; a strong enough incumbent stops the walk before the first open.
/// `open` is called once per opened candidate and returns its realized
/// value (the replay reveals corpus text; production fetches + re-scores).
pub fn pandora_walk(
    candidates: &[Candidate],
    initial_best: f64,
    budget: usize,
    mut open: impl FnMut(u64) -> f64,
) -> WalkResult {
    let mut ranked: Vec<(f64, Candidate)> = candidates
        .iter()
        .map(|c| (reservation_index(c), *c))
        .collect();
    // Descending z; ties broken by id for determinism.
    ranked.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.id.cmp(&b.1.id))
    });

    let mut best = initial_best;
    let mut opened = Vec::new();
    let mut iter = ranked.into_iter().peekable();
    loop {
        let Some(&(z_next, _)) = iter.peek() else {
            return WalkResult {
                opened,
                stopped_because: StopReason::Exhausted,
                est_gain_remaining: 0.0,
            };
        };
        if best >= z_next {
            return WalkResult {
                opened,
                stopped_because: StopReason::ValueBelowReservation,
                est_gain_remaining: 0.0,
            };
        }
        if opened.len() >= budget {
            return WalkResult {
                opened,
                stopped_because: StopReason::BudgetExhausted,
                est_gain_remaining: (z_next - best).max(0.0),
            };
        }
        let (_, c) = iter.next().expect("peeked");
        opened.push(c.id);
        best = best.max(open(c.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: u64, p: f64, gain: f64, cost: f64) -> Candidate {
        Candidate { id, p, gain, cost }
    }

    #[test]
    fn orders_by_reservation_not_raw_gain() {
        // Expensive-big: z = 10 − 4/0.5 = 2. Cheap-likely: z = 3 − 0.3/0.9 ≈ 2.67.
        // The smaller gain wins the first open — that is the whole point.
        let cands = [cand(1, 0.5, 10.0, 4.0), cand(2, 0.9, 3.0, 0.3)];
        let r = pandora_walk(&cands, 0.0, 10, |_| 0.0);
        assert_eq!(r.opened, vec![2, 1]);
        assert_eq!(r.stopped_because, StopReason::Exhausted);
    }

    #[test]
    fn stops_when_value_in_hand_meets_next_reservation() {
        // z = [5, 3, 1]; the first open realizes 4 ≥ z₂ = 3 → optimal stop.
        let cands = [
            cand(1, 1.0, 6.0, 1.0), // z = 5
            cand(2, 1.0, 4.0, 1.0), // z = 3
            cand(3, 1.0, 2.0, 1.0), // z = 1
        ];
        let r = pandora_walk(&cands, 0.0, 10, |id| if id == 1 { 4.0 } else { 0.0 });
        assert_eq!(r.opened, vec![1]);
        assert_eq!(r.stopped_because, StopReason::ValueBelowReservation);
        assert_eq!(r.est_gain_remaining, 0.0);
    }

    #[test]
    fn budget_stop_reports_gain_left_on_table() {
        let cands = [cand(1, 1.0, 6.0, 1.0), cand(2, 1.0, 4.0, 1.0)];
        let r = pandora_walk(&cands, 0.0, 1, |_| 0.5);
        assert_eq!(r.opened, vec![1]);
        assert_eq!(r.stopped_because, StopReason::BudgetExhausted);
        // z₂ = 3, best in hand 0.5 → 2.5 honestly left unexplored.
        assert!((r.est_gain_remaining - 2.5).abs() < 1e-12);
    }

    #[test]
    fn strong_incumbent_stops_before_any_fetch() {
        let cands = [cand(1, 1.0, 6.0, 1.0)];
        let r = pandora_walk(&cands, 5.0, 10, |_| panic!("must not open"));
        assert!(r.opened.is_empty());
        assert_eq!(r.stopped_because, StopReason::ValueBelowReservation);
    }

    #[test]
    fn hopeless_candidates_price_themselves_out() {
        // p=0 → index dives far below any incumbent; never opened.
        let cands = [cand(1, 0.0, 100.0, 1.0)];
        let r = pandora_walk(&cands, 0.0, 10, |_| panic!("must not open"));
        assert!(r.opened.is_empty());
        assert_eq!(r.stopped_because, StopReason::ValueBelowReservation);
    }

    #[test]
    fn empty_candidate_set_exhausts_cleanly() {
        let r = pandora_walk(&[], 0.0, 10, |_| 0.0);
        assert!(r.opened.is_empty());
        assert_eq!(r.stopped_because, StopReason::Exhausted);
    }

    #[test]
    fn additive_opens_only_positive_expected_net() {
        let cands = [
            cand(1, 0.5, 4.0, 0.6), // E = 1.4 → open
            cand(2, 0.3, 1.0, 0.6), // E = −0.3 → never
            cand(3, 0.9, 2.0, 0.6), // E = 1.2 → open second
        ];
        let r = additive_walk(&cands, 10, |_| {});
        assert_eq!(r.opened, vec![1, 3]);
        assert_eq!(r.stopped_because, StopReason::ValueBelowReservation);
        assert_eq!(r.est_gain_remaining, 0.0);
    }

    #[test]
    fn additive_budget_stop_reports_remaining() {
        let cands = [cand(1, 1.0, 4.0, 0.6), cand(2, 1.0, 3.0, 0.6)];
        let r = additive_walk(&cands, 1, |_| {});
        assert_eq!(r.opened, vec![1]);
        assert_eq!(r.stopped_because, StopReason::BudgetExhausted);
        assert!((r.est_gain_remaining - 2.4).abs() < 1e-12);
    }

    #[test]
    fn novelty_crushes_marginal_gain_in_the_model() {
        // Same doc, fresh vs redundant cluster: the copy's expected net
        // value must go negative purely through the gain discount.
        let fresh = candidate_with_betas(1, 4.0, 1.0, 1.0, 0.6, 0.1, 0.2);
        let copy = candidate_with_betas(2, 4.0, 0.05, 1.0, 0.6, 0.1, 0.2);
        assert!(fresh.p * fresh.gain - fresh.cost > 0.0);
        assert!(copy.p * copy.gain - copy.cost < 0.0);
    }
}
