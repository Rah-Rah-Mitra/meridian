//! The fetch ladder (SPEC §3 ingest pipeline): SSRF vet → per-domain budget →
//! robots gate → pinned GET → streamed size-capped body → extraction. Every
//! redirect hop re-runs the ladder from the vet step (SPEC §13.1).

use crate::FetchError;
use crate::budget::DomainBudget;
use crate::extract::{Extracted, extract_html, extract_plaintext};
use crate::robots::RobotsGate;
use crate::ssrf;
use futures_util::StreamExt;
use meridian_common::config::FetchConfig;
use meridian_egress::{Lane, LaneRegistry};
use std::sync::Arc;
use url::Url;

/// Outcome of a successful ladder run. The raw body has already been discarded;
/// only clean text + metadata survive (SPEC §6.1).
#[derive(Debug)]
pub struct FetchedDoc {
    /// Final URL after redirects (the one to index).
    pub url: Url,
    pub title: Option<String>,
    pub text: String,
    pub http_status: u16,
}

/// Cached extraction result (the raw body was never kept — SPEC §6.1).
#[derive(Clone)]
struct CachedFetch {
    url: String,
    title: Option<String>,
    text: String,
    http_status: u16,
}

pub struct Fetcher {
    lanes: Arc<LaneRegistry>,
    budget: DomainBudget,
    robots: RobotsGate,
    /// SPEC §8.4: fetch/extract cache, 96MB weighted, TTL 24h. Direct lane only
    /// (anon fetches must not share state — Phase 4 brings the ephemeral one).
    cache: moka::sync::Cache<String, Arc<CachedFetch>>,
    cfg: FetchConfig,
    allow_onion: bool,
}

impl Fetcher {
    pub fn new(lanes: Arc<LaneRegistry>, cfg: &FetchConfig, allow_onion: bool) -> Self {
        let cache = moka::sync::Cache::builder()
            .max_capacity(96 * 1024 * 1024)
            .weigher(|k: &String, v: &Arc<CachedFetch>| {
                (k.len() + v.url.len() + v.text.len() + 64) as u32
            })
            .time_to_live(std::time::Duration::from_secs(24 * 60 * 60))
            .build();
        Self {
            lanes,
            budget: DomainBudget::new(cfg.per_domain_interval_ms, cfg.per_domain_burst),
            robots: RobotsGate::new(cfg.robots_ttl_secs),
            cache,
            cfg: cfg.clone(),
            allow_onion,
        }
    }

    /// Fetch + extract one URL on the given lane. All three lanes; each fails
    /// closed in `LaneRegistry` when disabled/unready (SPEC §12.1).
    pub async fn fetch_extract(
        &self,
        raw_url: &str,
        lane: &Lane,
    ) -> Result<FetchedDoc, FetchError> {
        // Ladder rung 1: cache (direct lane only — SPEC §8.4 lane isolation:
        // neither anon nor region results may seed or read shared state).
        let cacheable = matches!(lane, Lane::Direct);
        if cacheable {
            if let Some(hit) = self.cache.get(raw_url) {
                return Ok(FetchedDoc {
                    url: Url::parse(&hit.url).map_err(|_| FetchError::BadUrl)?,
                    title: hit.title.clone(),
                    text: hit.text.clone(),
                    http_status: hit.http_status,
                });
            }
        }
        // One anon transport per logical fetch: robots + every redirect hop ride
        // the same isolation username = one circuit family (SPEC §12.4 —
        // per-request isolation, and no parallel duplicate circuits).
        let anon_client = match lane {
            Lane::Anon => Some(self.lanes.resolve(lane).map_err(FetchError::Lane)?),
            _ => None,
        };
        let mut current = raw_url.to_owned();
        for _hop in 0..=self.cfg.max_redirects {
            // 1. SSRF vet (SPEC §13.1): resolve-all + deny ranges + pin on
            //    direct/region; on anon, resolution happens inside Tor, so the
            //    static destination policy is the enforceable surface (the
            //    shared deny table also runs in the SOCKS front-end).
            let (target_url, host, client) = if let Some(client) = &anon_client {
                let url = ssrf::check_url(&current, self.allow_onion)?;
                let host = url.host_str().ok_or(FetchError::BadUrl)?.to_owned();
                (url, host, client.clone())
            } else {
                let target = ssrf::vet(&current, self.allow_onion).await?;
                let client = self
                    .lanes
                    .pinned(lane, &target.host, target.addr)
                    .map_err(FetchError::Lane)?;
                (target.url, target.host, client)
            };

            // 2. Per-domain budget — global across lanes (SPEC §12.5 inv. 4).
            self.budget.acquire(&host).await;

            // 3. robots.txt on the same lane (same pinned address / same circuit).
            if !self.robots.allows(&client, &target_url).await? {
                return Err(FetchError::RobotsDenied);
            }

            // 4. GET with streamed size cap.
            let response = client
                .get(target_url.clone())
                .send()
                .await
                .map_err(|e| FetchError::Network(redact_reqwest_error(&e)))?;
            let status = response.status();

            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or(FetchError::BadRedirect)?;
                // Relative redirects resolve against the current URL; the next
                // loop iteration re-vets from scratch.
                current = target_url
                    .join(location)
                    .map_err(|_| FetchError::BadRedirect)?
                    .to_string();
                continue;
            }
            if !status.is_success() {
                return Err(FetchError::Http(status.as_u16()));
            }

            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();
            let is_html =
                content_type.contains("text/html") || content_type.contains("application/xhtml");
            let is_text = content_type.starts_with("text/plain");
            if !(is_html || is_text || content_type.is_empty()) {
                return Err(FetchError::UnsupportedMediaType);
            }

            let mut body: Vec<u8> = Vec::with_capacity(64 * 1024);
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| FetchError::Network(redact_reqwest_error(&e)))?;
                if body.len() + chunk.len() > self.cfg.max_body_bytes {
                    return Err(FetchError::TooLarge);
                }
                body.extend_from_slice(&chunk);
            }
            let body_text = String::from_utf8_lossy(&body).into_owned();
            drop(body); // raw bytes end here (SPEC §6.1)

            // 5. Extract; raw HTML is dropped with `body_text` on return.
            let Extracted { title, text } = if is_html || content_type.is_empty() {
                extract_html(&body_text, target_url.as_str())
                    .map_err(|e| FetchError::Extract(e.to_string()))?
            } else {
                extract_plaintext(&body_text)
            };

            if cacheable {
                self.cache.insert(
                    raw_url.to_owned(),
                    Arc::new(CachedFetch {
                        url: target_url.to_string(),
                        title: title.clone(),
                        text: text.clone(),
                        http_status: status.as_u16(),
                    }),
                );
            }
            return Ok(FetchedDoc {
                url: target_url,
                title,
                text,
                http_status: status.as_u16(),
            });
        }
        Err(FetchError::TooManyRedirects)
    }
}

/// reqwest errors can embed the full URL — strip to the error kind so user URLs
/// never reach logs through error formatting (SPEC §13.4).
fn redact_reqwest_error(e: &reqwest::Error) -> String {
    let kind = if e.is_connect() {
        "connect"
    } else if e.is_timeout() {
        "timeout"
    } else if e.is_body() {
        "body"
    } else if e.is_decode() {
        "decode"
    } else {
        "transport"
    };
    format!("{kind} error")
}
