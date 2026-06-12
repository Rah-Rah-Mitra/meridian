//! Suite 14 — `ope`: offline-policy-evaluation estimator validation
//! (Phase 9 entry experiment, ADR-25 / 04-bench-plan §6).
//!
//! Before IPS/DR estimates are trusted on a real decision log, they must
//! recover a KNOWN truth on a synthetic one. This suite simulates the shipped
//! logging policy exactly — ε-greedy (ε = 0.1) over 3 arms with the
//! propensities the bandit emits at choice time ((1−ε)+ε/K greedy, ε/K
//! otherwise) — across 4 intent contexts with a fixed Bernoulli reward
//! matrix, then asks both estimators for the value of a candidate policy that
//! routes each context to its truly best arm. The analytic truth is just the
//! context-weighted max of the reward matrix.
//!
//! The incumbent greedy arm is deliberately WRONG for 3 of 4 contexts, so the
//! naive log mean is far from the candidate's value — the estimators only
//! pass by actually correcting the logging bias, not by riding a benign log.
//!
//! Gate (04-bench-plan §6): |estimate − truth| / truth < 5% for BOTH IPS and
//! DR, on the mean across replicates. Per-replicate spread is reported as an
//! informational diagnostic (DR should be tighter than IPS).

use super::{BenchConfig, SuiteResult};
use crate::stats::Rng;
use meridian_searx::ope::{Logged, dr, ips};
use std::time::Instant;

const EPSILON: f64 = 0.1; // mirrors meridian-searx ε-greedy
const ARMS: usize = 3; // fast / broad / reference
const CONTEXTS: usize = 4; // navigational / informational / transactional / local
const DECISIONS_PER_REPLICATE: usize = 10_000;
const REPLICATES: usize = 20;
const GATE_BIAS: f64 = 0.05;

/// True per-(context, arm) Bernoulli reward probabilities. Rows = contexts,
/// columns = arms. The best arm differs by context (that is the whole point
/// of contextual routing), and the incumbent's greedy choice (below) matches
/// it only for context 3.
const REWARD: [[f64; ARMS]; CONTEXTS] = [
    [0.30, 0.70, 0.40],
    [0.25, 0.45, 0.65],
    [0.60, 0.20, 0.35],
    [0.40, 0.55, 0.30],
];

/// The incumbent (logging) policy's greedy arm per context — wrong for
/// contexts 0–2, right for context 3.
const INCUMBENT_GREEDY: [usize; CONTEXTS] = [0, 0, 1, 1];

fn candidate(context: u32) -> usize {
    let row = &REWARD[context as usize % CONTEXTS];
    let mut best = 0;
    for a in 1..ARMS {
        if row[a] > row[best] {
            best = a;
        }
    }
    best
}

/// Analytic value of the candidate policy under uniform context arrival.
fn truth() -> f64 {
    (0..CONTEXTS)
        .map(|c| REWARD[c][candidate(c as u32)])
        .sum::<f64>()
        / CONTEXTS as f64
}

fn simulate_log(rng: &mut Rng) -> Vec<Logged> {
    let p_greedy = (1.0 - EPSILON) + EPSILON / ARMS as f64;
    let p_other = EPSILON / ARMS as f64;
    (0..DECISIONS_PER_REPLICATE)
        .map(|_| {
            let context = rng.below(CONTEXTS) as u32;
            let greedy = INCUMBENT_GREEDY[context as usize];
            // ε-greedy: explore uniformly over ALL arms with prob ε (the
            // greedy arm can be drawn in exploration too — that is exactly
            // why its propensity is (1−ε)+ε/K, not (1−ε)).
            let arm = if rng.next_f64() < EPSILON {
                rng.below(ARMS)
            } else {
                greedy
            };
            let propensity = if arm == greedy { p_greedy } else { p_other };
            let reward = if rng.next_f64() < REWARD[context as usize][arm] {
                1.0
            } else {
                0.0
            };
            Logged {
                context,
                arm,
                propensity,
                reward,
            }
        })
        .collect()
}

pub fn run(_cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("ope");
    let start = Instant::now();

    let v_true = truth();
    let mut rng = Rng::new(0x09E1_2026);
    let mut ips_estimates = Vec::with_capacity(REPLICATES);
    let mut dr_estimates = Vec::with_capacity(REPLICATES);
    let mut naive_means = Vec::with_capacity(REPLICATES);

    for _ in 0..REPLICATES {
        let log = simulate_log(&mut rng);
        naive_means.push(log.iter().map(|d| d.reward).sum::<f64>() / log.len() as f64);
        ips_estimates.push(ips(&log, candidate, 0.0));
        dr_estimates.push(dr(&log, candidate));
    }

    let mean = |xs: &[f64]| xs.iter().sum::<f64>() / xs.len() as f64;
    let spread = |xs: &[f64]| {
        let m = mean(xs);
        (xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / xs.len() as f64).sqrt()
    };

    let (m_ips, m_dr, m_naive) = (
        mean(&ips_estimates),
        mean(&dr_estimates),
        mean(&naive_means),
    );
    let bias_ips = (m_ips - v_true).abs() / v_true;
    let bias_dr = (m_dr - v_true).abs() / v_true;

    result.metric("decisions_per_replicate", DECISIONS_PER_REPLICATE);
    result.metric("replicates", REPLICATES);
    result.metric("truth_candidate_value", round4(v_true));
    result.metric("naive_log_mean", round4(m_naive));
    result.metric("ips_mean", round4(m_ips));
    result.metric("dr_mean", round4(m_dr));
    result.metric("ips_rel_bias", round4(bias_ips));
    result.metric("dr_rel_bias", round4(bias_dr));
    result.metric("ips_replicate_sd", round4(spread(&ips_estimates)));
    result.metric("dr_replicate_sd", round4(spread(&dr_estimates)));

    result.note(format!(
        "logger = shipped ε-greedy (ε={EPSILON}) with exact emitted propensities; incumbent \
         greedy arm wrong in 3/4 contexts, so the naive mean ({m_naive:.3}) sits far from the \
         candidate's true value ({v_true:.3}) — the estimators must do real correction"
    ));
    result.note(
        "passing this gate is the ADR-25 precondition for trusting DR on the real decision \
         log; the ship gate itself (DR uplift CI excludes zero) runs on ≥10k live decisions"
            .to_owned(),
    );

    result.gate(
        &format!("IPS and DR relative bias < {GATE_BIAS} vs analytic truth"),
        bias_ips < GATE_BIAS && bias_dr < GATE_BIAS,
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
    fn truth_is_context_weighted_max() {
        // max per row: 0.70, 0.65, 0.60, 0.55 → mean 0.625
        assert!((truth() - 0.625).abs() < 1e-12);
    }

    #[test]
    fn suite_gate_passes() {
        let r = run(&BenchConfig::default());
        assert_eq!(r.gate_passed, Some(true), "metrics: {:?}", r.metrics);
    }
}
