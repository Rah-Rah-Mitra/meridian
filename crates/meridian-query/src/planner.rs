//! Query planner v1 (SPEC §3 fast-mode pipeline, lexical slice): resolve lane →
//! local BM25 (rayon-bridged) ∥ SearXNG fan-out (deadline + direct-only hedging)
//! → RRF(k=60) → domain-diversity cap → results with rank_signals + timings.
//! ANN joins the fusion in Phase 2; LTR/rerank in Phase 3.

use crate::intent;
use crate::rrf::{RRF_K, rrf_fuse};
use crate::{Scope, SearchMode};
use meridian_common::config::{SearchConfig, VectorConfig};
use meridian_common::shed::ShedState;
use meridian_egress::{Lane, LaneRegistry};
use meridian_embed::Embedder;
use meridian_index::lexical::{GeoCells, LexicalIndex, SearchFilter, url_key};
use meridian_rank::{Features, LinearLtr, Scorer, title_match_ratio};
use meridian_rerank::{Pair, Reranker};
use meridian_searx::client::SearxClient;
use meridian_vector::VectorStore;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("lane unavailable: {0}")]
    Lane(#[from] meridian_egress::EgressError),
    /// The anon search admission budget (SPEC §12.4 citizenship) is exhausted.
    #[error("anon lane is at its concurrent-search budget")]
    AnonBusy,
    /// Invalid geo constraint (bad coords, oversized radius, malformed cell).
    #[error("invalid geo constraint: {0}")]
    BadGeo(String),
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
    /// Geo constraint (SPEC §10): point+radius or an explicit H3 cell.
    pub geo: Option<GeoConstraint>,
    /// `ts` window (unix seconds, inclusive).
    pub after: Option<u64>,
    pub before: Option<u64>,
    /// Skip query-cache read AND write (Phase 8: both compare halves bypass —
    /// a stale cached half would compare different points in time, and the
    /// combined response must never be cached).
    pub bypass_cache: bool,
    /// Skip bandit engine selection (instance-default engines, no reward).
    /// Phase 8: BOTH compare halves pin engines — the suite-12 probe measured
    /// the bandit's arm churn at p90 JSD 0.67 within the direct lane alone,
    /// which would drown any real vantage signal; pinning makes the two
    /// halves differ by vantage only.
    pub pin_engines: bool,
}

#[derive(Debug, Clone)]
pub enum GeoConstraint {
    Point { lat: f64, lon: f64, radius_km: f64 },
    Cell(u64),
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
    /// LTR re-score (SPEC §11). Always set after the rank stage.
    pub ltr: f32,
    /// Cross-encoder score (SPEC §11), only on `mode=deep` reranked results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ce: Option<f32>,
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
    /// H3 res-7 cell of geo-tagged local docs (SPEC §10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h3: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<u64>,
    /// Derivation-cluster annotation (Phase 7, ADR-18); `null` when the doc has
    /// no sketch (web result never ingested) — never guessed from a snippet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<crate::evidence::ResultEvidence>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub timings: HashMap<&'static str, u64>,
    pub lane_requested: String,
    pub lane_effective: String,
    pub degraded: Vec<&'static str>,
    /// Source-independence block (Phase 7, ADR-18/20); absent = feature off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<crate::evidence::EvidenceBlock>,
    /// Vantage-divergence block (Phase 8, ADR-22); only on compare=vantages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergence: Option<crate::compare::DivergenceBlock>,
}

/// Cached fusion output (SPEC §8.4 query cache: 128MB weighted, TTI 15m, TTL 2h
/// for the shared direct-lane cache; the anon cache is a separate ephemeral
/// 16MB/TTL-5m instance — anon results never touch shared state, SPEC §12.4).
#[derive(Clone)]
struct CachedSearch {
    results: Vec<SearchResult>,
    degraded: Vec<&'static str>,
    evidence: Option<crate::evidence::EvidenceBlock>,
}

/// Weighted cost of a cached entry. Counts every owned allocation (strings,
/// per-result struct, vec headers, moka entry bookkeeping) — the Phase-6 soak
/// proved the original snippet-bytes-only estimate off by an order of
/// magnitude once allocator overhead is included, which let the "256MB" cache
/// grow the process past every shed rung. The ×2 factor is the measured
/// mimalloc fragmentation allowance; budgets treat the cap as REAL bytes.
fn cached_search_weight(v: &Arc<CachedSearch>) -> u32 {
    let owned: usize = v
        .results
        .iter()
        .map(|r| {
            r.url.capacity()
                + r.title.capacity()
                + r.snippet.capacity()
                + std::mem::size_of::<SearchResult>()
        })
        .sum();
    let fixed = std::mem::size_of::<CachedSearch>()
        + v.degraded.capacity() * std::mem::size_of::<&'static str>()
        + v.evidence
            .as_ref()
            .map(|e| 64 + e.clusters.len() * 16)
            .unwrap_or(0)
        + 256; // moka entry + key + Arc bookkeeping
    ((owned + fixed) * 2) as u32
}

pub struct Planner {
    index: Arc<LexicalIndex>,
    embedder: Arc<Embedder>,
    vectors: Arc<VectorStore>,
    searx: Option<Arc<SearxClient>>,
    /// Tor-proxied `searxng-anon` backend; anon searches fail closed without it.
    searx_anon: Option<Arc<SearxClient>>,
    lanes: Arc<LaneRegistry>,
    shed: Arc<ShedState>,
    scorer: Arc<dyn Scorer>,
    reranker: Arc<Reranker>,
    bandit: Option<Arc<meridian_searx::bandit::Bandit>>,
    cache: moka::sync::Cache<[u8; 32], Arc<CachedSearch>>,
    /// Ephemeral anon cache (SPEC §8.4): in-memory only, never persisted, never
    /// shared with the direct cache (lane isolation, §12.4).
    anon_cache: moka::sync::Cache<[u8; 32], Arc<CachedSearch>>,
    /// Anon concurrent-search admission (SPEC §12.4 rate-limit citizenship).
    anon_permits: Arc<tokio::sync::Semaphore>,
    /// LTR domain_prior source (analytics PageRank; NoPrior until first run).
    priors: Arc<dyn meridian_common::prior::DomainPriorSource>,
    /// Sketch read handle for the evidence layer (Phase 7, ADR-18).
    /// `None` = evidence off (config kill-switch or no ingestor wired).
    sketches: Option<crate::ingest::SketchReader>,
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
        searx_anon: Option<Arc<SearxClient>>,
        reranker: Arc<Reranker>,
        bandit: Option<Arc<meridian_searx::bandit::Bandit>>,
        lanes: Arc<LaneRegistry>,
        shed: Arc<ShedState>,
        anon_max_searches: usize,
        priors: Arc<dyn meridian_common::prior::DomainPriorSource>,
        sketches: Option<crate::ingest::SketchReader>,
        cfg: &SearchConfig,
        vector_cfg: &VectorConfig,
    ) -> Self {
        let cache = moka::sync::Cache::builder()
            .max_capacity(128 * 1024 * 1024)
            .weigher(|_k, v: &Arc<CachedSearch>| cached_search_weight(v))
            .time_to_idle(std::time::Duration::from_secs(15 * 60))
            .time_to_live(std::time::Duration::from_secs(2 * 60 * 60))
            .build();
        let anon_cache = moka::sync::Cache::builder()
            .max_capacity(16 * 1024 * 1024)
            .weigher(|_k, v: &Arc<CachedSearch>| cached_search_weight(v))
            .time_to_live(std::time::Duration::from_secs(5 * 60))
            .build();
        Self {
            index,
            embedder,
            vectors,
            searx,
            searx_anon,
            lanes,
            shed,
            scorer: Arc::new(LinearLtr::default()),
            reranker,
            bandit,
            cache,
            anon_cache,
            anon_permits: Arc::new(tokio::sync::Semaphore::new(anon_max_searches.max(1))),
            priors,
            sketches,
            cfg: cfg.clone(),
            vector_cfg: vector_cfg.clone(),
        }
    }

    fn cache_key(req: &SearchRequest, limit: usize) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(req.q.as_bytes());
        hasher.update(&[req.scope as u8, req.mode as u8]);
        hasher.update(&limit.to_le_bytes());
        // Geo + time constraints MUST key the cache — a filtered result set
        // cached under the unfiltered key would poison every later query.
        match &req.geo {
            Some(GeoConstraint::Point {
                lat,
                lon,
                radius_km,
            }) => {
                hasher.update(&[1u8]);
                hasher.update(&lat.to_le_bytes());
                hasher.update(&lon.to_le_bytes());
                hasher.update(&radius_km.to_le_bytes());
            }
            Some(GeoConstraint::Cell(cell)) => {
                hasher.update(&[2u8]);
                hasher.update(&cell.to_le_bytes());
            }
            None => {
                hasher.update(&[0u8]);
            }
        }
        hasher.update(&req.after.unwrap_or(0).to_le_bytes());
        hasher.update(&req.before.unwrap_or(u64::MAX).to_le_bytes());
        *hasher.finalize().as_bytes()
    }

    /// Resolve the request's geo constraint to an index filter. Invalid
    /// coordinates/radii are a caller error surfaced as 400 by the API.
    fn build_filter(req: &SearchRequest) -> Result<(SearchFilter, Option<(f64, f64)>), PlanError> {
        let mut origin = None;
        let geo = match &req.geo {
            None => None,
            Some(GeoConstraint::Point {
                lat,
                lon,
                radius_km,
            }) => {
                origin = Some((*lat, *lon));
                let (res, cells) = meridian_geo::h3::filter_cells(*lat, *lon, *radius_km)
                    .map_err(|e| PlanError::BadGeo(e.to_string()))?;
                Some(match res {
                    meridian_geo::FilterRes::R5 => GeoCells::R5(cells),
                    meridian_geo::FilterRes::R7 => GeoCells::R7(cells),
                })
            }
            Some(GeoConstraint::Cell(cell)) => {
                origin = meridian_geo::h3::cell_to_latlng(*cell);
                let (res, cells) = meridian_geo::h3::cells_for_cell(*cell)
                    .map_err(|e| PlanError::BadGeo(e.to_string()))?;
                Some(match res {
                    meridian_geo::FilterRes::R5 => GeoCells::R5(cells),
                    meridian_geo::FilterRes::R7 => GeoCells::R7(cells),
                })
            }
        };
        Ok((
            SearchFilter {
                geo,
                after_ts: req.after,
                before_ts: req.before,
            },
            origin,
        ))
    }

    /// Lane observability for `GET /v1/lanes` (SPEC §10).
    pub fn lane_statuses(&self) -> Vec<(String, meridian_egress::LaneStatus)> {
        self.lanes.statuses()
    }

    /// `/v1/geo/heatmap` (SPEC §10): res-3..7 rollup of geo-tagged docs,
    /// optional q + window. Runs on the rayon pool (fast-field scan = CPU).
    pub async fn heatmap(
        &self,
        q: Option<String>,
        res: u8,
        after_ts: Option<u64>,
    ) -> Result<Vec<(u64, u32)>, PlanError> {
        if !(3..=7).contains(&res) {
            return Err(PlanError::BadGeo("res must be 3..=7".into()));
        }
        let index = self.index.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        rayon::spawn(move || {
            let _ = tx.send(index.heatmap(q.as_deref(), after_ts, res));
        });
        rx.await
            .map_err(|_| PlanError::Internal("heatmap stage dropped".into()))?
            .map_err(|e| PlanError::Index(e.to_string()))
    }

    /// `/v1/forget purge_caches=true`: drop BOTH query caches — cached SERPs
    /// may embed the forgotten document's snippet (SPEC §10).
    pub fn purge_caches(&self) {
        self.cache.invalidate_all();
        self.anon_cache.invalidate_all();
    }

    /// Honest cache occupancy for `/metrics` (Phase-6 finding: weighted-cap
    /// caches need entry/byte gauges or RSS growth is undiagnosable).
    /// `run_pending_tasks` first — moka's counters lag eviction otherwise.
    pub fn cache_stats(&self) -> [(&'static str, u64, u64); 2] {
        self.cache.run_pending_tasks();
        self.anon_cache.run_pending_tasks();
        [
            (
                "query",
                self.cache.entry_count(),
                self.cache.weighted_size(),
            ),
            (
                "query_anon",
                self.anon_cache.entry_count(),
                self.anon_cache.weighted_size(),
            ),
        ]
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
        // Intent: µs heuristic; keys the bandit (Phase-3) and is surfaced in the
        // response for explainability. Never logged with the query text.
        let query_intent = intent::classify(&req.q);

        // Query caches (SPEC §8.4): the shared one serves direct only; anon has
        // its own ephemeral instance (lane isolation, §12.4); region results are
        // vantage-dependent and never cached. Shedding stage 2 drops + bypasses.
        let cacheable = matches!(req.lane, Lane::Direct) && !req.bypass_cache;
        let anon_cacheable = matches!(req.lane, Lane::Anon) && !req.bypass_cache;
        let key = Self::cache_key(&req, limit);
        if self
            .shed
            .caches_dropped
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.cache.invalidate_all();
            self.anon_cache.invalidate_all();
        } else if cacheable || anon_cacheable {
            let hit = if cacheable {
                self.cache.get(&key)
            } else {
                self.anon_cache.get(&key)
            };
            if let Some(hit) = hit {
                timings.insert("cache", 1);
                return Ok(SearchResponse {
                    results: hit.results.clone(),
                    timings,
                    lane_requested: lane_name(&req.lane),
                    lane_effective: lane_name(&req.lane),
                    degraded: hit.degraded.clone(),
                    evidence: hit.evidence.clone(),
                    divergence: None,
                });
            }
        }

        let wants_web_scope = matches!(req.scope, Scope::Web | Scope::Both);

        // Anon admission budget (SPEC §12.4): bounded concurrent anon searches,
        // held for the whole request. Cache hits above don't consume it.
        let _anon_permit = if wants_web_scope && matches!(req.lane, Lane::Anon) {
            match Arc::clone(&self.anon_permits).try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => return Err(PlanError::AnonBusy),
            }
        } else {
            None
        };

        // Metasearch backend is lane-dependent: direct uses the `searxng`
        // sidecar; anon uses the Tor-proxied `searxng-anon` and fails CLOSED
        // when it isn't configured (no quiet local-only serving under an anon
        // label); region lanes have no metasearch backend at all — routing
        // their fan-out through the direct sidecar would break §12.5 inv. 1.
        let backend = match &req.lane {
            Lane::Anon => &self.searx_anon,
            _ => &self.searx,
        };
        if wants_web_scope {
            match &req.lane {
                Lane::Region(_) => {
                    return Err(PlanError::Lane(meridian_egress::EgressError::NotReady(
                        "region lanes serve fetch/ingest only; metasearch is direct or anon".into(),
                    )));
                }
                Lane::Anon if backend.is_none() => {
                    return Err(PlanError::Lane(meridian_egress::EgressError::NotReady(
                        "anon metasearch backend (searx.anon_url) is not configured".into(),
                    )));
                }
                _ => {}
            }
        }

        // Lane resolution is fail-closed: whenever the scope would touch the
        // network, the requested lane must resolve — even if the metasearch
        // backend is off — so a requested anon NEVER quietly yields a result it
        // didn't govern (SPEC §12.1). For anon this is also the Arti readiness
        // gate (bootstrapping/down ⇒ error, never direct). Local-only scope
        // needs no egress at all; lanes govern network only.
        let lane_client = if wants_web_scope {
            if backend.is_none() {
                degraded.push("searx_disabled");
            }
            Some(self.lanes.resolve(&req.lane)?)
        } else {
            None
        };
        let wants_web = wants_web_scope && backend.is_some();

        // Metasearch thermal shed (SPEC §8.6: >82°C → local-only).
        let metasearch_ok = self.shed.metasearch_allowed();
        if wants_web_scope && !metasearch_ok {
            degraded.push("metasearch_shed");
        }

        // Geo + ts prefilter (SPEC §11: applied BEFORE scoring).
        let (search_filter, geo_origin) = Self::build_filter(&req)?;

        // Geo/ts filters constrain LOCAL documents only — the metasearch
        // fan-out can't be geo-filtered (ADR-10). Pure web scope would make
        // the filter a silent no-op: refuse as a caller error. Mixed scope
        // serves filtered local + unfiltered web, flagged honestly.
        let has_filter = search_filter.geo.is_some()
            || search_filter.after_ts.is_some()
            || search_filter.before_ts.is_some();
        if has_filter && wants_web_scope {
            if !matches!(req.scope, Scope::Both) {
                return Err(PlanError::BadGeo(
                    "geo/time filters apply to local documents only; use scope=local or scope=both"
                        .into(),
                ));
            }
            degraded.push("geo_web_unfiltered");
        }

        // Local hybrid stages on the rayon pool (SPEC §7.2: no CPU on tokio):
        // BM25 top-1000 ∥-ish embed→ANN top-200 → resolve ANN-only docs.
        let wants_local = matches!(req.scope, Scope::Local | Scope::Both)
            // Web-only scope with the backend off: local is the honest fallback,
            // flagged via `degraded` above (direct lane only — anon without a
            // backend already failed closed).
            || (wants_web_scope && backend.is_none());
        let local_handle = wants_local.then(|| {
            let index = self.index.clone();
            let embedder = self.embedder.clone();
            let vectors = self.vectors.clone();
            let q = req.q.clone();
            let top_k = self.cfg.bm25_top_k;
            let ann_k = self.vector_cfg.top_k;
            let filter = search_filter.clone();
            let filtered =
                filter.geo.is_some() || filter.after_ts.is_some() || filter.before_ts.is_some();
            let (tx, rx) = tokio::sync::oneshot::channel();
            rayon::spawn(move || {
                let started = Instant::now();
                let bm25 = index.search_filtered(&q, top_k, &filter);
                let bm25_ms = started.elapsed().as_millis() as u64;

                let ann_started = Instant::now();
                // Dense ANN has no filter support (usearch); under a geo/ts
                // constraint the dense list would smuggle out-of-area docs into
                // the fusion, so the constrained path is lexical-only (exact).
                let ann = if vectors.is_empty() || filtered {
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

        // SearXNG fan-out, concurrent with the local stage. On the direct lane
        // the bandit picks the engine subset for this intent (ε-greedy) and is
        // rewarded below if its web results reach the final top-10 (SPEC §11).
        // The anon lane is firewalled from the bandit BOTH ways (SPEC §12.4):
        // it neither reads shared routing state (engine choice = instance
        // defaults) nor writes rewards (`chosen_arm` stays None).
        let is_direct = matches!(req.lane, Lane::Direct);
        let mut chosen_arm: Option<&'static str> = None;
        let searx_handle = match (backend, lane_client) {
            (Some(searx), Some(client)) if wants_web && metasearch_ok => {
                // Per-request exploration salt without a global RNG.
                let salt = blake3::hash(req.q.as_bytes()).as_bytes()[0] as u64
                    ^ (timings.len() as u64)
                    ^ (req.q.len() as u64).wrapping_mul(0x9E37);
                let engines: Vec<String> = match (&self.bandit, is_direct && !req.pin_engines) {
                    (Some(b), true) => {
                        let arm = b.choose(query_intent.key(), salt);
                        chosen_arm = Some(arm.id);
                        arm.engines.iter().map(|e| e.to_string()).collect()
                    }
                    _ => Vec::new(),
                };
                // The anon hop to `searxng-anon` rides the internal back-network
                // pool — actual EGRESS happens at the sidecar, through this
                // process's Tor SOCKS listener (fail-closed by topology). The
                // resolved `client` above already proved the anon lane is ready.
                let client = match &req.lane {
                    Lane::Anon => self.lanes.sidecar(Lane::Anon),
                    _ => client,
                };
                let searx = searx.clone();
                let q = req.q.clone();
                let hedge = is_direct;
                Some(tokio::spawn(async move {
                    let started = Instant::now();
                    let engine_refs: Vec<&str> = engines.iter().map(String::as_str).collect();
                    let r = searx.search(&client, &q, &engine_refs, hedge).await;
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
                    ltr: 0.0,
                    ce: None,
                },
                source: "local",
                h3: (hit.h3_r7 != 0).then_some(hit.h3_r7),
                ts: (hit.ts != 0).then_some(hit.ts),
                evidence: None,
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
                                ltr: 0.0,
                                ce: None,
                            },
                            source: "local",
                            h3: (d.h3_r7 != 0).then_some(d.h3_r7),
                            ts: (d.ts != 0).then_some(d.ts),
                            evidence: None,
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
                            ltr: 0.0,
                            ce: None,
                        },
                        source: "web",
                        h3: None,
                        ts: None,
                        evidence: None,
                    });
                }
            }
        }
        lists.extend(engine_lists.into_values());

        let fused = rrf_fuse(&lists, RRF_K);
        timings.insert("fuse_ms", fuse_start.elapsed().as_millis() as u64);

        // LTR re-score the top-100 fused candidates (SPEC §11). Cold-start linear
        // model (ADR-02); RRF-dominant so order is preserved unless a secondary
        // signal is decisive. `mode=deep` adds the CE rerank stage AFTER this.
        let rank_start = Instant::now();
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut scored: Vec<(u64, f32, f32)> = Vec::new(); // (key, rrf, ltr)
        for (key, rrf_score) in fused.iter().take(100) {
            let Some(r) = by_key.get(key) else { continue };
            let sig = &r.rank_signals;
            let source_count = sig.bm25.is_some() as u8 as f32
                + sig.ann.is_some() as u8 as f32
                + sig.searx_rank.is_some() as u8 as f32;
            // Freshness: exp-decay over doc age (SPEC §11), neutral 0.5 when
            // the doc has no timestamp. ~30-day half-life.
            let freshness = match r.ts {
                Some(doc_ts) if doc_ts > 0 => {
                    let age_days = (now_unix.saturating_sub(doc_ts)) as f32 / 86_400.0;
                    (-age_days / 43.0).exp().clamp(0.0, 1.0)
                }
                _ => 0.5,
            };
            // Geo closeness in [0,1] when both a query origin and a doc cell
            // exist (1/(1+km)); 0 otherwise. The cold-start LTR weighs it 0 —
            // geo influence comes from the FILTER; this feeds the future GBDT.
            let geo = match (geo_origin, r.h3) {
                (Some((lat, lon)), Some(cell)) => meridian_geo::h3::distance_km(cell, lat, lon)
                    .map(|d| 1.0 / (1.0 + d as f32))
                    .unwrap_or(0.0),
                _ => 0.0,
            };
            let features = Features {
                rrf: *rrf_score,
                bm25: sig.bm25.unwrap_or(0.0),
                ann: sig.ann.unwrap_or(0.0),
                title_match_ratio: title_match_ratio(&req.q, &r.title),
                source_count,
                snippet_len_norm: (r.snippet.len() as f32 / 240.0).min(1.0),
                freshness,
                domain_prior: self.priors.domain_prior(result_domain_hash(r)),
                geo,
            };
            scored.push((*key, *rrf_score, self.scorer.score(&features)));
        }
        scored.sort_unstable_by(|a, b| b.2.partial_cmp(&a.2).unwrap().then(a.0.cmp(&b.0)));
        timings.insert("ltr_ms", rank_start.elapsed().as_millis() as u64);

        // Domain-diversity cap (SPEC §11: ≤3 per domain), then cut to limit.
        let mut per_domain: HashMap<String, usize> = HashMap::new();
        let mut results = Vec::with_capacity(limit);
        for (key, rrf_score, ltr_score) in scored {
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
            r.score = ltr_score;
            r.rank_signals.rrf = rrf_score;
            r.rank_signals.ltr = ltr_score;
            results.push(r);
            if results.len() >= limit {
                break;
            }
        }

        // Deep-mode rerank (SPEC §11): cross-encoder over the top-20, 1.5s stage
        // deadline, Moka-cached. Degrades (never fails the query) when the model
        // isn't compiled in (musl image) or RSS shedding disabled it.
        if matches!(req.mode, SearchMode::Deep) {
            let rss_shed = self
                .shed
                .rerank_disabled
                .load(std::sync::atomic::Ordering::Relaxed);
            if rss_shed {
                degraded.push("rerank_shed");
            } else if !self.reranker.available() {
                degraded.push("rerank_unavailable");
            } else {
                let rr_start = Instant::now();
                let top = results.len().min(20);
                let pairs: Vec<Pair> = results[..top]
                    .iter()
                    .map(|r| Pair {
                        doc_key: url_key(&r.url),
                        title: r.title.clone(),
                        snippet: r.snippet.clone(),
                    })
                    .collect();
                let query = req.q.clone();
                let reranker = self.reranker.clone();
                let (tx, rx) = tokio::sync::oneshot::channel();
                rayon::spawn(move || {
                    let r = reranker.rerank(&query, &pairs, std::time::Duration::from_millis(1500));
                    let _ = tx.send(r);
                });
                match rx.await {
                    Ok(Ok(scored)) if !scored.is_empty() => {
                        let ce_by_key: HashMap<u64, f32> =
                            scored.iter().map(|s| (s.doc_key, s.ce_score)).collect();
                        // Reorder the reranked prefix by CE; tail (beyond top-20
                        // or past the deadline) keeps LTR order.
                        let order: HashMap<u64, usize> = scored
                            .iter()
                            .enumerate()
                            .map(|(i, s)| (s.doc_key, i))
                            .collect();
                        for r in results[..top].iter_mut() {
                            let k = url_key(&r.url);
                            if let Some(ce) = ce_by_key.get(&k) {
                                r.rank_signals.ce = Some(*ce);
                            }
                        }
                        let reranked_count = order.len();
                        results[..top].sort_by_key(|r| {
                            order
                                .get(&url_key(&r.url))
                                .copied()
                                .unwrap_or(reranked_count)
                        });
                        timings.insert("rerank_ms", rr_start.elapsed().as_millis() as u64);
                        if reranked_count < top {
                            degraded.push("rerank_timeout");
                        }
                    }
                    _ => degraded.push("rerank_timeout"),
                }
            }
        }

        // Evidence layer (Phase 7, ADR-18): derivation clusters over the FINAL
        // result set — the block describes exactly what the caller sees.
        // (ADR-18 sketches a pre-LTR slot; clustering moves there when LTR/MMR
        // start consuming cluster features — recorded in the P7 exit note.)
        // Sketch lookups are mmap'd point reads; the whole stage is bounded by
        // the ≤2ms suite-11 gate. `sketches: None` = feature off ⇒ no block.
        let evidence = self.sketches.as_ref().map(|reader| {
            let ev_start = Instant::now();
            let keys: Vec<u64> = results.iter().map(|r| url_key(&r.url)).collect();
            let found = reader.get_many(&keys);
            let block = crate::evidence::annotate(&mut results, &found);
            timings.insert("evidence_ms", ev_start.elapsed().as_millis() as u64);
            block
        });

        // Bandit reward (SPEC §11): chosen arm "appeared" if any web-sourced
        // result is in the final top-10. Direct lane only by construction —
        // `chosen_arm` is only ever set on the direct branch above, so anon
        // traffic can never warm shared routing state (SPEC §12.4 firewall).
        if let (Some(bandit), Some(arm_id)) = (&self.bandit, chosen_arm) {
            let appeared = results
                .iter()
                .take(10)
                .any(|r| matches!(r.source, "web" | "both"));
            let _ = bandit.reward(query_intent.key(), arm_id, appeared);
        }

        if !self
            .shed
            .caches_dropped
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            let entry = || {
                Arc::new(CachedSearch {
                    results: results.clone(),
                    degraded: degraded.clone(),
                    evidence: evidence.clone(),
                })
            };
            if cacheable {
                self.cache.insert(key, entry());
            } else if anon_cacheable {
                self.anon_cache.insert(key, entry());
            }
        }

        tracing::debug!(intent = query_intent.key(), "query planned");
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
            evidence,
            divergence: None,
        })
    }
}

/// Stable domain identity of a result (the LTR domain_prior key) — same hash
/// the index stores, derived from the URL host.
fn result_domain_hash(r: &SearchResult) -> u64 {
    url::Url::parse(&r.url)
        .ok()
        .and_then(|u| u.host_str().map(meridian_index::lexical::domain_hash))
        .unwrap_or(0)
}

fn lane_name(lane: &Lane) -> String {
    match lane {
        Lane::Direct => "direct".to_owned(),
        Lane::Anon => "anon".to_owned(),
        Lane::Region(id) => format!("region:{}", id.0),
    }
}
