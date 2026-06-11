//! Suite 12 — `divergence`: same-lane JSD noise floor (probe mode, Phase 7
//! task 7.4) and the cross-lane Phase-8 gate (`--cross-lane`, suite 12b)
//! (ADR-22 / 04-bench-plan §6).
//!
//! Gate mode drives the SHIPPED `compare=vantages` endpoint (so engine pinning
//! and cache bypass are exactly production behavior) and bootstrap-tests the
//! cross-lane JSD distribution against the committed same-lane floor mean.
//! Prerequisite for a timely run: `MERIDIAN_SEARCH__COMPARE_JITTER_MS_MAX=0`
//! on the instance (the suite reports the jitter it observed; results are
//! valid either way — just slow with the default 30s window).
//!
//! Issues the same queries repeatedly against a RUNNING meridiand, per lane
//! (direct, then anon if available), and bootstraps the distribution of
//! within-lane Jensen-Shannon divergence between the per-run result-domain
//! distributions. That distribution IS the deliverable: the Phase-8 gate
//! ("cross-lane JSD exceeds the same-lane noise floor, p<0.05") is meaningless
//! until this floor is measured. Informational — no gate.
//!
//! Device-only and network-touching (feature `bench-divergence`, never CI).
//! IMPORTANT: run against a meridiand whose query cache is disabled (or with a
//! TTL shorter than the repeat gap) — cache hits return byte-identical results
//! and fake a zero floor. The suite warns when a lane's runs are all identical.
//!
//! Bearer (if the instance requires one for anon search) is read from the
//! `MERIDIAN_BEARER` environment variable — never from argv.

use super::{BenchConfig, SuiteResult};
use crate::stats::{Rng, jsd, mean, percentile_ms};
use std::collections::HashMap;
use std::time::Instant;

const LANES: &[&str] = &["direct", "anon"];
const BOOTSTRAP_RESAMPLES: usize = 1_000;
/// Fixed result prefix the JSD analysis sees (responses are requested with
/// varying `limit` ≥ this for cache-busting, then truncated here).
/// Repeats must stay ≤ `max_limit − ANALYZE_PREFIX` (50 − 20 = 30).
const ANALYZE_PREFIX: usize = 20;

/// Region-sensitive default query classes (news / geopolitics / local services)
/// — the kinds of queries where vantage is expected to matter in Phase 8.
const DEFAULT_QUERIES: &[&str] = &[
    "election results coverage today",
    "data privacy law amendment",
    "border policy announcement",
    "fuel price increase response",
    "public broadcaster funding dispute",
    "renewable energy subsidy program",
    "housing market regulation news",
    "telecom outage cause report",
    "national rail strike updates",
    "river flooding emergency response",
    "best local pharmacy open now",
    "university admission protest",
];

pub fn run(cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("divergence");
    let start = Instant::now();

    let queries: Vec<String> = match cfg.queries.as_ref() {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(s) => s
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect(),
            Err(e) => {
                result.note(format!("cannot read --queries {}: {e}", path.display()));
                result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
                return result;
            }
        },
        None => DEFAULT_QUERIES.iter().map(|s| (*s).to_owned()).collect(),
    };
    let mut cfg = cfg.clone();
    // Cache-busting rides distinct `limit` values (ANALYZE_PREFIX + rep), and
    // the server clamps limit at 50 — beyond 30 repeats the busting would
    // silently stop working, so cap it instead.
    if cfg.repeats > 30 {
        result.note(format!(
            "repeats capped 30 (was {}) — limit-based cache-busting bound",
            cfg.repeats
        ));
        cfg.repeats = 30;
    }
    let cfg = &cfg;
    result.metric("queries", queries.len());
    result.metric("repeats_per_query", cfg.repeats);
    result.metric("api_base", cfg.api_base.clone());

    if cfg.cross_lane {
        return run_cross_lane(cfg, &queries, result, start);
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            result.note(format!("tokio runtime: {e}"));
            result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
            return result;
        }
    };

    let bearer = std::env::var("MERIDIAN_BEARER").ok();
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            result.note(format!("http client: {e}"));
            result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
            return result;
        }
    };

    for &lane in LANES {
        let outcome = rt.block_on(probe_lane(&client, cfg, &queries, lane, bearer.as_deref()));
        match outcome {
            Err(e) => result.note(format!("lane {lane}: unavailable ({e}) — skipped")),
            Ok(probe) => {
                let mut jsds = probe.within_lane_jsds.clone();
                if jsds.is_empty() {
                    result.note(format!(
                        "lane {lane}: {} ok responses but no comparable run pairs ({} empty — \
                         engine timeouts inside the lane deadline?)",
                        probe.ok_runs, probe.empty_runs
                    ));
                    continue;
                }
                let p50 = percentile_ms(&mut jsds, 50.0);
                let p90 = percentile_ms(&mut jsds, 90.0);
                let (lo, hi) = bootstrap_mean_ci(&probe.within_lane_jsds);
                result.metric(format!("{lane}_ok_runs").as_str(), probe.ok_runs);
                result.metric(format!("{lane}_failed_runs").as_str(), probe.failed_runs);
                result.metric(format!("{lane}_empty_runs").as_str(), probe.empty_runs);
                result.metric(
                    format!("{lane}_jsd_pairs").as_str(),
                    probe.within_lane_jsds.len(),
                );
                result.metric(
                    format!("{lane}_jsd_mean").as_str(),
                    round4(mean(&probe.within_lane_jsds)),
                );
                result.metric(format!("{lane}_jsd_p50").as_str(), round4(p50));
                result.metric(format!("{lane}_jsd_p90").as_str(), round4(p90));
                result.metric(format!("{lane}_jsd_mean_ci95_lo").as_str(), round4(lo));
                result.metric(format!("{lane}_jsd_mean_ci95_hi").as_str(), round4(hi));
                if probe.identical_pairs == probe.within_lane_jsds.len() {
                    result.note(format!(
                        "WARNING lane {lane}: every repeat returned identical domains — query \
                         cache likely enabled; this floor is invalid (disable the cache and re-run)"
                    ));
                }
            }
        }
    }

    result.note(
        "noise floor = the within-lane JSD distribution above; the Phase-8 cross-lane \
         gate must exceed it at p<0.05 (bootstrap). Commit this report to docs/plan/bench/."
            .to_owned(),
    );
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}

/// Suite 12b: the Phase-8 divergence gate. Drives `compare=vantages` per
/// (query, repeat) and tests mean cross-lane JSD against the committed
/// same-lane floor mean (one-sample bootstrap).
fn run_cross_lane(
    cfg: &BenchConfig,
    queries: &[String],
    mut result: SuiteResult,
    start: Instant,
) -> SuiteResult {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            result.note(format!("tokio runtime: {e}"));
            result.gate("cross-lane JSD > floor at p<0.05", false);
            result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
            return result;
        }
    };
    let bearer = std::env::var("MERIDIAN_BEARER").ok();
    let client = match reqwest::Client::builder()
        // A compare = direct fan-out + jitter + anon fan-out; generous.
        .timeout(std::time::Duration::from_secs(120))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            result.note(format!("http client: {e}"));
            result.gate("cross-lane JSD > floor at p<0.05", false);
            result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
            return result;
        }
    };

    let mut samples: Vec<f64> = Vec::new();
    let mut jitters: Vec<f64> = Vec::new();
    let mut failed = 0usize;
    rt.block_on(async {
        for q in queries {
            for _rep in 0..cfg.repeats {
                let url = format!(
                    "{}/v1/search?q={}&scope=web&compare=vantages&limit={ANALYZE_PREFIX}",
                    cfg.api_base.trim_end_matches('/'),
                    percent_encode(q),
                );
                let mut req = client.get(url);
                if let Some(b) = bearer.as_deref() {
                    req = req.header("authorization", format!("Bearer {b}"));
                }
                match req.send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.json::<serde_json::Value>().await {
                            Ok(body) => {
                                let d = &body["divergence"];
                                if let Some(jsd) = d["jsd"].as_f64() {
                                    samples.push(jsd);
                                    if let Some(j) = d["jitter_applied_ms"].as_f64() {
                                        jitters.push(j);
                                    }
                                } else {
                                    failed += 1;
                                }
                            }
                            Err(_) => failed += 1,
                        }
                    }
                    _ => failed += 1,
                }
                tokio::time::sleep(std::time::Duration::from_millis(2_500)).await;
            }
        }
    });

    result.metric("mode", "cross-lane (compare=vantages)");
    result.metric("cross_samples", samples.len());
    result.metric("cross_failed", failed);
    if !jitters.is_empty() {
        result.metric("mean_jitter_observed_ms", round4(mean(&jitters)));
    }
    if samples.len() < 10 {
        result.note(format!(
            "only {} cross-lane samples — not enough for the gate (lane down? \
             compare disabled?)",
            samples.len()
        ));
        result.gate("cross-lane JSD > floor at p<0.05", false);
        result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
        return result;
    }

    let mut sorted = samples.clone();
    let cross_p50 = percentile_ms(&mut sorted, 50.0);
    let cross_p90 = percentile_ms(&mut sorted, 90.0);
    result.metric("cross_jsd_mean", round4(mean(&samples)));
    result.metric("cross_jsd_p50", round4(cross_p50));
    result.metric("cross_jsd_p90", round4(cross_p90));
    result.metric("floor_mean_reference", round4(cfg.floor_mean));

    // One-sample bootstrap: p = P(resampled cross mean ≤ floor mean).
    let mut rng = Rng::new(0xC405_2026);
    let mut le = 0usize;
    for _ in 0..BOOTSTRAP_RESAMPLES {
        let m: f64 = (0..samples.len())
            .map(|_| samples[rng.below(samples.len())])
            .sum::<f64>()
            / samples.len() as f64;
        if m <= cfg.floor_mean {
            le += 1;
        }
    }
    let p_value = le as f64 / BOOTSTRAP_RESAMPLES as f64;
    result.metric("bootstrap_p_value", round4(p_value));
    result.note(
        "gate per SPEC §16 P8: mean cross-lane JSD (shipped compare=vantages, \
         engines pinned both halves) exceeds the committed same-lane floor mean \
         at p<0.05 (one-sample bootstrap)"
            .to_owned(),
    );
    result.gate("cross-lane JSD > floor at p<0.05 (bootstrap)", p_value < 0.05);
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

struct LaneProbe {
    ok_runs: usize,
    failed_runs: usize,
    /// HTTP 200 but zero parseable result domains — engine timeouts inside the
    /// lane deadline look exactly like this; silent dropping hid it once.
    empty_runs: usize,
    within_lane_jsds: Vec<f64>,
    identical_pairs: usize,
}

async fn probe_lane(
    client: &reqwest::Client,
    cfg: &BenchConfig,
    queries: &[String],
    lane: &str,
    bearer: Option<&str>,
) -> Result<LaneProbe, String> {
    // Engine-polite pacing on BOTH lanes: the first run of this probe paced
    // direct at 300ms and drove the upstream engines into timeout/rate-limit —
    // every direct fan-out came back empty inside meridiand's 800ms deadline.
    // 2.5s/query keeps the probe a slow trickle.
    let gap = std::time::Duration::from_millis(2_500);
    let _ = lane;

    let mut probe = LaneProbe {
        ok_runs: 0,
        failed_runs: 0,
        empty_runs: 0,
        within_lane_jsds: Vec::new(),
        identical_pairs: 0,
    };
    let mut lane_checked = false;

    for q in queries {
        // One domain distribution per repeat run.
        let mut runs: Vec<HashMap<u64, f64>> = Vec::new();
        for rep in 0..cfg.repeats {
            // Cache-busting: the planner's query caches key on `limit`, so a
            // distinct limit per repeat forces a fresh engine fan-out without
            // any server-side change; the analysis truncates every response to
            // the same top-ANALYZE_PREFIX results so distributions stay
            // comparable across repeats.
            let url = format!(
                "{}/v1/search?q={}&scope=web&lane={lane}&limit={}",
                cfg.api_base.trim_end_matches('/'),
                percent_encode(q),
                ANALYZE_PREFIX + rep,
            );
            let mut req = client.get(url);
            if let Some(b) = bearer {
                req = req.header("authorization", format!("Bearer {b}"));
            }
            match req.send().await {
                Err(e) => {
                    // First request failing to connect ⇒ the lane/instance is
                    // down; bail out instead of burning the full matrix.
                    if !lane_checked {
                        return Err(format!("first request failed: {e}"));
                    }
                    probe.failed_runs += 1;
                }
                Ok(resp) => {
                    lane_checked = true;
                    let status = resp.status();
                    if !status.is_success() {
                        if !probe_status_tolerable(status.as_u16()) {
                            return Err(format!("HTTP {status} from lane {lane}"));
                        }
                        probe.failed_runs += 1;
                    } else {
                        match resp.json::<serde_json::Value>().await {
                            Err(e) => {
                                probe.failed_runs += 1;
                                let _ = e;
                            }
                            Ok(body) => {
                                let dist = domain_distribution(&body);
                                if dist.is_empty() {
                                    probe.empty_runs += 1;
                                } else {
                                    runs.push(dist);
                                }
                                probe.ok_runs += 1;
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(gap).await;
        }

        // All within-lane run pairs for this query.
        for i in 0..runs.len() {
            for j in (i + 1)..runs.len() {
                let d = jsd(&runs[i], &runs[j]);
                if d.is_nan() {
                    continue;
                }
                if d == 0.0 {
                    probe.identical_pairs += 1;
                }
                probe.within_lane_jsds.push(d);
            }
        }
    }
    Ok(probe)
}

/// 503 (lane degraded / shedding) and 429 mid-run are tolerable single-run
/// failures; auth/4xx config errors are not worth burning the matrix on.
fn probe_status_tolerable(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

/// Result URLs → registered-domain counts over the first [`ANALYZE_PREFIX`]
/// results (responses are over-fetched for cache-busting). Registered domain
/// is approximated as the last two host labels (three when the second-to-last
/// is a well-known second-level registry label) — a PSL-exact mapping is not
/// needed for a floor measured against itself.
fn domain_distribution(body: &serde_json::Value) -> HashMap<u64, f64> {
    let mut dist: HashMap<u64, f64> = HashMap::new();
    let Some(results) = body.get("results").and_then(|r| r.as_array()) else {
        return dist;
    };
    for r in results.iter().take(ANALYZE_PREFIX) {
        let Some(url) = r.get("url").and_then(|u| u.as_str()) else {
            continue;
        };
        if let Some(domain) = registered_domain(url) {
            *dist.entry(fnv1a(domain.as_bytes())).or_insert(0.0) += 1.0;
        }
    }
    dist
}

fn registered_domain(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next()?.split('@').next_back()?;
    let host = host.split(':').next()?.to_ascii_lowercase();
    if host.is_empty() || host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() < 2 {
        return Some(host);
    }
    const SECOND_LEVEL: &[&str] = &["co", "com", "net", "org", "ac", "gov", "edu"];
    let take = if labels.len() >= 3 && SECOND_LEVEL.contains(&labels[labels.len() - 2]) {
        3
    } else {
        2
    };
    Some(labels[labels.len() - take..].join("."))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// RFC 3986 query-component percent-encoding (unreserved characters pass through).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Percentile-bootstrap 95% CI of the mean (deterministic seed — reproducible).
fn bootstrap_mean_ci(samples: &[f64]) -> (f64, f64) {
    if samples.is_empty() {
        return (f64::NAN, f64::NAN);
    }
    let mut rng = Rng::new(0xD1FF_2026);
    let mut means: Vec<f64> = (0..BOOTSTRAP_RESAMPLES)
        .map(|_| {
            let total: f64 = (0..samples.len())
                .map(|_| samples[rng.below(samples.len())])
                .sum();
            total / samples.len() as f64
        })
        .collect();
    let lo = percentile_ms(&mut means, 2.5);
    let hi = percentile_ms(&mut means, 97.5);
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_domain_approximation() {
        assert_eq!(
            registered_domain("https://news.example.com/a?b=1"),
            Some("example.com".into())
        );
        assert_eq!(
            registered_domain("http://www.bbc.co.uk/news"),
            Some("bbc.co.uk".into())
        );
        assert_eq!(
            registered_domain("https://example.org"),
            Some("example.org".into())
        );
        assert_eq!(registered_domain("https://127.0.0.1:8080/x"), None);
    }

    #[test]
    fn domain_distribution_counts() {
        let body: serde_json::Value = serde_json::json!({
            "results": [
                {"url": "https://a.example.com/1"},
                {"url": "https://b.example.com/2"},
                {"url": "https://other.org/3"}
            ]
        });
        let dist = domain_distribution(&body);
        assert_eq!(dist.len(), 2);
        assert_eq!(dist.values().sum::<f64>(), 3.0);
    }

    #[test]
    fn bootstrap_ci_brackets_mean() {
        let samples: Vec<f64> = (0..200).map(|i| 0.1 + 0.001 * f64::from(i % 10)).collect();
        let (lo, hi) = bootstrap_mean_ci(&samples);
        let m = mean(&samples);
        assert!(lo <= m && m <= hi, "{lo} ≤ {m} ≤ {hi}");
    }
}
