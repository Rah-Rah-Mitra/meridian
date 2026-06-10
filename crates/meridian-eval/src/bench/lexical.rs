//! Suites 3 + 7 — Tantivy lexical indexing/query latency, and disk merge
//! amplification (SPEC §15.3, §15.7). One pass produces both results: we index the
//! corpus with checkpoints (10k/50k/…/max_docs) measuring BM25 top-1000 latency at
//! each size, while a 1Hz sampler watches free disk to capture the merge transient.
//!
//! Gates: BM25 top-1000 p50 <30ms at the largest indexed size; merge transient
//! ≤1.0GB (Profile F) / ≤0.75GB (Profile R).

use super::{BenchConfig, SuiteResult};
use crate::probe::free_disk_bytes;
use crate::stats::{Rng, percentile_ms};
use std::io::BufRead;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{FAST, STORED, Schema, TEXT};
use tantivy::{Index, IndexWriter, TantivyDocument, doc};

const QUERIES: usize = 1_000;
const WRITER_THREADS: usize = 2;
const WRITER_HEAP: usize = 256 * 1024 * 1024;

struct CorpusDoc {
    title: String,
    body: String,
}

fn load_corpus(path: &Path, max_docs: usize) -> std::io::Result<Vec<CorpusDoc>> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut docs = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let title = v["title"].as_str().unwrap_or_default().to_owned();
        let body = v["body"].as_str().unwrap_or_default().to_owned();
        if title.is_empty() && body.is_empty() {
            continue;
        }
        docs.push(CorpusDoc { title, body });
        if docs.len() >= max_docs {
            break;
        }
    }
    Ok(docs)
}

/// Build 2-word queries by sampling words from random titles (≥4 alphabetic chars).
fn sample_queries(docs: &[CorpusDoc], n: usize, rng: &mut Rng) -> Vec<String> {
    let mut queries = Vec::with_capacity(n);
    let mut guard = 0;
    while queries.len() < n && guard < n * 50 {
        guard += 1;
        let d = &docs[rng.below(docs.len())];
        let words: Vec<&str> = d
            .title
            .split_whitespace()
            .chain(d.body.split_whitespace().take(40))
            .filter(|w| w.len() >= 4 && w.chars().all(|c| c.is_ascii_alphabetic()))
            .collect();
        if words.len() < 2 {
            continue;
        }
        let a = words[rng.below(words.len())];
        let b = words[rng.below(words.len())];
        queries.push(format!("{a} {b}"));
    }
    queries
}

fn dir_bytes(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for e in entries.flatten() {
            if let Ok(md) = e.metadata() {
                if md.is_file() {
                    total += md.len();
                } else if md.is_dir() {
                    total += dir_bytes(&e.path());
                }
            }
        }
    }
    total
}

pub fn run(cfg: &BenchConfig) -> Vec<SuiteResult> {
    let mut lexical = SuiteResult::new("lexical");
    let mut disk = SuiteResult::new("disk");
    let start = Instant::now();

    let Some(corpus_path) = cfg.corpus.as_deref() else {
        return vec![
            SuiteResult::skipped("lexical", "no --corpus given (run fetch-bench-corpus.sh)"),
            SuiteResult::skipped("disk", "no --corpus given"),
        ];
    };
    let docs = match load_corpus(corpus_path, cfg.max_docs) {
        Ok(d) if !d.is_empty() => d,
        Ok(_) => {
            return vec![
                SuiteResult::skipped("lexical", "corpus file is empty"),
                SuiteResult::skipped("disk", "corpus file is empty"),
            ];
        }
        Err(e) => {
            return vec![
                SuiteResult::skipped("lexical", &format!("corpus read failed: {e}")),
                SuiteResult::skipped("disk", "corpus read failed"),
            ];
        }
    };
    lexical.metric("corpus_docs", docs.len());

    let index_dir = cfg.scratch_dir.join("tantivy-bench");
    let _ = std::fs::remove_dir_all(&index_dir);
    if let Err(e) = std::fs::create_dir_all(&index_dir) {
        lexical.note(format!("scratch dir failed: {e}"));
        return vec![lexical, disk];
    }

    // 1Hz free-disk sampler for the merge-amplification measurement.
    let free0 = free_disk_bytes(&index_dir);
    let min_free = std::sync::Arc::new(AtomicU64::new(free0));
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let sampler = {
        let min_free = min_free.clone();
        let stop = stop.clone();
        let dir = index_dir.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let f = free_disk_bytes(&dir);
                min_free.fetch_min(f, Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        })
    };

    let run_inner = (|| -> tantivy::Result<()> {
        let mut schema_builder = Schema::builder();
        let f_title = schema_builder.add_text_field("title", TEXT | STORED);
        let f_body = schema_builder.add_text_field("body", TEXT);
        let f_ts = schema_builder.add_u64_field("ts", FAST);
        let schema = schema_builder.build();
        let index = Index::create_in_dir(&index_dir, schema)?;
        let mut writer: IndexWriter<TantivyDocument> =
            index.writer_with_num_threads(WRITER_THREADS, WRITER_HEAP)?;

        let mut rng = Rng::new(0x1E81);
        let queries = sample_queries(&docs, QUERIES, &mut rng);
        let parser = QueryParser::for_index(&index, vec![f_title, f_body]);

        // Checkpoints for the latency-vs-size curve.
        let mut checkpoints: Vec<usize> =
            [10_000usize, 50_000, 100_000, 250_000, 500_000, 1_000_000]
                .into_iter()
                .filter(|&c| c < docs.len())
                .collect();
        checkpoints.push(docs.len());

        let ingest_start = Instant::now();
        let mut indexed = 0usize;
        for &checkpoint in &checkpoints {
            while indexed < checkpoint {
                let d = &docs[indexed];
                writer.add_document(doc!(
                    f_title => d.title.as_str(),
                    f_body => d.body.as_str(),
                    f_ts => indexed as u64,
                ))?;
                indexed += 1;
            }
            writer.commit()?;
            let reader = index.reader()?;
            let searcher = reader.searcher();
            let mut samples = Vec::with_capacity(queries.len());
            for q in &queries {
                let Ok(query) = parser.parse_query(q) else {
                    continue;
                };
                let t = Instant::now();
                let hits = searcher.search(&query, &TopDocs::with_limit(1000).order_by_score())?;
                std::hint::black_box(hits.len());
                samples.push(t.elapsed().as_secs_f64() * 1e3);
            }
            let p50 = percentile_ms(&mut samples, 50.0);
            let p99 = percentile_ms(&mut samples, 99.0);
            lexical.metric(&format!("bm25_p50_ms_at_{checkpoint}"), p50);
            lexical.metric(&format!("bm25_p99_ms_at_{checkpoint}"), p99);
            if checkpoint == docs.len() {
                lexical.metric("docs_per_sec_ingest", {
                    let secs = ingest_start.elapsed().as_secs_f64();
                    (indexed as f64 / secs).round()
                });
                lexical.gate("BM25 top-1000 p50 < 30ms at max indexed size", p50 < 30.0);
            }
        }

        // Force-merge to one segment — the transient is what the sampler catches.
        let pre_merge_bytes = dir_bytes(&index_dir);
        let segment_ids = index.searchable_segment_ids()?;
        if segment_ids.len() > 1 {
            writer.merge(&segment_ids).wait()?;
        }
        writer.wait_merging_threads()?;
        let final_bytes = dir_bytes(&index_dir);
        disk.metric("index_bytes_pre_merge", pre_merge_bytes);
        disk.metric("index_bytes_final", final_bytes);
        lexical.metric("index_bytes_final", final_bytes);
        lexical.metric(
            "bytes_per_doc",
            (final_bytes as f64 / indexed as f64).round(),
        );
        Ok(())
    })();

    stop.store(true, Ordering::Relaxed);
    let _ = sampler.join();

    if let Err(e) = run_inner {
        lexical.note(format!("tantivy error: {e}"));
        lexical.gate("BM25 top-1000 p50 < 30ms at max indexed size", false);
    }

    let free_end = free_disk_bytes(&index_dir);
    let peak_used = free0.saturating_sub(min_free.load(Ordering::Relaxed));
    let settled_used = free0.saturating_sub(free_end);
    let transient = peak_used.saturating_sub(settled_used);
    disk.metric("peak_used_bytes", peak_used);
    disk.metric("settled_used_bytes", settled_used);
    disk.metric("merge_transient_bytes", transient);
    disk.gate(
        "merge transient ≤ 0.75GB (Profile R scratch floor)",
        transient <= 768 * 1024 * 1024,
    );
    disk.note("sampler is 1Hz — short merges can under-report the true peak");

    // Leave the index on disk only if the caller wants to inspect it; default clean.
    let _ = std::fs::remove_dir_all(&index_dir);

    let elapsed = start.elapsed().as_secs_f64() * 1e3;
    lexical.duration_ms = elapsed;
    disk.duration_ms = elapsed;
    vec![lexical, disk]
}
