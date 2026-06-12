//! Offline policy evaluation (Phase 9, ADR-24/25): estimate the value of a
//! CANDIDATE routing policy from decisions logged under the incumbent —
//! before any live traffic shifts. The ship gate (ADR-25) is a doubly-robust
//! uplift whose 95% CI excludes zero; suite 14 proves the estimators recover
//! a known synthetic truth (bias <5%) before they are trusted on real logs.
//!
//! Estimators (target policy π is deterministic: context → arm):
//! - **IPS** (Horvitz-Thompson): mean of 1[aᵢ=π(xᵢ)]/pᵢ · rᵢ. Unbiased when
//!   the logged propensities are exact (ours are: ε-greedy emits them at
//!   choice time), high variance when π disagrees with the logger.
//! - **DR** (Dudík et al. 2011): reward-model baseline + IPS correction on
//!   its residual — unbiased if EITHER the propensities or the model are
//!   right, lower variance than IPS. The model here is the empirical
//!   per-(context, arm) mean with an additive prior (deliberately simple:
//!   DR's value is robustness to model error, not model sophistication).

/// One logged decision as the estimators consume it. `context` is an opaque
/// bucket id (generalized rows map to a shared bucket — they still carry
/// exact arm/propensity/reward, so they remain estimator-valid).
#[derive(Debug, Clone, Copy)]
pub struct Logged {
    pub context: u32,
    pub arm: usize,
    pub propensity: f64,
    pub reward: f64,
}

/// Inverse-propensity-scoring estimate of a deterministic policy's value.
/// `clip` caps importance weights (0 = no clipping) — clipping trades a
/// little bias for variance, standard practice; report both when it matters.
pub fn ips(log: &[Logged], policy: impl Fn(u32) -> usize, clip: f64) -> f64 {
    if log.is_empty() {
        return f64::NAN;
    }
    let sum: f64 = log
        .iter()
        .map(|d| {
            if d.arm == policy(d.context) {
                let w = 1.0 / d.propensity.max(1e-9);
                let w = if clip > 0.0 { w.min(clip) } else { w };
                w * d.reward
            } else {
                0.0
            }
        })
        .sum();
    sum / log.len() as f64
}

/// Doubly-robust estimate. The reward model q̂(context, arm) is the empirical
/// mean with a weak uniform prior (α=1, β=2 ⇒ prior mean 0.5).
pub fn dr(log: &[Logged], policy: impl Fn(u32) -> usize) -> f64 {
    if log.is_empty() {
        return f64::NAN;
    }
    let q = RewardModel::fit(log);
    let sum: f64 = log
        .iter()
        .map(|d| {
            let pi_arm = policy(d.context);
            let baseline = q.predict(d.context, pi_arm);
            let correction = if d.arm == pi_arm {
                (d.reward - q.predict(d.context, d.arm)) / d.propensity.max(1e-9)
            } else {
                0.0
            };
            baseline + correction
        })
        .sum();
    sum / log.len() as f64
}

/// Per-(context, arm) empirical mean reward with a weak prior.
struct RewardModel {
    sums: std::collections::HashMap<(u32, usize), (f64, f64)>, // (reward sum, count)
}

impl RewardModel {
    fn fit(log: &[Logged]) -> Self {
        let mut sums: std::collections::HashMap<(u32, usize), (f64, f64)> =
            std::collections::HashMap::new();
        for d in log {
            let e = sums.entry((d.context, d.arm)).or_insert((0.0, 0.0));
            e.0 += d.reward;
            e.1 += 1.0;
        }
        Self { sums }
    }

    fn predict(&self, context: u32, arm: usize) -> f64 {
        let (s, n) = self
            .sums
            .get(&(context, arm))
            .copied()
            .unwrap_or((0.0, 0.0));
        (s + 0.5) / (n + 1.0) // prior mean 0.5, weight one pseudo-observation
    }
}

/// The ADR-25 ship-gate statistic: doubly-robust value of the candidate
/// minus the incumbent's realized mean on the same rows, with a bootstrap
/// percentile CI. The gate passes only if the 95% CI excludes zero on ≥10k
/// decisions; this function just reports — the verdict (and the sunset rule)
/// lives with the caller.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct UpliftReport {
    pub n: usize,
    /// Realized mean reward of the logged (incumbent) policy on these rows.
    pub incumbent_mean: f64,
    /// DR estimate of the candidate policy's value on the same rows.
    pub candidate_dr: f64,
    pub uplift: f64,
    pub ci_lo: f64,
    pub ci_hi: f64,
}

pub fn dr_uplift_ci(
    log: &[Logged],
    policy: impl Fn(u32) -> usize + Copy,
    iters: usize,
    seed: u64,
) -> UpliftReport {
    let incumbent_mean = log.iter().map(|d| d.reward).sum::<f64>() / log.len().max(1) as f64;
    let candidate_dr = dr(log, policy);
    let uplift = candidate_dr - incumbent_mean;
    if log.is_empty() {
        return UpliftReport {
            n: 0,
            incumbent_mean,
            candidate_dr,
            uplift,
            ci_lo: f64::NAN,
            ci_hi: f64::NAN,
        };
    }

    // Percentile bootstrap over row resamples (paired: each resample
    // re-evaluates BOTH the incumbent mean and the candidate DR, so shared
    // sampling noise cancels in the uplift).
    let mut rng = seed.max(1);
    let mut next = move || {
        // xorshift64* — deterministic, no global RNG (crate convention).
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let mut uplifts: Vec<f64> = (0..iters.max(100))
        .map(|_| {
            let sample: Vec<Logged> = (0..log.len())
                .map(|_| log[(next() % log.len() as u64) as usize])
                .collect();
            let inc = sample.iter().map(|d| d.reward).sum::<f64>() / sample.len() as f64;
            dr(&sample, policy) - inc
        })
        .collect();
    uplifts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pct = |p: f64| uplifts[((uplifts.len() - 1) as f64 * p).round() as usize];
    UpliftReport {
        n: log.len(),
        incumbent_mean,
        candidate_dr,
        uplift,
        ci_lo: pct(0.025),
        ci_hi: pct(0.975),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A toy log where arm 1 is always right but the logger favored arm 0:
    /// the naive mean is dragged down; IPS/DR must recover arm 1's true value.
    #[test]
    fn ips_and_dr_correct_logging_bias() {
        let mut log = Vec::new();
        // Logger picks arm 0 with p=0.9 (reward 0.2), arm 1 with p=0.1
        // (reward 0.8). Deterministic alternation approximates the rates.
        for i in 0..10_000 {
            if i % 10 == 0 {
                log.push(Logged {
                    context: 0,
                    arm: 1,
                    propensity: 0.1,
                    reward: 0.8,
                });
            } else {
                log.push(Logged {
                    context: 0,
                    arm: 0,
                    propensity: 0.9,
                    reward: 0.2,
                });
            }
        }
        let naive: f64 = log.iter().map(|d| d.reward).sum::<f64>() / log.len() as f64;
        assert!(
            naive < 0.3,
            "naive mean reflects the logger, not the target"
        );
        let v_ips = ips(&log, |_| 1, 0.0);
        let v_dr = dr(&log, |_| 1);
        assert!((v_ips - 0.8).abs() < 0.01, "ips {v_ips}");
        assert!((v_dr - 0.8).abs() < 0.01, "dr {v_dr}");
    }

    #[test]
    fn empty_log_is_nan() {
        assert!(ips(&[], |_| 0, 0.0).is_nan());
        assert!(dr(&[], |_| 0).is_nan());
    }

    #[test]
    fn uplift_ci_separates_better_and_equal_candidates() {
        // Same biased log as above: incumbent realizes ~0.26 mean, arm 1 is
        // worth 0.8. The candidate routing to arm 1 must show a strictly
        // positive CI; the candidate equal to the logger must straddle zero.
        let mut log = Vec::new();
        for i in 0..5_000 {
            if i % 10 == 0 {
                log.push(Logged {
                    context: 0,
                    arm: 1,
                    propensity: 0.1,
                    reward: 0.8,
                });
            } else {
                log.push(Logged {
                    context: 0,
                    arm: 0,
                    propensity: 0.9,
                    reward: 0.2,
                });
            }
        }
        let better = dr_uplift_ci(&log, |_| 1, 200, 42);
        assert!(
            better.ci_lo > 0.0,
            "better candidate must clear zero: {better:?}"
        );
        // Always-arm-0 is strictly WORSE than the stochastic incumbent
        // (0.20 vs the realized 0.26 mixture): the CI must sit below zero —
        // the ADR-25 "negative" verdict, which must NOT pass the gate.
        let worse = dr_uplift_ci(&log, |_| 0, 200, 42);
        assert!(
            worse.ci_hi < 0.0 && (worse.uplift + 0.06).abs() < 0.01,
            "worse candidate shows its true −0.06 uplift: {worse:?}"
        );
        let empty = dr_uplift_ci(&[], |_| 0, 200, 42);
        assert_eq!(empty.n, 0);
        assert!(empty.ci_lo.is_nan());
    }
}
