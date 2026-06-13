//! E3 — day-of-week seasonal de-trending for the trends/mover statistics.
//!
//! GDELT media volume carries a strong weekly cycle (weekend dip). The mover
//! baseline (`stats::mover_stats`) is the unweighted window mean; when the
//! latest day is a high-volume weekday judged against a baseline window that
//! contains low-volume weekend days, the latest/baseline excess — and hence the
//! quasi-NB z — is inflated. Because the cycle is a COMMON multiplicative factor
//! across all ~20 CAMEO root codes, the inflation is correlated across the whole
//! BH family, so BH-FDR cannot absorb it (every p-value shifts together).
//!
//! The fix is a multiplicative day-of-week pre-adjustment applied BEFORE the EB
//! fit: estimate a per-weekday seasonal index (that weekday's mean over the
//! overall mean), shrink it toward 1.0, and divide each day by its index.
//!
//! Privacy: GDELT-only, no document data, no persisted structure (the index is
//! recomputed per report) — no ADR-19 deletion class, no forget-test surface,
//! anon lane untouched. Constants frozen by the suite-17 seasonal extension
//! (risk-#21 tuning/hold-out protocol).

/// Minimum window length for seasonal adjustment: ~3 observations per weekday.
/// Below this the per-weekday index is too noisy to trust; adjustment is the
/// identity (the shipped behaviour) so short windows are never destabilised.
pub const SEASONAL_MIN_DAYS: usize = 21;

/// Shrinkage pseudo-count toward an index of 1.0. Larger = more conservative
/// (closer to no adjustment). Frozen by the suite-17 seasonal sweep.
pub const SEASONAL_LAMBDA: f64 = 8.0;

/// Per-weekday multiplicative seasonal indices, shrunk toward 1.0.
///
/// `dow0` is the weekday (0..6) of `days[0]`; weekday of `days[i]` is
/// `(dow0 + i) % 7`. `index[d] = (n_d * r_d + lambda) / (n_d + lambda)` where
/// `r_d = mean(days on weekday d) / mean(all days)` and `n_d` is the number
/// of observations of weekday `d`. Returns all-1.0 when `days.len() <
/// SEASONAL_MIN_DAYS`, when the overall mean is non-positive, or (per weekday)
/// when that weekday has no observations.
pub fn weekday_indices(days: &[f64], dow0: usize, lambda: f64) -> [f64; 7] {
    let mut idx = [1.0f64; 7];
    if days.len() < SEASONAL_MIN_DAYS {
        return idx;
    }
    let overall_mean = days.iter().sum::<f64>() / days.len() as f64;
    if overall_mean <= 0.0 {
        return idx;
    }
    let mut sum = [0.0f64; 7];
    let mut cnt = [0u32; 7];
    for (i, &v) in days.iter().enumerate() {
        let d = (dow0 + i) % 7;
        sum[d] += v;
        cnt[d] += 1;
    }
    for d in 0..7 {
        if cnt[d] == 0 {
            continue;
        }
        let weekday_mean = sum[d] / cnt[d] as f64;
        let r_d = weekday_mean / overall_mean;
        let n_d = cnt[d] as f64;
        idx[d] = (n_d * r_d + lambda) / (n_d + lambda);
    }
    idx
}

/// Divide each day by its weekday seasonal index (mean-preserving DOW
/// flattening). The index is estimated from the window EXCLUDING the latest day
/// (mirroring the mover baseline) so the latest-day value never deflates its own
/// weekday index — protecting a genuine latest-day spike/ramp. Returns the input
/// unchanged when `days.len() < SEASONAL_MIN_DAYS`.
pub fn deseasonalize(days: &[f64], dow0: usize, lambda: f64) -> Vec<f64> {
    if days.len() < SEASONAL_MIN_DAYS {
        return days.to_vec();
    }
    let idx = weekday_indices(&days[..days.len() - 1], dow0, lambda);
    days.iter()
        .enumerate()
        .map(|(i, &v)| v / idx[(dow0 + i) % 7].max(1e-6))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_window_is_identity() {
        let days = vec![1.0, 2.0, 3.0];
        assert_eq!(deseasonalize(&days, 0, 8.0), days);
    }

    #[test]
    fn flat_series_indices_are_one() {
        let days = vec![5.0; 28];
        let idx = weekday_indices(&days[..27], 0, 8.0);
        for f in idx {
            assert!((f - 1.0).abs() < 1e-9, "flat index {f}");
        }
    }

    #[test]
    fn weekend_dip_index_below_one_then_flattened() {
        // 28 days, weekend (dow 5,6) at half volume; dow0 = 0 (Monday).
        let mut days = vec![0.0; 28];
        for (i, v) in days.iter_mut().enumerate() {
            *v = if (i % 7) >= 5 { 5.0 } else { 10.0 };
        }
        let idx = weekday_indices(&days[..27], 0, 8.0);
        assert!(
            idx[5] < 1.0 && idx[6] < 1.0,
            "weekend index should shrink-toward-1 but stay <1"
        );
        assert!(idx[0] > 1.0, "weekday index should be >1");
        // Deseasonalized series should be flatter (lower coefficient of variation).
        let adj = deseasonalize(&days, 0, 8.0);
        assert!(
            cv(&adj) < cv(&days),
            "deseasonalized CV {} should be < raw CV {}",
            cv(&adj),
            cv(&days)
        );
    }

    fn cv(xs: &[f64]) -> f64 {
        let m = xs.iter().sum::<f64>() / xs.len() as f64;
        let v = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / xs.len() as f64;
        v.sqrt() / m
    }
}
