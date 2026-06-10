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
}

fn build_components(config: MeridianConfig) -> Result<Components, String> {
    let lanes = Arc::new(LaneRegistry::new(&config.lanes).map_err(|e| e.to_string())?);
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
    let ingestor = Arc::new(
        Ingestor::new(
            index.clone(),
            fetcher.clone(),
            embedder.clone(),
            vectors.clone(),
            &config.index.data_dir,
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
    // ε-greedy engine-routing bandit (SPEC §11), persisted in egress.redb.
    let bandit = if config.searx.enabled {
        Some(Arc::new(
            Bandit::open(&config.index.data_dir, 0.1).map_err(|e| e.to_string())?,
        ))
    } else {
        None
    };
    // Deep reranker: real on the gnu/ort image; inert (degrades mode=deep) on the
    // musl image or if the model is absent.
    let reranker = Arc::new(Reranker::load(&config.models.dir).unwrap_or_else(|e| {
        tracing::info!(reason = %e, "deep rerank unavailable; mode=deep will degrade to LTR");
        Reranker::unavailable()
    }));
    let planner = Arc::new(Planner::new(
        index,
        embedder,
        vectors,
        searx,
        reranker,
        bandit,
        lanes,
        shed.clone(),
        &config.search,
        &config.vector,
    ));
    Ok(Components {
        config,
        planner,
        ingestor,
        fetcher,
        shed,
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
        let ingestor_for_shutdown = components.ingestor.clone();
        let state = AppState::new(
            components.config,
            components.planner,
            components.ingestor,
            components.fetcher,
            components.shed,
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
