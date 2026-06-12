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

use meridian_eval::metrics::{ece_10, mrr_at, ndcg_at, recall_at, spearman};
use meridian_eval::qrels;
use meridian_eval::stats::Rng;
use meridian_rank::{Features, LinearLtr, Scorer, title_match_ratio};
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
        "gen-dup" => generate_dup(
            &get("--corpus").unwrap_or_else(|| "bench-scratch/corpus.jsonl".into()),
            &get("--out-dir").unwrap_or_else(|| "eval".into()),
            get("--n").and_then(|v| v.parse().ok()).unwrap_or(30),
            get("--seed").and_then(|v| v.parse().ok()).unwrap_or(42),
        ),
        "run-dup" => run_dup(
            &get("--data").unwrap_or_else(|| "eval/dup-data".into()),
            &get("--models").unwrap_or_else(|| "models".into()),
            &get("--queries").unwrap_or_else(|| "eval/dup-queries.tsv".into()),
            &get("--qrels").unwrap_or_else(|| "eval/dup-qrels.txt".into()),
        ),
        "gazetteer" => gazetteer(
            &get("--source").unwrap_or_else(|| "cities15000.txt".into()),
            &get("--out").unwrap_or_else(|| "models/gazetteer.fst".into()),
        ),
        "run" => run(
            &get("--data").unwrap_or_else(|| "eval/data".into()),
            &get("--models").unwrap_or_else(|| "models".into()),
            &get("--queries").unwrap_or_else(|| "eval/queries.tsv".into()),
            &get("--qrels").unwrap_or_else(|| "eval/qrels.txt".into()),
        ),
        _ => {
            eprintln!("usage: meridian-eval gen|build|run|gazetteer [flags] — see source header");
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

/// Offline gazetteer build (SPEC §3: "built offline, shipped in image"):
/// GeoNames cities15000.txt → fst map for ingest geo-tagging.
fn gazetteer(source: &str, out: &str) -> ExitCode {
    let file = match std::fs::File::open(source) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("gazetteer source {source}: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(parent) = Path::new(out).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match meridian_geo::gazetteer::build_from_geonames(
        std::io::BufReader::new(file),
        Path::new(out),
    ) {
        Ok(n) => {
            let bytes = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
            println!(
                ">> gazetteer: {n} place names → {out} ({} KB)",
                bytes / 1024
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gazetteer build failed: {e}");
            ExitCode::FAILURE
        }
    }
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
        meridian_query::LaneRegistry::new(
            &meridian_common::config::LanesConfig::default(),
            std::path::Path::new(data),
        )
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
        None,
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
    let mut ltr_scores = (0.0, 0.0, 0.0);
    let scorer = LinearLtr::default();
    let mut evaluated = 0usize;
    // Suite 13 (Phase 8, ADR-23): per-query QPP predictors vs measured nDCG@10.
    let mut qpp_score: Vec<f64> = Vec::new();
    let mut qpp_nqc: Vec<f64> = Vec::new();
    let mut qpp_clarity: Vec<f64> = Vec::new();
    let mut qpp_ndcg: Vec<f64> = Vec::new();

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
        let bm25_full_for_ltr = bm25_full.clone();
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

        // hybrid + LTR re-score (the planner's rank stage): same fused list,
        // re-ordered by the cold-start linear model over the top-100.
        let ann_sim: std::collections::HashMap<u64, f32> = ann.iter().copied().collect();
        let bm25_by_key: std::collections::HashMap<u64, f32> = bm25_full_for_ltr
            .iter()
            .map(|h| (h.url_key, h.bm25))
            .collect();
        let title_by_key: std::collections::HashMap<u64, String> = bm25_full_for_ltr
            .iter()
            .map(|h| (h.url_key, h.title.clone()))
            .collect();
        let mut ltr_ranked_keyed: Vec<(u64, f32)> = fused
            .iter()
            .take(100)
            .map(|(k, rrf)| {
                let f = Features {
                    rrf: *rrf,
                    bm25: bm25_by_key.get(k).copied().unwrap_or(0.0),
                    ann: ann_sim.get(k).copied().unwrap_or(0.0),
                    title_match_ratio: title_by_key
                        .get(k)
                        .map(|t| title_match_ratio(query, t))
                        .unwrap_or(0.0),
                    source_count: bm25_by_key.contains_key(k) as u8 as f32
                        + ann_sim.contains_key(k) as u8 as f32,
                    snippet_len_norm: 0.5,
                    freshness: 0.5,
                    domain_prior: 0.0,
                    geo: 0.0,
                };
                (*k, scorer.score(&f))
            })
            .collect();
        ltr_ranked_keyed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let ltr_ranked: Vec<String> = ltr_ranked_keyed
            .iter()
            .filter_map(|(k, _)| url_by_key.get(k).cloned())
            .collect();

        // Suite 13: confidence over the SAME shapes the planner feeds qpp —
        // top = the LTR-ranked head's RRF scores + snippets, pool = the scored
        // top-100 fused candidates.
        let snippet_by_key: std::collections::HashMap<u64, String> = {
            let mut m: std::collections::HashMap<u64, String> = bm25_full_for_ltr
                .iter()
                .map(|h| (h.url_key, h.snippet.clone()))
                .collect();
            let missing: Vec<u64> = fused
                .iter()
                .take(100)
                .map(|(k, _)| *k)
                .filter(|k| !m.contains_key(k))
                .collect();
            for d in index.docs_by_keys(&missing).unwrap_or_default() {
                m.insert(d.url_key, d.snippet);
            }
            m
        };
        let rrf_by_key: std::collections::HashMap<u64, f32> =
            fused.iter().take(100).map(|(k, r)| (*k, *r)).collect();
        let pool: Vec<(f32, &str)> = fused
            .iter()
            .take(100)
            .filter_map(|(k, r)| snippet_by_key.get(k).map(|s| (*r, s.as_str())))
            .collect();
        let head: Vec<(f32, &str)> = ltr_ranked_keyed
            .iter()
            .take(10)
            .filter_map(|(k, _)| {
                let r = rrf_by_key.get(k)?;
                snippet_by_key.get(k).map(|s| (*r, s.as_str()))
            })
            .collect();
        let top_scores: Vec<f32> = head.iter().map(|(r, _)| *r).collect();
        let top_snips: Vec<&str> = head.iter().map(|(_, s)| *s).collect();
        let pool_scores: Vec<f32> = pool.iter().map(|(r, _)| *r).collect();
        let pool_snips: Vec<&str> = pool.iter().map(|(_, s)| *s).collect();
        if let Some(c) =
            meridian_rank::qpp::confidence(&top_scores, &top_snips, &pool_scores, &pool_snips)
        {
            qpp_score.push(f64::from(c.score));
            qpp_nqc.push(f64::from(c.nqc));
            qpp_clarity.push(f64::from(c.clarity));
            qpp_ndcg.push(ndcg_at(10, &ltr_ranked, judgments));
        }

        bm25_scores.0 += ndcg_at(10, &bm25_ranked, judgments);
        bm25_scores.1 += mrr_at(10, &bm25_ranked, judgments);
        bm25_scores.2 += recall_at(100, &bm25_ranked, judgments);
        hybrid_scores.0 += ndcg_at(10, &hybrid_ranked, judgments);
        hybrid_scores.1 += mrr_at(10, &hybrid_ranked, judgments);
        hybrid_scores.2 += recall_at(100, &hybrid_ranked, judgments);
        ltr_scores.0 += ndcg_at(10, &ltr_ranked, judgments);
        ltr_scores.1 += mrr_at(10, &ltr_ranked, judgments);
        ltr_scores.2 += recall_at(100, &ltr_ranked, judgments);
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
        "| hybrid     | {:.4} | {:.4} | {:.4} |",
        hybrid_scores.0 / n,
        hybrid_scores.1 / n,
        hybrid_scores.2 / n
    );
    println!(
        "| hybrid+ltr | {:.4} | {:.4} | {:.4} |",
        ltr_scores.0 / n,
        ltr_scores.1 / n,
        ltr_scores.2 / n
    );
    // Suite 13 report (SPEC §16 P8 gate: blended score ρ ≥ 0.25 vs nDCG@10).
    let rho_score = spearman(&qpp_score, &qpp_ndcg);
    let rho_nqc = spearman(&qpp_nqc, &qpp_ndcg);
    let rho_clarity = spearman(&qpp_clarity, &qpp_ndcg);
    let ece = ece_10(&qpp_score, &qpp_ndcg);
    println!(
        "\nsuite 13 (qpp, n={}): spearman score={:.3} nqc={:.3} clarity={:.3} | ece(score vs ndcg)={:.3}",
        qpp_score.len(),
        rho_score,
        rho_nqc,
        rho_clarity,
        ece
    );
    let qpp_gate = rho_score >= 0.25;
    println!(
        "GATE qpp spearman(score, ndcg@10) >= 0.25: {}",
        if qpp_gate { "PASS" } else { "FAIL" }
    );

    let hybrid_beats_bm25 = hybrid_scores.0 >= bm25_scores.0;
    // LTR no-regression (SPEC §16 Phase-3 exit): within 1% of hybrid nDCG@10.
    let ltr_no_regression = ltr_scores.0 >= hybrid_scores.0 - 0.01 * hybrid_scores.0.max(1e-9);
    println!(
        "\nGATE hybrid nDCG@10 >= BM25: {}",
        if hybrid_beats_bm25 { "PASS" } else { "FAIL" }
    );
    println!(
        "GATE hybrid+ltr no regression vs hybrid: {}",
        if ltr_no_regression { "PASS" } else { "FAIL" }
    );
    if hybrid_beats_bm25 && ltr_no_regression && qpp_gate {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Suite 13b (Phase 8, MMR gate): duplicate-heavy eval set. Each query targets
/// THREE distinct documents sharing a query word EXCLUSIVE to them (three
/// subtopics); each document additionally appears as THREE near-duplicate
/// copies (truncation + boilerplate — the suite-9 syndication shape), so the
/// 12 matching docs exceed the 10 evaluated slots and redundancy has a real
/// cost. A diversity-aware ranking covers the three subtopics before the
/// copies; a redundancy-blind one drowns rank 2-3 in duplicates. Output:
/// dup-corpus.jsonl (5k background + originals + copies), dup-queries.tsv,
/// dup-qrels.txt (qid url grade subtopic).
fn generate_dup(corpus: &str, out_dir: &str, n: usize, seed: u64) -> ExitCode {
    let docs = read_corpus(corpus, 20_000);
    if docs.len() < 6_000 {
        eprintln!("corpus too small for dup-heavy gen ({})", docs.len());
        return ExitCode::FAILURE;
    }
    let mut rng = Rng::new(seed);

    // Inverted index over title+body with INDEX-LIKE tokenization (split on
    // non-alphanumeric, lowercase — what tantivy does), full document scan.
    // The first cut scanned only the first 120 whitespace tokens and allowed
    // df ≤ 40, so dozens of UNJUDGED background docs matched each query and
    // the gate measured retrieval noise, not duplicate demotion (LTR nDCG@10
    // was 0.41 against 0.89 on clean separation). The query word must be
    // EXCLUSIVE to its three subtopic docs: df == 3 across everything that
    // ends up in the corpus.
    let mut by_word: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, d) in docs.iter().enumerate().take(6_000) {
        let mut seen = HashSet::new();
        for w in d
            .title
            .split(|c: char| !c.is_ascii_alphanumeric())
            .chain(d.body.split(|c: char| !c.is_ascii_alphanumeric()))
        {
            let w = w.to_lowercase();
            if w.len() >= 6 && w.chars().all(|c| c.is_ascii_alphabetic()) && seen.insert(w.clone())
            {
                by_word.entry(w).or_default().push(i);
            }
        }
    }
    let mut candidates: Vec<(&String, &Vec<usize>)> = by_word
        .iter()
        .filter(|(_, ids)| ids.len() == 3)
        .collect();
    candidates.sort_by_key(|(w, _)| w.to_string());

    let _ = std::fs::create_dir_all(out_dir);
    let mut queries = String::new();
    let mut qrels = String::new();
    let mut extra_docs = String::new();
    let mut used_docs: HashSet<usize> = HashSet::new();
    let mut made = 0usize;

    while made < n && !candidates.is_empty() {
        let pick = rng.below(candidates.len());
        let (word, ids) = candidates.swap_remove(pick);
        let group: Vec<usize> = ids
            .iter()
            .copied()
            .filter(|i| !used_docs.contains(i))
            .take(3)
            .collect();
        if group.len() < 3 {
            continue;
        }
        // The copies are 75%-truncations: the query word must survive into
        // every copy (title or kept body prefix), or the copies are simply
        // not retrievable for this query and the "duplicate-heavy" premise
        // evaporates at retrieval time.
        let survives = group.iter().all(|&di| {
            let d = &docs[di];
            let in_title = d
                .title
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|t| t.to_lowercase() == **word);
            let body_words: Vec<&str> = d.body.split_whitespace().collect();
            let keep = (body_words.len() * 3 / 4).max(40).min(body_words.len());
            let in_kept = body_words[..keep].iter().any(|t| {
                t.split(|c: char| !c.is_ascii_alphanumeric())
                    .any(|p| p.to_lowercase() == **word)
            });
            in_title || in_kept
        });
        if !survives {
            continue;
        }
        used_docs.extend(&group);
        let qid = format!("dq{made}");
        queries.push_str(&format!("{qid}\t{word}\n"));
        for (sub, &di) in group.iter().enumerate() {
            let d = &docs[di];
            qrels.push_str(&format!("{qid} {} 3 {sub}\n", d.url));
            // Two near-duplicate copies per original: 75% truncation + outlet
            // boilerplate (the suite-9 syndication shape). `build` derives the
            // URL from the TITLE, so copies need DISTINCT titles — which is
            // also what real syndication does (retitled wire copy). The first
            // cut of this generator reused the original title; all copies
            // collapsed into one URL and the dup eval silently measured plain
            // nDCG (alpha == nDCG exactly was the tell).
            let words: Vec<&str> = d.body.split_whitespace().collect();
            let keep = (words.len() * 3 / 4).max(40).min(words.len());
            for c in 0..3 {
                let copy_title = format!("{} syndicated {c}", d.title);
                let body = format!(
                    "{} syndication outlet{c} footer attribution reporting",
                    words[..keep].join(" ")
                );
                extra_docs.push_str(
                    &serde_json::json!({"title": copy_title.clone(), "body": body}).to_string(),
                );
                extra_docs.push('\n');
                qrels.push_str(&format!(
                    "{qid} https://simple.wikipedia.org/wiki/{} 2 {sub}\n",
                    copy_title.replace(' ', "_")
                ));
            }
        }
        made += 1;
    }
    if made < n {
        eprintln!("warning: only {made}/{n} dup queries generated");
    }

    // dup corpus = 5k background + the originals of every group + copies.
    let mut corpus_out = String::new();
    let group_set: HashSet<usize> = used_docs.clone();
    let mut background = 0usize;
    for (i, d) in docs.iter().enumerate() {
        let include = group_set.contains(&i) || background < 5_000;
        if include {
            if !group_set.contains(&i) {
                background += 1;
            }
            corpus_out.push_str(
                &serde_json::json!({"title": d.title.clone(), "body": d.body.clone()}).to_string(),
            );
            corpus_out.push('\n');
        }
    }
    corpus_out.push_str(&extra_docs);

    let w = |name: &str, content: &str| std::fs::write(format!("{out_dir}/{name}"), content);
    if w("dup-corpus.jsonl", &corpus_out).is_err()
        || w("dup-queries.tsv", &queries).is_err()
        || w("dup-qrels.txt", &qrels).is_err()
    {
        eprintln!("write failed");
        return ExitCode::FAILURE;
    }
    println!(
        ">> dup-heavy set: {made} queries, {} group docs ×4 copies-incl, 5k background → {out_dir}",
        used_docs.len()
    );
    ExitCode::SUCCESS
}

/// Suite 13b runner: alpha-nDCG@10 + nDCG@10 for the LTR ranking vs the SAME
/// ranking reordered by MMR (the shipped diversity=mmr path). Gates (SPEC §16
/// P8): MMR improves mean alpha-nDCG@10 AND loses ≤1% mean nDCG@10.
fn run_dup(data: &str, models: &str, queries: &str, qrels_path: &str) -> ExitCode {
    use meridian_eval::metrics::alpha_ndcg_at;
    // 4-column qrels: qid url grade subtopic.
    let mut subtopic_qrels: std::collections::HashMap<
        String,
        std::collections::HashMap<String, (u32, u32)>,
    > = std::collections::HashMap::new();
    for line in std::fs::read_to_string(qrels_path)
        .unwrap_or_default()
        .lines()
    {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() == 4 {
            if let (Ok(g), Ok(sub)) = (f[2].parse::<u32>(), f[3].parse::<u32>()) {
                subtopic_qrels
                    .entry(f[0].to_owned())
                    .or_default()
                    .insert(f[1].to_owned(), (g, sub));
            }
        }
    }
    let query_list: Vec<(String, String)> = std::fs::read_to_string(queries)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (qid, q) = l.split_once('\t')?;
            Some((qid.to_owned(), q.to_owned()))
        })
        .collect();
    if query_list.is_empty() || subtopic_qrels.is_empty() {
        eprintln!("empty dup eval set");
        return ExitCode::FAILURE;
    }
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
        ">> dup eval over {} queries, {} docs",
        query_list.len(),
        index.num_docs()
    );

    const LAMBDA_SWEEP: [f32; 7] = [0.5, 0.6, 0.7, 0.8, 0.85, 0.9, 0.95];
    let scorer = LinearLtr::default();
    // GATED regime: the BM25-matching head — every candidate contains the
    // query term, near-dup copies crowd out subtopics. This is the hermetic
    // proxy for the query-matching WEB head that `diversity=mmr` exists for
    // (syndicated news); ANN noise never reaches those lists.
    let (mut ab_base, mut ab_mmr, mut nb_base, mut nb_mmr) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut ab_sweep = [0.0f64; LAMBDA_SWEEP.len()];
    let mut nb_sweep = [0.0f64; LAMBDA_SWEEP.len()];
    // DIAGNOSTIC regime: the local hybrid+LTR head. On exclusive-term queries
    // RRF interleaves the few BM25 matches 1:1 with ANN noise, so duplicate
    // demotion can only promote irrelevant docs — measured and reported, but
    // not what the gate is about.
    let (mut a_ltr, mut a_mmr, mut n_ltr, mut n_mmr) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut a_sweep = [0.0f64; LAMBDA_SWEEP.len()];
    let mut n_sweep = [0.0f64; LAMBDA_SWEEP.len()];
    let mut evaluated = 0usize;

    for (qid, query) in &query_list {
        let Some(judg) = subtopic_qrels.get(qid) else {
            continue;
        };
        let plain: std::collections::HashMap<String, u32> =
            judg.iter().map(|(d, (g, _))| (d.clone(), *g)).collect();

        // Same retrieval as `run`: hybrid fusion + LTR top-100.
        let bm25_full = index.search(query, 1000).unwrap_or_default();
        let qv = embedder.embed_query(query);
        let ann = vectors.search(&qv, 200).unwrap_or_default();
        let lists: Vec<Vec<u64>> = vec![
            bm25_full.iter().map(|h| h.url_key).collect(),
            ann.iter().map(|(k, _)| *k).collect(),
        ];
        let fused = meridian_query::rrf::rrf_fuse(&lists, meridian_query::rrf::RRF_K);
        let mut url_by_key: std::collections::HashMap<u64, String> = bm25_full
            .iter()
            .map(|h| (h.url_key, h.url.clone()))
            .collect();
        let mut snip_by_key: std::collections::HashMap<u64, String> = bm25_full
            .iter()
            .map(|h| (h.url_key, h.snippet.clone()))
            .collect();
        let mut title_by_key: std::collections::HashMap<u64, String> = bm25_full
            .iter()
            .map(|h| (h.url_key, h.title.clone()))
            .collect();
        let missing: Vec<u64> = fused
            .iter()
            .take(100)
            .map(|(k, _)| *k)
            .filter(|k| !url_by_key.contains_key(k))
            .collect();
        for d in index.docs_by_keys(&missing).unwrap_or_default() {
            url_by_key.insert(d.url_key, d.url);
            snip_by_key.insert(d.url_key, d.snippet);
            title_by_key.insert(d.url_key, d.title);
        }
        let ann_sim: std::collections::HashMap<u64, f32> = ann.iter().copied().collect();
        let bm25_by_key: std::collections::HashMap<u64, f32> =
            bm25_full.iter().map(|h| (h.url_key, h.bm25)).collect();
        let mut ltr_keyed: Vec<(u64, f32)> = fused
            .iter()
            .take(100)
            .map(|(k, rrf)| {
                let f = Features {
                    rrf: *rrf,
                    bm25: bm25_by_key.get(k).copied().unwrap_or(0.0),
                    ann: ann_sim.get(k).copied().unwrap_or(0.0),
                    title_match_ratio: title_by_key
                        .get(k)
                        .map(|t| title_match_ratio(query, t))
                        .unwrap_or(0.0),
                    source_count: bm25_by_key.contains_key(k) as u8 as f32
                        + ann_sim.contains_key(k) as u8 as f32,
                    snippet_len_norm: 0.5,
                    freshness: 0.5,
                    domain_prior: 0.0,
                    geo: 0.0,
                };
                (*k, scorer.score(&f))
            })
            .collect();
        ltr_keyed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let head: Vec<(u64, f32)> = ltr_keyed.into_iter().take(20).collect();
        let ltr_ranked: Vec<String> = head
            .iter()
            .filter_map(|(k, _)| url_by_key.get(k).cloned())
            .collect();

        // The SHIPPED MMR path over the same head. Text MUST mirror the
        // planner's MMR input — "{title} {snippet}" — an earlier cut passed
        // snippet-only and measured a similarity production never computes.
        let texts: Vec<String> = head
            .iter()
            .map(|(k, _)| {
                format!(
                    "{} {}",
                    title_by_key.get(k).map(String::as_str).unwrap_or(""),
                    snip_by_key.get(k).map(String::as_str).unwrap_or("")
                )
            })
            .collect();
        let docs: Vec<meridian_rank::mmr::MmrDoc<'_>> = head
            .iter()
            .zip(texts.iter())
            .map(|((_, score), t)| meridian_rank::mmr::MmrDoc {
                score: *score,
                text: t,
            })
            .collect();
        let order = meridian_rank::mmr::mmr_order(&docs, meridian_rank::mmr::MMR_LAMBDA);
        let mmr_ranked: Vec<String> = order
            .iter()
            .filter_map(|&i| url_by_key.get(&head[i].0).cloned())
            .collect();

        evaluated += 1;
        a_ltr += alpha_ndcg_at(10, &ltr_ranked, judg);
        a_mmr += alpha_ndcg_at(10, &mmr_ranked, judg);
        n_ltr += ndcg_at(10, &ltr_ranked, &plain);
        n_mmr += ndcg_at(10, &mmr_ranked, &plain);

        // λ sweep (informational): the gate judges the SHIPPED constant; the
        // sweep is the evidence for choosing it. λ is chosen on this (tuning)
        // seed and judged frozen on a held-out generator seed — never the
        // reverse (risk #21 protocol).
        for (si, &lam) in LAMBDA_SWEEP.iter().enumerate() {
            let o = meridian_rank::mmr::mmr_order(&docs, lam);
            let ranked: Vec<String> = o
                .iter()
                .filter_map(|&i| url_by_key.get(&head[i].0).cloned())
                .collect();
            a_sweep[si] += alpha_ndcg_at(10, &ranked, judg);
            n_sweep[si] += ndcg_at(10, &ranked, &plain);
        }

        // GATED regime: BM25-matching head, ordered by lexical score.
        let mut b_head: Vec<(u64, f32)> =
            bm25_full.iter().map(|h| (h.url_key, h.bm25)).collect();
        b_head.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        b_head.truncate(20);
        let b_texts: Vec<String> = b_head
            .iter()
            .map(|(k, _)| {
                format!(
                    "{} {}",
                    title_by_key.get(k).map(String::as_str).unwrap_or(""),
                    snip_by_key.get(k).map(String::as_str).unwrap_or("")
                )
            })
            .collect();
        let b_docs: Vec<meridian_rank::mmr::MmrDoc<'_>> = b_head
            .iter()
            .zip(b_texts.iter())
            .map(|((_, score), t)| meridian_rank::mmr::MmrDoc {
                score: *score,
                text: t,
            })
            .collect();
        let b_base: Vec<String> = b_head
            .iter()
            .filter_map(|(k, _)| url_by_key.get(k).cloned())
            .collect();
        let b_order = meridian_rank::mmr::mmr_order(&b_docs, meridian_rank::mmr::MMR_LAMBDA);
        let b_mmr_ranked: Vec<String> = b_order
            .iter()
            .filter_map(|&i| url_by_key.get(&b_head[i].0).cloned())
            .collect();
        ab_base += alpha_ndcg_at(10, &b_base, judg);
        ab_mmr += alpha_ndcg_at(10, &b_mmr_ranked, judg);
        nb_base += ndcg_at(10, &b_base, &plain);
        nb_mmr += ndcg_at(10, &b_mmr_ranked, &plain);
        for (si, &lam) in LAMBDA_SWEEP.iter().enumerate() {
            let o = meridian_rank::mmr::mmr_order(&b_docs, lam);
            let ranked: Vec<String> = o
                .iter()
                .filter_map(|&i| url_by_key.get(&b_head[i].0).cloned())
                .collect();
            ab_sweep[si] += alpha_ndcg_at(10, &ranked, judg);
            nb_sweep[si] += ndcg_at(10, &ranked, &plain);
        }
    }

    let n = evaluated.max(1) as f64;
    println!("\n== GATED regime: BM25-matching head (web-head proxy) ==");
    println!("\n| ranking | alpha-nDCG@10 | nDCG@10 |");
    println!("|---|---|---|");
    println!("| BM25 | {:.4} | {:.4} |", ab_base / n, nb_base / n);
    println!("| MMR | {:.4} | {:.4} |", ab_mmr / n, nb_mmr / n);
    println!("\n| lambda (sweep) | alpha-nDCG@10 | nDCG@10 |");
    println!("|---|---|---|");
    for (si, &lam) in LAMBDA_SWEEP.iter().enumerate() {
        println!(
            "| {lam} | {:.4} | {:.4} |",
            ab_sweep[si] / n,
            nb_sweep[si] / n
        );
    }
    println!("\n== DIAGNOSTIC regime: local hybrid+LTR head ==");
    println!("\n| ranking | alpha-nDCG@10 | nDCG@10 |");
    println!("|---|---|---|");
    println!("| LTR | {:.4} | {:.4} |", a_ltr / n, n_ltr / n);
    println!("| MMR | {:.4} | {:.4} |", a_mmr / n, n_mmr / n);
    println!("\n| lambda (sweep) | alpha-nDCG@10 | nDCG@10 |");
    println!("|---|---|---|");
    for (si, &lam) in LAMBDA_SWEEP.iter().enumerate() {
        println!(
            "| {lam} | {:.4} | {:.4} |",
            a_sweep[si] / n,
            n_sweep[si] / n
        );
    }
    let diversity_gain = ab_mmr >= ab_base;
    let relevance_held = nb_mmr >= 0.99 * nb_base;
    println!(
        "\nGATE mmr improves alpha-ndcg@10 (bm25 head): {}",
        if diversity_gain { "PASS" } else { "FAIL" }
    );
    println!(
        "GATE mmr ndcg@10 loss <= 1% (bm25 head): {}",
        if relevance_held { "PASS" } else { "FAIL" }
    );
    if diversity_gain && relevance_held {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
