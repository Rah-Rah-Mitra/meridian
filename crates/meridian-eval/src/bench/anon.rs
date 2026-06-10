//! Suite 8 — Arti bootstrap + isolated-circuit timing + RSS delta (SPEC §15.8).
//! Informational in Phase 0: numbers ground the anon deadline (≤8s), the circuit
//! cap (6), and the §6.2 Arti RSS estimate (80–150MB). The packet-level leak test
//! (nftables: zero non-Tor egress) is a separate scripted drill — a userspace
//! process cannot honestly assert its own absence of traffic.
//!
//! NOTE: this suite touches the live Tor network (directory bootstrap + a few
//! streams to example.com:80) — run it deliberately, not in CI.

use super::{BenchConfig, SuiteResult};
use crate::probe::rss_bytes;
use crate::stats::percentile_ms;
use arti_client::config::TorClientConfigBuilder;
use arti_client::isolation::IsolationToken;
use arti_client::{StreamPrefs, TorClient};
use std::time::{Duration, Instant};

const CIRCUITS: usize = 10;
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(180);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(45);

pub fn run(cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("anon");
    let start = Instant::now();

    // arti-client's rustls path requires the app to install a crypto provider.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let state_dir = cfg.scratch_dir.join("arti-state");
    let cache_dir = cfg.scratch_dir.join("arti-cache");
    if std::fs::create_dir_all(&state_dir).is_err() || std::fs::create_dir_all(&cache_dir).is_err()
    {
        result.note("cannot create arti state/cache dirs");
        return result;
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            result.note(format!("tokio runtime: {e}"));
            return result;
        }
    };

    let rss0 = rss_bytes();
    let outcome = rt.block_on(async {
        let config = TorClientConfigBuilder::from_directories(&state_dir, &cache_dir)
            .build()
            .map_err(|e| format!("config: {e}"))?;

        let t = Instant::now();
        let client =
            tokio::time::timeout(BOOTSTRAP_TIMEOUT, TorClient::create_bootstrapped(config))
                .await
                .map_err(|_| "bootstrap timed out".to_owned())?
                .map_err(|e| format!("bootstrap: {e}"))?;
        let bootstrap_ms = t.elapsed().as_secs_f64() * 1e3;

        let mut samples = Vec::with_capacity(CIRCUITS);
        let mut failures = 0usize;
        for _ in 0..CIRCUITS {
            // Fresh isolation token per request — distinct circuits, per SPEC §12.4.
            let mut prefs = StreamPrefs::new();
            prefs.set_isolation(IsolationToken::new());
            let t = Instant::now();
            match tokio::time::timeout(
                CONNECT_TIMEOUT,
                client.connect_with_prefs(("example.com", 80), &prefs),
            )
            .await
            {
                Ok(Ok(stream)) => {
                    samples.push(t.elapsed().as_secs_f64() * 1e3);
                    drop(stream);
                }
                _ => failures += 1,
            }
        }
        Ok::<_, String>((bootstrap_ms, samples, failures))
    });

    match outcome {
        Ok((bootstrap_ms, mut samples, failures)) => {
            result.metric("bootstrap_ms", bootstrap_ms.round());
            result.metric("isolated_connects_attempted", CIRCUITS);
            result.metric("isolated_connect_failures", failures);
            if !samples.is_empty() {
                result.metric("connect_p50_ms", percentile_ms(&mut samples, 50.0).round());
                result.metric("connect_p99_ms", percentile_ms(&mut samples, 99.0).round());
            }
            result.metric(
                "arti_rss_delta_mb",
                (rss_bytes().saturating_sub(rss0)) / (1024 * 1024),
            );
            let state_bytes = dir_size(&state_dir) + dir_size(&cache_dir);
            result.metric("state_plus_cache_mb", state_bytes / (1024 * 1024));
            result.note("informational — grounds anon deadline / circuit cap / RSS budget");
        }
        Err(e) => result.note(format!("anon suite failed: {e}")),
    }

    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}

fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for e in entries.flatten() {
            if let Ok(md) = e.metadata() {
                if md.is_file() {
                    total += md.len();
                } else if md.is_dir() {
                    total += dir_size(&e.path());
                }
            }
        }
    }
    total
}
