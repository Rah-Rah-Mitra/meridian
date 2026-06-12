//! `meridiand` — the single Meridian process (SPEC §2): config → privacy
//! telemetry → rayon pool → components → axum serve. Subcommands:
//!   meridiand                      serve (default)
//!   meridiand --healthcheck        TCP probe of /healthz (scratch-image friendly)
//!   meridiand ingest-file <jsonl>  bulk-load {"title","body"} lines (dev tooling)

use meridian_api::state::AppState;
use meridian_common::MeridianConfig;
use meridian_common::shed::ShedState;
use meridian_egress::LaneRegistry;
use meridian_embed::Embedder;
use meridian_fetch::Fetcher;
use meridian_index::lexical::LexicalIndex;
use meridian_query::ingest::{IngestText, Ingestor};
use meridian_query::planner::Planner;
use meridian_rerank::Reranker;
use meridian_searx::bandit::Bandit;
use meridian_searx::client::SearxClient;
use meridian_vector::VectorStore;
use std::process::ExitCode;

// SPEC §8: mimalloc as the global allocator (secure mode off). Not a luxury:
// musl's mallocng never consolidates the mixed-lifetime pattern this process
// produces (moka cache entries pinned among 1000-candidate transient vecs) —
// the Phase-6 soak measured ~20x RSS amplification per cached search and an
// unbounded climb into the shed rungs. mimalloc's sharded heaps + page purge
// hold the same load flat. 16KB-page kernel compatibility validated on the
// Pi 5 deployment target (Phase-6 soak).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
use std::sync::Arc;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--healthcheck") => healthcheck(),
        Some("ingest-file") => match args.get(1) {
            Some(path) => ingest_file(path),
            None => {
                eprintln!("usage: meridiand ingest-file <corpus.jsonl>");
                ExitCode::FAILURE
            }
        },
        Some("--version") => {
            println!("meridiand {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => serve(),
    }
}

fn load_config() -> Result<MeridianConfig, ExitCode> {
    MeridianConfig::load().map_err(|e| {
        eprintln!("config error: {e}");
        ExitCode::FAILURE
    })
}

/// SPEC §7.2: one global rayon pool, 4 threads, nice 5 — CPU stages only.
fn init_rayon() {
    let result = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .thread_name(|i| format!("meridian-cpu-{i}"))
        .start_handler(|_| {
            // Lower CPU-stage priority below the IO runtime (SPEC §7.2).
            #[allow(unsafe_code)]
            unsafe {
                libc::nice(5);
            }
        })
        .build_global();
    if let Err(e) = result {
        tracing::warn!(error = %e, "rayon global pool already initialized");
    }
}

struct Components {
    config: MeridianConfig,
    planner: Arc<Planner>,
    ingestor: Arc<Ingestor>,
    fetcher: Arc<Fetcher>,
    shed: Arc<ShedState>,
    lanes: Arc<LaneRegistry>,
    analytics: Option<Arc<meridian_analytics::Analytics>>,
    decision_log: Option<Arc<meridian_searx::decision_log::DecisionLog>>,
}

fn build_components(config: MeridianConfig) -> Result<Components, String> {
    let lanes = Arc::new(
        LaneRegistry::new(&config.lanes, &config.index.data_dir).map_err(|e| e.to_string())?,
    );
    let shed = Arc::new(ShedState::default());
    let fetcher = Arc::new(Fetcher::new(
        lanes.clone(),
        &config.fetch,
        config.lanes.allow_onion,
    ));
    let index = Arc::new(LexicalIndex::open_or_create(&config.index).map_err(|e| e.to_string())?);
    let embedder = Arc::new(Embedder::load(&config.models.dir).map_err(|e| e.to_string())?);
    let vectors = Arc::new(
        VectorStore::open_or_create(&config.index.data_dir, embedder.dims(), &config.vector)
            .map_err(|e| e.to_string())?,
    );
    // Gazetteer (SPEC §3 geo-tagging): optional artifact; absence = geo off.
    let gazetteer = match meridian_geo::Gazetteer::open(&config.models.gazetteer_path()) {
        Ok(g) => {
            tracing::info!(places = g.len(), "gazetteer loaded; ingest geo-tagging on");
            Some(Arc::new(g))
        }
        Err(_) => {
            tracing::info!("no gazetteer artifact; ingest geo-tagging off");
            None
        }
    };
    let ingestor = Arc::new(
        Ingestor::new(
            index.clone(),
            fetcher.clone(),
            embedder.clone(),
            vectors.clone(),
            &config.index.data_dir,
            gazetteer,
            &config.ingest,
            &config.vector,
        )
        .map_err(|e| e.to_string())?,
    );
    let searx = if config.searx.enabled {
        Some(Arc::new(
            SearxClient::new(
                &config.searx.url,
                config.search.searx_deadline_ms,
                lanes.direct().hedge_after(),
            )
            .map_err(|e| e.to_string())?,
        ))
    } else {
        None
    };
    // Tor-proxied metasearch backend (SPEC §12.4): generous deadline, no hedging
    // EVER (single attempt per request is part of anon citizenship).
    let searx_anon = match (config.searx.enabled, &config.searx.anon_url) {
        (true, Some(url)) => Some(Arc::new(
            SearxClient::new(url, config.search.anon_searx_deadline_ms, None)
                .map_err(|e| e.to_string())?,
        )),
        _ => None,
    };
    // ε-greedy engine-routing bandit (SPEC §11), persisted in egress.redb.
    let bandit = if config.searx.enabled {
        Some(Arc::new(
            Bandit::open(&config.index.data_dir, 0.1).map_err(|e| e.to_string())?,
        ))
    } else {
        None
    };
    // Per-decision routing log (Phase 9, ADR-24): shares the bandit's redb
    // file, OFF by default through v0.3.x. Startup runs the TTL/size sweep so
    // a node that was down past a retention boundary catches up immediately.
    let decision_log = match (&bandit, config.searx.decision_log) {
        (Some(b), true) => {
            let dl = Arc::new(
                meridian_searx::decision_log::DecisionLog::new(b.database())
                    .map_err(|e| e.to_string())?,
            );
            let today = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                / 86_400) as u32;
            dl.sweep(today).map_err(|e| e.to_string())?;
            tracing::info!(
                "per-decision routing log ENABLED (ADR-24: coarse buckets only, 30d TTL, \
                 k-anonymity floor, 20MB cap)"
            );
            Some(dl)
        }
        _ => None,
    };
    // Analytics (SPEC §9.4): operator opt-in. When on, the store also feeds the
    // LTR domain_prior; when off, the prior is the cold default (0 everywhere).
    let analytics = if config.analytics.enabled {
        Some(Arc::new(
            meridian_analytics::Analytics::open(
                &config.index.data_dir,
                config.analytics.retention_days,
                config.analytics.max_edges,
            )
            .map_err(|e| e.to_string())?,
        ))
    } else {
        None
    };
    let priors: Arc<dyn meridian_common::prior::DomainPriorSource> = match &analytics {
        Some(a) => Arc::new(meridian_analytics::StorePrior(a.store().clone())),
        None => Arc::new(meridian_common::prior::NoPrior),
    };
    // Deep reranker: real on the gnu/ort image; inert (degrades mode=deep) on the
    // musl image or if the model is absent.
    let reranker = Arc::new(Reranker::load(&config.models.dir).unwrap_or_else(|e| {
        tracing::info!(reason = %e, "deep rerank unavailable; mode=deep will degrade to LTR");
        Reranker::unavailable()
    }));
    // Evidence layer (Phase 7, ADR-18): the config kill-switch decides whether
    // the planner gets a sketch read handle at all — `None` = no evidence block.
    let sketches = config.evidence.enabled.then(|| ingestor.sketch_reader());
    let planner = Arc::new(Planner::new(
        index,
        embedder,
        vectors,
        searx,
        searx_anon,
        reranker,
        bandit,
        decision_log.clone(),
        lanes.clone(),
        shed.clone(),
        config.lanes.anon.max_concurrent_searches,
        priors,
        sketches,
        &config.search,
        &config.vector,
    ));
    Ok(Components {
        config,
        planner,
        ingestor,
        fetcher,
        shed,
        lanes,
        analytics,
        decision_log,
    })
}

fn serve() -> ExitCode {
    let config = match load_config() {
        Ok(c) => c,
        Err(code) => return code,
    };
    meridian_privacy::telemetry::init(&config.privacy);
    init_rayon();

    let bearer = match meridian_privacy::secret::load_bearer_token(&config.auth) {
        Ok(token) => token,
        Err(e) => {
            tracing::error!(error = %e, "bearer token load failed");
            return ExitCode::FAILURE;
        }
    };
    if bearer.is_none() {
        tracing::warn!("no bearer token configured — mutating endpoints will refuse requests");
    }

    let metrics_handle =
        match metrics_exporter_prometheus::PrometheusBuilder::new().install_recorder() {
            Ok(h) => h,
            Err(e) => {
                tracing::error!(error = %e, "metrics recorder install failed");
                return ExitCode::FAILURE;
            }
        };

    // SPEC §7.2: tokio multi-thread runtime, 4 workers, IO-bound work only.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(error = %e, "tokio runtime build failed");
            return ExitCode::FAILURE;
        }
    };

    runtime.block_on(async move {
        let components = match build_components(config) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "component init failed");
                return ExitCode::FAILURE;
            }
        };
        let bind = format!(
            "{}:{}",
            components.config.server.bind, components.config.server.port
        );
        meridian_common::shed::spawn_monitor(
            components.shed.clone(),
            components.config.index.data_dir.clone(),
        );
        // Bring up anon (Arti bootstrap + SOCKS listener) / region (verification
        // loops) when enabled. Failure here is fatal: an operator who turned a
        // lane on must not get a process that silently lacks it (fail-visible).
        if let Err(e) = components.lanes.start().await {
            tracing::error!(error = %e, "egress lane startup failed");
            return ExitCode::FAILURE;
        }
        // Retention observability (SPEC §6.1 caps): per-store disk gauges every
        // 30 min. The ACTING sweeps live with their stores (analytics daily
        // compaction below; geocode sweep when the geocoder is wired); dedup
        // hashes + forget tombstones are deliberately permanent (correctness).
        {
            let data_dir = components.config.index.data_dir.clone();
            let planner = components.planner.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(30 * 60));
                loop {
                    tick.tick().await;
                    for store in ["dedup.redb", "egress.redb", "geo.redb", "analytics.redb"] {
                        let bytes = std::fs::metadata(data_dir.join(store))
                            .map(|m| m.len())
                            .unwrap_or(0);
                        metrics::gauge!("meridian_store_bytes", "store" => store).set(bytes as f64);
                    }
                    // In-RAM cache occupancy (Phase-6 soak finding): weighted
                    // caps are only trustworthy when observable.
                    for (cache, entries, weighted) in planner.cache_stats() {
                        metrics::gauge!("meridian_cache_entries", "cache" => cache)
                            .set(entries as f64);
                        metrics::gauge!("meridian_cache_weighted_bytes", "cache" => cache)
                            .set(weighted as f64);
                    }
                }
            });
        }
        // Analytics jobs (SPEC §9.4): 15-min GDELT pull on the direct lane;
        // daily retention compaction + PageRank → domain_prior.
        if let Some(analytics) = components.analytics.clone() {
            let lanes = components.lanes.clone();
            let cfg = components.config.analytics.clone();
            tokio::spawn(async move {
                let mut tick =
                    tokio::time::interval(std::time::Duration::from_secs(cfg.pull_interval_secs));
                loop {
                    tick.tick().await;
                    match lanes.resolve(&meridian_egress::Lane::Direct) {
                        Ok(client) => match analytics.pull_once(&client, &cfg.gdelt_base).await {
                            Ok(rows) if rows > 0 => {
                                tracing::info!(rows, "gdelt slice applied");
                            }
                            Ok(_) => {}
                            Err(e) => tracing::warn!(error = %e, "gdelt pull failed"),
                        },
                        Err(e) => tracing::warn!(error = %e, "gdelt pull: no direct lane: {e}"),
                    }
                }
            });
            let analytics = components.analytics.clone().expect("checked above");
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
                tick.tick().await; // first tick is immediate; jobs run at startup
                loop {
                    match analytics.compact() {
                        Ok((c, e)) => {
                            tracing::info!(
                                counters_dropped = c,
                                edges_dropped = e,
                                "analytics retention"
                            )
                        }
                        Err(e) => tracing::warn!(error = %e, "analytics retention failed"),
                    }
                    match analytics.recompute_priors() {
                        Ok(n) => tracing::info!(domains = n, "domain priors recomputed"),
                        Err(e) => tracing::warn!(error = %e, "pagerank failed"),
                    }
                    tick.tick().await;
                }
            });
        }
        let ingestor_for_shutdown = components.ingestor.clone();
        let state = AppState::new(
            components.config,
            components.planner,
            components.ingestor,
            components.fetcher,
            components.shed,
            components.analytics.clone(),
            components.decision_log.clone(),
            bearer,
            metrics_handle,
        );
        let router = meridian_api::router(state);

        let listener = match tokio::net::TcpListener::bind(&bind).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(error = %e, "bind failed");
                return ExitCode::FAILURE;
            }
        };
        tracing::info!(bind = %bind, version = env!("CARGO_PKG_VERSION"), "meridiand serving");

        let serve = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal());
        let outcome = serve.await;
        // Vectors persist on coarse cadence during serving; flush the tail.
        if let Err(e) = ingestor_for_shutdown.flush_vectors() {
            tracing::warn!(error = %e, "vector flush on shutdown failed");
        }
        match outcome {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                tracing::error!(error = %e, "server error");
                ExitCode::FAILURE
            }
        }
    })
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("sigterm");
    tokio::select! {
        _ = ctrl_c => {},
        _ = sigterm.recv() => {},
    }
    tracing::info!("shutdown signal received");
}

/// Self-contained /healthz probe for the scratch image (no shell, no curl).
fn healthcheck() -> ExitCode {
    use std::io::{Read, Write};
    let port = std::env::var("MERIDIAN_SERVER__PORT").unwrap_or_else(|_| "8080".to_owned());
    let addr = format!("127.0.0.1:{port}");
    let Ok(mut stream) = std::net::TcpStream::connect(&addr) else {
        return ExitCode::FAILURE;
    };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    if stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    let mut buf = [0u8; 64];
    let n = stream.read(&mut buf).unwrap_or(0);
    if String::from_utf8_lossy(&buf[..n]).contains(" 200 ") {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Bulk loader for {"title","body"} JSONL (dev tooling; the API path is the
/// production surface). Synthesizes simplewiki URLs for source attribution.
fn ingest_file(path: &str) -> ExitCode {
    let config = match load_config() {
        Ok(c) => c,
        Err(code) => return code,
    };
    meridian_privacy::telemetry::init(&config.privacy);

    let components = match build_components(config) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "component init failed");
            return ExitCode::FAILURE;
        }
    };
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(error = %e, "corpus open failed");
            return ExitCode::FAILURE;
        }
    };

    use std::io::BufRead;
    let reader = std::io::BufReader::new(file);
    let mut batch: Vec<IngestText> = Vec::with_capacity(1000);
    let mut total_accepted = 0usize;
    let mut total_deduped = 0usize;
    let started = std::time::Instant::now();

    let mut flush = |batch: &mut Vec<IngestText>| -> Result<(), String> {
        if batch.is_empty() {
            return Ok(());
        }
        let stats = components
            .ingestor
            .ingest_batch(batch)
            .map_err(|e| e.to_string())?;
        total_accepted += stats.accepted;
        total_deduped += stats.deduped;
        batch.clear();
        if total_accepted % 10_000 < 1000 {
            eprintln!(
                ">> ingested {total_accepted} (+{total_deduped} deduped) in {:.0}s",
                started.elapsed().as_secs_f64()
            );
        }
        Ok(())
    };

    for line in reader.lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let title = v["title"].as_str().unwrap_or_default().to_owned();
        let body = v["body"].as_str().unwrap_or_default().to_owned();
        if body.is_empty() {
            continue;
        }
        let url = format!(
            "https://simple.wikipedia.org/wiki/{}",
            title.replace(' ', "_")
        );
        batch.push(IngestText {
            text: body,
            url: Some(url),
            title: Some(title),
            ts: None,
        });
        if batch.len() >= 1000 {
            if let Err(e) = flush(&mut batch) {
                tracing::error!(error = %e, "ingest batch failed");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Err(e) = flush(&mut batch) {
        tracing::error!(error = %e, "ingest batch failed");
        return ExitCode::FAILURE;
    }
    if let Err(e) = components.ingestor.flush_vectors() {
        tracing::error!(error = %e, "vector flush failed");
        return ExitCode::FAILURE;
    }
    eprintln!(
        ">> done: {total_accepted} accepted, {total_deduped} deduped, {:.0}s",
        started.elapsed().as_secs_f64()
    );
    ExitCode::SUCCESS
}
