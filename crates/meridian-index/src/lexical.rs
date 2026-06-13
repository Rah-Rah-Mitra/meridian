//! Tantivy lexical index (SPEC §8.3, §9.1, §11).
//!
//! Schema (SPEC §6.1 hard rules — store ONLY url/title/snippet + fast fields):
//! - `url`    raw, stored (result link + Phase-5 delete term)
//! - `title`  tokenized with positions, stored, boost 2.0 at query time
//! - `body`   tokenized WITH FREQS ONLY (no positions), never stored
//! - `snippet` stored only (generated at ingest; zstd docstore)
//! - `url_key` u64 INDEXED|FAST — blake3(url) truncated; the shared identity
//!   between this index and the vector store (ANN hits resolve through it)
//! - fast fields: `h3_r7:u64` + `h3_r5:u64` (both INDEXED for the §11 k-ring
//!   TermSet prefilter at fine/coarse radius), `ts:u64` (INDEXED for range
//!   windows), `domain_hash:u64` (INDEXED for /v1/forget domain enumeration),
//!   `lang:u64`, `quality:f64`
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
const SCHEMA_VERSION: u32 = 3;

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
    /// Coarse parent of `h3_r7` (res-5); ingest computes it so large-radius
    /// filters need no per-doc parent math at query time.
    pub h3_r5: u64,
    pub quality: f64,
}

/// Search prefilters (SPEC §11): applied as MUST clauses before scoring.
#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    pub geo: Option<GeoCells>,
    /// Unix-seconds window over the `ts` fast field (inclusive).
    pub after_ts: Option<u64>,
    pub before_ts: Option<u64>,
}

/// A k-ring term set targeting one of the two indexed H3 resolutions.
#[derive(Debug, Clone)]
pub enum GeoCells {
    R5(Vec<u64>),
    R7(Vec<u64>),
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
    /// 0 = not geo-tagged.
    pub h3_r7: u64,
    /// 0 = unknown (LTR freshness stays neutral).
    pub ts: u64,
}

struct Fields {
    url: Field,
    url_key: Field,
    title: Field,
    body: Field,
    snippet: Field,
    h3_r7: Field,
    h3_r5: Field,
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
    let h3_r7 = b.add_u64_field("h3_r7", INDEXED | FAST);
    let h3_r5 = b.add_u64_field("h3_r5", INDEXED | FAST);
    let ts = b.add_u64_field("ts", INDEXED | FAST);
    let domain_hash = b.add_u64_field("domain_hash", INDEXED | FAST);
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
            h3_r5,
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
            self.fields.h3_r5 => d.h3_r5,
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
        self.search_filtered(query_text, top_k, &SearchFilter::default())
    }

    /// BM25 with the §11 prefilters: H3 k-ring TermSet (fine res-7 or coarse
    /// res-5 — see `meridian-geo::filter_cells`) and/or a `ts` window. Filters
    /// apply BEFORE scoring (BooleanQuery MUST clauses over indexed fields).
    pub fn search_filtered(
        &self,
        query_text: &str,
        top_k: usize,
        filter: &SearchFilter,
    ) -> Result<Vec<LexicalHit>, IndexError> {
        let searcher = self.reader.searcher();
        let mut parser =
            QueryParser::for_index(&self.index, vec![self.fields.title, self.fields.body]);
        parser.set_field_boost(self.fields.title, 2.0);
        // Lenient parse: user queries are never a hard error; unparsable parts
        // are dropped by tantivy and the rest still searches.
        let (text_query, _errors) = parser.parse_query_lenient(query_text);

        let query = self.compose_query(text_query, filter);
        let hits = searcher.search(&query, &TopDocs::with_limit(top_k.max(1)).order_by_score())?;
        let mut out = Vec::with_capacity(hits.len());
        for (score, addr) in hits {
            out.push(self.hit_from(&searcher, addr, score)?);
        }
        Ok(out)
    }

    fn compose_query(
        &self,
        text_query: Box<dyn tantivy::query::Query>,
        filter: &SearchFilter,
    ) -> Box<dyn tantivy::query::Query> {
        use tantivy::query::{BooleanQuery, Occur, Query, RangeQuery, TermSetQuery};
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = vec![(Occur::Must, text_query)];
        match &filter.geo {
            Some(GeoCells::R7(cells)) => {
                let terms = cells
                    .iter()
                    .map(|&c| tantivy::Term::from_field_u64(self.fields.h3_r7, c));
                clauses.push((Occur::Must, Box::new(TermSetQuery::new(terms))));
            }
            Some(GeoCells::R5(cells)) => {
                let terms = cells
                    .iter()
                    .map(|&c| tantivy::Term::from_field_u64(self.fields.h3_r5, c));
                clauses.push((Occur::Must, Box::new(TermSetQuery::new(terms))));
            }
            None => {}
        }
        if filter.after_ts.is_some() || filter.before_ts.is_some() {
            let lower = filter.after_ts.map_or(std::ops::Bound::Unbounded, |t| {
                std::ops::Bound::Included(tantivy::Term::from_field_u64(self.fields.ts, t))
            });
            let upper = filter.before_ts.map_or(std::ops::Bound::Unbounded, |t| {
                std::ops::Bound::Included(tantivy::Term::from_field_u64(self.fields.ts, t))
            });
            clauses.push((Occur::Must, Box::new(RangeQuery::new(lower, upper))));
        }
        if clauses.len() == 1 {
            clauses.remove(0).1
        } else {
            Box::new(BooleanQuery::new(clauses))
        }
    }

    fn hit_from(
        &self,
        searcher: &tantivy::Searcher,
        addr: tantivy::DocAddress,
        score: f32,
    ) -> Result<LexicalHit, IndexError> {
        let stored: TantivyDocument = searcher.doc(addr)?;
        let get_str = |f: Field| {
            stored
                .get_first(f)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        let get_u64 = |f: Field| stored.get_first(f).and_then(|v| v.as_u64()).unwrap_or(0);
        let domain_hash = get_u64(self.fields.domain_hash);
        // h3/ts are fast fields (not stored): read the columnar values.
        let ff = searcher.segment_reader(addr.segment_ord).fast_fields();
        let h3_r7 = ff
            .u64("h3_r7")
            .ok()
            .and_then(|c| c.first(addr.doc_id))
            .unwrap_or(0);
        let ts = ff
            .u64("ts")
            .ok()
            .and_then(|c| c.first(addr.doc_id))
            .unwrap_or(0);
        let url = get_str(self.fields.url);
        Ok(LexicalHit {
            url_key: url_key(&url),
            url,
            title: get_str(self.fields.title),
            snippet: get_str(self.fields.snippet),
            bm25: score,
            domain_hash,
            h3_r7,
            ts,
        })
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
                let mut hit = self.hit_from(&searcher, *addr, 0.0)?;
                hit.url_key = key;
                out.push(hit);
            }
        }
        Ok(out)
    }
}

impl LexicalIndex {
    /// Heatmap rollup (SPEC §10 `/v1/geo/heatmap`): count geo-tagged docs
    /// matching `query_text` (None = everything) within the `ts` window,
    /// grouped by the res-7 cell's parent at `res`. Fast-field scan over
    /// matching docs — gated at ≤150ms p50 on 100k docs (§6.3).
    pub fn heatmap(
        &self,
        query_text: Option<&str>,
        after_ts: Option<u64>,
        res: u8,
    ) -> Result<Vec<(u64, u32)>, IndexError> {
        use tantivy::collector::{Collector, SegmentCollector};

        struct CellCount {
            after_ts: Option<u64>,
        }
        struct CellCountSegment {
            after_ts: Option<u64>,
            h3: tantivy::columnar::Column<u64>,
            ts: tantivy::columnar::Column<u64>,
            counts: std::collections::HashMap<u64, u32>,
        }
        impl Collector for CellCount {
            type Fruit = std::collections::HashMap<u64, u32>;
            type Child = CellCountSegment;

            fn for_segment(
                &self,
                _ord: tantivy::SegmentOrdinal,
                segment: &tantivy::SegmentReader,
            ) -> tantivy::Result<Self::Child> {
                Ok(CellCountSegment {
                    after_ts: self.after_ts,
                    h3: segment.fast_fields().u64("h3_r7")?,
                    ts: segment.fast_fields().u64("ts")?,
                    counts: std::collections::HashMap::new(),
                })
            }

            fn requires_scoring(&self) -> bool {
                false
            }

            fn merge_fruits(&self, fruits: Vec<Self::Fruit>) -> tantivy::Result<Self::Fruit> {
                let mut merged = std::collections::HashMap::new();
                for fruit in fruits {
                    for (cell, n) in fruit {
                        *merged.entry(cell).or_insert(0) += n;
                    }
                }
                Ok(merged)
            }
        }
        impl SegmentCollector for CellCountSegment {
            type Fruit = std::collections::HashMap<u64, u32>;

            fn collect(&mut self, doc: tantivy::DocId, _score: tantivy::Score) {
                let cell = self.h3.first(doc).unwrap_or(0);
                if cell == 0 {
                    return; // untagged docs don't heat the map
                }
                if let Some(after) = self.after_ts {
                    if self.ts.first(doc).unwrap_or(0) < after {
                        return;
                    }
                }
                *self.counts.entry(cell).or_insert(0) += 1;
            }

            fn harvest(self) -> Self::Fruit {
                self.counts
            }
        }

        let searcher = self.reader.searcher();
        let query: Box<dyn tantivy::query::Query> = match query_text {
            Some(q) if !q.trim().is_empty() => {
                let mut parser =
                    QueryParser::for_index(&self.index, vec![self.fields.title, self.fields.body]);
                parser.set_field_boost(self.fields.title, 2.0);
                parser.parse_query_lenient(q).0
            }
            _ => Box::new(tantivy::query::AllQuery),
        };
        let per_r7 = searcher.search(&query, &CellCount { after_ts })?;

        // Roll res-7 cells up to the requested resolution in one pass.
        let mut rolled: std::collections::HashMap<u64, u32> = std::collections::HashMap::new();
        for (cell, n) in per_r7 {
            // Invalid cells (corrupt ingest) are dropped rather than crashing
            // the endpoint; parent_at(None) only fires on malformed u64s.
            if let Some(parent) = parent_rollup(cell, res) {
                *rolled.entry(parent).or_insert(0) += n;
            }
        }
        let mut out: Vec<(u64, u32)> = rolled.into_iter().collect();
        out.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(out)
    }

    /// Stage deletion of every doc whose `url_key` matches (SPEC §10
    /// `/v1/forget`). Visible after [`Self::commit`].
    pub fn delete_by_url_key(&self, key: u64) {
        let writer = self.writer.lock().expect("index writer lock");
        writer.delete_term(tantivy::Term::from_field_u64(self.fields.url_key, key));
    }

    /// All url_keys under a domain (`/v1/forget` by domain). Bounded scan.
    pub fn url_keys_by_domain(&self, domain_hash: u64) -> Result<Vec<u64>, IndexError> {
        let searcher = self.reader.searcher();
        let term = tantivy::Term::from_field_u64(self.fields.domain_hash, domain_hash);
        let query = tantivy::query::TermQuery::new(term, IndexRecordOption::Basic);
        let hits = searcher.search(&query, &TopDocs::with_limit(10_000).order_by_score())?;
        let mut keys = Vec::with_capacity(hits.len());
        for (_, addr) in hits {
            let ff = searcher.segment_reader(addr.segment_ord).fast_fields();
            if let Some(key) = ff.u64("url_key").ok().and_then(|c| c.first(addr.doc_id)) {
                keys.push(key);
            }
        }
        Ok(keys)
    }
}

/// H3 parent rollup via h3o (external leaf dep — no internal-crate edge).
/// `None` only for malformed cells, which the heatmap drops silently.
fn parent_rollup(cell: u64, res: u8) -> Option<u64> {
    let cell = h3o::CellIndex::try_from(cell).ok()?;
    let res = h3o::Resolution::try_from(res).ok()?;
    if res >= cell.resolution() {
        return Some(u64::from(cell));
    }
    cell.parent(res).map(u64::from)
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
            h3_r5: 0,
            quality: 0.0,
        }
    }

    fn geo_doc(url: &str, title: &str, body: &str, lat: f64, lon: f64, ts: u64) -> IndexDoc {
        // Mirror ingest: res-7 cell + its res-5 parent, via h3o directly so the
        // test doesn't depend on meridian-geo.
        let ll = h3o::LatLng::new(lat, lon).unwrap();
        let r7 = u64::from(ll.to_cell(h3o::Resolution::Seven));
        let r5 = u64::from(ll.to_cell(h3o::Resolution::Five));
        IndexDoc {
            ts,
            h3_r7: r7,
            h3_r5: r5,
            ..doc(url, title, body)
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
    fn geo_filter_ts_window_heatmap_and_forget() {
        let (index, dir) = temp_index();
        // Two Berlin docs (different days), one Tokyo doc, one untagged doc.
        // The doc() helper hashes the full URL; per-domain forget needs real
        // per-HOST hashes, exactly like ingest computes them.
        let host = |d: &str, g: IndexDoc| IndexDoc {
            domain_hash: domain_hash(d),
            ..g
        };
        index
            .add(&host(
                "a.example",
                geo_doc(
                    "https://a.example/b1",
                    "Berlin transit",
                    "u-bahn lines and stations",
                    52.52,
                    13.40,
                    100,
                ),
            ))
            .unwrap();
        index
            .add(&host(
                "a.example",
                geo_doc(
                    "https://a.example/b2",
                    "Berlin museums",
                    "museum island exhibits",
                    52.53,
                    13.41,
                    200,
                ),
            ))
            .unwrap();
        index
            .add(&host(
                "b.example",
                geo_doc(
                    "https://b.example/t1",
                    "Tokyo transit",
                    "yamanote line stations",
                    35.68,
                    139.65,
                    150,
                ),
            ))
            .unwrap();
        index
            .add(&doc(
                "https://c.example/x",
                "Transit elsewhere",
                "stations of no fixed place",
            ))
            .unwrap();
        index.commit().unwrap();

        // Geo prefilter: a 10km ring around Berlin keeps Berlin docs only.
        let ll = h3o::LatLng::new(52.52, 13.40).unwrap();
        let center6 = ll.to_cell(h3o::Resolution::Six);
        let cells: Vec<u64> = center6
            .grid_disk::<Vec<_>>(3)
            .into_iter()
            .flat_map(|c| c.children(h3o::Resolution::Seven))
            .map(u64::from)
            .collect();
        let filter = SearchFilter {
            geo: Some(GeoCells::R7(cells)),
            ..Default::default()
        };
        let hits = index
            .search_filtered("transit stations", 10, &filter)
            .unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].url, "https://a.example/b1");
        assert!(
            hits[0].h3_r7 != 0 && hits[0].ts == 100,
            "fast fields surfaced"
        );

        // ts window: after=150 drops the older Berlin doc everywhere.
        let filter = SearchFilter {
            after_ts: Some(150),
            ..Default::default()
        };
        let hits = index
            .search_filtered("transit stations", 10, &filter)
            .unwrap();
        assert!(hits.iter().all(|h| h.ts >= 150), "{hits:?}");

        // Heatmap at res-5: Berlin cell counts 2, Tokyo 1; untagged ignored.
        let map = index.heatmap(None, None, 5).unwrap();
        assert_eq!(map.len(), 2, "{map:?}");
        assert_eq!(map[0].1, 2, "Berlin parent cell holds two docs");
        assert_eq!(map[1].1, 1);
        // Res-3 rollup still sums to 3 tagged docs.
        let coarse = index.heatmap(None, None, 3).unwrap();
        assert_eq!(coarse.iter().map(|(_, n)| n).sum::<u32>(), 3);
        // Query-scoped heatmap: museums only heat Berlin.
        let museums = index.heatmap(Some("museums"), None, 5).unwrap();
        assert_eq!(museums.len(), 1);

        // Forget by domain: a.example enumerates both Berlin docs.
        let keys = index.url_keys_by_domain(domain_hash("a.example")).unwrap();
        assert_eq!(keys.len(), 2);
        for k in &keys {
            index.delete_by_url_key(*k);
        }
        index.commit().unwrap();
        assert_eq!(index.num_docs(), 2);
        assert!(
            index.search("museums", 5).unwrap().is_empty(),
            "deleted doc unfindable"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    /// G2 regression (roadmap §10.2 F1): `url_keys_by_domain` enumerates at most
    /// 10k docs per call (a `TopDocs` bound), so a domain with >10k docs cannot
    /// be forgotten in a single enumerate-delete pass. The forget path drains it
    /// by looping enumerate→delete→commit→reload until enumeration returns empty.
    /// This test exercises that exact mechanism directly against `LexicalIndex`
    /// (the loop the `Ingestor::forget_domain` wrapper relies on) and asserts
    /// nothing of the domain survives — guarding the privacy.md promise that
    /// domain forget removes "every currently indexed document of the domain".
    #[test]
    fn forget_domain_drains_past_the_10k_enumeration_cap() {
        let (index, dir) = temp_index();
        let dh = domain_hash("big.example");
        // 12_500 docs under ONE domain: more than one 10k enumeration window, so
        // a single pass cannot drain it. A handful of other-domain docs verify
        // the drain is domain-scoped and leaves the rest intact.
        let n_target: u64 = 12_500;
        let n_other: u64 = 7;
        for i in 0..n_target {
            let url = format!("https://big.example/page/{i}");
            index
                .add(&IndexDoc {
                    domain_hash: dh,
                    ..doc(&url, "Bulk", "domain forget bulk doc")
                })
                .unwrap();
        }
        for i in 0..n_other {
            let url = format!("https://other.example/{i}");
            index
                .add(&IndexDoc {
                    domain_hash: domain_hash("other.example"),
                    ..doc(&url, "Keep", "unrelated domain doc")
                })
                .unwrap();
        }
        index.commit().unwrap();
        assert_eq!(index.num_docs(), n_target + n_other);

        // One pass alone is capped: it can enumerate at most 10k of the 12_500.
        let first = index.url_keys_by_domain(dh).unwrap();
        assert_eq!(
            first.len(),
            10_000,
            "single enumeration is bounded by the TopDocs cap"
        );

        // The drain loop the forget path uses: enumerate (≤10k) → delete → commit
        // → reload, repeat until the domain enumeration drains to empty.
        let mut total_removed = 0usize;
        let max_passes = (index.num_docs() as usize / 10_000) + 2;
        let mut passes = 0;
        loop {
            passes += 1;
            assert!(passes <= max_passes, "drain did not converge within bound");
            let keys = index.url_keys_by_domain(dh).unwrap();
            if keys.is_empty() {
                break;
            }
            for k in &keys {
                index.delete_by_url_key(*k);
            }
            index.commit().unwrap();
            total_removed += keys.len();
        }

        // The whole domain is gone; the other-domain docs are untouched.
        assert_eq!(total_removed, n_target as usize, "every domain doc removed");
        assert!(
            index.url_keys_by_domain(dh).unwrap().is_empty(),
            "no doc of the forgotten domain remains enumerable"
        );
        assert_eq!(
            index.num_docs(),
            n_other,
            "only the unrelated-domain docs survive"
        );
        assert!(
            passes > 1,
            "the >10k domain required multiple drain passes ({passes})"
        );

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
