//! robots.txt enforcement (SPEC §13.2): honored on EVERY lane, 24h cache, one
//! chokepoint below lane selection. Parser: `texting_robots` (ADR-12 — validated
//! against Google's suite + 34M real-world files).
//!
//! Availability policy per RFC 9309 guidance: 4xx/absent → allow; network error
//! or 5xx → conservative disallow (do not crawl what you cannot check).

use crate::FetchError;
use meridian_egress::LaneClient;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use texting_robots::Robot;
use url::Url;

struct CachedPolicy {
    /// None = no robots.txt (allow-all); Some = parsed rules.
    robots_body: Option<Vec<u8>>,
    fetched: Instant,
}

pub struct RobotsGate {
    ttl: Duration,
    agent: String,
    /// Tiny hand-rolled TTL map: Phase 2 brings Moka; robots bodies are small
    /// and per-domain cardinality is bounded by the per-domain budget anyway.
    cache: Mutex<HashMap<String, CachedPolicy>>,
}

impl RobotsGate {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_secs),
            agent: "MeridianSearch".to_owned(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// True if `url` may be fetched. `client` must already be lane-bound; the
    /// robots.txt request itself rides the same lane as the fetch it gates.
    pub async fn allows(&self, client: &LaneClient, url: &Url) -> Result<bool, FetchError> {
        let host = url.host_str().ok_or(FetchError::BadUrl)?.to_owned();
        let origin = format!(
            "{}://{}{}",
            url.scheme(),
            host,
            match url.port() {
                Some(p) => format!(":{p}"),
                None => String::new(),
            }
        );

        let cached_body: Option<Option<Vec<u8>>> = {
            let cache = self.cache.lock().expect("robots cache lock");
            cache
                .get(&origin)
                .and_then(|c| (c.fetched.elapsed() < self.ttl).then(|| c.robots_body.clone()))
        };

        let body = match cached_body {
            Some(body) => body,
            None => {
                let fetched = self.fetch_robots(client, &origin).await?;
                self.cache.lock().expect("robots cache lock").insert(
                    origin.clone(),
                    CachedPolicy {
                        robots_body: fetched.clone(),
                        fetched: Instant::now(),
                    },
                );
                fetched
            }
        };

        Ok(match body {
            None => true, // 4xx/absent → allow (RFC 9309)
            Some(bytes) => match Robot::new(&self.agent, &bytes) {
                Ok(robot) => robot.allowed(url.as_str()),
                // Unparseable robots.txt → conservative allow per RFC 9309
                // (malformed lines are ignored, not treated as a ban).
                Err(_) => true,
            },
        })
    }

    /// `Ok(None)` = no rules (allow); `Ok(Some(body))` = rules; `Err` = 5xx or
    /// network failure (caller treats as disallow).
    async fn fetch_robots(
        &self,
        client: &LaneClient,
        origin: &str,
    ) -> Result<Option<Vec<u8>>, FetchError> {
        let robots_url: Url = format!("{origin}/robots.txt")
            .parse()
            .map_err(|_| FetchError::BadUrl)?;
        let response = client
            .get(robots_url)
            .send()
            .await
            .map_err(|_| FetchError::RobotsUnavailable)?;
        let status = response.status();
        if status.is_client_error() {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(FetchError::RobotsUnavailable);
        }
        // robots.txt over 512KB is hostile; truncate (RFC 9309 allows partial).
        let body = response
            .bytes()
            .await
            .map_err(|_| FetchError::RobotsUnavailable)?;
        let body = body.slice(..body.len().min(512 * 1024));
        Ok(Some(body.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use texting_robots::Robot;

    #[test]
    fn parser_honors_agent_rules() {
        let robots = b"User-agent: *\nDisallow: /private/\nAllow: /\n";
        let robot = Robot::new("MeridianSearch", robots).unwrap();
        assert!(robot.allowed("https://example.com/page"));
        assert!(!robot.allowed("https://example.com/private/x"));
    }
}
