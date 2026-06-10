//! Relevance eval harness (SPEC §15): BM25 vs hybrid over a qrels set, run
//! IN-PROCESS against a self-built index (no server interference, no writer-lock
//! contention with the deployed daemon).
//!
//!   meridian-eval gen   --corpus C.jsonl --out-dir D [--n 100] [--seed 42]
//!   meridian-eval build --corpus C.jsonl --data DIR --models DIR [--docs N]
//!   meridian-eval run   --data DIR --models DIR --queries Q.tsv --qrels R.txt
//!
//! `gen` derives a KNOWN-ITEM set: queries are content words sampled from a
//! document's body EXCLUDING its title words (describe-without-naming), the
//! document itself is the single grade-3 relevant. Synthetic v0 — the
//! operator-labeled set (06-operator-questions Q1) remains open.

use meridian_eval::metrics::{mrr_at, ndcg_at, recall_at};
use meridian_eval::qrels;
use meridian_eval::stats::Rng;
use std::collections::HashSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let get = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1).cloned())
    };
    match cmd {
        "gen" => generate(
            &get("--corpus").unwrap_or_else(|| "bench-scratch/corpus.jsonl".into()),
            &get("--out-dir").unwrap_or_else(|| "eval".into()),
            get("--n").and_then(|v| v.parse().ok()).unwrap_or(100),
            get("--seed").and_then(|v| v.parse().ok()).unwrap_or(42),
        ),
        "build" => build(
            &get("--corpus").unwrap_or_else(|| "bench-scratch/corpus.jsonl".into()),
            &get("--data").unwrap_or_else(|| "eval/data".into()),
            &get("--models").unwrap_or_else(|| "models".into()),
            get("--docs")
                .and_then(|v| v.parse().ok())
                .unwrap_or(usize::MAX),
        ),
        "run" => run(
            &get("--data").unwrap_or_else(|| "eval/data".into()),
            &get("--models").unwrap_or_else(|| "models".into()),
            &get("--queries").unwrap_or_else(|| "eval/queries.tsv".into()),
            &get("--qrels").unwrap_or_else(|| "eval/qrels.txt".into()),
        ),
        _ => {
            eprintln!("usage: meridian-eval gen|build|run [flags] — see source header");
            ExitCode::FAILURE
        }
    }
}

struct CorpusDoc {
    url: String,
    title: String,
    body: String,
}

fn read_corpus(path: &str, max: usize) -> Vec<CorpusDoc> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut docs = Vec::new();
    for line in std::io::BufReader::new(file).lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let title = v["title"].as_str().unwrap_or_default().to_owned();
        let body = v["body"].as_str().unwrap_or_default().to_owned();
        if title.is_empty() || body.len() < 200 {
            continue;
        }
        let url = format!(
            "https://simple.wikipedia.org/wiki/{}",
            title.replace(' ', "_")
        );
        docs.push(CorpusDoc { url, title, body });
        if docs.len() >= max {
            break;
        }
    }
    docs
}

/// Known-item query: 6 informative body words that do NOT appear in the title.
fn known_item_query(doc: &CorpusDoc, rng: &mut Rng) -> Option<String> {
    let title_words: HashSet<String> = doc
        .title
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect();
    let candidates: Vec<&str> = doc
        .body
        .split_whitespace()
        .filter(|w| w.len() >= 5 && w.chars().all(|c| c.is_ascii_alphabetic()))
        .filter(|w| !title_words.contains(&w.to_lowercase()))
        .collect();
    if candidates.len() < 12 {
        return None;
    }
    let mut picked = Vec::new();
    let mut seen = HashSet::new();
    let mut guard = 0;
    while picked.len() < 6 && guard < 200 {
        guard += 1;
        let w = candidates[rng.below(candidates.len())].to_lowercase();
        if seen.insert(w.clone()) {
            picked.push(w);
        }
    }
    (picked.len() >= 4).then(|| picked.join(" "))
}

fn generate(corpus: &str, out_dir: &str, n: usize, seed: u64) -> ExitCode {
    let docs = read_corpus(corpus, usize::MAX);
    if docs.is_empty() {
        eprintln!("no usable corpus docs at {corpus}");
        return ExitCode::FAILURE;
    }
    let mut rng = Rng::new(seed);
    let mut queries = String::new();
    let mut qrels = String::new();
    let mut made = 0;
    let mut guard = 0;
    while made < n && guard < n * 50 {
        guard += 1;
        let doc = &docs[rng.below(docs.len())];
        if let Some(q) = known_item_query(doc, &mut rng) {
            made += 1;
            queries.push_str(&format!("q{made}\t{q}\n"));
            qrels.push_str(&format!("q{made} 0 {} 3\n", doc.url));
        }
    }
    if std::fs::create_dir_all(out_dir).is_err()
        || std::fs::write(format!("{out_dir}/queries.tsv"), queries).is_err()
        || std::fs::write(format!("{out_dir}/qrels.txt"), qrels).is_err()
    {
        eprintln!("cannot write {out_dir}");
        return ExitCode::FAILURE;
    }
    println!(">> generated {made} known-item queries into {out_dir}/ (seed {seed})");
    ExitCode::SUCCESS
}

struct Stack {
    index: Arc<meridian_index::lexical::LexicalIndex>,
    embedder: Arc<meridian_embed::Embedder>,
    vectors: Arc<meridian_vector::VectorStore>,
    vector_cfg: meridian_common::config::VectorConfig,
}

fn open_stack(data: &str, models: &str) -> Result<Stack, String> {
    let index_cfg = meridian_common::config::IndexConfig {
        data_dir: PathBuf::from(data),
        ..Default::default()
    };
    let vector_cfg = meridian_common::config::VectorConfig::default();
    let index = Arc::new(
        meridian_index::lexical::LexicalIndex::open_or_create(&index_cfg)
            .map_err(|e| e.to_string())?,
    );
    let embedder =
        Arc::new(meridian_embed::Embedder::load(Path::new(models)).map_err(|e| e.to_string())?);
    let vectors = Arc::new(
        meridian_vector::VectorStore::open_or_create(Path::new(data), embedder.dims(), &vector_cfg)
            .map_err(|e| e.to_string())?,
    );
    Ok(Stack {
        index,
        embedder,
        vectors,
        vector_cfg,
    })
}

fn build(corpus: &str, data: &str, models: &str, max_docs: usize) -> ExitCode {
    let stack = match open_stack(data, models) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("stack init failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let lanes = Arc::new(
        meridian_query::LaneRegistry::new(&meridian_common::config::LanesConfig::default())
            .expect("lanes"),
    );
    let fetcher = Arc::new(meridian_query::Fetcher::new(
        lanes,
        &meridian_common::config::FetchConfig::default(),
        false,
    ));
    let vectors = stack.vectors;
    let ingestor = match meridian_query::ingest::Ingestor::new(
        stack.index,
        fetcher,
        stack.embedder,
        vectors.clone(),
        Path::new(data),
        &meridian_common::config::IngestConfig::default(),
        &stack.vector_cfg,
    ) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("ingestor init failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    let docs = read_corpus(corpus, max_docs);
    let started = std::time::Instant::now();
    let mut total = 0usize;
    for chunk in docs.chunks(1000) {
        let batch: Vec<meridian_query::ingest::IngestText> = chunk
            .iter()
            .map(|d| meridian_query::ingest::IngestText {
                text: d.body.clone(),
                url: Some(d.url.clone()),
                title: Some(d.title.clone()),
                ts: None,
            })
            .collect();
        match ingestor.ingest_batch(&batch) {
            Ok(stats) => total += stats.accepted,
            Err(e) => {
                eprintln!("ingest failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Err(e) = ingestor.flush_vectors() {
        eprintln!("vector flush failed: {e}");
        return ExitCode::FAILURE;
    }
    println!(
        ">> eval index built: {total} docs, {} vectors, {:.0}s",
        vectors.len(),
        started.elapsed().as_secs_f64()
    );
    ExitCode::SUCCESS
}

fn run(data: &str, models: &str, queries: &str, qrels_path: &str) -> ExitCode {
    let set = match qrels::load(Path::new(queries), Path::new(qrels_path)) {
        Ok(s) if !s.queries.is_empty() => s,
        Ok(_) => {
            eprintln!("empty eval set");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("eval set load failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let Stack {
        index,
        embedder,
        vectors,
        ..
    } = match open_stack(data, models) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("stack init failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        ">> eval over {} queries, {} docs, {} vectors",
        set.queries.len(),
        index.num_docs(),
        vectors.len()
    );

    // Per-system accumulators: (ndcg@10, mrr@10, recall@100)
    let mut bm25_scores = (0.0, 0.0, 0.0);
    let mut hybrid_scores = (0.0, 0.0, 0.0);
    let mut evaluated = 0usize;

    for (qid, query) in &set.queries {
        let Some(judgments) = set.qrels.get(qid) else {
            continue;
        };
        evaluated += 1;

        // BM25-only ranking.
        let bm25_ranked: Vec<String> = index
            .search(query, 100)
            .unwrap_or_default()
            .into_iter()
            .map(|h| h.url)
            .collect();

        // Hybrid: RRF over {bm25 top-1000, ann top-200} — the planner's exact
        // local fusion (same rrf_fuse, same url_key identity).
        let bm25_full = index.search(query, 1000).unwrap_or_default();
        let qv = embedder.embed_query(query);
        let ann = vectors.search(&qv, 200).unwrap_or_default();
        let lists: Vec<Vec<u64>> = vec![
            bm25_full.iter().map(|h| h.url_key).collect(),
            ann.iter().map(|(k, _)| *k).collect(),
        ];
        let fused = meridian_query::rrf::rrf_fuse(&lists, meridian_query::rrf::RRF_K);

        let mut url_by_key: std::collections::HashMap<u64, String> =
            bm25_full.into_iter().map(|h| (h.url_key, h.url)).collect();
        let missing: Vec<u64> = fused
            .iter()
            .map(|(k, _)| *k)
            .filter(|k| !url_by_key.contains_key(k))
            .collect();
        for d in index.docs_by_keys(&missing).unwrap_or_default() {
            url_by_key.insert(d.url_key, d.url);
        }
        let hybrid_ranked: Vec<String> = fused
            .iter()
            .filter_map(|(k, _)| url_by_key.get(k).cloned())
            .take(100)
            .collect();

        bm25_scores.0 += ndcg_at(10, &bm25_ranked, judgments);
        bm25_scores.1 += mrr_at(10, &bm25_ranked, judgments);
        bm25_scores.2 += recall_at(100, &bm25_ranked, judgments);
        hybrid_scores.0 += ndcg_at(10, &hybrid_ranked, judgments);
        hybrid_scores.1 += mrr_at(10, &hybrid_ranked, judgments);
        hybrid_scores.2 += recall_at(100, &hybrid_ranked, judgments);
    }

    let n = evaluated.max(1) as f64;
    println!("\n| system | nDCG@10 | MRR@10 | Recall@100 |");
    println!("|---|---|---|---|");
    println!(
        "| BM25   | {:.4} | {:.4} | {:.4} |",
        bm25_scores.0 / n,
        bm25_scores.1 / n,
        bm25_scores.2 / n
    );
    println!(
        "| hybrid | {:.4} | {:.4} | {:.4} |",
        hybrid_scores.0 / n,
        hybrid_scores.1 / n,
        hybrid_scores.2 / n
    );
    let pass = hybrid_scores.0 >= bm25_scores.0;
    println!(
        "\nGATE hybrid nDCG@10 >= BM25: {}",
        if pass { "PASS" } else { "FAIL" }
    );
    if pass {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
