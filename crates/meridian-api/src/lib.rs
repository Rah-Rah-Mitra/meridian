//! HTTP API surface (SPEC §10): axum router with explicit guardrail middleware.
//!
//! Privacy-by-default (SPEC §13.4): no query text or client IPs in logs (the
//! access event carries route/status/latency only), `x-request-id` is random per
//! request, no `Set-Cookie`, CORS closed (no CORS layer exists), `/metrics` is
//! aggregate-only with bounded label cardinality.

pub mod problem;
pub mod state;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use meridian_query::ingest::IngestText;
use meridian_query::planner::{SearchRequest, SearchResponse};
use meridian_query::{Lane, Scope, SearchMode};
use problem::Problem;
use serde::Deserialize;
use state::AppState;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

pub fn router(state: Arc<AppState>) -> Router {
    let body_limit = state.config.server.body_limit_bytes;
    Router::new()
        .route("/v1/search", get(search))
        .route("/v1/ingest", post(ingest))
        .route("/v1/fetch", get(fetch))
        .route("/v1/lanes", get(lanes))
        .route("/v1/forget", post(forget))
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics_endpoint))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            guardrails,
        ))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(body_limit))
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(state)
}

/// One middleware, every cross-cutting rule, in order: request-id → rate limit
/// (hashed IP) → concurrency cap → whole-request timeout → access event.
async fn guardrails(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let started = Instant::now();
    let route = request.uri().path().to_owned();
    let request_id = random_request_id();

    // Health/metrics are scrape targets — exempt from rate limiting.
    let limited_route = route.starts_with("/v1/");
    if limited_route {
        // X-Forwarded-For (first hop) only to derive the rate key; dropped
        // immediately after hashing (SPEC §13.4).
        let client_ip = request
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .and_then(|v| v.trim().parse::<std::net::IpAddr>().ok())
            .unwrap_or(peer.ip());
        let key = state.iphash.key(client_ip);
        if state.ip_limiter.check_key(&key).is_err() {
            metrics::counter!("meridian_requests_total", "route" => route.clone(), "status" => "429")
                .increment(1);
            return Problem::new(StatusCode::TOO_MANY_REQUESTS, "rate limited", "slow down")
                .into_response();
        }
    }

    // Concurrency cap (SPEC §7.2): shed instead of queueing unboundedly.
    let _permit = match state.inflight.try_acquire() {
        Ok(p) => Some(p),
        Err(_) if limited_route => {
            metrics::counter!("meridian_requests_total", "route" => route.clone(), "status" => "429")
                .increment(1);
            return Problem::new(
                StatusCode::TOO_MANY_REQUESTS,
                "overloaded",
                "concurrency limit",
            )
            .into_response();
        }
        Err(_) => None,
    };

    let timeout = std::time::Duration::from_millis(state.config.server.request_timeout_ms);
    let mut response = match tokio::time::timeout(timeout, next.run(request)).await {
        Ok(r) => r,
        Err(_) => Problem::new(StatusCode::GATEWAY_TIMEOUT, "timeout", "request timed out")
            .into_response(),
    };

    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&request_id).expect("hex id"),
    );

    let status = response.status().as_u16();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    // Access event: route/status/latency ONLY (SPEC §13.4) — no query string,
    // no IPs; the redaction layer backstops anything a future field adds.
    tracing::info!(route = %route, status, elapsed_ms, "request");
    metrics::counter!("meridian_requests_total", "route" => route.clone(), "status" => status.to_string())
        .increment(1);
    metrics::histogram!("meridian_request_ms", "route" => route).record(elapsed_ms as f64);
    response
}

fn random_request_id() -> String {
    let mut bytes = [0u8; 8];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn bearer_ok(state: &AppState, headers: &HeaderMap) -> Result<(), Problem> {
    let Some(expected) = &state.bearer else {
        return Err(Problem::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "auth not configured",
            "set the bearer token to enable this endpoint",
        ));
    };
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match presented {
        Some(token) if meridian_privacy::secret::token_matches(expected, token) => Ok(()),
        _ => Err(Problem::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "bearer required",
        )),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchParams {
    q: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    lane: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

fn parse_lane(s: Option<&str>) -> Result<Lane, Problem> {
    match s.unwrap_or("direct") {
        "direct" => Ok(Lane::Direct),
        "anon" => Ok(Lane::Anon),
        other => match other.strip_prefix("region:") {
            Some(id) if !id.is_empty() => {
                Ok(Lane::Region(meridian_common::ids::RegionId(id.to_owned())))
            }
            _ => Err(Problem::new(
                StatusCode::BAD_REQUEST,
                "invalid lane",
                "unknown lane",
            )),
        },
    }
}

async fn search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResponse>, Problem> {
    if state.config.auth.require_bearer_for_search {
        bearer_ok(&state, &headers)?;
    }
    let mode = match params.mode.as_deref() {
        None | Some("fast") => SearchMode::Fast,
        Some("deep") => SearchMode::Deep,
        Some(_) => {
            return Err(Problem::new(
                StatusCode::BAD_REQUEST,
                "invalid mode",
                "fast|deep",
            ));
        }
    };
    let scope = match params.scope.as_deref() {
        Some("local") => Scope::Local,
        Some("web") => Scope::Web,
        None | Some("both") => Scope::Both,
        Some(_) => {
            return Err(Problem::new(
                StatusCode::BAD_REQUEST,
                "invalid scope",
                "local|web|both",
            ));
        }
    };
    let request = SearchRequest {
        q: params.q,
        mode,
        scope,
        lane: parse_lane(params.lane.as_deref())?,
        limit: params.limit.unwrap_or(state.config.search.default_limit),
    };
    let response = state
        .planner
        .search(request)
        .await
        .map_err(problem::plan_error)?;
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IngestItem {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    ts: Option<u64>,
    #[serde(default)]
    lane: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct IngestResponse {
    accepted: usize,
    deduped: usize,
    queued: usize,
}

async fn ingest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(items): Json<Vec<IngestItem>>,
) -> Result<(StatusCode, Json<IngestResponse>), Problem> {
    bearer_ok(&state, &headers)?;
    // Shedding ladder (SPEC §8.6): ingest pauses under RSS/thermal/disk stress.
    if !state.shed.ingest_allowed() {
        return Err(Problem::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "ingest paused",
            "shedding under resource pressure; retry later",
        ));
    }
    if items.len() > state.config.ingest.batch_max {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "batch too large",
            format!("max {}", state.config.ingest.batch_max),
        ));
    }

    // Split: text docs ingest synchronously in ONE batch; url docs ride the
    // fetch ladder sequentially (the politeness budget paces them anyway).
    let mut texts = Vec::new();
    let mut urls = Vec::new();
    for item in items {
        match (item.text, item.url) {
            (Some(text), url) => texts.push(IngestText {
                text,
                url,
                title: item.title,
                ts: item.ts,
            }),
            (None, Some(url)) => urls.push((url, parse_lane(item.lane.as_deref())?)),
            (None, None) => {
                return Err(Problem::new(
                    StatusCode::BAD_REQUEST,
                    "invalid item",
                    "need text or url",
                ));
            }
        }
    }

    let mut accepted = 0;
    let mut deduped = 0;
    if !texts.is_empty() {
        let ingestor = state.ingestor.clone();
        let stats = tokio::task::spawn_blocking(move || ingestor.ingest_batch(&texts))
            .await
            .map_err(|_| {
                Problem::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error",
                    "ingest failed",
                )
            })?
            .map_err(|e| {
                Problem::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "ingest failed",
                    e.to_string(),
                )
            })?;
        accepted += stats.accepted;
        deduped += stats.deduped;
    }
    for (url, lane) in &urls {
        match state.ingestor.ingest_url(url, lane).await {
            Ok(stats) => {
                accepted += stats.accepted;
                deduped += stats.deduped;
            }
            Err(_) => {
                // Per-URL failures degrade the count, not the batch; detail
                // stays out of the response to avoid echoing URLs.
            }
        }
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(IngestResponse {
            accepted,
            deduped,
            queued: 0,
        }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FetchParams {
    url: String,
    #[serde(default)]
    lane: Option<String>,
}

async fn fetch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<FetchParams>,
) -> Result<Json<serde_json::Value>, Problem> {
    // Fetch drives outbound traffic — bearer-gated like other write-ish ops.
    bearer_ok(&state, &headers)?;
    let lane = parse_lane(params.lane.as_deref())?;
    let doc = state
        .fetcher
        .fetch_extract(&params.url, &lane)
        .await
        .map_err(|e| problem::fetch_error(&e))?;
    Ok(Json(serde_json::json!({
        "url": doc.url.to_string(),
        "title": doc.title,
        "text": doc.text,
        "http_status": doc.http_status,
    })))
}

async fn lanes(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    use meridian_query::LaneStatus;
    let lanes: Vec<serde_json::Value> = state
        .planner
        .lane_statuses()
        .into_iter()
        .map(|(id, status)| {
            // Stable machine-readable status + optional human detail (SPEC §10).
            let (name, detail) = match &status {
                LaneStatus::Up => ("up", None),
                LaneStatus::Bootstrapping(pct) => ("bootstrapping", Some(format!("{pct}%"))),
                LaneStatus::Degraded(reason) => ("degraded", Some(reason.clone())),
                LaneStatus::Down => ("down", None),
                LaneStatus::Disabled => ("disabled", None),
            };
            serde_json::json!({
                "id": id,
                "status": name,
                "detail": detail,
            })
        })
        .collect();
    Json(serde_json::json!(lanes))
}

async fn forget(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Problem {
    if let Err(p) = bearer_ok(&state, &headers) {
        return p;
    }
    Problem::new(
        StatusCode::NOT_IMPLEMENTED,
        "not implemented",
        "POST /v1/forget arrives in Phase 5",
    )
}

async fn healthz(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "uptime_secs": state.started.elapsed().as_secs(),
    }))
}

async fn metrics_endpoint(State(state): State<Arc<AppState>>) -> Response {
    // Aggregate-only by construction: every label in this process is
    // route/status — never per-IP, per-user, or per-query (SPEC §13.4).
    state.metrics.render().into_response()
}
