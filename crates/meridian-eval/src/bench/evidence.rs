//! Suite 11 — `evidence`: latency of the Phase-7 evidence stage
//! (04-bench-plan §6, ADR-18; budget row "fast+evidence ≤ Phase-6 fast +2ms").
//!
//! Measures exactly what the planner runs per query: sketch lookups from
//! `dedup.redb` (`SketchReader::get_many`) + containment clustering
//! (`evidence::annotate`) over a result set. Gate: ≤2ms p50 at the API's
//! maximum result limit (50). A 100-result run is reported as information —
//! the stage is O(s²) in sketched results.
//!
//! The corpus here is 1k docs in a temp store; the on-device exit re-run
//! repeats this against the real 100k volume (redb point lookups grow ~log).

use super::{BenchConfig, SuiteResult};
use crate::stats::{Rng, percentile_ms};
use meridian_query::evidence::annotate;
use meridian_query::ingest::{IngestText, Ingestor};
use meridian_query::planner::{RankSignals, SearchResult};
use std::sync::Arc;
use std::time::Instant;

const CORPUS_DOCS: usize = 1_000;
const ITERATIONS: usize = 200;

pub fn run(cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("evidence");
    let start = Instant::now();

    let dir = cfg
        .scratch_dir
        .join(format!("evidence-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ingestor = match build_ingestor(&dir) {
        Ok(i) => i,
        Err(e) => {
            result.note(format!("setup failed: {e}"));
            result.gate("≤2ms p50 at limit=50", false);
            result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
            return result;
        }
    };

    // Corpus with syndication structure: every 10th doc is an origin whose
    // next 6 docs are near-copies — result sets then exercise real clustering
    // work, not the all-singletons fast path.
    let mut rng = Rng::new(0xE11D_2026);
    let mut urls = Vec::with_capacity(CORPUS_DOCS);
    let mut batch = Vec::with_capacity(100);
    let mut origin_text = String::new();
    for i in 0..CORPUS_DOCS {
        let text = if i % 10 == 0 {
            origin_text = synthetic_article(&mut rng, i);
            origin_text.clone()
        } else if i % 10 <= 6 {
            // Near-copy: trim the tail, add outlet boilerplate.
            let words: Vec<&str> = origin_text.split_whitespace().collect();
            let keep = words.len() * 8 / 10;
            format!("{} outlet{i} syndication footer", words[..keep].join(" "))
        } else {
            synthetic_article(&mut rng, i * 7919)
        };
        let url = format!("https://bench{:02}.example/doc/{i}", i % 97);
        urls.push(url.clone());
        batch.push(IngestText {
            text,
            url: Some(url),
            title: Some(format!("doc {i}")),
            ts: Some(1_700_000_000),
        });
        if batch.len() == 100 {
            if let Err(e) = ingestor.ingest_batch(&batch) {
                result.note(format!("ingest failed: {e}"));
            }
            batch.clear();
        }
    }
    let reader = ingestor.sketch_reader();

    let measure = |result_count: usize, rng: &mut Rng| -> (f64, f64, u32) {
        let mut samples = Vec::with_capacity(ITERATIONS);
        let mut last_independent = 0u32;
        for _ in 0..ITERATIONS {
            // A fresh page of results each iteration (cache-realistic mix).
            let first = rng.below(CORPUS_DOCS - result_count);
            let mut results: Vec<SearchResult> = urls[first..first + result_count]
                .iter()
                .map(|u| bench_result(u))
                .collect();
            let t = Instant::now();
            let keys: Vec<u64> = results
                .iter()
                .map(|r| meridian_index::lexical::url_key(&r.url))
                .collect();
            let sketches = reader.get_many(&keys);
            let block = annotate(&mut results, &sketches);
            samples.push(t.elapsed().as_secs_f64() * 1e3);
            last_independent = block.independent_source_count;
        }
        (
            percentile_ms(&mut samples, 50.0),
            percentile_ms(&mut samples, 99.0),
            last_independent,
        )
    };

    let (p50_50, p99_50, indep_50) = measure(50, &mut rng);
    let (p50_100, p99_100, _) = measure(100, &mut rng);

    result.metric("corpus_docs", CORPUS_DOCS);
    result.metric("iterations", ITERATIONS);
    result.metric("p50_ms_limit50", round3(p50_50));
    result.metric("p99_ms_limit50", round3(p99_50));
    result.metric("p50_ms_limit100", round3(p50_100));
    result.metric("p99_ms_limit100", round3(p99_100));
    result.metric("sample_independent_sources_at_50", indep_50);
    result.note(
        "stage = SketchReader::get_many + evidence::annotate, the exact planner path; \
         re-run on-device against the 100k volume at the P7 exit"
            .to_owned(),
    );
    result.gate(
        "evidence stage ≤2ms p50 at the API max limit (50 results)",
        p50_50 <= 2.0,
    );

    let _ = std::fs::remove_dir_all(&dir);
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn synthetic_article(rng: &mut Rng, salt: usize) -> String {
    let len = 120 + rng.below(80);
    (0..len)
        .map(|i| {
            format!(
                "w{:x}",
                (salt.wrapping_mul(31).wrapping_add(i * 7)) % 4096 + rng.below(64)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn bench_result(url: &str) -> SearchResult {
    SearchResult {
        url: url.to_owned(),
        title: String::new(),
        snippet: String::new(),
        score: 0.0,
        rank_signals: RankSignals {
            rrf: 0.0,
            bm25: None,
            ann: None,
            searx_rank: None,
            ltr: 0.0,
            ce: None,
        },
        source: "local",
        h3: None,
        ts: None,
        evidence: None,
    }
}

fn build_ingestor(dir: &std::path::Path) -> Result<Ingestor, String> {
    use meridian_common::config::{
        FetchConfig, IndexConfig, IngestConfig, LanesConfig, VectorConfig,
    };
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let index_cfg = IndexConfig {
        data_dir: dir.to_path_buf(),
        writer_threads: 1,
        writer_heap_bytes: 32 * 1024 * 1024,
        merge_max_docs: 100_000,
    };
    let index = Arc::new(
        meridian_index::lexical::LexicalIndex::open_or_create(&index_cfg)
            .map_err(|e| e.to_string())?,
    );
    let lanes = Arc::new(
        meridian_query::LaneRegistry::new(&LanesConfig::default(), dir)
            .map_err(|e| e.to_string())?,
    );
    let fetcher = Arc::new(meridian_query::Fetcher::new(
        lanes,
        &FetchConfig::default(),
        false,
    ));
    let vector_cfg = VectorConfig::default();
    let embedder = Arc::new(meridian_embed::Embedder::test_stub(64));
    let vectors = Arc::new(
        meridian_vector::VectorStore::open_or_create(dir, 64, &vector_cfg)
            .map_err(|e| e.to_string())?,
    );
    Ingestor::new(
        index,
        fetcher,
        embedder,
        vectors,
        dir,
        None,
        &IngestConfig::default(),
        &vector_cfg,
    )
    .map_err(|e| e.to_string())
}
