//! SearXNG JSON client (SPEC §5): one endpoint per instance (direct now; the
//! Tor-proxied `searxng-anon` instance arrives in Phase 4), results normalized
//! to a common type, per-engine health counters, deadline + optional hedging
//! (direct lane ONLY — SPEC §11).

use meridian_egress::LaneClient;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum SearxError {
    #[error("searx is disabled")]
    Disabled,
    #[error("searx deadline exceeded")]
    Deadline,
    #[error("searx transport error")]
    Transport,
    #[error("searx returned status {0}")]
    Http(u16),
    #[error("searx response unparsable")]
    BadResponse,
}

/// Normalized metasearch result.
#[derive(Debug, Clone)]
pub struct WebResult {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub engine: String,
    /// 0-based rank within the searx response (RRF input).
    pub rank: usize,
}

#[derive(Debug, Deserialize)]
struct RawResponse {
    #[serde(default)]
    results: Vec<RawResult>,
}

#[derive(Debug, Deserialize)]
struct RawResult {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    engine: String,
}

#[derive(Debug, Default, Clone)]
pub struct EngineHealth {
    pub responses: u64,
    pub results: u64,
}

pub struct SearxClient {
    base: reqwest::Url,
    deadline: Duration,
    hedge_after: Option<Duration>,
    /// Aggregate per-engine counters (no query text, no IPs — SPEC §13.4).
    health: Mutex<HashMap<String, EngineHealth>>,
}

impl SearxClient {
    pub fn new(
        base_url: &str,
        deadline_ms: u64,
        hedge_after: Option<Duration>,
    ) -> Result<Self, SearxError> {
        Ok(Self {
            base: reqwest::Url::parse(base_url).map_err(|_| SearxError::BadResponse)?,
            deadline: Duration::from_millis(deadline_ms),
            hedge_after,
            health: Mutex::new(HashMap::new()),
        })
    }

    /// Fan out one query. `client` must be lane-bound by the caller; hedging
    /// fires a single duplicate after the configured delay and takes whichever
    /// answers first (never on anon — the planner passes `hedge=false` there).
    pub async fn search(
        &self,
        client: &LaneClient,
        query: &str,
        hedge: bool,
    ) -> Result<Vec<WebResult>, SearxError> {
        let request = || async {
            let mut url = self
                .base
                .join("search")
                .map_err(|_| SearxError::BadResponse)?;
            url.query_pairs_mut()
                .append_pair("q", query)
                .append_pair("format", "json");
            let response = client
                .get(url)
                .send()
                .await
                .map_err(|_| SearxError::Transport)?;
            let status = response.status();
            if !status.is_success() {
                return Err(SearxError::Http(status.as_u16()));
            }
            let raw: RawResponse = response.json().await.map_err(|_| SearxError::BadResponse)?;
            Ok::<_, SearxError>(raw)
        };

        let raw = tokio::time::timeout(self.deadline, async {
            match (hedge, self.hedge_after) {
                (true, Some(delay)) => {
                    // First request, plus one duplicate if it is slow; first
                    // success wins (SPEC §11: hedging on the direct lane only).
                    let first = std::pin::pin!(request());
                    let mut first = first;
                    tokio::select! {
                        r = &mut first => r,
                        _ = tokio::time::sleep(delay) => {
                            tokio::select! {
                                r = first => r,
                                r = request() => r,
                            }
                        }
                    }
                }
                _ => request().await,
            }
        })
        .await
        .map_err(|_| SearxError::Deadline)??;

        let mut results = Vec::with_capacity(raw.results.len());
        {
            let mut health = self.health.lock().expect("searx health lock");
            for (rank, r) in raw.results.into_iter().enumerate() {
                if r.url.is_empty() {
                    continue;
                }
                let entry = health.entry(r.engine.clone()).or_default();
                entry.results += 1;
                if rank == 0 {
                    entry.responses += 1;
                }
                results.push(WebResult {
                    url: r.url,
                    title: r.title,
                    snippet: r.content,
                    engine: r.engine,
                    rank,
                });
            }
        }
        Ok(results)
    }

    pub fn engine_health(&self) -> HashMap<String, EngineHealth> {
        self.health.lock().expect("searx health lock").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_response_parses_searxng_shape() {
        let json = r#"{
            "query": "test",
            "number_of_results": 2,
            "results": [
                {"url": "https://a.example/", "title": "A", "content": "alpha", "engine": "duckduckgo"},
                {"url": "https://b.example/", "title": "B", "content": "beta", "engine": "brave", "extra": 1}
            ],
            "answers": [], "corrections": [], "infoboxes": [], "suggestions": []
        }"#;
        let raw: RawResponse = serde_json::from_str(json).unwrap();
        assert_eq!(raw.results.len(), 2);
        assert_eq!(raw.results[0].engine, "duckduckgo");
    }
}
