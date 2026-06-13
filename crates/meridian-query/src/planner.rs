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
    /// VoI deep-mode fetching (Phase 9, ADR-26): how many result pages this
    /// request may fetch to re-score on full text. 0 (default) = none —
    /// exactly the pre-v0.4.0 behavior. Capped by `search.deep_fetch_max`;
    /// deep mode + direct lane only (the API enforces; the planner guards).
    pub fetch_budget: usize,
    /// Answer mode (Phase 10, ADR-29): switch the fetch selector to the
    /// single-best objective (`pandora_walk` regime) and attach the
    /// `best_passage` block. Requires `fetch_budget` ≥ 1 and inherits every
    /// fetch_budget restriction (the API enforces; the planner guards).
    pub answer: bool,
    /// `diversity=evidence` (v0.6.0, the MMR replacement): reorder the final
    /// list so each ADR-18 cluster's CANONICAL document leads and its
    /// syndicated copies defer to the back. Pure local reordering — no
    /// privacy surface; a no-op when the evidence layer is off.
    pub diversity_evidence: bool,
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
    /// QPP confidence block (Phase 8, ADR-23) — raw predictors, uncalibrated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<meridian_rank::qpp::ConfidenceBlock>,
    /// VoI fetch-phase honesty block (Phase 9, ADR-26): why the engine
    /// stopped reading. Only on deep responses with `fetch_budget` > 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis: Option<AnalysisBlock>,
    /// Answer mode (Phase 10, ADR-29): the best (query, passage) pair among
    /// the FETCHED full texts — extractive only, never generated, and
    /// `ce_score` is a relevance score, NOT a correctness probability
    /// (api.md says so). Absent unless `answer=true` found a passage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_passage: Option<BestPassage>,
}

/// ADR-29 `best_passage` block (additive, ADR-20 — carries its own schema).
#[derive(Debug, Clone, serde::Serialize)]
pub struct BestPassage {
    pub schema: u32,
    /// Sentence-aligned extract, ≤ 500 chars, verbatim from the fetched page.
    pub text: String,
    /// The page the passage was read from.
    pub url: String,
    /// Cross-encoder (query, passage) score — relevance, not correctness.
    pub ce_score: f32,
}

/// ADR-26 `analysis` block (additive, ADR-20 — carries its own schema).
#[derive(Debug, Clone, serde::Serialize)]
pub struct AnalysisBlock {
    pub schema: u32,
    pub fetches_made: usize,
    pub search_stopped_because: meridian_fetch::voi::StopReason,
    /// Best expected net value left unopened (gain units; 0 on an optimal
    /// or exhaustive stop) — what stopping left on the table, said plainly.
    pub estimated_marginal_gain_remaining: f64,
}

/// Cached fusion output (SPEC §8.4 query cache: 128MB weighted, TTI 15m, TTL 2h
/// for the shared direct-lane cache; the anon cache is a separate ephemeral
/// 16MB/TTL-5m instance — anon results never touch shared state, SPEC §12.4).
#[derive(Clone)]
struct CachedSearch {
    results: Vec<SearchResult>,
    degraded: Vec<&'static str>,
    evidence: Option<crate::evidence::EvidenceBlock>,
    confidence: Option<meridian_rank::qpp::ConfidenceBlock>,
    analysis: Option<AnalysisBlock>,
    best_passage: Option<BestPassage>,
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

/// Coarse, non-identifying decision context (ADR-24) — built identically at
/// the contextual-choice site and the log site so a logged row replays the
/// exact features the policy routed on.
fn decision_context(
    q: &str,
    intent: intent::Intent,
    geo_filter: bool,
    now_unix: u64,
) -> meridian_searx::decision_log::Context {
    meridian_searx::decision_log::Context {
        intent: intent.index(),
        len_bucket: match q.chars().count() {
            0..=20 => 0,
            21..=40 => 1,
            41..=80 => 2,
            _ => 3,
        },
        lang: whichlang::detect_language(q) as u8,
        tod_bucket: ((now_unix % 86_400) / 10_800) as u8,
        geo_filter,
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
    /// Per-decision routing log (Phase 9, ADR-24). `None` = off (the v0.3.x
    /// default). Written only where the bandit is rewarded — the anon branch
    /// reaches neither (SPEC §12.4).
    decision_log: Option<Arc<meridian_searx::decision_log::DecisionLog>>,
    /// ADR-25 contextual routing (EXPERIMENTAL, dark by default): overrides
    /// the ε-greedy arm choice on the direct lane when constructed. The
    /// composition root only builds it when the decision log is also on —
    /// the log is its training data and its persistence.
    contextual: Option<Arc<std::sync::RwLock<meridian_searx::contextual::ContextualPolicy>>>,
    /// VoI deep-mode fetch phase (Phase 9, ADR-26). `None` = no fetching
    /// regardless of `fetch_budget` (degrades with `fetch_unavailable`).
    fetcher: Option<Arc<meridian_fetch::Fetcher>>,
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
        decision_log: Option<Arc<meridian_searx::decision_log::DecisionLog>>,
        contextual: Option<Arc<std::sync::RwLock<meridian_searx::contextual::ContextualPolicy>>>,
        fetcher: Option<Arc<meridian_fetch::Fetcher>>,
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
            decision_log,
            contextual,
            fetcher,
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
        // A budget-2 response is a different artifact than a budget-0 one.
        hasher.update(&req.fetch_budget.to_le_bytes());
        hasher.update(&[u8::from(req.answer)]);
        hasher.update(&[u8::from(req.diversity_evidence)]);
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
                    confidence: hit.confidence.clone(),
                    analysis: hit.analysis.clone(),
                    best_passage: hit.best_passage.clone(),
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
        let mut chosen_propensity: f32 = 0.0;
        let searx_handle = match (backend, lane_client) {
            (Some(searx), Some(client)) if wants_web && metasearch_ok => {
                // Per-request exploration salt without a global RNG.
                let salt = blake3::hash(req.q.as_bytes()).as_bytes()[0] as u64
                    ^ (timings.len() as u64)
                    ^ (req.q.len() as u64).wrapping_mul(0x9E37);
                let engines: Vec<String> = match (&self.bandit, is_direct && !req.pin_engines) {
                    (Some(b), true) => {
                        // Propensity is captured at choice time (ADR-24): the
                        // incumbent ε-greedy emits exact values; the ADR-25
                        // contextual override (dark by default) estimates its
                        // own by Monte Carlo (~1ms, documented imprecision).
                        let arm = match &self.contextual {
                            Some(cp) => {
                                let ctx = decision_context(
                                    &req.q,
                                    query_intent,
                                    req.geo.is_some(),
                                    unix_now(),
                                );
                                let guard = cp.read().expect("contextual policy lock");
                                let arm = guard.choose(&ctx, salt);
                                let idx = meridian_searx::bandit::ARMS
                                    .iter()
                                    .position(|a| a.id == arm.id)
                                    .unwrap_or(0);
                                chosen_propensity = guard.propensity(&ctx, idx, salt);
                                arm
                            }
                            None => {
                                let (arm, propensity) =
                                    b.choose_with_propensity(query_intent.key(), salt);
                                chosen_propensity = propensity;
                                arm
                            }
                        };
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

        // QPP pool snapshot (Phase 8, ADR-23): fused scores + snippets of the
        // whole candidate pool, captured before the diversity cap drains the
        // map. ≤100 short clones — bounded.
        let qpp_pool: Vec<(f32, String)> = scored
            .iter()
            .filter_map(|(key, rrf, _)| by_key.get(key).map(|r| (*rrf, r.snippet.clone())))
            .collect();

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

        // VoI fetch phase (Phase 9, ADR-26 / 07-voi-design.md): opt-in via
        // fetch_budget, deep mode + direct lane only (the API enforces it;
        // this guard is defense in depth). Selected result pages go through
        // the standard fetch ladder (SSRF/robots/budget/extract) and are
        // re-scored by the cross-encoder on their FULL text — used in RAM
        // only, never ingested; the ladder's 24h extract cache is the only
        // persistence (identical to /v1/fetch).
        let mut analysis: Option<AnalysisBlock> = None;
        let mut best_passage: Option<BestPassage> = None;
        if req.fetch_budget > 0 && is_direct && matches!(req.mode, SearchMode::Deep) {
            let rss_shed = self
                .shed
                .rerank_disabled
                .load(std::sync::atomic::Ordering::Relaxed);
            match (&self.fetcher, self.reranker.available() && !rss_shed) {
                (Some(fetcher), true) => {
                    let phase_start = Instant::now();
                    let answer_mode = req.answer;
                    // Answer mode does strictly more work per fetch (a
                    // passage-CE batch realizes the walk's value), so it has
                    // its own phase deadline (02-budgets: answer p50 ≤3.0s —
                    // the deep 2.5s budget is not silently busted).
                    let deadline = std::time::Duration::from_millis(if answer_mode {
                        self.cfg.answer_deadline_ms
                    } else {
                        self.cfg.deep_fetch_deadline_ms
                    });
                    let budget = req.fetch_budget.min(self.cfg.deep_fetch_max.max(1));
                    let head = results.len().min(20);
                    // Standardize head scores once — the value model's score_z.
                    let n = head.max(1) as f64;
                    let mean = results[..head].iter().map(|r| r.score as f64).sum::<f64>() / n;
                    let sd = (results[..head]
                        .iter()
                        .map(|r| (r.score as f64 - mean).powi(2))
                        .sum::<f64>()
                        / n)
                        .sqrt()
                        .max(1e-9);
                    let dcg_headroom =
                        |rank: usize| 7.0 * (1.0 - 1.0 / ((rank as f64 + 2.0).log2()));
                    let max_net = |cands: &[meridian_fetch::voi::Candidate]| {
                        cands
                            .iter()
                            .map(|c| c.p * c.gain - c.cost)
                            .fold(0.0f64, f64::max)
                            .max(0.0)
                    };

                    let mut fetched_sketches: Vec<meridian_index::sketch::Sketch> = Vec::new();
                    let mut fetched_docs: Vec<(usize, String, String)> = Vec::new();
                    // Answer mode: the value in hand (best passage CE so far)
                    // and per-doc realized CE for the head re-order.
                    let mut best_value = 0.0f64;
                    let mut doc_ce: Vec<(usize, f32)> = Vec::new();
                    let mut fetched_idx: std::collections::HashSet<usize> =
                        std::collections::HashSet::new();
                    let mut fetches_made = 0usize;
                    // Every loop exit assigns `stopped` — deferred init lets
                    // the compiler prove it.
                    let stopped;
                    let mut est_remaining = 0.0f64;
                    loop {
                        // Re-pose the box each round: a fetched page's sketch
                        // crushes the marginal value of its near-copies.
                        let cands: Vec<meridian_fetch::voi::Candidate> = results[..head]
                            .iter()
                            .enumerate()
                            .filter(|(i, r)| r.source == "web" && !fetched_idx.contains(i))
                            .map(|(i, r)| {
                                let snip = meridian_index::sketch::Sketch::compute(&format!(
                                    "{} {}",
                                    r.title, r.snippet
                                ));
                                let novelty = 1.0
                                    - fetched_sketches
                                        .iter()
                                        .map(|f| snip.containment(f))
                                        .fold(0.0f64, f64::max);
                                let score_z = (r.score as f64 - mean) / sd;
                                if answer_mode {
                                    // ADR-29: single-best regime, passage-CE
                                    // units (gain = novelty, cost frozen by
                                    // suite 18).
                                    meridian_fetch::voi::answer_candidate(
                                        i as u64, novelty, score_z,
                                    )
                                } else {
                                    meridian_fetch::voi::candidate_from_signals(
                                        i as u64,
                                        dcg_headroom(i),
                                        novelty,
                                        score_z,
                                        meridian_fetch::voi::DEFAULT_FETCH_COST,
                                    )
                                }
                            })
                            .collect();
                        if cands.is_empty() {
                            stopped = meridian_fetch::voi::StopReason::Exhausted;
                            break;
                        }
                        // What stopping now would leave on the table, in the
                        // mode's own units (page: best expected net value;
                        // answer: best reservation index minus the passage
                        // value already in hand).
                        let est_left = if answer_mode {
                            (cands
                                .iter()
                                .map(meridian_fetch::voi::reservation_index)
                                .fold(f64::MIN, f64::max)
                                - best_value)
                                .max(0.0)
                        } else {
                            max_net(&cands)
                        };
                        if fetches_made >= budget {
                            stopped = meridian_fetch::voi::StopReason::BudgetExhausted;
                            est_remaining = est_left;
                            break;
                        }
                        if phase_start.elapsed() >= deadline {
                            stopped = meridian_fetch::voi::StopReason::Deadline;
                            est_remaining = est_left;
                            break;
                        }
                        let idx = if answer_mode {
                            // Pandora's rule inline (the walk's callback is
                            // sync; the fetch is not): open the best
                            // reservation index unless the value in hand
                            // already meets it — the optimal stop.
                            let top = cands
                                .iter()
                                .map(|c| (meridian_fetch::voi::reservation_index(c), c.id))
                                .max_by(|a, b| {
                                    a.0.partial_cmp(&b.0)
                                        .unwrap_or(std::cmp::Ordering::Equal)
                                        .then(b.1.cmp(&a.1))
                                })
                                .expect("cands non-empty");
                            if best_value >= top.0 {
                                stopped = meridian_fetch::voi::StopReason::ValueBelowReservation;
                                break;
                            }
                            top.1 as usize
                        } else {
                            let walk = meridian_fetch::voi::additive_walk(&cands, 1, |_| {});
                            let Some(&open_id) = walk.opened.first() else {
                                stopped = walk.stopped_because;
                                break;
                            };
                            open_id as usize
                        };
                        fetches_made += 1;
                        fetched_idx.insert(idx);
                        let remaining = deadline.saturating_sub(phase_start.elapsed());
                        // Failed/timed-out fetches consume budget — honest
                        // no-op; the page stays on its snippet score.
                        if let Ok(Ok(doc)) = tokio::time::timeout(
                            remaining,
                            fetcher.fetch_extract(&results[idx].url, &Lane::Direct),
                        )
                        .await
                        {
                            fetched_sketches
                                .push(meridian_index::sketch::Sketch::compute(&doc.text));
                            let title = doc.title.unwrap_or_else(|| results[idx].title.clone());
                            if answer_mode {
                                // Realize the walk's value NOW: one CE batch
                                // over this doc's passages (the stop decision
                                // between opens needs it — suite-18 replay
                                // semantics, mirrored exactly).
                                let passages = meridian_fetch::passage::split_passages(
                                    &doc.text,
                                    500,
                                    self.cfg.answer_passage_cap.max(1),
                                );
                                if !passages.is_empty() {
                                    let pairs: Vec<Pair> = passages
                                        .iter()
                                        .enumerate()
                                        .map(|(pi, p)| Pair {
                                            doc_key: pi as u64,
                                            title: title.clone(),
                                            snippet: p.clone(),
                                        })
                                        .collect();
                                    let query = req.q.clone();
                                    let reranker = self.reranker.clone();
                                    let (tx, rx) = tokio::sync::oneshot::channel();
                                    rayon::spawn(move || {
                                        let r = reranker.rerank(
                                            &query,
                                            &pairs,
                                            std::time::Duration::from_millis(800),
                                        );
                                        let _ = tx.send(r);
                                    });
                                    if let Ok(Ok(scored)) = rx.await {
                                        if let Some(top) = scored.iter().max_by(|a, b| {
                                            a.ce_score
                                                .partial_cmp(&b.ce_score)
                                                .unwrap_or(std::cmp::Ordering::Equal)
                                        }) {
                                            // Latency-study telemetry: WHERE
                                            // in the doc winners live decides
                                            // how low the cap can go. No
                                            // query text, no URL — position
                                            // only.
                                            tracing::debug!(
                                                passage_index = top.doc_key,
                                                passages_scored = scored.len(),
                                                "answer passage realized"
                                            );
                                            doc_ce.push((idx, top.ce_score));
                                            best_value = best_value.max(f64::from(top.ce_score));
                                            let better = best_passage
                                                .as_ref()
                                                .map(|bp| top.ce_score > bp.ce_score)
                                                .unwrap_or(true);
                                            if better {
                                                best_passage = Some(BestPassage {
                                                    schema: 1,
                                                    text: passages[top.doc_key as usize].clone(),
                                                    url: results[idx].url.clone(),
                                                    ce_score: top.ce_score,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                            fetched_docs.push((idx, title, doc.text));
                        }
                    }
                    // Answer mode already realized per-doc CE (max passage
                    // score) during the walk — re-order the head from it and
                    // skip the second full-text batch; the honesty marker
                    // fires when answer mode came back empty-handed.
                    if answer_mode {
                        if !doc_ce.is_empty() {
                            for (i, ce) in &doc_ce {
                                results[*i].rank_signals.ce = Some(*ce);
                            }
                            results[..head].sort_by(|a, b| {
                                let ka = a.rank_signals.ce.unwrap_or(f32::MIN);
                                let kb = b.rank_signals.ce.unwrap_or(f32::MIN);
                                kb.partial_cmp(&ka).unwrap_or(std::cmp::Ordering::Equal)
                            });
                        }
                        // H3 selective abstention (§8.3, suite-20 judge): when the
                        // winning passage's relevance falls below the operator's
                        // threshold, WITHHOLD it rather than ship a low-relevance
                        // answer — distinct from the mechanical `answer_unavailable`.
                        // Default-OFF (threshold 0.0). Honest framing: this filters
                        // low-relevance passages, it does NOT certify shown ones
                        // (no coverage guarantee — conformal bands died in suite 16).
                        if let Some(bp) = &best_passage {
                            if bp.ce_score < self.cfg.answer_abstain_threshold {
                                best_passage = None;
                                degraded.push("answer_below_threshold");
                            }
                        }
                        if best_passage.is_none()
                            && !degraded.contains(&"answer_below_threshold")
                        {
                            degraded.push("answer_unavailable");
                        }
                    }
                    // One CE batch over the fetched FULL texts; those results'
                    // ce values are replaced and the head re-orders by ce
                    // (same scale as the snippet-pair rerank above).
                    if !answer_mode && !fetched_docs.is_empty() {
                        let pairs: Vec<Pair> = fetched_docs
                            .iter()
                            .map(|(i, title, text)| Pair {
                                doc_key: url_key(&results[*i].url),
                                title: title.clone(),
                                snippet: text.chars().take(1200).collect(),
                            })
                            .collect();
                        let query = req.q.clone();
                        let reranker = self.reranker.clone();
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        rayon::spawn(move || {
                            let r = reranker.rerank(
                                &query,
                                &pairs,
                                std::time::Duration::from_millis(800),
                            );
                            let _ = tx.send(r);
                        });
                        if let Ok(Ok(scored)) = rx.await {
                            let ce_by_key: HashMap<u64, f32> =
                                scored.iter().map(|p| (p.doc_key, p.ce_score)).collect();
                            for r in results[..head].iter_mut() {
                                if let Some(ce) = ce_by_key.get(&url_key(&r.url)) {
                                    r.rank_signals.ce = Some(*ce);
                                }
                            }
                            results[..head].sort_by(|a, b| {
                                let ka = a.rank_signals.ce.unwrap_or(f32::MIN);
                                let kb = b.rank_signals.ce.unwrap_or(f32::MIN);
                                kb.partial_cmp(&ka).unwrap_or(std::cmp::Ordering::Equal)
                            });
                        }
                    }
                    timings.insert("fetch_phase_ms", phase_start.elapsed().as_millis() as u64);
                    analysis = Some(AnalysisBlock {
                        schema: 1,
                        fetches_made,
                        search_stopped_because: stopped,
                        estimated_marginal_gain_remaining: (est_remaining * 1e4).round() / 1e4,
                    });
                }
                _ => degraded.push("fetch_unavailable"),
            }
        }

        // No MMR diversity stage: token-Jaccard MMR FAILED suite 13b on both
        // generator seeds (alpha-nDCG gain only at >1% nDCG cost — symmetric
        // similarity demotes canonical originals along with their copies),
        // so the `diversity=mmr` surface was withdrawn before v0.3.0 shipped
        // it. Evidence-cluster-aware diversity (ADR-18 sketch clusters) is
        // the carried-forward replacement; see meridian-rank/src/mmr.rs.

        // QPP confidence (Phase 8, ADR-23): raw NQC + clarity-lite predictors
        // over the response head vs the candidate pool. ≤1ms budget; absent
        // when there is nothing to predict from.
        let confidence = {
            let qpp_start = Instant::now();
            let top_scores: Vec<f32> = results.iter().map(|r| r.rank_signals.rrf).collect();
            let top_snips: Vec<&str> = results.iter().map(|r| r.snippet.as_str()).collect();
            let pool_scores: Vec<f32> = qpp_pool.iter().map(|(s, _)| *s).collect();
            let pool_snips: Vec<&str> = qpp_pool.iter().map(|(_, t)| t.as_str()).collect();
            let block =
                meridian_rank::qpp::confidence(&top_scores, &top_snips, &pool_scores, &pool_snips);
            timings.insert("qpp_ms", qpp_start.elapsed().as_millis() as u64);
            block
        };

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

        // Evidence-cluster diversity (v0.6.0, the carried MMR replacement —
        // gated by the same dup harness that withdrew MMR): each cluster's
        // canonical document takes the cluster's best slot, copies defer to
        // the back, unsketched results never move. `rank_signals` stay raw —
        // the promoted canonical can show a lower score than a copy below
        // it, and that is the explainable truth of what happened.
        if req.diversity_evidence && evidence.is_some() {
            let tags: Vec<Option<(u32, bool)>> = results
                .iter()
                .map(|r| r.evidence.as_ref().map(|e| (e.cluster, e.canonical)))
                .collect();
            let order = meridian_rank::diversity::cluster_diversify(&tags);
            let mut pos = vec![usize::MAX; results.len()];
            for (rank, &i) in order.iter().enumerate() {
                pos[i] = rank;
            }
            let drained: Vec<SearchResult> = std::mem::take(&mut results);
            let mut paired: Vec<(usize, SearchResult)> = pos.into_iter().zip(drained).collect();
            paired.sort_by_key(|(p, _)| *p);
            results = paired.into_iter().map(|(_, r)| r).collect();
        }

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

            // Decision log (Phase 9, ADR-24): same site, so it inherits the
            // same anon firewall — `chosen_arm` is only ever set on the
            // direct branch. The row is 13 bytes of coarse buckets: no query
            // text, no URLs, no timestamp finer than the 3h bucket. redb
            // commits fsync, so the write rides spawn_blocking off the
            // request path.
            if self.decision_log.is_some() || self.contextual.is_some() {
                let arm = meridian_searx::bandit::ARMS
                    .iter()
                    .position(|a| a.id == arm_id)
                    .unwrap_or(0) as u8;
                let now_unix = unix_now();
                let ctx = decision_context(&req.q, query_intent, req.geo.is_some(), now_unix);
                // Online update for the ADR-25 policy mirrors the log row.
                if let Some(cp) = &self.contextual {
                    if let Ok(mut guard) = cp.write() {
                        guard.observe(&ctx, arm as usize, appeared);
                    }
                }
                if let Some(dl) = &self.decision_log {
                    // A2 (ADR-25 §7.4): the rank-weighted graded reward over the
                    // top-10 — every web result came from the chosen arm, so
                    // DCG-weight the positions it occupied against the ideal.
                    // Logged ALONGSIDE the binary `appeared` (row[11]); purely
                    // rank-derived (no query content), so the privacy envelope is
                    // unchanged. The gate keeps the binary estimand until A2 earns
                    // the switch on organic rows (and an ADR-24 re-sign-off).
                    let reward_graded = {
                        let w = |rank1: usize| 1.0 / ((rank1 + 1) as f64).log2();
                        let ideal: f64 = (1..=10).map(w).sum();
                        let earned: f64 = results
                            .iter()
                            .take(10)
                            .enumerate()
                            .filter(|(_, r)| matches!(r.source, "web" | "both"))
                            .map(|(i, _)| w(i + 1))
                            .sum();
                        ((earned / ideal).clamp(0.0, 1.0) * 255.0).round() as u8
                    };
                    let dl = dl.clone();
                    let propensity = chosen_propensity;
                    tokio::task::spawn_blocking(move || {
                        let _ = dl.log(now_unix, ctx, arm, propensity, appeared, reward_graded);
                    });
                }
            }
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
                    confidence: confidence.clone(),
                    analysis: analysis.clone(),
                    best_passage: best_passage.clone(),
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
            confidence,
            analysis,
            best_passage,
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
