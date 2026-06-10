//! Query planner v1 (SPEC §3 fast-mode pipeline, lexical slice): resolve lane →
//! local BM25 (rayon-bridged) ∥ SearXNG fan-out (deadline + direct-only hedging)
//! → RRF(k=60) → domain-diversity cap → results with rank_signals + timings.
//! ANN joins the fusion in Phase 2; LTR/rerank in Phase 3.

use crate::rrf::{RRF_K, rrf_fuse};
use crate::{Scope, SearchMode};
use meridian_common::config::SearchConfig;
use meridian_egress::{Lane, LaneRegistry};
use meridian_index::lexical::LexicalIndex;
use meridian_searx::client::SearxClient;
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

pub struct Planner {
    index: Arc<LexicalIndex>,
    searx: Option<Arc<SearxClient>>,
    lanes: Arc<LaneRegistry>,
    cfg: SearchConfig,
}

impl Planner {
    pub fn new(
        index: Arc<LexicalIndex>,
        searx: Option<Arc<SearxClient>>,
        lanes: Arc<LaneRegistry>,
        cfg: &SearchConfig,
    ) -> Self {
        Self {
            index,
            searx,
            lanes,
            cfg: cfg.clone(),
        }
    }

    pub async fn search(&self, req: SearchRequest) -> Result<SearchResponse, PlanError> {
        let mut timings: HashMap<&'static str, u64> = HashMap::new();
        let mut degraded: Vec<&'static str> = Vec::new();
        let limit = req.limit.clamp(1, self.cfg.max_limit);

        // Lane resolution is fail-closed: a requested lane that cannot be
        // served is an error — never a silent direct fallback (SPEC §12.1).
        let wants_web = matches!(req.scope, Scope::Web | Scope::Both) && self.searx.is_some();
        let lane_client = if wants_web {
            Some(self.lanes.resolve(&req.lane)?)
        } else {
            // Local-only search needs no egress; lanes govern network only.
            None
        };

        // Local BM25 on the rayon pool (SPEC §7.2: no CPU work on tokio).
        let wants_local = matches!(req.scope, Scope::Local | Scope::Both);
        let local_handle = wants_local.then(|| {
            let index = self.index.clone();
            let q = req.q.clone();
            let top_k = self.cfg.bm25_top_k;
            let (tx, rx) = tokio::sync::oneshot::channel();
            rayon::spawn(move || {
                let started = Instant::now();
                let hits = index.search(&q, top_k);
                let _ = tx.send((hits, started.elapsed().as_millis() as u64));
            });
            rx
        });

        // SearXNG fan-out, concurrent with the local stage.
        let searx_handle = match (&self.searx, lane_client) {
            (Some(searx), Some(client)) if wants_web => {
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

        // Collect local.
        let mut local_hits = Vec::new();
        if let Some(rx) = local_handle {
            let (hits, ms) = rx
                .await
                .map_err(|_| PlanError::Internal("local stage dropped".into()))?;
            timings.insert("bm25_ms", ms);
            local_hits = hits.map_err(|e| PlanError::Index(e.to_string()))?;
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

        let mut local_list = Vec::with_capacity(local_hits.len());
        for hit in &local_hits {
            let key = url_key(&hit.url);
            local_list.push(key);
            by_key.entry(key).or_insert_with(|| SearchResult {
                url: hit.url.clone(),
                title: hit.title.clone(),
                snippet: hit.snippet.clone(),
                score: 0.0,
                rank_signals: RankSignals {
                    rrf: 0.0,
                    bm25: Some(hit.bm25),
                    searx_rank: None,
                },
                source: "local",
            });
        }
        if !local_list.is_empty() {
            lists.push(local_list);
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

        Ok(SearchResponse {
            results,
            timings,
            lane_requested: lane_name(&req.lane),
            lane_effective: lane_name(&req.lane),
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

/// Stable 64-bit key for URL identity inside one fusion pass.
fn url_key(url: &str) -> u64 {
    let digest = blake3::hash(url.as_bytes());
    u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("8 bytes"))
}
