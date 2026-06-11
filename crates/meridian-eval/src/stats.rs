//! Percentile/timing helpers shared by the bench suites.

use std::time::{Duration, Instant};

/// Nearest-rank percentile over millisecond samples. `p` in [0,100].
pub fn percentile_ms(samples: &mut [f64], p: f64) -> f64 {
    if samples.is_empty() {
        return f64::NAN;
    }
    samples.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
    let rank = ((p / 100.0) * samples.len() as f64).ceil().max(1.0) as usize;
    samples[rank.min(samples.len()) - 1]
}

/// Time one call, returning (result, elapsed-ms).
pub fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let out = f();
    (out, ms(start.elapsed()))
}

pub fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// Deterministic xorshift64* PRNG — keeps the suites dependency-free and the
/// synthetic corpora reproducible across runs/machines.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0,1).
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Standard normal via Box-Muller.
    pub fn next_gaussian(&mut self) -> f32 {
        let u1 = self.next_f32().max(1e-7);
        let u2 = self.next_f32();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos()
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Uniform in [0,1) at f64 precision (for p-value-scale draws).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Poisson draw via Knuth's product method — fine for the λ ≲ 50 regime the
    /// spike suite simulates; not for large λ.
    pub fn next_poisson(&mut self, lambda: f64) -> u32 {
        let l = (-lambda).exp();
        let mut k = 0u32;
        let mut p = 1.0;
        loop {
            p *= self.next_f64();
            if p <= l {
                return k;
            }
            k += 1;
        }
    }

    /// Gamma(shape, scale) draw — Marsaglia-Tsang for shape ≥ 1, boost trick below.
    pub fn next_gamma(&mut self, shape: f64, scale: f64) -> f64 {
        if shape < 1.0 {
            let u = self.next_f64().max(1e-12);
            return self.next_gamma(shape + 1.0, scale) * u.powf(1.0 / shape);
        }
        let d = shape - 1.0 / 3.0;
        let c = 1.0 / (9.0 * d).sqrt();
        loop {
            let x = f64::from(self.next_gaussian());
            let v = (1.0 + c * x).powi(3);
            if v <= 0.0 {
                continue;
            }
            let u = self.next_f64().max(1e-12);
            if u.ln() < 0.5 * x * x + d - d * v + d * v.ln() {
                return d * v * scale;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Distribution statistics shared by the post-v0.1.0 suites (9, 10, 12).
// ---------------------------------------------------------------------------

/// Jensen-Shannon divergence between two discrete distributions given as
/// (key → count) maps; log base 2, so the result is bounded [0, 1].
pub fn jsd(
    p: &std::collections::HashMap<u64, f64>,
    q: &std::collections::HashMap<u64, f64>,
) -> f64 {
    let sum_p: f64 = p.values().sum();
    let sum_q: f64 = q.values().sum();
    if sum_p <= 0.0 || sum_q <= 0.0 {
        return f64::NAN;
    }
    let mut keys: Vec<u64> = p.keys().chain(q.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();
    let mut acc = 0.0;
    for k in keys {
        let pi = p.get(&k).copied().unwrap_or(0.0) / sum_p;
        let qi = q.get(&k).copied().unwrap_or(0.0) / sum_q;
        let mi = 0.5 * (pi + qi);
        if pi > 0.0 {
            acc += 0.5 * pi * (pi / mi).log2();
        }
        if qi > 0.0 {
            acc += 0.5 * qi * (qi / mi).log2();
        }
    }
    acc.clamp(0.0, 1.0)
}

/// Standard-normal survival function 1 − Φ(z) via the Abramowitz-Stegun 7.1.26
/// erfc polynomial (|ε| < 1.5e-7 — ample for screening p-values).
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

/// Benjamini-Hochberg q-values, returned in the input order.
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

/// Mean of a slice (NaN on empty).
pub fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

#[cfg(test)]
mod tests {
    use super::{bh_qvalues, jsd, mean, normal_sf, percentile_ms};
    use std::collections::HashMap;

    #[test]
    fn percentile_nearest_rank() {
        let mut s = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(percentile_ms(&mut s, 50.0), 30.0);
        assert_eq!(percentile_ms(&mut s, 99.0), 50.0);
        assert_eq!(percentile_ms(&mut s, 1.0), 10.0);
    }

    #[test]
    fn jsd_bounds() {
        let p: HashMap<u64, f64> = [(1, 10.0), (2, 10.0)].into();
        let q_same = p.clone();
        assert!(jsd(&p, &q_same).abs() < 1e-12);
        let q_disjoint: HashMap<u64, f64> = [(3, 5.0), (4, 5.0)].into();
        assert!((jsd(&p, &q_disjoint) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn normal_sf_reference_points() {
        assert!((normal_sf(0.0) - 0.5).abs() < 1e-6);
        assert!((normal_sf(1.96) - 0.025).abs() < 5e-4);
        assert!((normal_sf(-1.96) - 0.975).abs() < 5e-4);
    }

    #[test]
    fn bh_monotone_and_bounded() {
        let p = vec![0.001, 0.04, 0.04, 0.9];
        let q = bh_qvalues(&p);
        assert!(q.iter().all(|&v| (0.0..=1.0).contains(&v)));
        // The smallest p keeps the smallest q; ties share a q.
        assert!(q[0] <= q[1]);
        assert!((q[1] - q[2]).abs() < 1e-12);
        assert!((mean(&q) - q.iter().sum::<f64>() / 4.0).abs() < 1e-12);
    }
}
