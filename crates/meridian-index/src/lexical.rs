//! Tantivy lexical index (SPEC §8.3, §9.1, §11).
//!
//! Schema (SPEC §6.1 hard rules — store ONLY url/title/snippet + fast fields):
//! - `url`    raw, stored (result link + Phase-5 delete term)
//! - `title`  tokenized with positions, stored, boost 2.0 at query time
//! - `body`   tokenized WITH FREQS ONLY (no positions), never stored
//! - `snippet` stored only (generated at ingest; zstd docstore)
//! - `url_key` u64 INDEXED|FAST — blake3(url) truncated; the shared identity
//!   between this index and the vector store (ANN hits resolve through it)
//! - fast fields: `h3_r7:u64`, `ts:u64`, `domain_hash:u64`, `lang:u64`,
//!   `quality:f64`
//!
//! Tokenizer: simple + lowercase + English stemmer (`en_stem_meridian`).

use meridian_common::config::IndexConfig;
use std::sync::Mutex;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{
    FAST, Field, INDEXED, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing,
    TextOptions, Value,
};
use tantivy::tokenizer::{Language, LowerCaser, SimpleTokenizer, Stemmer, TextAnalyzer};
use tantivy::{Index, IndexReader, IndexWriter, TantivyDocument, doc};

const TOKENIZER: &str = "en_stem_meridian";
/// Bump on ANY schema change; `open_or_create` refuses a mismatched index
/// (SPEC §14: versioned format, `--reindex` migration).
const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, thiserror::Error)]
#[error("index: {0}")]
pub struct IndexError(String);

impl From<tantivy::TantivyError> for IndexError {
    fn from(e: tantivy::TantivyError) -> Self {
        Self(e.to_string())
    }
}

/// One document entering the index. `body` is indexed and DISCARDED — it is
/// never stored (SPEC §6.1).
#[derive(Debug, Clone)]
pub struct IndexDoc {
    pub url: String,
    pub url_key: u64,
    pub title: String,
    pub snippet: String,
    pub body: String,
    pub ts: u64,
    pub domain_hash: u64,
    pub lang: u64,
    pub h3_r7: u64,
    pub quality: f64,
}

/// One lexical hit leaving the index.
#[derive(Debug, Clone)]
pub struct LexicalHit {
    pub url: String,
    pub url_key: u64,
    pub title: String,
    pub snippet: String,
    pub bm25: f32,
    pub domain_hash: u64,
}

struct Fields {
    url: Field,
    url_key: Field,
    title: Field,
    body: Field,
    snippet: Field,
    h3_r7: Field,
    ts: Field,
    domain_hash: Field,
    lang: Field,
    quality: Field,
}

pub struct LexicalIndex {
    index: Index,
    writer: Mutex<IndexWriter<TantivyDocument>>,
    reader: IndexReader,
    fields: Fields,
}

fn build_schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let text_with_positions = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let text_freqs_only = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqs),
    );
    let url = b.add_text_field("url", STRING | STORED);
    let url_key = b.add_u64_field("url_key", INDEXED | FAST);
    let title = b.add_text_field("title", text_with_positions | STORED);
    let body = b.add_text_field("body", text_freqs_only);
    let snippet = b.add_text_field("snippet", STORED);
    let h3_r7 = b.add_u64_field("h3_r7", FAST);
    let ts = b.add_u64_field("ts", FAST);
    let domain_hash = b.add_u64_field("domain_hash", FAST);
    let lang = b.add_u64_field("lang", FAST);
    let quality = b.add_f64_field("quality", FAST);
    (
        b.build(),
        Fields {
            url,
            url_key,
            title,
            body,
            snippet,
            h3_r7,
            ts,
            domain_hash,
            lang,
            quality,
        },
    )
}

impl LexicalIndex {
    pub fn open_or_create(cfg: &IndexConfig) -> Result<Self, IndexError> {
        let dir = cfg.data_dir.join("index");
        std::fs::create_dir_all(&dir).map_err(|e| IndexError(e.to_string()))?;
        let version_file = dir.join("SCHEMA_VERSION");
        let (schema, fields) = build_schema();
        let index = if Index::exists(
            &tantivy::directory::MmapDirectory::open(&dir)
                .map_err(|e| IndexError(e.to_string()))?,
        )
        .map_err(|e| IndexError(e.to_string()))?
        {
            let on_disk: u32 = std::fs::read_to_string(&version_file)
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            if on_disk != SCHEMA_VERSION {
                return Err(IndexError(format!(
                    "index schema v{on_disk} != v{SCHEMA_VERSION}; wipe the data dir and re-ingest                      (SPEC §14 --reindex migration arrives with the first post-1.0 format change)"
                )));
            }
            Index::open_in_dir(&dir)?
        } else {
            let index = Index::create_in_dir(&dir, schema)?;
            std::fs::write(&version_file, SCHEMA_VERSION.to_string())
                .map_err(|e| IndexError(e.to_string()))?;
            index
        };
        index.tokenizers().register(
            TOKENIZER,
            TextAnalyzer::builder(SimpleTokenizer::default())
                .filter(LowerCaser)
                .filter(Stemmer::new(Language::English))
                .build(),
        );

        let writer: IndexWriter<TantivyDocument> =
            index.writer_with_num_threads(cfg.writer_threads, cfg.writer_heap_bytes)?;
        // SPEC §9.1: 256MB segment cap, expressed in docs (ADR-05: tantivy's
        // knob is doc count; 527 B/doc measured → merge_max_docs default 500k).
        let mut merge_policy = tantivy::merge_policy::LogMergePolicy::default();
        merge_policy.set_max_docs_before_merge(cfg.merge_max_docs);
        writer.set_merge_policy(Box::new(merge_policy));

        let reader = index.reader()?;
        Ok(Self {
            index,
            writer: Mutex::new(writer),
            reader,
            fields,
        })
    }

    /// Stage one document (visible after the next [`Self::commit`]).
    pub fn add(&self, d: &IndexDoc) -> Result<(), IndexError> {
        let writer = self.writer.lock().expect("index writer lock");
        writer.add_document(doc!(
            self.fields.url => d.url.as_str(),
            self.fields.url_key => d.url_key,
            self.fields.title => d.title.as_str(),
            self.fields.body => d.body.as_str(),
            self.fields.snippet => d.snippet.as_str(),
            self.fields.h3_r7 => d.h3_r7,
            self.fields.ts => d.ts,
            self.fields.domain_hash => d.domain_hash,
            self.fields.lang => d.lang,
            self.fields.quality => d.quality,
        ))?;
        Ok(())
    }

    pub fn commit(&self) -> Result<(), IndexError> {
        self.writer.lock().expect("index writer lock").commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// BM25 top-k with `title^2.0, body^1.0` (SPEC §11). CPU-bound — callers on
    /// the async path bridge via rayon/oneshot (SPEC §7.2).
    pub fn search(&self, query_text: &str, top_k: usize) -> Result<Vec<LexicalHit>, IndexError> {
        let searcher = self.reader.searcher();
        let mut parser =
            QueryParser::for_index(&self.index, vec![self.fields.title, self.fields.body]);
        parser.set_field_boost(self.fields.title, 2.0);
        // Lenient parse: user queries are never a hard error; unparsable parts
        // are dropped by tantivy and the rest still searches.
        let (query, _errors) = parser.parse_query_lenient(query_text);

        let hits = searcher.search(&query, &TopDocs::with_limit(top_k.max(1)).order_by_score())?;
        let mut out = Vec::with_capacity(hits.len());
        for (score, addr) in hits {
            let stored: TantivyDocument = searcher.doc(addr)?;
            let get_str = |f: Field| {
                stored
                    .get_first(f)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned()
            };
            let domain_hash = stored
                .get_first(self.fields.domain_hash)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let url = get_str(self.fields.url);
            out.push(LexicalHit {
                url_key: url_key(&url),
                url,
                title: get_str(self.fields.title),
                snippet: get_str(self.fields.snippet),
                bm25: score,
                domain_hash,
            });
        }
        Ok(out)
    }

    pub fn num_docs(&self) -> u64 {
        self.reader.searcher().num_docs()
    }

    /// Resolve vector-store keys to renderable docs (ANN-only fusion hits).
    /// One term lookup per key — bounded by the ANN top-k (SPEC §11: 200).
    pub fn docs_by_keys(&self, keys: &[u64]) -> Result<Vec<LexicalHit>, IndexError> {
        let searcher = self.reader.searcher();
        let mut out = Vec::with_capacity(keys.len());
        for &key in keys {
            let term = tantivy::Term::from_field_u64(self.fields.url_key, key);
            let query = tantivy::query::TermQuery::new(term, IndexRecordOption::Basic);
            let hits = searcher.search(&query, &TopDocs::with_limit(1).order_by_score())?;
            if let Some((_, addr)) = hits.first() {
                let stored: TantivyDocument = searcher.doc(*addr)?;
                let get_str = |f: Field| {
                    stored
                        .get_first(f)
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned()
                };
                let domain_hash = stored
                    .get_first(self.fields.domain_hash)
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let url = get_str(self.fields.url);
                out.push(LexicalHit {
                    url_key: key,
                    url,
                    title: get_str(self.fields.title),
                    snippet: get_str(self.fields.snippet),
                    bm25: 0.0,
                    domain_hash,
                });
            }
        }
        Ok(out)
    }
}

/// Canonical 64-bit URL identity shared by the lexical index, the vector store,
/// and RRF fusion: blake3(url) truncated to 8 LE bytes.
pub fn url_key(url: &str) -> u64 {
    let digest = blake3::hash(url.as_bytes());
    u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("8 bytes"))
}

/// Stable 64-bit domain identity for diversity caps and facets.
pub fn domain_hash(domain: &str) -> u64 {
    let digest = blake3::hash(domain.as_bytes());
    u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("8 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_index() -> (LexicalIndex, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "meridian-index-test-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let cfg = IndexConfig {
            data_dir: dir.clone(),
            writer_threads: 1,
            writer_heap_bytes: 16 * 1024 * 1024,
            merge_max_docs: 100_000,
        };
        (LexicalIndex::open_or_create(&cfg).unwrap(), dir)
    }

    fn doc(url: &str, title: &str, body: &str) -> IndexDoc {
        IndexDoc {
            url_key: url_key(url),
            url: url.to_owned(),
            title: title.to_owned(),
            snippet: body.chars().take(50).collect(),
            body: body.to_owned(),
            ts: 1,
            domain_hash: domain_hash(url),
            lang: 0,
            h3_r7: 0,
            quality: 0.0,
        }
    }

    #[test]
    fn index_search_roundtrip_with_title_boost_and_stemming() {
        let (index, dir) = temp_index();
        index
            .add(&doc(
                "https://a.example/1",
                "Rust compilers",
                "a page about borrow checking",
            ))
            .unwrap();
        index
            .add(&doc(
                "https://b.example/2",
                "Gardening",
                "compiling rust programs in the garden shed",
            ))
            .unwrap();
        index
            .add(&doc(
                "https://c.example/3",
                "Cooking",
                "nothing relevant here",
            ))
            .unwrap();
        index.commit().unwrap();
        assert_eq!(index.num_docs(), 3);

        // Stemmed match: query "compiler" hits "compilers"/"compiling".
        let hits = index.search("compiler", 10).unwrap();
        assert_eq!(hits.len(), 2, "{hits:?}");
        // Title boost: the title match outranks the body match.
        assert_eq!(hits[0].url, "https://a.example/1");
        assert!(!hits[0].snippet.is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reopen_preserves_documents() {
        let (index, dir) = temp_index();
        index
            .add(&doc("https://a.example/1", "Persistence", "durable data"))
            .unwrap();
        index.commit().unwrap();
        drop(index);

        let cfg = IndexConfig {
            data_dir: dir.clone(),
            ..Default::default()
        };
        let reopened = LexicalIndex::open_or_create(&cfg).unwrap();
        assert_eq!(reopened.num_docs(), 1);
        assert_eq!(reopened.search("durable", 5).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
