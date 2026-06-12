//! Two-state burst decode for trends day series (Phase 10, ADR-28).
//!
//! Complements the ADR-21 EB latest-day z, which tests ONLY the latest day
//! against the shrunk window baseline and is structurally blind to a
//! multi-day ramp: each elevated day inflates the baseline the next day is
//! judged against, so a story building over 3–5 days never makes any single
//! day extreme. The burst decode answers the question z cannot — "is this
//! series inside a sustained elevation, and since when".
//!
//! Model: Kleinberg's burst automaton restricted to two states — baseline
//! rate λ₀ vs elevated rate s·λ₀ — with negative-binomial emissions (pooled
//! per-series dispersion from the window's own moments, the same NB2 identity
//! ADR-21 adopted after the Poisson scale failed its overdispersed hold-out)
//! and an entering-cost γ·ln(n) per 0→1 transition. Exact Viterbi, O(2n),
//! deterministic. λ₀ is a lower-trimmed mean so the elevation being tested
//! does not contaminate its own baseline.
//!
//! Constants `(s, γ)` are FIXED by the suite-17 sweep (tuning variant) and
//! judged frozen on an overdispersed hold-out — ADR-28 records the run.

/// Burst automaton parameters. Defaults are the suite-17-frozen constants.
#[derive(Debug, Clone, Copy)]
pub struct BurstParams {
    /// Elevated-state rate multiplier (λ₁ = s·λ₀).
    pub s: f64,
    /// Transition-cost coefficient: entering the elevated state costs
    /// γ·ln(n) nats (n = window length).
    pub gamma: f64,
}

impl Default for BurstParams {
    fn default() -> Self {
        // FROZEN by the suite-17 sweep (2026-06-12, tuning variant; ADR-28):
        // best admissible ramp TPR 0.692 (z baseline 0.408) at null FPR
        // 0.0089 ≤ z's 0.0196, chosen by the stated rule (max TPR among
        // FPR-admissible configs, largest γ within 0.05 TPR tolerance).
        // bench/2026-06-12-pi5-p10-changepoint.json.
        Self { s: 2.0, gamma: 1.0 }
    }
}

/// Trailing-burst summary for one series (additive API fields, ADR-20).
#[derive(Debug, Clone, serde::Serialize)]
pub struct BurstSummary {
    /// The latest day is inside an elevated run.
    pub active: bool,
    /// Index into the window (0-based, oldest first) where the trailing
    /// elevated run began. Present only when `active`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_index: Option<usize>,
    /// Length of the trailing elevated run in days (0 when not active).
    pub days_active: usize,
}

/// Minimum window length for a meaningful decode: below this the baseline
/// estimate and the transition cost have nothing to work with.
pub const MIN_WINDOW_DAYS: usize = 7;

/// Viterbi decode of the two-state automaton over a day series (oldest
/// first). Returns the elevated-state indicator per day, or None when the
/// window is too short (`MIN_WINDOW_DAYS`) or has no signal at all.
///
/// Baseline moments come from the HEAD 60% of the window — twice amended by
/// suite 17 (ADR-28 records the falsifications): (v0) lower-trimmed moments
/// truncate exactly the upper-tail evidence that distinguishes overdispersion
/// from elevation, so NB noise decoded as bursts (hold-out null FPR 0.104 vs
/// the z baseline's 0.037); (v1) two-pass re-estimation from decoded-baseline
/// days is circular on null series — a liberal first pass labels the noise
/// tail "elevated" and the clean residue confirms the false fire (0.055,
/// still above z). The head window is what the feature's question implies:
/// burst means a TRAILING elevation against the earlier baseline, so the
/// earlier window is the uncontaminated estimation region, with unbiased
/// dispersion on quiet series. An elevation old enough to dominate the head
/// is the series' new normal, not a burst — the latest-day z still owns the
/// single-day question either way.
pub fn decode(days: &[u32], params: &BurstParams) -> Option<Vec<bool>> {
    let n = days.len();
    if n < MIN_WINDOW_DAYS || days.iter().all(|&d| d == 0) {
        return None;
    }

    let head = &days[..(n * 3).div_ceil(5)];
    let m = (head.iter().map(|&x| f64::from(x)).sum::<f64>() / head.len() as f64).max(0.1);
    let s2 = head
        .iter()
        .map(|&x| (f64::from(x) - m).powi(2))
        .sum::<f64>()
        / (head.len() as f64 - 1.0).max(1.0);
    let inv_r = ((s2 - m).max(0.0) / (m * m)).min(10.0);
    Some(viterbi(days, m, inv_r, params))
}

/// Exact two-state Viterbi with NB emissions: baseline rate λ₀ vs s·λ₀,
/// entering the elevated state costs γ·ln(n) nats, leaving is free.
fn viterbi(days: &[u32], lambda0: f64, inv_r: f64, params: &BurstParams) -> Vec<bool> {
    let n = days.len();
    let lambda1 = params.s * lambda0;
    let nll = |x: u32, lambda: f64| -> f64 { -nb_log_pmf(x, lambda, inv_r) };
    let enter_cost = params.gamma * (n as f64).ln();

    let mut cost0 = nll(days[0], lambda0);
    let mut cost1 = enter_cost + nll(days[0], lambda1);
    let mut back: Vec<(bool, bool)> = Vec::with_capacity(n); // prev state feeding (0, 1)
    back.push((false, false));
    for &x in &days[1..] {
        let stay0 = cost0;
        let from1 = cost1;
        let (best0_prev, best0) = if stay0 <= from1 {
            (false, stay0)
        } else {
            (true, from1)
        };
        let stay1 = cost1;
        let from0 = cost0 + enter_cost;
        let (best1_prev, best1) = if stay1 <= from0 {
            (true, stay1)
        } else {
            (false, from0)
        };
        cost0 = best0 + nll(x, lambda0);
        cost1 = best1 + nll(x, lambda1);
        back.push((best0_prev, best1_prev));
    }

    let mut states = vec![false; n];
    let mut state = cost1 < cost0; // tie → baseline
    states[n - 1] = state;
    for t in (1..n).rev() {
        let (p0, p1) = back[t];
        state = if state { p1 } else { p0 };
        states[t - 1] = state;
    }
    states
}

/// Summarize the trailing elevated run of a decode.
pub fn summarize(states: &[bool]) -> BurstSummary {
    if states.last() != Some(&true) {
        return BurstSummary {
            active: false,
            onset_index: None,
            days_active: 0,
        };
    }
    let run = states.iter().rev().take_while(|&&s| s).count();
    BurstSummary {
        active: true,
        onset_index: Some(states.len() - run),
        days_active: run,
    }
}

/// Negative-binomial (NB2) log-pmf with mean λ and inverse-dispersion 1/r;
/// degrades to the Poisson log-pmf as inv_r → 0.
fn nb_log_pmf(x: u32, lambda: f64, inv_r: f64) -> f64 {
    let xf = f64::from(x);
    if inv_r < 1e-9 {
        return xf * lambda.ln() - lambda - ln_gamma(xf + 1.0);
    }
    let r = 1.0 / inv_r;
    ln_gamma(xf + r) - ln_gamma(r) - ln_gamma(xf + 1.0)
        + r * (r / (r + lambda)).ln()
        + xf * (lambda / (r + lambda)).ln()
}

/// Lanczos (g = 7, n = 9) log-gamma — plenty for count likelihoods.
fn ln_gamma(x: f64) -> f64 {
    // The canonical published table; the extra digits are f64-truncated, kept
    // verbatim so the source is recognizable.
    #[allow(clippy::excessive_precision)]
    const COEF: [f64; 9] = [
        0.999_999_999_999_809_93,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_13,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // Reflection: Γ(x)Γ(1−x) = π / sin(πx)
        return std::f64::consts::PI.ln()
            - (std::f64::consts::PI * x).sin().ln()
            - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = COEF[0];
    let t = x + 7.5;
    for (i, &c) in COEF.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ln_gamma_matches_factorials() {
        // Γ(n) = (n−1)!
        for (n, fact) in [(1.0, 1.0f64), (2.0, 1.0), (5.0, 24.0), (10.0, 362_880.0)] {
            assert!(
                (ln_gamma(n) - fact.ln()).abs() < 1e-9,
                "ln Γ({n}) = {} want {}",
                ln_gamma(n),
                fact.ln()
            );
        }
    }

    #[test]
    fn sustained_ramp_decodes_active_with_onset() {
        // 20 quiet days at ~5, then a ramp to 15 sustained for 8 days — the
        // exact shape the latest-day z is blind to.
        let mut days = vec![5u32; 20];
        days.extend([8, 11, 14, 15, 15, 15, 15, 15]);
        let states = decode(&days, &BurstParams::default()).unwrap();
        let s = summarize(&states);
        assert!(
            s.active,
            "sustained elevation must decode active: {states:?}"
        );
        assert!(s.days_active >= 5, "run too short: {s:?}");
        assert!(
            s.onset_index.unwrap() >= 19 && s.onset_index.unwrap() <= 23,
            "{s:?}"
        );
    }

    #[test]
    fn stationary_series_stays_quiet() {
        let days = vec![5u32, 6, 4, 5, 7, 5, 4, 6, 5, 5, 6, 4, 5, 5];
        let states = decode(&days, &BurstParams::default()).unwrap();
        assert!(!summarize(&states).active, "{states:?}");
    }

    #[test]
    fn single_day_blip_does_not_open_a_burst() {
        // One elevated day cannot pay the entering cost — that case belongs
        // to the latest-day z, not to burst (ADR-28 division of labor).
        let mut days = vec![5u32; 13];
        days.push(9);
        let states = decode(&days, &BurstParams::default()).unwrap();
        assert!(!summarize(&states).active, "{states:?}");
    }

    #[test]
    fn short_or_empty_windows_refuse() {
        assert!(decode(&[5; 6], &BurstParams::default()).is_none());
        assert!(decode(&[0; 14], &BurstParams::default()).is_none());
    }
}
