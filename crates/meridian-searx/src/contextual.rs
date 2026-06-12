//! Linear Thompson-sampling contextual routing policy (Phase 9, ADR-25) —
//! **ships dark**: constructed only when `searx.contextual_policy` is on,
//! which itself is gated behind the ADR-25 ship decision (doubly-robust
//! uplift whose 95% CI excludes zero on ≥10k logged decisions; inconclusive
//! ⇒ ε-greedy stays and this module stays cold). No silent limbo: the gate
//! verdict is recorded at the v0.4.0 exit either way.
//!
//! Model: one Bayesian linear regression per arm over a one-hot context
//! vector (the ADR-24 buckets — nothing the decision log does not already
//! hold). Posterior precision `A = λI + Σ x xᵀ`, moment `b = Σ r x`; a draw
//! θ̃ ~ N(A⁻¹b, v²A⁻¹) per arm, route to argmax x·θ̃. The 26×26 Cholesky is
//! hand-rolled — pulling a linalg stack for a matrix this size fails the
//! Pi-budget smell test (ADR-25 rejected heavier learners for the same
//! reason).
//!
//! Propensities (the decision log wants them at choice time): Thompson
//! sampling has no closed form, so they are estimated by Monte Carlo over
//! posterior draws — documented imprecision, recorded in the row as-is. The
//! estimator side (DR) is robust to moderate propensity error by
//! construction; suite 14's gate covers the exact-propensity regime and the
//! v0.4.0 exit re-validates against logged MC propensities before trusting
//! any uplift number.
//!
//! State is IN-MEMORY ONLY, rebuilt at boot by replaying the decision log
//! (`from_decisions`) — the log is the persistence layer; no new tables, no
//! new deletion surface. Generalized rows (k-anonymity floor) carry no
//! context features and are skipped for updates; they still count toward the
//! gate's decision tally.

use crate::bandit::{ARMS, Arm};
use crate::decision_log::{Context, Decision};

/// bias + intent(4) + len bucket(4) + lang(8, id mod 8) + tod(8) + geo(1).
/// whichlang's 16 ids fold into 8 coarse buckets to keep the design near the
/// ADR-25 "~20-dim" envelope; the fold is stable (id mod 8) so logged rows
/// replay identically across boots.
pub const DIM: usize = 1 + 4 + 4 + 8 + 8 + 1;

/// Ridge prior λ (posterior stays proper before any data) and the
/// exploration scale v² (LinTS convention; modest because rewards are 0/1).
const LAMBDA: f64 = 1.0;
const V2: f64 = 0.25;
/// Posterior draws per propensity estimate.
const PROPENSITY_DRAWS: usize = 64;

pub fn features(ctx: &Context) -> [f64; DIM] {
    let mut x = [0.0; DIM];
    x[0] = 1.0;
    x[1 + (ctx.intent as usize).min(3)] = 1.0;
    x[5 + (ctx.len_bucket as usize).min(3)] = 1.0;
    x[9 + (ctx.lang as usize) % 8] = 1.0;
    x[17 + (ctx.tod_bucket as usize).min(7)] = 1.0;
    if ctx.geo_filter {
        x[25] = 1.0;
    }
    x
}

/// Per-arm Bayesian linear regression sufficient statistics.
struct ArmPosterior {
    /// Precision matrix A = λI + Σ x xᵀ (row-major DIM×DIM).
    a: Vec<f64>,
    /// Moment vector b = Σ r·x.
    b: [f64; DIM],
}

impl ArmPosterior {
    fn new() -> Self {
        let mut a = vec![0.0; DIM * DIM];
        for i in 0..DIM {
            a[i * DIM + i] = LAMBDA;
        }
        Self { a, b: [0.0; DIM] }
    }

    fn update(&mut self, x: &[f64; DIM], reward: f64) {
        for i in 0..DIM {
            if x[i] == 0.0 {
                continue;
            }
            for j in 0..DIM {
                self.a[i * DIM + j] += x[i] * x[j];
            }
            self.b[i] += reward * x[i];
        }
    }

    /// One posterior draw θ̃ = μ + √v² L⁻ᵀ z, where A = LLᵀ and μ = A⁻¹b.
    fn sample(&self, rng: &mut Xoshiro) -> [f64; DIM] {
        let l = cholesky(&self.a);
        // μ: solve L y = b, then Lᵀ μ = y.
        let y = forward_sub(&l, &self.b);
        let mu = back_sub(&l, &y);
        // L⁻ᵀ z: solve Lᵀ w = z.
        let mut z = [0.0; DIM];
        for zi in z.iter_mut() {
            *zi = rng.next_gaussian();
        }
        let w = back_sub(&l, &z);
        let mut theta = [0.0; DIM];
        for i in 0..DIM {
            theta[i] = mu[i] + V2.sqrt() * w[i];
        }
        theta
    }
}

pub struct ContextualPolicy {
    arms: Vec<ArmPosterior>,
    /// Decisions replayed/observed — the ADR-25 gate needs the tally.
    pub decisions_seen: u64,
}

impl Default for ContextualPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextualPolicy {
    pub fn new() -> Self {
        Self {
            arms: (0..ARMS.len()).map(|_| ArmPosterior::new()).collect(),
            decisions_seen: 0,
        }
    }

    /// Rebuild from the decision log (boot path). Generalized rows have no
    /// context features — they advance the tally but cannot update a
    /// posterior (skipping them is unbiased for the model: generalization is
    /// keyed on combo rarity, not on reward).
    pub fn from_decisions(decisions: &[Decision]) -> Self {
        let mut p = Self::new();
        for d in decisions {
            p.decisions_seen += 1;
            if let Some(ctx) = &d.context {
                if let Some(arm) = p.arms.get_mut(d.arm as usize) {
                    arm.update(&features(ctx), d.reward as u8 as f64);
                }
            }
        }
        p
    }

    pub fn observe(&mut self, ctx: &Context, arm_idx: usize, reward: bool) {
        self.decisions_seen += 1;
        if let Some(arm) = self.arms.get_mut(arm_idx) {
            arm.update(&features(ctx), reward as u8 as f64);
        }
    }

    /// Deterministic greedy action: argmax over posterior MEANS (no
    /// sampling). This is the candidate the ADR-25 gate evaluates — IPS/DR
    /// take a deterministic policy, and the deployed exploitation behavior
    /// after learning converges is exactly this.
    pub fn greedy(&self, ctx: &Context) -> usize {
        let x = features(ctx);
        let mut best = 0;
        let mut best_score = f64::NEG_INFINITY;
        for (i, arm) in self.arms.iter().enumerate() {
            let l = cholesky(&arm.a);
            let y = forward_sub(&l, &arm.b);
            let mu = back_sub(&l, &y);
            let score: f64 = (0..DIM).map(|d| mu[d] * x[d]).sum();
            if score > best_score {
                best_score = score;
                best = i;
            }
        }
        best
    }

    /// Thompson draw: one posterior sample per arm, route to the argmax.
    /// Deterministic in (state, salt) — same no-global-RNG convention as the
    /// ε-greedy bandit.
    pub fn choose(&self, ctx: &Context, salt: u64) -> &'static Arm {
        let x = features(ctx);
        let mut rng = Xoshiro::new(salt);
        let mut best = 0;
        let mut best_score = f64::NEG_INFINITY;
        for (i, arm) in self.arms.iter().enumerate() {
            let theta = arm.sample(&mut rng);
            let score: f64 = (0..DIM).map(|d| theta[d] * x[d]).sum();
            if score > best_score {
                best_score = score;
                best = i;
            }
        }
        &ARMS[best]
    }

    /// Monte-Carlo propensity of `arm_idx` under the current posterior
    /// (PROPENSITY_DRAWS independent Thompson draws; floored so a logged row
    /// can never carry propensity 0 — IPS divides by it).
    pub fn propensity(&self, ctx: &Context, arm_idx: usize, salt: u64) -> f32 {
        let x = features(ctx);
        let mut hits = 0usize;
        for k in 0..PROPENSITY_DRAWS {
            let mut rng = Xoshiro::new(salt ^ ((k as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)));
            let mut best = 0;
            let mut best_score = f64::NEG_INFINITY;
            for (i, arm) in self.arms.iter().enumerate() {
                let theta = arm.sample(&mut rng);
                let score: f64 = (0..DIM).map(|d| theta[d] * x[d]).sum();
                if score > best_score {
                    best_score = score;
                    best = i;
                }
            }
            if best == arm_idx {
                hits += 1;
            }
        }
        ((hits as f32) / (PROPENSITY_DRAWS as f32)).max(1.0 / PROPENSITY_DRAWS as f32)
    }
}

/// Cholesky factor L (lower-triangular, row-major) of a symmetric
/// positive-definite matrix. A is SPD by construction (λI plus a sum of
/// outer products), so the sqrt argument stays positive; the max(ε) guards
/// float round-off only.
fn cholesky(a: &[f64]) -> Vec<f64> {
    let mut l = vec![0.0; DIM * DIM];
    for i in 0..DIM {
        for j in 0..=i {
            let mut sum = a[i * DIM + j];
            for k in 0..j {
                sum -= l[i * DIM + k] * l[j * DIM + k];
            }
            if i == j {
                l[i * DIM + i] = sum.max(1e-12).sqrt();
            } else {
                l[i * DIM + j] = sum / l[j * DIM + j];
            }
        }
    }
    l
}

/// Solve L y = rhs (L lower-triangular).
fn forward_sub(l: &[f64], rhs: &[f64; DIM]) -> [f64; DIM] {
    let mut y = [0.0; DIM];
    for i in 0..DIM {
        let mut sum = rhs[i];
        for (j, yj) in y.iter().enumerate().take(i) {
            sum -= l[i * DIM + j] * yj;
        }
        y[i] = sum / l[i * DIM + i];
    }
    y
}

/// Solve Lᵀ x = rhs (so x = L⁻ᵀ rhs).
fn back_sub(l: &[f64], rhs: &[f64; DIM]) -> [f64; DIM] {
    let mut x = [0.0; DIM];
    for i in (0..DIM).rev() {
        let mut sum = rhs[i];
        for (j, xj) in x.iter().enumerate().skip(i + 1) {
            sum -= l[j * DIM + i] * xj;
        }
        x[i] = sum / l[i * DIM + i];
    }
    x
}

/// xoshiro256** — same no-global-RNG convention as the rest of the crate;
/// deterministic per (seed), cheap, and good enough for posterior draws.
struct Xoshiro {
    s: [u64; 4],
}

impl Xoshiro {
    fn new(seed: u64) -> Self {
        // splitmix64 expansion of the seed.
        let mut sm = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut next = || {
            sm = sm.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = sm;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        Self {
            s: [next(), next(), next(), next()],
        }
    }

    fn next_u64(&mut self) -> u64 {
        let r = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        r
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Box-Muller; one value per call (the pair's twin is discarded —
    /// simplicity over thrift at this call volume).
    fn next_gaussian(&mut self) -> f64 {
        let u1 = self.next_f64().max(1e-12);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(intent: u8, lang: u8) -> Context {
        Context {
            intent,
            len_bucket: 1,
            lang,
            tod_bucket: 2,
            geo_filter: false,
        }
    }

    #[test]
    fn features_are_one_hot_per_field() {
        let x = features(&ctx(2, 11));
        assert_eq!(x[0], 1.0, "bias");
        assert_eq!(x.iter().filter(|&&v| v != 0.0).count(), 5);
        assert_eq!(x[1 + 2], 1.0);
        assert_eq!(x[9 + 3], 1.0, "lang 11 mod 8 = 3");
    }

    #[test]
    fn cholesky_roundtrips_identity_scaled() {
        let p = ArmPosterior::new(); // A = λI
        let l = cholesky(&p.a);
        for i in 0..DIM {
            assert!((l[i * DIM + i] - LAMBDA.sqrt()).abs() < 1e-12);
        }
        // L y = e0 then Lᵀ x = y must equal A⁻¹ e0 = e0/λ.
        let mut e0 = [0.0; DIM];
        e0[0] = 1.0;
        let y = forward_sub(&l, &e0);
        let x = back_sub(&l, &y);
        assert!((x[0] - 1.0 / LAMBDA).abs() < 1e-12);
    }

    #[test]
    fn learns_context_dependent_routing() {
        // Arm 1 is right for intent 0; arm 2 for intent 1. Train on enough
        // simulated decisions and the policy must route each context to its
        // arm nearly always (Thompson noise allows rare exploration).
        let mut p = ContextualPolicy::new();
        let mut rng = Xoshiro::new(42);
        for i in 0..4_000 {
            let intent = (i % 2) as u8;
            let c = ctx(intent, 0);
            let arm = (i / 2) % ARMS.len(); // round-robin training data
            let good = (intent == 0 && arm == 1) || (intent == 1 && arm == 2);
            let reward = if good {
                rng.next_f64() < 0.7
            } else {
                rng.next_f64() < 0.2
            };
            p.observe(&c, arm, reward);
        }
        for (intent, want) in [(0u8, "broad"), (1u8, "reference")] {
            let c = ctx(intent, 0);
            let hits = (0..100)
                .filter(|&s| p.choose(&c, 1000 + s).id == want)
                .count();
            assert!(hits >= 90, "intent {intent}: {hits}/100 routed to {want}");
        }
    }

    #[test]
    fn propensities_floor_and_concentrate() {
        let p = ContextualPolicy::new();
        let c = ctx(0, 0);
        // Untrained: roughly uniform, every arm > floor.
        let total: f32 = (0..ARMS.len()).map(|a| p.propensity(&c, a, 7)).sum();
        assert!((total - 1.0).abs() < 0.2, "≈ sums to 1, got {total}");

        let mut trained = ContextualPolicy::new();
        let mut rng = Xoshiro::new(7);
        for i in 0..3_000 {
            let arm = i % ARMS.len();
            trained.observe(&c, arm, arm == 0 && rng.next_f64() < 0.9);
        }
        assert!(trained.propensity(&c, 0, 9) > 0.8, "winner concentrates");
        assert!(trained.propensity(&c, 1, 9) >= 1.0 / PROPENSITY_DRAWS as f32);
    }

    #[test]
    fn replay_from_decisions_counts_generalized_rows() {
        let decisions = vec![
            Decision {
                day: 20_000,
                context: Some(ctx(0, 0)),
                arm: 0,
                propensity: 0.93,
                reward: true,
            },
            Decision {
                day: 20_000,
                context: None, // generalized: tally only
                arm: 1,
                propensity: 0.03,
                reward: false,
            },
        ];
        let p = ContextualPolicy::from_decisions(&decisions);
        assert_eq!(p.decisions_seen, 2);
    }
}
