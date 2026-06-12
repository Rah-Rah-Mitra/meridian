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
        let (s, n) = self.sums.get(&(context, arm)).copied().unwrap_or((0.0, 0.0));
        (s + 0.5) / (n + 1.0) // prior mean 0.5, weight one pseudo-observation
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
                log.push(Logged { context: 0, arm: 1, propensity: 0.1, reward: 0.8 });
            } else {
                log.push(Logged { context: 0, arm: 0, propensity: 0.9, reward: 0.2 });
            }
        }
        let naive: f64 = log.iter().map(|d| d.reward).sum::<f64>() / log.len() as f64;
        assert!(naive < 0.3, "naive mean reflects the logger, not the target");
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
}
