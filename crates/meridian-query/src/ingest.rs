//! Ingest pipeline (SPEC §3): source → [fetch ladder] → clean text → lang-ID →
//! dedup (blake3 in redb) → snippet → index. Raw bodies never persist; the
//! content hash is the only memory of them (SPEC §6.1).
//!
//! Geo-tagging (SPEC §3): gazetteer fst scan over title + lead text → H3 res-7
//! cell (+ res-5 parent for coarse filters). Zero network — the remote geocoder
//! is an operator opt-in fallback handled at a higher layer. Untagged docs
//! carry h3 = 0 and simply never match geo filters.

use meridian_common::config::{IngestConfig, VectorConfig};
use meridian_egress::Lane;
use meridian_embed::Embedder;
use meridian_fetch::Fetcher;
use meridian_index::lexical::{IndexDoc, LexicalIndex, domain_hash, url_key};
use meridian_index::sketch;
use meridian_vector::VectorStore;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// content-hash → ingest unix-time.
const DEDUP_TABLE: TableDefinition<&[u8], u64> = TableDefinition::new("dedup_v1");
/// url_key → content-hash (16B): the reverse map `/v1/forget` needs to
/// tombstone a document's CONTENT when asked to forget its URL.
const HASH_BY_KEY: TableDefinition<u64, &[u8]> = TableDefinition::new("hash_by_key_v1");
/// Tombstoned content hashes: re-ingest of forgotten content is REFUSED until
/// the operator clears the tombstone (SPEC §10 `/v1/forget`).
const TOMBSTONES: TableDefinition<&[u8], u64> = TableDefinition::new("forget_tombstones_v1");
/// url_key → 64-byte derivation sketch (Phase 7, ADR-18). Written in the SAME
/// transaction as the dedup rows and removed in the SAME transaction as a
/// forget — a sketch may never outlive its document (ADR-19, risk #17).
/// Additive table: v0.1.0 volumes simply lack rows until docs are re-ingested.
const SKETCHES: TableDefinition<u64, &[u8]> = TableDefinition::new("sketch_v1");

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("dedup store: {0}")]
    Dedup(String),
    #[error("vector store: {0}")]
    Vector(String),
    #[error("index: {0}")]
    Index(String),
    #[error("fetch: {0}")]
    Fetch(#[from] meridian_fetch::FetchError),
    #[error("document is empty after extraction")]
    Empty,
}

/// One document offered for ingest (the `{text,...}` form of POST /v1/ingest;
/// the `{url}` form goes through [`Ingestor::ingest_url`] first).
#[derive(Debug, Clone)]
pub struct IngestText {
    pub text: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub ts: Option<u64>,
}

#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
pub struct IngestStats {
    pub accepted: usize,
    pub deduped: usize,
    /// Refused because the content hash is tombstoned (`/v1/forget`).
    pub refused: usize,
}

pub struct Ingestor {
    index: Arc<LexicalIndex>,
    dedup: Arc<Database>,
    fetcher: Arc<Fetcher>,
    embedder: Arc<Embedder>,
    vectors: Arc<VectorStore>,
    /// `None` = geo-tagging off (no gazetteer artifact shipped/found).
    gazetteer: Option<Arc<meridian_geo::Gazetteer>>,
    cfg: IngestConfig,
    persist_every: usize,
    docs_since_persist: AtomicUsize,
}

impl Ingestor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index: Arc<LexicalIndex>,
        fetcher: Arc<Fetcher>,
        embedder: Arc<Embedder>,
        vectors: Arc<VectorStore>,
        data_dir: &std::path::Path,
        gazetteer: Option<Arc<meridian_geo::Gazetteer>>,
        cfg: &IngestConfig,
        vector_cfg: &VectorConfig,
    ) -> Result<Self, IngestError> {
        std::fs::create_dir_all(data_dir).map_err(|e| IngestError::Dedup(e.to_string()))?;
        let dedup = Database::create(data_dir.join("dedup.redb"))
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        // Ensure the tables exist so first reads do not error.
        let wtx = dedup
            .begin_write()
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        wtx.open_table(DEDUP_TABLE)
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        wtx.open_table(HASH_BY_KEY)
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        wtx.open_table(TOMBSTONES)
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        wtx.open_table(SKETCHES)
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        wtx.commit()
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        Ok(Self {
            index,
            dedup: Arc::new(dedup),
            fetcher,
            embedder,
            vectors,
            gazetteer,
            cfg: cfg.clone(),
            persist_every: vector_cfg.persist_every_docs.max(1),
            docs_since_persist: AtomicUsize::new(0),
        })
    }

    /// Persist the vector store now (graceful shutdown / end of bulk load).
    pub fn flush_vectors(&self) -> Result<(), IngestError> {
        self.vectors
            .persist()
            .map_err(|e| IngestError::Vector(e.to_string()))
    }

    /// Read handle on the sketch table for query-time evidence clustering
    /// (Phase 7). Shares the dedup database — redb is single-open per process.
    pub fn sketch_reader(&self) -> SketchReader {
        SketchReader {
            db: self.dedup.clone(),
        }
    }

    /// Ingest a batch of text documents in ONE dedup transaction + ONE index
    /// commit — the only sane write pattern on SD-card storage (Profile R).
    pub fn ingest_batch(&self, docs: &[IngestText]) -> Result<IngestStats, IngestError> {
        let mut stats = IngestStats::default();
        let mut accepted: Vec<(u64, String)> = Vec::new(); // (url_key, text) → embed
        let wtx = self
            .dedup
            .begin_write()
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        {
            let mut table = wtx
                .open_table(DEDUP_TABLE)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            let mut by_key = wtx
                .open_table(HASH_BY_KEY)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            let tombstones = wtx
                .open_table(TOMBSTONES)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            let mut sketches = wtx
                .open_table(SKETCHES)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            for doc in docs {
                let text = doc.text.trim();
                if text.is_empty() {
                    continue;
                }
                // SPEC §9.3: dedup key = blake3(content) truncated to 16 bytes.
                let digest = blake3::hash(text.as_bytes());
                let key = &digest.as_bytes()[..16];
                // Forgotten content stays forgotten (SPEC §10 /v1/forget).
                if tombstones
                    .get(key)
                    .map_err(|e| IngestError::Dedup(e.to_string()))?
                    .is_some()
                {
                    stats.refused += 1;
                    continue;
                }
                let seen = table
                    .get(key)
                    .map_err(|e| IngestError::Dedup(e.to_string()))?
                    .is_some();
                if seen {
                    stats.deduped += 1;
                    continue;
                }
                let ts = doc.ts.unwrap_or_else(now_unix);
                table
                    .insert(key, ts)
                    .map_err(|e| IngestError::Dedup(e.to_string()))?;

                let index_doc = self.build_doc(doc, text, ts);
                by_key
                    .insert(index_doc.url_key, key)
                    .map_err(|e| IngestError::Dedup(e.to_string()))?;
                // Derivation sketch (Phase 7, ADR-18): same transaction as the
                // dedup rows so forget atomicity covers it (ADR-19).
                let sketch = meridian_index::sketch::Sketch::compute(text).encode();
                sketches
                    .insert(index_doc.url_key, sketch.as_slice())
                    .map_err(|e| IngestError::Dedup(e.to_string()))?;
                accepted.push((index_doc.url_key, text.to_owned()));
                self.index
                    .add(&index_doc)
                    .map_err(|e| IngestError::Index(e.to_string()))?;
                stats.accepted += 1;
            }
        }
        wtx.commit()
            .map_err(|e| IngestError::Dedup(e.to_string()))?;

        // Dense path (SPEC §3): batch-embed accepted texts → vector store under
        // the same url_key identity as the lexical doc.
        if !accepted.is_empty() {
            let texts: Vec<String> = accepted.iter().map(|(_, t)| t.clone()).collect();
            let embeddings = self.embedder.embed_batch(&texts);
            for ((key, _), vector) in accepted.iter().zip(embeddings.iter()) {
                self.vectors
                    .add(*key, vector)
                    .map_err(|e| IngestError::Vector(e.to_string()))?;
            }
        }

        self.index
            .commit()
            .map_err(|e| IngestError::Index(e.to_string()))?;

        // Coarse persist cadence (full-file save; SD-friendly). A crash loses
        // vectors since the last persist — lexical docs survive, so the worst
        // case is reduced dense recall until re-ingest (exit-note item).
        let since = self
            .docs_since_persist
            .fetch_add(stats.accepted, Ordering::Relaxed)
            + stats.accepted;
        if since >= self.persist_every {
            self.docs_since_persist.store(0, Ordering::Relaxed);
            self.flush_vectors()?;
        }
        Ok(stats)
    }

    /// The `{url}` ingest form: run the fetch ladder, then the text path.
    pub async fn ingest_url(&self, url: &str, lane: &Lane) -> Result<IngestStats, IngestError> {
        let fetched = self.fetcher.fetch_extract(url, lane).await?;
        if fetched.text.is_empty() {
            return Err(IngestError::Empty);
        }
        self.ingest_batch(&[IngestText {
            text: fetched.text,
            url: Some(fetched.url.to_string()),
            title: fetched.title,
            ts: None,
        }])
    }

    fn build_doc(&self, doc: &IngestText, text: &str, ts: u64) -> IndexDoc {
        let url = doc.url.clone().unwrap_or_default();
        let domain = url::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        let title = doc
            .title
            .clone()
            .unwrap_or_else(|| text.chars().take(80).collect::<String>());

        // Snippet at ingest so queries never need the body (SPEC §9.2).
        let snippet: String = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(self.cfg.snippet_max_chars)
            .collect();

        let lang = whichlang::detect_language(text) as u64;

        // Geo-tag (SPEC §3): gazetteer scan → res-7 cell + res-5 parent.
        // 0 = untagged; geo filters then simply never match this doc.
        let (h3_r7, h3_r5) = self
            .gazetteer
            .as_ref()
            .and_then(|g| g.scan(&title, text))
            .and_then(|(lat, lon)| {
                let r7 =
                    meridian_geo::h3::latlng_to_cell(lat, lon, meridian_geo::INDEX_RES).ok()?;
                let r5 = meridian_geo::h3::parent_at(r7, meridian_geo::ANALYTICS_RES)?;
                Some((r7, r5))
            })
            .unwrap_or((0, 0));

        IndexDoc {
            url_key: url_key(&url),
            url,
            title,
            snippet,
            body: text.to_owned(),
            ts,
            domain_hash: domain_hash(&domain),
            lang,
            h3_r7,
            h3_r5,
            quality: 0.0,
        }
    }

    /// `/v1/forget` (SPEC §10): remove documents by url_key from the lexical
    /// index + vector store, tombstone their content hashes (re-ingest refused),
    /// and drop their dedup entries. Idempotent; returns docs actually removed.
    pub fn forget_keys(&self, keys: &[u64]) -> Result<usize, IngestError> {
        let mut removed = 0;
        let wtx = self
            .dedup
            .begin_write()
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        {
            let mut dedup = wtx
                .open_table(DEDUP_TABLE)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            let mut by_key = wtx
                .open_table(HASH_BY_KEY)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            let mut tombstones = wtx
                .open_table(TOMBSTONES)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            let mut sketches = wtx
                .open_table(SKETCHES)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            for &key in keys {
                let hash: Option<Vec<u8>> = by_key
                    .get(key)
                    .map_err(|e| IngestError::Dedup(e.to_string()))?
                    .map(|v| v.value().to_vec());
                if let Some(hash) = hash {
                    tombstones
                        .insert(hash.as_slice(), now_unix())
                        .map_err(|e| IngestError::Dedup(e.to_string()))?;
                    dedup
                        .remove(hash.as_slice())
                        .map_err(|e| IngestError::Dedup(e.to_string()))?;
                    by_key
                        .remove(key)
                        .map_err(|e| IngestError::Dedup(e.to_string()))?;
                    removed += 1;
                }
                // A sketch may never outlive its document (ADR-19, risk #17) —
                // removed unconditionally, same transaction.
                sketches
                    .remove(key)
                    .map_err(|e| IngestError::Dedup(e.to_string()))?;
                self.index.delete_by_url_key(key);
                let _ = self
                    .vectors
                    .remove(key)
                    .map_err(|e| IngestError::Vector(e.to_string()))?;
            }
        }
        wtx.commit()
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        self.index
            .commit()
            .map_err(|e| IngestError::Index(e.to_string()))?;
        self.flush_vectors()?;
        Ok(removed)
    }

    /// Forget every document under a domain (SPEC §10 `{domain}` form).
    pub fn forget_domain(&self, domain: &str) -> Result<usize, IngestError> {
        let keys = self
            .index
            .url_keys_by_domain(domain_hash(domain))
            .map_err(|e| IngestError::Index(e.to_string()))?;
        self.forget_keys(&keys)
    }

    /// Forget by content hash (SPEC §10 `{content_hash}` form, 16-byte hex).
    /// Tombstones the hash UNCONDITIONALLY (so content never seen locally is
    /// still refused at future ingest), then removes any docs carrying it.
    pub fn forget_content_hash(&self, hash: &[u8]) -> Result<usize, IngestError> {
        // Find url_keys carrying this hash (bounded scan: one entry per doc).
        let keys: Vec<u64> = {
            let rtx = self
                .dedup
                .begin_read()
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            let by_key = rtx
                .open_table(HASH_BY_KEY)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            by_key
                .iter()
                .map_err(|e| IngestError::Dedup(e.to_string()))?
                .filter_map(|entry| entry.ok())
                .filter(|(_, v)| v.value() == hash)
                .map(|(k, _)| k.value())
                .collect()
        };
        let removed = self.forget_keys(&keys)?;
        // Tombstone even when no doc matched.
        let wtx = self
            .dedup
            .begin_write()
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        {
            let mut tombstones = wtx
                .open_table(TOMBSTONES)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            tombstones
                .insert(hash, now_unix())
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            let mut dedup = wtx
                .open_table(DEDUP_TABLE)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            dedup
                .remove(hash)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
        }
        wtx.commit()
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        Ok(removed)
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Read-only view of the sketch table for query-time evidence clustering
/// (Phase 7). Cheap to clone; reads are mmap'd point lookups.
#[derive(Clone)]
pub struct SketchReader {
    db: Arc<Database>,
}

impl SketchReader {
    /// Standalone read handle for harnesses that own the process (the
    /// dup-eval diversity gate): opens the dedup db directly, no Ingestor.
    /// Production uses `Ingestor::sketch_reader` (redb is single-open per
    /// process; a harness is the only opener in its process).
    pub fn open(data_dir: &std::path::Path) -> Result<Self, IngestError> {
        let db = Database::create(data_dir.join("dedup.redb"))
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        Ok(Self { db: Arc::new(db) })
    }

    /// Fetch sketches for the given url_keys. Keys without a sketch (web
    /// results never ingested, docs from a pre-v0.2.0 volume) are simply
    /// absent from the map — the evidence layer reports them as un-asserted.
    pub fn get_many(&self, keys: &[u64]) -> std::collections::HashMap<u64, sketch::Sketch> {
        let mut out = std::collections::HashMap::new();
        let Ok(rtx) = self.db.begin_read() else {
            return out;
        };
        let Ok(table) = rtx.open_table(SKETCHES) else {
            return out; // table absent on old volumes — evidence degrades to null
        };
        for &key in keys {
            if let Ok(Some(v)) = table.get(key) {
                if let Some(s) = sketch::Sketch::decode(v.value()) {
                    out.insert(key, s);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meridian_common::config::{FetchConfig, IndexConfig, LanesConfig};
    use meridian_egress::LaneRegistry;

    fn temp_ingestor() -> (Ingestor, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "meridian-ingest-test-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let index_cfg = IndexConfig {
            data_dir: dir.clone(),
            writer_threads: 1,
            writer_heap_bytes: 16 * 1024 * 1024,
            merge_max_docs: 100_000,
        };
        let index = Arc::new(LexicalIndex::open_or_create(&index_cfg).unwrap());
        let lanes = Arc::new(LaneRegistry::new(&LanesConfig::default(), &dir).unwrap());
        let fetcher = Arc::new(Fetcher::new(lanes, &FetchConfig::default(), false));
        let vector_cfg = meridian_common::config::VectorConfig::default();
        let embedder = Arc::new(Embedder::test_stub(64));
        let vectors = Arc::new(VectorStore::open_or_create(&dir, 64, &vector_cfg).unwrap());
        let ingestor = Ingestor::new(
            index,
            fetcher,
            embedder,
            vectors,
            &dir,
            None,
            &meridian_common::config::IngestConfig::default(),
            &vector_cfg,
        )
        .unwrap();
        (ingestor, dir)
    }

    #[test]
    fn dedup_by_content_hash_and_snippet_generation() {
        let (ingestor, dir) = temp_ingestor();
        let doc = IngestText {
            text: "The   meridian \n arc measurement of 1792 established the metre. ".to_owned(),
            url: Some("https://history.example/metre".to_owned()),
            title: Some("Metre history".to_owned()),
            ts: Some(42),
        };
        let stats = ingestor.ingest_batch(&[doc.clone(), doc.clone()]).unwrap();
        assert_eq!(
            stats.accepted, 1,
            "second copy in the same batch must dedup"
        );
        assert_eq!(stats.deduped, 1);

        // Same content again in a later batch — still deduped (persisted hash).
        let stats2 = ingestor.ingest_batch(std::slice::from_ref(&doc)).unwrap();
        assert_eq!(stats2.accepted, 0);
        assert_eq!(stats2.deduped, 1);

        let hits = ingestor.index.search("meridian metre", 10).unwrap();
        assert_eq!(hits.len(), 1);
        // Sketch row written in the same transaction (Phase 7, ADR-18/19).
        let key = meridian_index::lexical::url_key("https://history.example/metre");
        let sketches = ingestor.sketch_reader().get_many(&[key]);
        assert!(
            sketches.contains_key(&key),
            "ingested doc must carry a derivation sketch"
        );
        // Dense path actually ran: one vector under the doc's url_key, and the
        // stub embedding of the same text retrieves it.
        assert_eq!(ingestor.vectors.len(), 1);
        let qv = ingestor
            .embedder
            .embed_query("meridian arc measurement metre");
        let ann = ingestor.vectors.search(&qv, 1).unwrap();
        assert_eq!(ann[0].0, hits[0].url_key);
        assert!(
            hits[0].snippet.contains("meridian arc"),
            "whitespace-normalized snippet"
        );

        // /v1/forget semantics (SPEC §10): removal + tombstone + refusal.
        let removed = ingestor.forget_keys(&[key]).unwrap();
        assert_eq!(removed, 1);
        assert!(
            ingestor
                .index
                .search("meridian metre", 10)
                .unwrap()
                .is_empty(),
            "forgotten doc must leave the lexical index"
        );
        assert_eq!(ingestor.vectors.len(), 0, "vector dropped too");
        // The sketch may not outlive the document (ADR-19, risk #17).
        assert!(
            ingestor.sketch_reader().get_many(&[key]).is_empty(),
            "forgotten doc's sketch must be gone"
        );
        // Re-ingest of the SAME content is refused (tombstone), not deduped.
        let stats3 = ingestor.ingest_batch(&[doc]).unwrap();
        assert_eq!(stats3.accepted, 0);
        assert_eq!(stats3.refused, 1, "tombstoned content must be refused");
        let _ = std::fs::remove_dir_all(dir);
    }
}
