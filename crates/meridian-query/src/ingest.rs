//! Ingest pipeline (SPEC §3): source → [fetch ladder] → clean text → lang-ID →
//! dedup (blake3 in redb) → snippet → index. Raw bodies never persist; the
//! content hash is the only memory of them (SPEC §6.1).
//!
//! Geo-tagging (gazetteer → H3) is Phase 5; `h3_r7` is indexed as 0 until then
//! so the schema never needs a reindex.

use meridian_common::config::IngestConfig;
use meridian_egress::Lane;
use meridian_fetch::Fetcher;
use meridian_index::lexical::{IndexDoc, LexicalIndex, domain_hash};
use redb::{Database, ReadableTable, TableDefinition};
use std::sync::Arc;

/// content-hash → ingest unix-time. Tombstones (Phase 5 `/v1/forget`) will live
/// in a sibling table.
const DEDUP_TABLE: TableDefinition<&[u8], u64> = TableDefinition::new("dedup_v1");

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("dedup store: {0}")]
    Dedup(String),
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
}

pub struct Ingestor {
    index: Arc<LexicalIndex>,
    dedup: Database,
    fetcher: Arc<Fetcher>,
    cfg: IngestConfig,
}

impl Ingestor {
    pub fn new(
        index: Arc<LexicalIndex>,
        fetcher: Arc<Fetcher>,
        data_dir: &std::path::Path,
        cfg: &IngestConfig,
    ) -> Result<Self, IngestError> {
        std::fs::create_dir_all(data_dir).map_err(|e| IngestError::Dedup(e.to_string()))?;
        let dedup = Database::create(data_dir.join("dedup.redb"))
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        // Ensure the table exists so first reads do not error.
        let wtx = dedup
            .begin_write()
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        wtx.open_table(DEDUP_TABLE)
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        wtx.commit()
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        Ok(Self {
            index,
            dedup,
            fetcher,
            cfg: cfg.clone(),
        })
    }

    /// Ingest a batch of text documents in ONE dedup transaction + ONE index
    /// commit — the only sane write pattern on SD-card storage (Profile R).
    pub fn ingest_batch(&self, docs: &[IngestText]) -> Result<IngestStats, IngestError> {
        let mut stats = IngestStats::default();
        let wtx = self
            .dedup
            .begin_write()
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        {
            let mut table = wtx
                .open_table(DEDUP_TABLE)
                .map_err(|e| IngestError::Dedup(e.to_string()))?;
            for doc in docs {
                let text = doc.text.trim();
                if text.is_empty() {
                    continue;
                }
                // SPEC §9.3: dedup key = blake3(content) truncated to 16 bytes.
                let digest = blake3::hash(text.as_bytes());
                let key = &digest.as_bytes()[..16];
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

                self.index
                    .add(&self.build_doc(doc, text, ts))
                    .map_err(|e| IngestError::Index(e.to_string()))?;
                stats.accepted += 1;
            }
        }
        wtx.commit()
            .map_err(|e| IngestError::Dedup(e.to_string()))?;
        self.index
            .commit()
            .map_err(|e| IngestError::Index(e.to_string()))?;
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

        IndexDoc {
            url,
            title,
            snippet,
            body: text.to_owned(),
            ts,
            domain_hash: domain_hash(&domain),
            lang,
            h3_r7: 0,
            quality: 0.0,
        }
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
        let lanes = Arc::new(LaneRegistry::new(&LanesConfig::default()).unwrap());
        let fetcher = Arc::new(Fetcher::new(lanes, &FetchConfig::default(), false));
        let ingestor = Ingestor::new(
            index,
            fetcher,
            &dir,
            &meridian_common::config::IngestConfig::default(),
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
        let stats2 = ingestor.ingest_batch(&[doc]).unwrap();
        assert_eq!(stats2.accepted, 0);
        assert_eq!(stats2.deduped, 1);

        let hits = ingestor.index.search("meridian metre", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].snippet.contains("meridian arc"),
            "whitespace-normalized snippet"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
