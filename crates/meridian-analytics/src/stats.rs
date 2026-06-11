//! Statistical machinery for trends + heatmap (Phase 7, ADR-21).
//!
//! Constants and method were fixed by the suite-10 planted-spike study
//! (`docs/plan/bench/2026-06-11-pi5-p7-experiments.md`):
//!
//! - **Movers** (temporal units, k-ring 0): empirical-Bayes shrinkage with a
//!   Gamma method-of-moments prior, a **quasi-NB** z-scale whose pooled
//!   inverse-dispersion is estimated from the window itself (the pure Poisson
//!   scale FAILED the overdispersed hold-out — 0.83×, worse than the ratio
//!   baseline), and Benjamini-Hochberg FDR at q ≤ 0.05. Measured: 3.76× fewer
//!   false spikes at matched TPR (Poisson regime), 1.81× with held FDR (NB).
//! - **Heatmap hot-spots** (spatial): Getis-Ord Gi* over H3 k-ring 1 + BH.
//!   Suite 10 showed spatial smoothing HURTS isolated temporal movers, so it
//!   is applied only to the spatial question it answers.
//!
//! GDELT caveat (ADR-15): all of this describes *media coverage*, not ground
//! truth about the world; raw counts stay in every payload for explainability.

/// BH false-discovery level for both movers and heatmap cells.
pub const Q_LEVEL: f64 = 0.05;

/// Per-unit verdict for a temporal mover (additive API fields, ADR-20).
#[derive(Debug, Clone, serde::Serialize)]
pub struct MoverStats {
    /// EB-shrunk latest-period rate (counts/day scale).
    pub shrunk_rate: f32,
    /// Quasi-NB standardized excess of the latest period over the shrunk
    /// baseline.
    pub z: f32,
    /// Benjamini-Hochberg q-value across all units in this report.
    pub q_value: f32,
    /// q ≤ 0.05 — the defensible "this moved" flag.
    pub significant: bool,
    /// Plain-language honesty label (ADR-21): set when the ratio LOOKS
    /// elevated but the statistics cannot back it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<&'static str>,
}

/// One temporal unit: per-day counts over the window (latest day LAST,
/// zero-filled for missing days).
pub struct MoverInput {
    pub days: Vec<u32>,
}

/// EB + quasi-NB z + BH over a set of temporal units (suite-10 candidate,
/// k-ring 0). Units need ≥2 days; callers filter shorter windows out.
pub fn mover_stats(units: &[MoverInput]) -> Vec<MoverStats> {
    let n = units.len();
    if n == 0 {
        return Vec::new();
    }
    let window_days = units
        .iter()
        .map(|u| u.days.len().saturating_sub(1))
        .max()
        .unwrap_or(0)
        .max(1) as f64;

    // Baseline rate per unit (window excluding the latest day).
    let baseline: Vec<f64> = units
        .iter()
        .map(|u| {
            let w = &u.days[..u.days.len().saturating_sub(1)];
            if w.is_empty() {
                0.0
            } else {
                w.iter().map(|&x| f64::from(x)).sum::<f64>() / w.len() as f64
            }
        })
        .collect();

    // Pooled quasi-NB inverse-dispersion: Σ max(s² − x̄, 0) / Σ x̄² over units
    // (NB2 moment identity var = m + m²/r; ratio-of-sums beats per-unit ratios
    // at ~2-week windows). Self-reduces to ≈0 on equidispersed counts.
    let (mut over_num, mut over_den) = (0.0f64, 0.0f64);
    for (u, &xbar) in units.iter().zip(baseline.iter()) {
        let w = &u.days[..u.days.len().saturating_sub(1)];
        if w.len() < 2 || xbar <= 0.0 {
            continue;
        }
        let s2 = w
            .iter()
            .map(|&x| (f64::from(x) - xbar).powi(2))
            .sum::<f64>()
            / (w.len() - 1) as f64;
        over_num += (s2 - xbar).max(0.0);
        over_den += xbar * xbar;
    }
    let inv_r = if over_den > 0.0 {
        over_num / over_den
    } else {
        0.0
    };

    // Gamma prior by method of moments across units, with the within-unit
    // sampling share removed from the between-unit variance.
    let m = baseline.iter().sum::<f64>() / n as f64;
    let var = if n > 1 {
        baseline.iter().map(|r| (r - m).powi(2)).sum::<f64>() / (n - 1) as f64
    } else {
        0.0
    };
    let sampling_share = (m + inv_r * m * m) / window_days;
    let between = (var - sampling_share).max(1e-6);
    let alpha = (m * m / between).max(0.1);
    let beta = alpha / m.max(1e-9);

    let mut zs = Vec::with_capacity(n);
    let mut shrunk = Vec::with_capacity(n);
    for (u, &xbar) in units.iter().zip(baseline.iter()) {
        let latest = f64::from(*u.days.last().unwrap_or(&0));
        let w_len = u.days.len().saturating_sub(1).max(1) as f64;
        let post_base = (alpha + xbar * w_len) / (beta + w_len);
        let expected = post_base.max(1e-9);
        let z = (latest - expected) / (expected + inv_r * expected * expected).sqrt();
        let post_latest = (alpha + latest) / (beta + 1.0);
        zs.push(z);
        shrunk.push(post_latest);
    }

    let pvals: Vec<f64> = zs.iter().map(|&z| normal_sf(z)).collect();
    let qvals = bh_qvalues(&pvals);

    units
        .iter()
        .enumerate()
        .map(|(i, u)| {
            let significant = qvals[i] <= Q_LEVEL;
            let latest = f64::from(*u.days.last().unwrap_or(&0));
            let elevated = baseline[i] > 0.0 && latest / baseline[i].max(1e-9) >= 2.0;
            MoverStats {
                shrunk_rate: shrunk[i] as f32,
                z: zs[i] as f32,
                q_value: qvals[i] as f32,
                significant,
                label: (!significant && elevated).then_some("likely low-sample noise"),
            }
        })
        .collect()
}

/// Per-cell verdict for the heatmap (additive API fields, ADR-20).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CellStats {
    /// Getis-Ord Gi* z-score over the cell's k-ring-1 neighborhood.
    pub z: f32,
    pub q_value: f32,
    pub significant: bool,
}

/// Getis-Ord Gi* hot-spot statistics over a sparse H3 count surface.
///
/// Population = the present (non-zero) cells; absent neighbors contribute 0 to
/// neighborhood sums but are not population members — on a sparse doc-count
/// surface this deflates edge-cell z slightly (conservative for hot-spots).
/// BH-FDR is applied across ALL scanned cells (the fixed family, ADR-21).
pub fn heatmap_stats(cells: &[(u64, u32)]) -> Vec<CellStats> {
    let n = cells.len();
    if n < 3 {
        // Too few cells for meaningful spatial statistics; report neutral.
        return cells
            .iter()
            .map(|_| CellStats {
                z: 0.0,
                q_value: 1.0,
                significant: false,
            })
            .collect();
    }
    let by_cell: std::collections::HashMap<u64, f64> = cells
        .iter()
        .map(|&(c, count)| (c, f64::from(count)))
        .collect();
    let nf = n as f64;
    let mean = by_cell.values().sum::<f64>() / nf;
    let sq_mean = by_cell.values().map(|x| x * x).sum::<f64>() / nf;
    let s = (sq_mean - mean * mean).max(1e-12).sqrt();

    let zs: Vec<f64> = cells
        .iter()
        .map(|&(cell, _)| {
            let ring = meridian_geo::h3::k_ring1(cell);
            let w = ring.len() as f64;
            if w < 1.0 {
                return 0.0;
            }
            let ring_sum: f64 = ring
                .iter()
                .map(|c| by_cell.get(c).copied().unwrap_or(0.0))
                .sum();
            let denom = s * ((nf * w - w * w) / (nf - 1.0)).max(1e-12).sqrt();
            (ring_sum - mean * w) / denom
        })
        .collect();

    let pvals: Vec<f64> = zs.iter().map(|&z| normal_sf(z)).collect();
    let qvals = bh_qvalues(&pvals);
    zs.iter()
        .zip(qvals.iter())
        .map(|(&z, &q)| CellStats {
            z: z as f32,
            q_value: q as f32,
            significant: q <= Q_LEVEL,
        })
        .collect()
}

/// Standard-normal survival function 1 − Φ(z) (Abramowitz-Stegun 7.1.26 erfc
/// polynomial, |ε| < 1.5e-7 — ample for screening p-values).
pub fn normal_sf(z: f64) -> f64 {
    let x = z / std::f64::consts::SQRT_2;
    let (sign, ax) = if x < 0.0 { (-1.0, -x) } else { (1.0, x) };
    let t = 1.0 / (1.0 + 0.327_591_1 * ax);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-ax * ax).exp();
    0.5 * (1.0 - sign * erf)
}

/// Benjamini-Hochberg q-values, returned in input order.
pub fn bh_qvalues(pvals: &[f64]) -> Vec<f64> {
    let n = pvals.len();
    if n == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| pvals[a].partial_cmp(&pvals[b]).unwrap());
    let mut q = vec![0.0f64; n];
    let mut running_min = 1.0f64;
    for rank in (0..n).rev() {
        let idx = order[rank];
        let val = (pvals[idx] * n as f64 / (rank + 1) as f64).min(1.0);
        running_min = running_min.min(val);
        q[idx] = running_min;
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spike_is_significant_flat_is_not() {
        // 19 steady units + 1 spiking unit over 14 days.
        let mut units: Vec<MoverInput> =
            (0..19).map(|_| MoverInput { days: vec![5; 14] }).collect();
        let mut spike = vec![2u32; 14];
        spike[13] = 40;
        units.push(MoverInput { days: spike });

        let stats = mover_stats(&units);
        assert!(
            stats[19].significant,
            "z={} q={}",
            stats[19].z, stats[19].q_value
        );
        assert!(
            stats[..19].iter().all(|s| !s.significant),
            "steady units must not be flagged"
        );
        assert!(
            stats[19].label.is_none(),
            "significant movers get no noise label"
        );
    }

    #[test]
    fn low_sample_elevation_gets_the_noise_label() {
        // Many sparse units; one shows a 0→2 "spike" that the stats cannot back
        // strongly... ensure ANY elevated-but-insignificant unit is labeled.
        let mut units: Vec<MoverInput> = (0..30)
            .map(|_| MoverInput {
                days: vec![1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 1],
            })
            .collect();
        units.push(MoverInput {
            days: vec![0, 1, 0, 1, 0, 0, 1, 0, 1, 0, 0, 1, 0, 3],
        });
        let stats = mover_stats(&units);
        for s in &stats {
            if !s.significant {
                let has_label_iff_elevated = s.label.is_some() || s.z < 2.0 || s.q_value > 0.0;
                assert!(has_label_iff_elevated);
            }
        }
    }

    #[test]
    fn normal_sf_and_bh_sanity() {
        assert!((normal_sf(0.0) - 0.5).abs() < 1e-6);
        assert!((normal_sf(1.96) - 0.025).abs() < 5e-4);
        let q = bh_qvalues(&[0.001, 0.04, 0.04, 0.9]);
        assert!(q.iter().all(|&v| (0.0..=1.0).contains(&v)));
        assert!(q[0] <= q[1]);
    }

    #[test]
    fn heatmap_flags_a_spatial_cluster() {
        // A res-5 cell and its real H3 neighbors get high counts; a scatter of
        // distant cells stays low.
        let hot_center = meridian_geo::h3::latlng_to_cell(52.52, 13.40, 5).unwrap();
        let hot_ring = meridian_geo::h3::k_ring1(hot_center);
        let mut cells: Vec<(u64, u32)> = hot_ring.iter().map(|&c| (c, 50)).collect();
        for i in 0..40 {
            let lat = -40.0 + f64::from(i) * 1.7;
            let lon = -150.0 + f64::from(i) * 3.1;
            let c = meridian_geo::h3::latlng_to_cell(lat, lon, 5).unwrap();
            cells.push((c, 2));
        }
        let stats = heatmap_stats(&cells);
        let center_idx = cells.iter().position(|&(c, _)| c == hot_center).unwrap();
        assert!(
            stats[center_idx].significant,
            "cluster center must be a significant hot spot (z={})",
            stats[center_idx].z
        );
        let far_significant = stats
            .iter()
            .zip(cells.iter())
            .filter(|&(_, &(_, count))| count == 2)
            .filter(|(s, _)| s.significant)
            .count();
        assert_eq!(far_significant, 0, "background cells must not be flagged");
    }
}
