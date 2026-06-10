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
}

#[cfg(test)]
mod tests {
    use super::percentile_ms;

    #[test]
    fn percentile_nearest_rank() {
        let mut s = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(percentile_ms(&mut s, 50.0), 30.0);
        assert_eq!(percentile_ms(&mut s, 99.0), 50.0);
        assert_eq!(percentile_ms(&mut s, 1.0), 10.0);
    }
}
