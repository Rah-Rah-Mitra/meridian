//! Suite 2 — USearch ANN: build 1M×256-d int8, then an `ef_search` SWEEP measuring
//! latency + recall at each setting (SPEC §15: this bench *re-derives* ef_search),
//! plus the ADR-01 16K-page mmap `view()` smoke test.
//! Gate: some swept ef reaches recall@10 ≥0.95 with p99 <40ms (and view() works).
//!
//! Recall is measured against exact brute-force over usearch's OWN int8
//! representation (unit vector × 127) — i.e. pure graph recall over exactly what
//! the index stores and compares. End-to-end quantization loss vs f32 is a
//! ranking-quality question, owned by the Phase-2 eval harness (nDCG hybrid≥BM25).

use super::{BenchConfig, SuiteResult};
use crate::probe::rss_bytes;
use crate::stats::{Rng, percentile_ms};
use std::time::Instant;
use usearch::Index;
use usearch::ffi::{IndexOptions, MetricKind, ScalarKind};

const DIMS: usize = 256;
const QUERIES: usize = 1_000;
const RECALL_QUERIES: usize = 100;
const TOP_K: usize = 10;
const EF_SWEEP: [usize; 4] = [64, 128, 192, 256];

/// Deterministic random unit vector (xorshift + Box-Muller + L2 norm).
fn unit_gaussian(seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed ^ 0xA11CE);
    let mut v: Vec<f32> = (0..DIMS).map(|_| rng.next_gaussian()).collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

/// Clustered vector per the bench plan: a Gaussian MIXTURE, not uniform noise.
/// Real embeddings live on low-dimensional manifolds; on uniformly random 256-d
/// vectors all distances concentrate and HNSW recall is meaningless (measured:
/// recall@10 = 0.012 on the first run — the methodology bug this fixes).
/// centroid(key % 4096) + per-vector noise scale drawn from [0.3, 1.2] relative —
/// varied distances give real nearest-neighbor structure with margins (uniform-σ
/// clusters put ~244 near-equidistant candidates on a knife edge and recall
/// measures tie-breaking, not the graph).
fn unit_vector(key: u64) -> Vec<f32> {
    let centroid = unit_gaussian(0xC0DE_0000 + (key % 4096));
    let mut rng = Rng::new(key.wrapping_mul(0x9E37_79B9) ^ 0x5EED);
    let rel_noise = 0.3 + 0.9 * rng.next_f32();
    let sigma = rel_noise / (DIMS as f32).sqrt();
    let mut v: Vec<f32> = centroid
        .iter()
        .map(|c| c + sigma * rng.next_gaussian())
        .collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

/// usearch's i8 scheme for cosine: unit-normalized components × 127. Using the
/// identical representation makes the brute-force reference rank with exactly
/// the values the index stores — recall then isolates graph quality.
fn quantize_usearch(v: &[f32]) -> Vec<i8> {
    v.iter()
        .map(|x| (x * 127.0).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

fn options() -> IndexOptions {
    IndexOptions {
        dimensions: DIMS,
        metric: MetricKind::Cos,
        quantization: ScalarKind::I8,
        connectivity: 16,
        expansion_add: 128,
        expansion_search: 64,
        ..Default::default()
    }
}

pub fn run(cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("ann");
    let start = Instant::now();
    let n = cfg.ann_vectors;
    result.metric("vectors", n);

    let run_inner = (|| -> Result<(), String> {
        let err = |e: cxx::Exception| format!("usearch: {e}");

        // Reference int8 copy for brute-force recall (256MB at 1M — bounded).
        let mut ref_i8: Vec<i8> = Vec::with_capacity(n * DIMS);
        for key in 0..n as u64 {
            let v = unit_vector(key);
            ref_i8.extend_from_slice(&quantize_usearch(&v));
        }

        let rss0 = rss_bytes();
        let index = Index::new(&options()).map_err(err)?;
        index.reserve(n).map_err(err)?;

        // Concurrent build across all cores (usearch add is thread-safe post-reserve).
        let build_start = Instant::now();
        let threads = std::thread::available_parallelism()
            .map(|t| t.get())
            .unwrap_or(4);
        std::thread::scope(|scope| {
            let index = &index;
            for t in 0..threads {
                scope.spawn(move || {
                    let mut key = t as u64;
                    while (key as usize) < n {
                        let v = unit_vector(key);
                        let _ = index.add(key, &v);
                        key += threads as u64;
                    }
                });
            }
        });
        let build_secs = build_start.elapsed().as_secs_f64();
        result.metric("build_vectors_per_sec", (n as f64 / build_secs).round());
        result.metric(
            "build_rss_delta_mb",
            (rss_bytes().saturating_sub(rss0)) / (1024 * 1024),
        );
        result.metric("index_size", index.size());
        result.metric("memory_usage_mb", index.memory_usage() / (1024 * 1024));

        // Exact truth sets (i8 brute force over the index's own representation) —
        // independent of ef, so computed once for the whole sweep.
        let truths: Vec<std::collections::HashSet<u64>> = (0..RECALL_QUERIES as u64)
            .map(|qi| {
                let qq = quantize_usearch(&unit_vector(0xFFFF_0000 + qi));
                let mut scored: Vec<(i32, u64)> = (0..n)
                    .map(|i| {
                        let row = &ref_i8[i * DIMS..(i + 1) * DIMS];
                        let dot: i32 = row
                            .iter()
                            .zip(qq.iter())
                            .map(|(a, b)| *a as i32 * *b as i32)
                            .sum();
                        (dot, i as u64)
                    })
                    .collect();
                scored.sort_unstable_by_key(|p| std::cmp::Reverse(p.0));
                scored[..TOP_K].iter().map(|p| p.1).collect()
            })
            .collect();

        // ef_search sweep: latency (1k queries, single-threaded) + recall@10 per ef.
        let mut derived_ef = None;
        let mut derived_p99 = f64::NAN;
        for ef in EF_SWEEP {
            index.change_expansion_search(ef);
            let mut samples = Vec::with_capacity(QUERIES);
            for qi in 0..QUERIES as u64 {
                let q = unit_vector(0xFFFF_0000 + qi);
                let t = Instant::now();
                let matches = index.search(&q, TOP_K).map_err(err)?;
                std::hint::black_box(matches.keys.len());
                samples.push(t.elapsed().as_secs_f64() * 1e3);
            }
            let p50 = percentile_ms(&mut samples, 50.0);
            let p99 = percentile_ms(&mut samples, 99.0);
            let mut hits = 0usize;
            for (qi, truth) in truths.iter().enumerate() {
                let qf = unit_vector(0xFFFF_0000 + qi as u64);
                let matches = index.search(&qf, TOP_K).map_err(err)?;
                hits += matches.keys.iter().filter(|k| truth.contains(k)).count();
            }
            let recall = hits as f64 / (RECALL_QUERIES * TOP_K) as f64;
            result.metric(
                &format!("search_p50_ms_ef_{ef}"),
                (p50 * 1000.0).round() / 1000.0,
            );
            result.metric(
                &format!("search_p99_ms_ef_{ef}"),
                (p99 * 1000.0).round() / 1000.0,
            );
            result.metric(
                &format!("recall_at_10_ef_{ef}"),
                (recall * 1000.0).round() / 1000.0,
            );
            if derived_ef.is_none() && recall >= 0.95 && p99 < 40.0 {
                derived_ef = Some(ef);
                derived_p99 = p99;
            }
        }
        if let Some(ef) = derived_ef {
            result.metric("derived_ef_search", ef);
            result.note(format!(
                "ef_search re-derived per SPEC §15: {ef} (recall ≥0.95 at p99 {derived_p99:.2}ms)"
            ));
        }

        // ADR-01: serialize + mmap view() on this kernel (16K pages on the Pi 5).
        std::fs::create_dir_all(&cfg.scratch_dir).map_err(|e| e.to_string())?;
        let path = cfg.scratch_dir.join("usearch-bench.usearch");
        let path_str = path.to_string_lossy().to_string();
        index.save(&path_str).map_err(err)?;
        let viewed = Index::new(&options()).map_err(err)?;
        viewed.view(&path_str).map_err(err)?;
        let q = unit_vector(0xFFFF_0042);
        let m = viewed.search(&q, TOP_K).map_err(err)?;
        let view_ok = m.keys.len() == TOP_K;
        result.metric("mmap_view_smoke", if view_ok { "pass" } else { "FAIL" });
        drop(viewed);
        let _ = std::fs::remove_file(&path);

        let gate_ok = derived_ef.is_some() && view_ok;
        result.gate(
            "some swept ef reaches recall@10 ≥0.95 at p99 <40ms AND 16K mmap view ok",
            gate_ok,
        );
        Ok(())
    })();

    if let Err(e) = run_inner {
        result.note(e);
        result.gate(
            "some swept ef reaches recall@10 ≥0.95 at p99 <40ms AND 16K mmap view ok",
            false,
        );
    }
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}
