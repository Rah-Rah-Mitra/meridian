//! Suite 5 — RRF + LTR-style scoring microbench (SPEC §15.5).
//! Gate: fusing 1000 BM25 + 200 ANN + 3×20 engine lists, then linearly scoring the
//! top-100 over 9 features, must complete in <2ms per call.

use super::{BenchConfig, SuiteResult};
use crate::stats::{Rng, percentile_ms};
use meridian_query::rrf::{RRF_K, rrf_fuse};
use std::time::Instant;

const FEATURES: usize = 9;
const ITERATIONS: usize = 1_000;

pub fn run(_cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("fusion");
    let start = Instant::now();
    let mut rng = Rng::new(0xF051);

    // Candidate lists with realistic overlap: draws from a 5k-doc universe.
    let mut make_list = |len: usize| -> Vec<u64> {
        let mut seen = std::collections::HashSet::new();
        let mut list = Vec::with_capacity(len);
        while list.len() < len {
            let key = rng.next_u64() % 5_000;
            if seen.insert(key) {
                list.push(key);
            }
        }
        list
    };
    let lists: Vec<Vec<u64>> = [1000, 200, 20, 20, 20]
        .iter()
        .map(|&n| make_list(n))
        .collect();

    // Cold-start linear LTR stand-in: dot(weights, features) over the top-100
    // (the Phase-3 GBDT replaces the scorer, not the shape of this stage).
    let weights: [f32; FEATURES] = [0.9, 0.4, 0.5, 0.3, 0.2, 0.25, 0.1, 0.05, 0.15];

    let mut samples = Vec::with_capacity(ITERATIONS);
    let mut sink = 0.0f32;
    for _ in 0..ITERATIONS {
        let t = Instant::now();
        let fused = rrf_fuse(&lists, RRF_K);
        for (key, rrf_score) in fused.iter().take(100) {
            let mut features = [0.0f32; FEATURES];
            features[0] = *rrf_score;
            for (i, f) in features.iter_mut().enumerate().skip(1) {
                // Deterministic pseudo-features derived from the doc key.
                *f = ((key.wrapping_mul(i as u64 + 1) % 1000) as f32) / 1000.0;
            }
            sink += features
                .iter()
                .zip(weights.iter())
                .map(|(a, b)| a * b)
                .sum::<f32>();
        }
        samples.push(t.elapsed().as_secs_f64() * 1e3);
    }
    std::hint::black_box(sink);

    let p50 = percentile_ms(&mut samples, 50.0);
    let p99 = percentile_ms(&mut samples, 99.0);
    result.metric("p50_ms", p50);
    result.metric("p99_ms", p99);
    result.metric("iterations", ITERATIONS);
    result.gate(
        "RRF(1000+200+3×20) + linear LTR top-100 < 2ms p50",
        p50 < 2.0,
    );
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}
