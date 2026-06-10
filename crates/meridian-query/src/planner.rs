//! Query planner v1 (SPEC §3 fast-mode pipeline, lexical slice): resolve lane →
//! local BM25 (rayon-bridged) ∥ SearXNG fan-out (deadline + direct-only hedging)
//! → RRF(k=60) → domain-diversity cap → results with rank_signals + timings.
//! ANN joins the fusion in Phase 2; LTR/rerank in Phase 3.

use crate::rrf::{RRF_K, rrf_fuse};
use crate::{Scope, SearchMode};
use meridian_common::config::{SearchConfig, VectorConfig};
use meridian_common::shed::ShedState;
use meridian_egress::{Lane, LaneRegistry};
use meridian_embed::Embedder;
use meridian_index::lexical::{LexicalIndex, url_key};
use meridian_searx::client::SearxClient;
use meridian_vector::VectorStore;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("lane unavailable: {0}")]
    Lane(#[from] meridian_egress::EgressError),
    #[error("index: {0}")]
    Index(String),
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub q: String,
    pub mode: SearchMode,
    pub scope: Scope,
    pub lane: Lane,
    pub limit: usize,
}

/// Explainability block (SPEC §10): every surfaced signal, always populated.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RankSignals {
    pub rrf: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ann: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub searx_rank: Option<usize>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
    pub rank_signals: RankSignals,
    /// "local" | "web" | "both"
    pub source: &'static str,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub timings: HashMap<&'static str, u64>,
    pub lane_requested: String,
    pub lane_effective: String,
    pub degraded: Vec<&'static str>,
}

/// Cached fusion output (SPEC §8.4 query cache: 256MB weighted, TTI 15m, TTL 2h;
/// direct-lane results only — the anon cache is separate and arrives in Phase 4).
#[derive(Clone)]
struct CachedSearch {
    results: Vec<SearchResult>,
    degraded: Vec<&'static str>,
}

pub struct Planner {
    index: Arc<LexicalIndex>,
    embedder: Arc<Embedder>,
    vectors: Arc<VectorStore>,
    searx: Option<Arc<SearxClient>>,
    lanes: Arc<LaneRegistry>,
    shed: Arc<ShedState>,
    cache: moka::sync::Cache<[u8; 32], Arc<CachedSearch>>,
    cfg: SearchConfig,
    vector_cfg: VectorConfig,
}

impl Planner {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index: Arc<LexicalIndex>,
        embedder: Arc<Embedder>,
        vectors: Arc<VectorStore>,
        searx: Option<Arc<SearxClient>>,
        lanes: Arc<LaneRegistry>,
        shed: Arc<ShedState>,
        cfg: &SearchConfig,
        vector_cfg: &VectorConfig,
    ) -> Self {
        let cache = moka::sync::Cache::builder()
            .max_capacity(256 * 1024 * 1024)
            .weigher(|_k, v: &Arc<CachedSearch>| {
                let bytes: usize = v
                    .results
                    .iter()
                    .map(|r| r.url.len() + r.title.len() + r.snippet.len() + 64)
                    .sum();
                (bytes + 64) as u32
            })
            .time_to_idle(std::time::Duration::from_secs(15 * 60))
            .time_to_live(std::time::Duration::from_secs(2 * 60 * 60))
            .build();
        Self {
            index,
            embedder,
            vectors,
            searx,
            lanes,
            shed,
            cache,
            cfg: cfg.clone(),
            vector_cfg: vector_cfg.clone(),
        }
    }

    fn cache_key(req: &SearchRequest, limit: usize) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(req.q.as_bytes());
        hasher.update(&[req.scope as u8, req.mode as u8]);
        hasher.update(&limit.to_le_bytes());
        *hasher.finalize().as_bytes()
    }

    /// Lane observability for `GET /v1/lanes` (SPEC §10).
    pub fn lane_statuses(&self) -> Vec<(String, meridian_egress::LaneStatus)> {
        self.lanes.statuses()
    }

    /// Aggregate per-engine counters for `/metrics` (no user data).
    pub fn engine_health(
        &self,
    ) -> std::collections::HashMap<String, meridian_searx::client::EngineHealth> {
        self.searx
            .as_ref()
            .map(|s| s.engine_health())
            .unwrap_or_default()
    }

    pub async fn search(&self, req: SearchRequest) -> Result<SearchResponse, PlanError> {
        let mut timings: HashMap<&'static str, u64> = HashMap::new();
        let mut degraded: Vec<&'static str> = Vec::new();
        let limit = req.limit.clamp(1, self.cfg.max_limit);

        // Query cache (direct lane only; anon results must never share state —
        // SPEC §8.4/§12.4). Shedding stage 2 drops + bypasses it.
        let cacheable = matches!(req.lane, Lane::Direct);
        let key = Self::cache_key(&req, limit);
        if self
            .shed
            .caches_dropped
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.cache.invalidate_all();
        } else if cacheable {
            if let Some(hit) = self.cache.get(&key) {
                timings.insert("cache", 1);
                return Ok(SearchResponse {
                    results: hit.results.clone(),
                    timings,
                    lane_requested: lane_name(&req.lane),
                    lane_effective: lane_name(&req.lane),
                    degraded: hit.degraded.clone(),
                });
            }
        }

        // Lane resolution is fail-closed: whenever the scope would touch the
        // network, the requested lane must resolve — even if the metasearch
        // backend is off — so a requested anon/region NEVER quietly yields a
        // result it didn't govern (SPEC §12.1). Local-only scope needs no
        // egress at all; lanes govern network only.
        let wants_web_scope = matches!(req.scope, Scope::Web | Scope::Both);
        let lane_client = if wants_web_scope {
            if self.searx.is_none() {
                degraded.push("searx_disabled");
            }
            Some(self.lanes.resolve(&req.lane)?)
        } else {
            None
        };
        let wants_web = wants_web_scope && self.searx.is_some();

        // Metasearch thermal shed (SPEC §8.6: >82°C → local-only).
        let metasearch_ok = self.shed.metasearch_allowed();
        if wants_web_scope && !metasearch_ok {
            degraded.push("metasearch_shed");
        }

        // Local hybrid stages on the rayon pool (SPEC §7.2: no CPU on tokio):
        // BM25 top-1000 ∥-ish embed→ANN top-200 → resolve ANN-only docs.
        let wants_local = matches!(req.scope, Scope::Local | Scope::Both)
            // Web-only scope with the backend off: local is the honest fallback,
            // flagged via `degraded` above.
            || (wants_web_scope && self.searx.is_none());
        let local_handle = wants_local.then(|| {
            let index = self.index.clone();
            let embedder = self.embedder.clone();
            let vectors = self.vectors.clone();
            let q = req.q.clone();
            let top_k = self.cfg.bm25_top_k;
            let ann_k = self.vector_cfg.top_k;
            let (tx, rx) = tokio::sync::oneshot::channel();
            rayon::spawn(move || {
                let started = Instant::now();
                let bm25 = index.search(&q, top_k);
                let bm25_ms = started.elapsed().as_millis() as u64;

                let ann_started = Instant::now();
                let ann = if vectors.is_empty() {
                    Ok(Vec::new())
                } else {
                    let query_vec = embedder.embed_query(&q);
                    vectors.search(&query_vec, ann_k)
                };
                // Resolve ANN keys the lexical list doesn't already carry.
                let ann_docs = match (&bm25, &ann) {
                    (Ok(hits), Ok(pairs)) => {
                        let have: std::collections::HashSet<u64> =
                            hits.iter().map(|h| h.url_key).collect();
                        let missing: Vec<u64> = pairs
                            .iter()
                            .map(|(k, _)| *k)
                            .filter(|k| !have.contains(k))
                            .collect();
                        index.docs_by_keys(&missing)
                    }
                    _ => Ok(Vec::new()),
                };
                let ann_ms = ann_started.elapsed().as_millis() as u64;
                let _ = tx.send((bm25, ann, ann_docs, bm25_ms, ann_ms));
            });
            rx
        });

        // SearXNG fan-out, concurrent with the local stage.
        let searx_handle = match (&self.searx, lane_client) {
            (Some(searx), Some(client)) if wants_web && metasearch_ok => {
                let searx = searx.clone();
                let q = req.q.clone();
                let hedge = matches!(req.lane, Lane::Direct);
                Some(tokio::spawn(async move {
                    let started = Instant::now();
                    let r = searx.search(&client, &q, hedge).await;
                    (r, started.elapsed().as_millis() as u64)
                }))
            }
            _ => None,
        };

        // Collect local (lexical + dense).
        let mut local_hits = Vec::new();
        let mut ann_pairs: Vec<(u64, f32)> = Vec::new();
        let mut ann_docs = Vec::new();
        if let Some(rx) = local_handle {
            let (bm25, ann, ann_doc_result, bm25_ms, ann_ms) = rx
                .await
                .map_err(|_| PlanError::Internal("local stage dropped".into()))?;
            timings.insert("bm25_ms", bm25_ms);
            timings.insert("ann_ms", ann_ms);
            local_hits = bm25.map_err(|e| PlanError::Index(e.to_string()))?;
            match ann {
                Ok(pairs) => ann_pairs = pairs,
                Err(_) => degraded.push("ann_unavailable"),
            }
            ann_docs = ann_doc_result.map_err(|e| PlanError::Index(e.to_string()))?;
        }

        // Collect web (partial results are fine — degrade, don't fail).
        let mut web_hits = Vec::new();
        if let Some(handle) = searx_handle {
            match handle.await {
                Ok((Ok(results), ms)) => {
                    timings.insert("searx_ms", ms);
                    web_hits = results;
                }
                Ok((Err(_), ms)) => {
                    timings.insert("searx_ms", ms);
                    degraded.push("searx_unavailable");
                }
                Err(_) => degraded.push("searx_unavailable"),
            }
        }

        // RRF over {bm25, per-engine searx lists} keyed by URL hash (SPEC §11).
        let fuse_start = Instant::now();
        let mut by_key: HashMap<u64, SearchResult> = HashMap::new();
        let mut lists: Vec<Vec<u64>> = Vec::new();

        let mut bm25_list = Vec::with_capacity(local_hits.len());
        for hit in &local_hits {
            bm25_list.push(hit.url_key);
            by_key.entry(hit.url_key).or_insert_with(|| SearchResult {
                url: hit.url.clone(),
                title: hit.title.clone(),
                snippet: hit.snippet.clone(),
                score: 0.0,
                rank_signals: RankSignals {
                    rrf: 0.0,
                    bm25: Some(hit.bm25),
                    ann: None,
                    searx_rank: None,
                },
                source: "local",
            });
        }
        if !bm25_list.is_empty() {
            lists.push(bm25_list);
        }

        // Dense list (SPEC §11: ANN top-200 joins the fusion). ANN-only docs
        // were resolved to stored fields inside the rayon stage.
        let ann_doc_by_key: HashMap<u64, _> =
            ann_docs.into_iter().map(|d| (d.url_key, d)).collect();
        let mut ann_list = Vec::with_capacity(ann_pairs.len());
        for (key, sim) in &ann_pairs {
            match by_key.entry(*key) {
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    ann_list.push(*key);
                    e.get_mut().rank_signals.ann = Some(*sim);
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    if let Some(d) = ann_doc_by_key.get(key) {
                        ann_list.push(*key);
                        e.insert(SearchResult {
                            url: d.url.clone(),
                            title: d.title.clone(),
                            snippet: d.snippet.clone(),
                            score: 0.0,
                            rank_signals: RankSignals {
                                rrf: 0.0,
                                bm25: None,
                                ann: Some(*sim),
                                searx_rank: None,
                            },
                            source: "local",
                        });
                    }
                    // Key with no resolvable doc (vector for a deleted/lost doc)
                    // is silently skipped — recall degraded, not correctness.
                }
            }
        }
        if !ann_list.is_empty() {
            lists.push(ann_list);
        }

        // One RRF list per engine (SPEC §11: "each searx engine list").
        let mut engine_lists: HashMap<String, Vec<u64>> = HashMap::new();
        for hit in &web_hits {
            let key = url_key(&hit.url);
            engine_lists
                .entry(hit.engine.clone())
                .or_default()
                .push(key);
            match by_key.entry(key) {
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    let r = e.get_mut();
                    r.source = "both";
                    if r.rank_signals.searx_rank.is_none_or(|prev| hit.rank < prev) {
                        r.rank_signals.searx_rank = Some(hit.rank);
                    }
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(SearchResult {
                        url: hit.url.clone(),
                        title: hit.title.clone(),
                        snippet: hit.snippet.clone(),
                        score: 0.0,
                        rank_signals: RankSignals {
                            rrf: 0.0,
                            bm25: None,
                            ann: None,
                            searx_rank: Some(hit.rank),
                        },
                        source: "web",
                    });
                }
            }
        }
        lists.extend(engine_lists.into_values());

        let fused = rrf_fuse(&lists, RRF_K);
        timings.insert("fuse_ms", fuse_start.elapsed().as_millis() as u64);

        // Domain-diversity cap (SPEC §11: ≤3 per domain), then cut to limit.
        let mut per_domain: HashMap<String, usize> = HashMap::new();
        let mut results = Vec::with_capacity(limit);
        for (key, rrf_score) in fused {
            let Some(mut r) = by_key.remove(&key) else {
                continue;
            };
            let domain = url::Url::parse(&r.url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_default();
            let count = per_domain.entry(domain).or_insert(0);
            if *count >= self.cfg.max_per_domain {
                continue;
            }
            *count += 1;
            r.score = rrf_score;
            r.rank_signals.rrf = rrf_score;
            results.push(r);
            if results.len() >= limit {
                break;
            }
        }

        if cacheable
            && !self
                .shed
                .caches_dropped
                .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.cache.insert(
                key,
                Arc::new(CachedSearch {
                    results: results.clone(),
                    degraded: degraded.clone(),
                }),
            );
        }

        Ok(SearchResponse {
            results,
            timings,
            lane_requested: lane_name(&req.lane),
            // Effective = the lane that actually carried traffic; a local-only
            // request used none.
            lane_effective: if wants_web_scope {
                lane_name(&req.lane)
            } else {
                "local-only".to_owned()
            },
            degraded,
        })
    }
}

fn lane_name(lane: &Lane) -> String {
    match lane {
        Lane::Direct => "direct".to_owned(),
        Lane::Anon => "anon".to_owned(),
        Lane::Region(id) => format!("region:{}", id.0),
    }
}
