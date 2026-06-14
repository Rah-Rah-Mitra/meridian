//! HTTP API surface (SPEC §10): axum router with explicit guardrail middleware.
//!
//! Privacy-by-default (SPEC §13.4): no query text or client IPs in logs (the
//! access event carries route/status/latency only), `x-request-id` is random per
//! request, no `Set-Cookie`, CORS closed (no CORS layer exists), `/metrics` is
//! aggregate-only with bounded label cardinality.

pub mod problem;
pub mod state;
pub mod tuning;
pub mod ui;

/// Image/release version surfaced by `GET /v1/config`. The release image version
/// is the git tag (release.yml), NOT Cargo.toml (pinned at 0.1.0) — so it's
/// injected at build time via `MERIDIAN_BUILD_VERSION` and falls back to the
/// crate version for local/dev builds.
pub const VERSION: &str = match option_env!("MERIDIAN_BUILD_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

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
    let api = Router::new()
        .route("/v1/search", get(search))
        .route("/v1/ingest", post(ingest))
        .route("/v1/fetch", get(fetch))
        .route("/v1/lanes", get(lanes))
        .route("/v1/geo/heatmap", get(heatmap))
        .route("/v1/trends", get(trends))
        .route("/v1/forget", post(forget))
        .route("/v1/decision-log", get(decision_log_status))
        .route("/v1/decision-log/wipe", post(decision_log_wipe))
        .route("/v1/decision-log/ope", get(decision_log_ope))
        .route("/v1/config", get(config_endpoint))
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics_endpoint))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            guardrails,
        ))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(body_limit))
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(state);

    // The embedded operator console (candidate-ADR `05-ui-track`): a GET-only
    // static shell served straight from `include_bytes!` bytes. It is merged
    // OUTSIDE the guardrail + body-limit layer on purpose — immutable in-binary
    // assets are not a rate-limit/concurrency-shed surface, and the JS only
    // issues GETs to the existing `/v1/*` handlers above (which keep their own
    // guardrails). The shell is unauthenticated and inherits the existing
    // "enable auth + TLS before exposing" operator gate; per-request bearer is
    // attached by the JS to guarded endpoints only.
    let ui = Router::new()
        .route("/ui", get(ui::index))
        .route("/ui/", get(ui::index))
        .route("/ui/{*path}", get(ui::asset));

    Router::new().merge(api).merge(ui)
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
    /// `vantages` runs the query over direct AND anon and attaches the
    /// `divergence` block (Phase 8, ADR-22). Explicit per-request opt-in —
    /// the query is intentionally observable from two vantages.
    #[serde(default)]
    compare: Option<String>,
    /// VoI deep-mode fetching (v0.4.0, ADR-26): fetch up to N result pages
    /// to re-score on full text. Requires mode=deep on the direct lane;
    /// capped by `search.deep_fetch_max`. Per-query egress to result
    /// domains — see privacy.md.
    #[serde(default)]
    fetch_budget: Option<usize>,
    /// Answer mode (v0.5.0, ADR-29): switch the fetch selector to the
    /// single-best objective and attach the extractive `best_passage`
    /// block. Requires fetch_budget ≥ 1 (and therefore inherits every
    /// fetch_budget restriction). Same egress surface as fetch_budget.
    #[serde(default)]
    answer: Option<bool>,
    /// `evidence` (v0.6.0): cluster-aware result diversification — each
    /// ADR-18 cluster's canonical document leads, syndicated copies defer.
    /// The MMR replacement; `mmr` was withdrawn by suite 13b pre-release.
    #[serde(default)]
    diversity: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    // Geo constraint (SPEC §10): lat+lon+radius_km together, OR h3.
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    lon: Option<f64>,
    #[serde(default)]
    radius_km: Option<f64>,
    #[serde(default)]
    h3: Option<String>,
    // ts window (unix seconds, inclusive).
    #[serde(default)]
    after: Option<u64>,
    #[serde(default)]
    before: Option<u64>,
    // ----- per-request tuning overrides (ADR-30) -----
    // Each replaces the config default for THIS request only; clamped to a safe
    // range server-side (saturating, never 400) and reflected in
    // `applied_overrides`. Persists nothing; never logged with the query text.
    // `deny_unknown_fields` (above) makes a misspelled knob a loud 400.
    #[serde(default)]
    ov_bm25_top_k: Option<usize>,
    #[serde(default)]
    ov_vector_top_k: Option<usize>,
    #[serde(default)]
    ov_max_per_domain: Option<usize>,
    #[serde(default)]
    ov_searx_deadline_ms: Option<u64>,
    #[serde(default)]
    ov_deep_fetch_max: Option<usize>,
    #[serde(default)]
    ov_deep_fetch_deadline_ms: Option<u64>,
    #[serde(default)]
    ov_answer_deadline_ms: Option<u64>,
    #[serde(default)]
    ov_answer_passage_cap: Option<usize>,
    #[serde(default)]
    ov_answer_abstain_threshold: Option<f32>,
    #[serde(default)]
    ov_answer_corroborate: Option<bool>,
    #[serde(default)]
    ov_answer_corroboration_tau: Option<f32>,
    #[serde(default)]
    ov_compare_jitter_ms_max: Option<u64>,
    #[serde(default)]
    ov_compare_noise_floor_p90: Option<f64>,
    #[serde(default)]
    ov_rrf_k: Option<u32>,
    #[serde(default)]
    ov_containment_tau: Option<f64>,
    #[serde(default)]
    ov_rerank_deadline_ms: Option<u64>,
    #[serde(default)]
    ov_answer_passage_deadline_ms: Option<u64>,
    #[serde(default)]
    ov_answer_corroboration_deadline_ms: Option<u64>,
}

/// `h3` accepts the canonical hex form (e.g. `871f1d489ffffff`) or decimal.
fn parse_h3(raw: &str) -> Result<u64, Problem> {
    u64::from_str_radix(raw, 16)
        .or_else(|_| raw.parse::<u64>())
        .map_err(|_| Problem::new(StatusCode::BAD_REQUEST, "invalid h3", "hex or decimal cell"))
}

fn parse_geo(
    params: &SearchParams,
) -> Result<Option<meridian_query::planner::GeoConstraint>, Problem> {
    use meridian_query::planner::GeoConstraint;
    match (
        params.lat,
        params.lon,
        params.radius_km,
        params.h3.as_deref(),
    ) {
        (None, None, None, None) => Ok(None),
        (_, _, _, Some(h3)) if params.lat.is_none() && params.lon.is_none() => {
            Ok(Some(GeoConstraint::Cell(parse_h3(h3)?)))
        }
        (Some(lat), Some(lon), radius, None) => Ok(Some(GeoConstraint::Point {
            lat,
            lon,
            radius_km: radius.unwrap_or(25.0),
        })),
        _ => Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "invalid geo constraint",
            "use lat+lon(+radius_km) OR h3, not both",
        )),
    }
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
) -> Result<Json<serde_json::Value>, Problem> {
    if state.config.auth.require_bearer_for_search {
        bearer_ok(&state, &headers)?;
    }
    // Per-request tuning overrides (ADR-30): clamp each ov_* to its safe range
    // (saturating) and record the changed knobs for the honest `applied_overrides`
    // readout. Nothing here is persisted or logged with the query text.
    let (overrides, applied_overrides) = build_overrides(&params);
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
    let geo = parse_geo(&params)?;
    let compare = match params.compare.as_deref() {
        None => false,
        Some("vantages") => true,
        Some(_) => {
            return Err(Problem::new(
                StatusCode::BAD_REQUEST,
                "invalid compare",
                "compare=vantages",
            ));
        }
    };
    let lane = parse_lane(params.lane.as_deref())?;
    let fetch_budget = match params.fetch_budget {
        None | Some(0) => 0,
        Some(n) => {
            if !matches!(mode, SearchMode::Deep) {
                return Err(Problem::new(
                    StatusCode::BAD_REQUEST,
                    "fetch_budget requires deep mode",
                    "mode=deep",
                ));
            }
            if !matches!(lane, Lane::Direct) {
                return Err(Problem::new(
                    StatusCode::BAD_REQUEST,
                    "fetch_budget is direct-lane only",
                    "query-time fetching over anon/region lanes does not fit their                      latency/privacy envelope (ADR-26; see privacy.md)",
                ));
            }
            if compare {
                return Err(Problem::new(
                    StatusCode::BAD_REQUEST,
                    "fetch_budget cannot combine with compare",
                    "a compare must not fetch — its anon half would inherit the budget",
                ));
            }
            // The override (already clamped ≤ the absolute deep_fetch_max ceiling)
            // is the operator's explicit per-query egress budget.
            n.min(
                overrides
                    .deep_fetch_max
                    .unwrap_or(state.config.search.deep_fetch_max),
            )
        }
    };
    let diversity_evidence = match params.diversity.as_deref() {
        None => false,
        Some("evidence") => true,
        Some(_) => {
            return Err(Problem::new(
                StatusCode::BAD_REQUEST,
                "invalid diversity",
                "diversity=evidence (mmr was withdrawn by its own gate — see api.md)",
            ));
        }
    };
    let answer = params.answer.unwrap_or(false);
    if answer && fetch_budget == 0 {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "answer requires fetch_budget",
            "answer mode reads pages to extract a passage — set fetch_budget ≥ 1              (mode=deep, direct lane; ADR-29)",
        ));
    }
    let request = SearchRequest {
        q: params.q,
        mode,
        scope,
        lane,
        limit: params.limit.unwrap_or(state.config.search.default_limit),
        geo,
        after: params.after,
        before: params.before,
        bypass_cache: false,
        pin_engines: false,
        fetch_budget,
        answer,
        diversity_evidence,
        overrides,
    };
    if compare {
        // Compare governs lanes itself and is web-scoped by definition; a
        // local-only or lane-pinned compare is a caller error, not a guess.
        if !matches!(request.scope, Scope::Web) {
            return Err(Problem::new(
                StatusCode::BAD_REQUEST,
                "compare=vantages requires scope=web",
                "the comparison is between web vantages",
            ));
        }
        if params.lane.is_some() {
            return Err(Problem::new(
                StatusCode::BAD_REQUEST,
                "compare=vantages governs lanes",
                "omit the lane parameter",
            ));
        }
        // The decorrelation jitter must FIT the synchronous request ceiling:
        // ceiling − direct deadline − anon deadline − 1s slack. With defaults
        // (12s − 2.5s − 8s − 1s) that leaves ~0.5s — weak decorrelation, which
        // is the honest truth of a sync API (privacy.md: strong decorrelation
        // wants the configured window AND a raised request ceiling; an async
        // compare is a Phase-10 candidate). Found live: 30s jitter inside a
        // 12s ceiling made default-config compares 504 most of the time.
        let budget_ms = state.config.server.request_timeout_ms.saturating_sub(
            state.config.search.searx_deadline_ms
                + state.config.search.anon_searx_deadline_ms
                + 1_000,
        );
        let jitter_max = request
            .overrides
            .compare_jitter_ms_max
            .unwrap_or(state.config.search.compare_jitter_ms_max);
        let noise_floor = request
            .overrides
            .compare_noise_floor_p90
            .unwrap_or(state.config.search.compare_noise_floor_p90);
        let effective_jitter = jitter_max.min(budget_ms);
        let response = meridian_query::compare::compare_vantages(
            &state.planner,
            request,
            effective_jitter,
            noise_floor,
        )
        .await
        .map_err(problem::plan_error)?;
        return with_applied(response, applied_overrides);
    }
    let response = state
        .planner
        .search(request)
        .await
        .map_err(problem::plan_error)?;
    with_applied(response, applied_overrides)
}

/// Build the clamped per-request overrides (ADR-30) + the `applied_overrides`
/// readout map (only the knobs the operator changed, with their clamped effective
/// value). Clamping saturates to `tuning::KNOBS` bounds — the same ranges
/// `GET /v1/config` advertises, so the drawer's range can never exceed what's
/// enforced.
fn build_overrides(
    p: &SearchParams,
) -> (
    meridian_query::planner::SearchOverrides,
    std::collections::BTreeMap<String, serde_json::Value>,
) {
    use meridian_query::planner::SearchOverrides;
    use tuning::{clamp_f32, clamp_f64, clamp_u32, clamp_u64, clamp_usize};
    let mut o = SearchOverrides::default();
    let mut a: std::collections::BTreeMap<String, serde_json::Value> =
        std::collections::BTreeMap::new();
    // usize knobs
    if let Some(v) = p.ov_bm25_top_k {
        let c = clamp_usize("bm25_top_k", v);
        o.bm25_top_k = Some(c);
        a.insert("bm25_top_k".to_owned(), c.into());
    }
    if let Some(v) = p.ov_vector_top_k {
        let c = clamp_usize("vector_top_k", v);
        o.vector_top_k = Some(c);
        a.insert("vector_top_k".to_owned(), c.into());
    }
    if let Some(v) = p.ov_max_per_domain {
        let c = clamp_usize("max_per_domain", v);
        o.max_per_domain = Some(c);
        a.insert("max_per_domain".to_owned(), c.into());
    }
    if let Some(v) = p.ov_deep_fetch_max {
        let c = clamp_usize("deep_fetch_max", v);
        o.deep_fetch_max = Some(c);
        a.insert("deep_fetch_max".to_owned(), c.into());
    }
    if let Some(v) = p.ov_answer_passage_cap {
        let c = clamp_usize("answer_passage_cap", v);
        o.answer_passage_cap = Some(c);
        a.insert("answer_passage_cap".to_owned(), c.into());
    }
    // u64 (ms) knobs
    if let Some(v) = p.ov_searx_deadline_ms {
        let c = clamp_u64("searx_deadline_ms", v);
        o.searx_deadline_ms = Some(c);
        a.insert("searx_deadline_ms".to_owned(), c.into());
    }
    if let Some(v) = p.ov_deep_fetch_deadline_ms {
        let c = clamp_u64("deep_fetch_deadline_ms", v);
        o.deep_fetch_deadline_ms = Some(c);
        a.insert("deep_fetch_deadline_ms".to_owned(), c.into());
    }
    if let Some(v) = p.ov_answer_deadline_ms {
        let c = clamp_u64("answer_deadline_ms", v);
        o.answer_deadline_ms = Some(c);
        a.insert("answer_deadline_ms".to_owned(), c.into());
    }
    if let Some(v) = p.ov_compare_jitter_ms_max {
        let c = clamp_u64("compare_jitter_ms_max", v);
        o.compare_jitter_ms_max = Some(c);
        a.insert("compare_jitter_ms_max".to_owned(), c.into());
    }
    if let Some(v) = p.ov_rerank_deadline_ms {
        let c = clamp_u64("rerank_deadline_ms", v);
        o.rerank_deadline_ms = Some(c);
        a.insert("rerank_deadline_ms".to_owned(), c.into());
    }
    if let Some(v) = p.ov_answer_passage_deadline_ms {
        let c = clamp_u64("answer_passage_deadline_ms", v);
        o.answer_passage_deadline_ms = Some(c);
        a.insert("answer_passage_deadline_ms".to_owned(), c.into());
    }
    if let Some(v) = p.ov_answer_corroboration_deadline_ms {
        let c = clamp_u64("answer_corroboration_deadline_ms", v);
        o.answer_corroboration_deadline_ms = Some(c);
        a.insert("answer_corroboration_deadline_ms".to_owned(), c.into());
    }
    // u32
    if let Some(v) = p.ov_rrf_k {
        let c = clamp_u32("rrf_k", v);
        o.rrf_k = Some(c);
        a.insert("rrf_k".to_owned(), c.into());
    }
    // f32 (CE logits)
    if let Some(v) = p.ov_answer_abstain_threshold {
        let c = clamp_f32("answer_abstain_threshold", v);
        o.answer_abstain_threshold = Some(c);
        a.insert("answer_abstain_threshold".to_owned(), c.into());
    }
    if let Some(v) = p.ov_answer_corroboration_tau {
        let c = clamp_f32("answer_corroboration_tau", v);
        o.answer_corroboration_tau = Some(c);
        a.insert("answer_corroboration_tau".to_owned(), c.into());
    }
    // f64
    if let Some(v) = p.ov_compare_noise_floor_p90 {
        let c = clamp_f64("compare_noise_floor_p90", v);
        o.compare_noise_floor_p90 = Some(c);
        a.insert("compare_noise_floor_p90".to_owned(), c.into());
    }
    if let Some(v) = p.ov_containment_tau {
        let c = clamp_f64("containment_tau", v);
        o.containment_tau = Some(c);
        a.insert("containment_tau".to_owned(), c.into());
    }
    // bool (no clamp)
    if let Some(v) = p.ov_answer_corroborate {
        o.answer_corroborate = Some(v);
        a.insert("answer_corroborate".to_owned(), v.into());
    }
    (o, a)
}

/// Serialize the search response and, when overrides were applied, attach the
/// `applied_overrides` readout (the clamped effective values) — returned to the
/// caller only, never persisted.
fn with_applied(
    resp: SearchResponse,
    applied: std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<Json<serde_json::Value>, Problem> {
    let mut v = serde_json::to_value(&resp).map_err(|e| {
        Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialize failed",
            e.to_string(),
        )
    })?;
    if !applied.is_empty() {
        if let Some(obj) = v.as_object_mut() {
            obj.insert(
                "applied_overrides".to_owned(),
                serde_json::Value::Object(applied.into_iter().collect()),
            );
        }
    }
    Ok(Json(v))
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
    /// Tombstoned content refused at re-ingest (SPEC §10 `/v1/forget`).
    /// v0.2.0 addition: operators could not previously distinguish a dedup
    /// from a forget-refusal in the response.
    refused: usize,
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
    let mut refused = 0;
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
        refused += stats.refused;
    }
    for (url, lane) in &urls {
        match state.ingestor.ingest_url(url, lane).await {
            Ok(stats) => {
                accepted += stats.accepted;
                deduped += stats.deduped;
                refused += stats.refused;
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
            refused,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeatmapParams {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    res: Option<u8>,
    /// Window like "7d" / "24h"; default 7d. Docs without a ts are excluded
    /// only when a window narrower than "all" is requested.
    #[serde(default)]
    window: Option<String>,
}

fn parse_window(raw: Option<&str>) -> Result<Option<u64>, Problem> {
    let raw = raw.unwrap_or("7d");
    if raw == "all" {
        return Ok(None);
    }
    let (num, unit) = raw.split_at(raw.len().saturating_sub(1));
    let n: u64 = num.parse().map_err(|_| {
        Problem::new(
            StatusCode::BAD_REQUEST,
            "invalid window",
            "e.g. 7d, 24h, all",
        )
    })?;
    let secs = match unit {
        "d" => n * 86_400,
        "h" => n * 3_600,
        _ => {
            return Err(Problem::new(
                StatusCode::BAD_REQUEST,
                "invalid window",
                "e.g. 7d, 24h, all",
            ));
        }
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(Some(now.saturating_sub(secs)))
}

/// SPEC §10 `GET /v1/geo/heatmap`:
/// `{res, schema, cells:[{h3,count,lat,lon,z,q_value,significant}]}`.
/// Phase 7 (ADR-21): per-cell Getis-Ord Gi* hot-spot statistics over the
/// k-ring-1 neighborhood with BH-FDR across the scanned cells — `significant`
/// is the defensible flag; raw counts stay for explainability.
async fn heatmap(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HeatmapParams>,
) -> Result<Json<serde_json::Value>, Problem> {
    let res = params.res.unwrap_or(5);
    let after_ts = parse_window(params.window.as_deref())?;
    let cells = state
        .planner
        .heatmap(params.q.clone(), res, after_ts)
        .await
        .map_err(problem::plan_error)?;
    let stats = meridian_analytics::stats::heatmap_stats(&cells);
    let cells: Vec<serde_json::Value> = cells
        .into_iter()
        .zip(stats)
        .map(|((cell, count), s)| {
            let centroid = meridian_geo::h3::cell_to_latlng(cell);
            serde_json::json!({
                "h3": format!("{cell:x}"),
                "count": count,
                "lat": centroid.map(|c| c.0),
                "lon": centroid.map(|c| c.1),
                "z": s.z,
                "q_value": s.q_value,
                "significant": s.significant,
            })
        })
        .collect();
    Ok(Json(
        serde_json::json!({ "res": res, "schema": 1, "cells": cells }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrendsParams {
    /// GDELT EventRootCode (1..=20).
    #[serde(default)]
    topic: Option<u8>,
    /// H3 res-5 cell (hex or decimal).
    #[serde(default)]
    h3: Option<String>,
    #[serde(default)]
    window: Option<String>,
}

/// SPEC §10 `GET /v1/trends`: time series + top movers from the analytics
/// counters. 404 when analytics is disabled (nothing exists to query).
async fn trends(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TrendsParams>,
) -> Result<Json<meridian_analytics::TrendsReport>, Problem> {
    let Some(analytics) = &state.analytics else {
        return Err(Problem::new(
            StatusCode::NOT_FOUND,
            "analytics disabled",
            "enable [analytics] in config",
        ));
    };
    let after_ts = parse_window(params.window.as_deref())?;
    let now_day = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        / 86_400) as u32;
    let from_day = after_ts.map(|t| (t / 86_400) as u32).unwrap_or(0);
    let h3_r5 = params.h3.as_deref().map(parse_h3).transpose()?;
    let store = analytics.store().clone();
    let report = tokio::task::spawn_blocking(move || {
        meridian_analytics::trends(&store, from_day, now_day, params.topic, h3_r5)
    })
    .await
    .map_err(|_| {
        Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "trends failed",
            "internal",
        )
    })?
    .map_err(|e| {
        Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "trends failed",
            e.to_string(),
        )
    })?;
    Ok(Json(report))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ForgetBody {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    content_hash: Option<String>,
    #[serde(default)]
    purge_caches: Option<bool>,
}

/// SPEC §10 `POST /v1/forget`: operator deletion path. Removes matching docs
/// from Tantivy + USearch, tombstones content hashes (re-ingest refused), and
/// optionally drops the caches (default true — cached SERPs may embed the
/// forgotten snippet).
async fn forget(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ForgetBody>,
) -> Result<Json<serde_json::Value>, Problem> {
    bearer_ok(&state, &headers)?;
    let selectors =
        body.url.is_some() as u8 + body.domain.is_some() as u8 + body.content_hash.is_some() as u8;
    if selectors != 1 {
        return Err(Problem::new(
            StatusCode::BAD_REQUEST,
            "invalid forget request",
            "exactly one of url | domain | content_hash",
        ));
    }
    let ingestor = state.ingestor.clone();
    let forget_result = tokio::task::spawn_blocking(move || {
        if let Some(url) = body.url {
            let key = meridian_index::lexical::url_key(&url);
            ingestor.forget_keys(&[key])
        } else if let Some(domain) = body.domain {
            ingestor.forget_domain(&domain)
        } else {
            let raw = body.content_hash.unwrap_or_default();
            let bytes = (0..raw.len().saturating_sub(1))
                .step_by(2)
                .map(|i| u8::from_str_radix(&raw[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>();
            match bytes {
                Ok(hash) if hash.len() == 16 => ingestor.forget_content_hash(&hash),
                _ => Err(meridian_query::ingest::IngestError::Dedup(
                    "content_hash must be 32 hex chars (16 bytes)".to_owned(),
                )),
            }
        }
    })
    .await
    .map_err(|_| {
        Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "forget failed",
            "internal",
        )
    })?;

    let removed = forget_result.map_err(|e| match e {
        meridian_query::ingest::IngestError::Dedup(msg) if msg.contains("hex") => {
            Problem::new(StatusCode::BAD_REQUEST, "invalid content_hash", msg)
        }
        other => Problem::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "forget failed",
            other.to_string(),
        ),
    })?;

    let purge = body.purge_caches.unwrap_or(true);
    if purge {
        state.planner.purge_caches();
        state.fetcher.purge_cache();
    }
    // Deletion is an operator action worth an audit line — but the SELECTOR is
    // user data and never logged (SPEC §13.4).
    tracing::info!(removed, caches_purged = purge, "forget executed");
    Ok(Json(serde_json::json!({
        "removed": removed,
        "caches_purged": purge,
    })))
}

/// `GET /v1/decision-log` (ADR-24 operator surface): row count, the same
/// bytes-approximation the 20MB cap enforces, retained day range. Bearer-gated
/// (routing telemetry is operator data); 404 while `searx.decision_log` is off.
async fn decision_log_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, Problem> {
    bearer_ok(&state, &headers)?;
    let Some(dl) = &state.decision_log else {
        return Err(Problem::new(
            StatusCode::NOT_FOUND,
            "decision log disabled",
            "searx.decision_log = false",
        ));
    };
    let dl = dl.clone();
    let stats = tokio::task::spawn_blocking(move || dl.stats())
        .await
        .map_err(|_| Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "status failed", "join"))?
        .map_err(|e| {
            Problem::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "status failed",
                e.to_string(),
            )
        })?;
    Ok(Json(serde_json::json!({
        "enabled": true,
        "rows": stats.rows,
        "approx_bytes": stats.approx_bytes,
        "oldest_day": stats.oldest_day,
        "newest_day": stats.newest_day,
        "retention_days": meridian_searx::decision_log::RETENTION_DAYS,
        "max_bytes": meridian_searx::decision_log::MAX_BYTES,
    })))
}

/// `POST /v1/decision-log/wipe` (ADR-24 erasure path): drop every retained
/// decision row, now. The count is audit-logged; rows carry no user data to
/// begin with (13-byte coarse buckets), but the wipe is still the operator's
/// kill switch for the whole telemetry class.
async fn decision_log_wipe(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, Problem> {
    bearer_ok(&state, &headers)?;
    let Some(dl) = &state.decision_log else {
        return Err(Problem::new(
            StatusCode::NOT_FOUND,
            "decision log disabled",
            "searx.decision_log = false",
        ));
    };
    let dl = dl.clone();
    let removed = tokio::task::spawn_blocking(move || dl.wipe())
        .await
        .map_err(|_| Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "wipe failed", "join"))?
        .map_err(|e| {
            Problem::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "wipe failed",
                e.to_string(),
            )
        })?;
    tracing::info!(removed, "decision log wiped");
    Ok(Json(serde_json::json!({ "removed": removed })))
}

/// `GET /v1/decision-log/ope` — the ADR-25 ship-gate report. Temporal 80/20
/// split of the retained log: the linear-TS candidate trains on the older
/// 80%, and its GREEDY policy is evaluated doubly-robust against the
/// incumbent's realized reward on the held-out 20% (bootstrap 95% CI).
/// Generalized rows (k-anonymity floor) carry no replayable features and are
/// skipped — counted in the response. The verdict encodes ADR-25 verbatim:
/// pass only if the CI excludes zero FROM ABOVE on ≥10k total decisions;
/// a straddling CI is `inconclusive` (ε-greedy stays), an all-negative CI is
/// `negative`. This endpoint only reports — flipping `contextual_policy` is
/// the operator's call, recorded at the v0.4.0 exit either way.
async fn decision_log_ope(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, Problem> {
    bearer_ok(&state, &headers)?;
    let Some(dl) = &state.decision_log else {
        return Err(Problem::new(
            StatusCode::NOT_FOUND,
            "decision log disabled",
            "searx.decision_log = false",
        ));
    };
    let dl = dl.clone();
    let report = tokio::task::spawn_blocking(move || -> Result<serde_json::Value, String> {
        use meridian_searx::contextual::ContextualPolicy;
        use meridian_searx::decision_log::Context;
        use meridian_searx::ope::{Logged, dr_uplift_ci};

        let decisions = dl.read_all().map_err(|e| e.to_string())?;
        let n_total = decisions.len();
        let split = (n_total * 4) / 5;
        let (train, eval) = decisions.split_at(split);

        let policy = ContextualPolicy::from_decisions(train);
        let mut skipped_generalized = 0usize;
        let log: Vec<Logged> = eval
            .iter()
            .filter_map(|d| match &d.context {
                Some(ctx) => Some(Logged {
                    context: ctx.bucket_id(),
                    arm: d.arm as usize,
                    propensity: d.propensity as f64,
                    reward: d.reward as u8 as f64,
                }),
                None => {
                    skipped_generalized += 1;
                    None
                }
            })
            .collect();

        let candidate = |bucket: u32| policy.greedy(&Context::from_bucket_id(bucket));
        let report = dr_uplift_ci(&log, candidate, 200, 0x0ADE_2026);

        const MIN_DECISIONS: usize = 10_000;
        let verdict = if n_total < MIN_DECISIONS {
            "insufficient_data"
        } else if report.ci_lo > 0.0 {
            "pass"
        } else if report.ci_hi < 0.0 {
            "negative"
        } else {
            "inconclusive"
        };
        Ok(serde_json::json!({
            "n_total": n_total,
            "n_train": split,
            "n_eval": log.len(),
            "n_eval_generalized_skipped": skipped_generalized,
            "report": report,
            "gate": {
                "min_decisions": MIN_DECISIONS,
                "rule": "95% bootstrap CI of DR uplift must exclude zero from above (ADR-25)",
                "verdict": verdict,
            },
        }))
    })
    .await
    .map_err(|_| Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "ope failed", "join"))?
    .map_err(|e| Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "ope failed", e))?;
    Ok(Json(report))
}

/// `GET /v1/config` (ADR-30): the effective per-request tuning defaults + safe
/// clamp ranges (so the console's tuning drawer pre-fills from THIS deployment),
/// the read-only deploy/index config with the env var per field, and feature
/// availability (cross-encoder → deep/answer/corroboration live vs dormant).
/// Bearer-optional — gated only when `auth.require_bearer_for_search`, like
/// search. NEVER serializes secrets: no bearer, no token file, no internal
/// searx URLs/paths (only an `anon_configured` boolean).
async fn config_endpoint(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, Problem> {
    if state.config.auth.require_bearer_for_search {
        bearer_ok(&state, &headers)?;
    }
    let s = &state.config.search;
    let v = &state.config.vector;
    let e = &state.config.evidence;
    let a = &state.config.analytics;
    let ce = state.planner.reranker_available();

    // Tunable defaults from LIVE config (so they reflect env overrides), paired
    // with the bounds/unit/rationale from the shared KNOBS table.
    let defaults: &[(&str, serde_json::Value)] = &[
        ("bm25_top_k", s.bm25_top_k.into()),
        ("vector_top_k", v.top_k.into()),
        ("max_per_domain", s.max_per_domain.into()),
        ("searx_deadline_ms", s.searx_deadline_ms.into()),
        ("deep_fetch_max", s.deep_fetch_max.into()),
        ("deep_fetch_deadline_ms", s.deep_fetch_deadline_ms.into()),
        ("answer_deadline_ms", s.answer_deadline_ms.into()),
        ("answer_passage_cap", s.answer_passage_cap.into()),
        (
            "answer_abstain_threshold",
            s.answer_abstain_threshold.into(),
        ),
        (
            "answer_corroboration_tau",
            s.answer_corroboration_tau.into(),
        ),
        ("compare_jitter_ms_max", s.compare_jitter_ms_max.into()),
        ("compare_noise_floor_p90", s.compare_noise_floor_p90.into()),
        ("rrf_k", s.rrf_k.into()),
        ("containment_tau", e.containment_tau.into()),
        ("rerank_deadline_ms", s.rerank_deadline_ms.into()),
        (
            "answer_passage_deadline_ms",
            s.answer_passage_deadline_ms.into(),
        ),
        (
            "answer_corroboration_deadline_ms",
            s.answer_corroboration_deadline_ms.into(),
        ),
    ];
    let mut tunable = serde_json::Map::new();
    for (key, def) in defaults.iter() {
        if let Some(k) = tuning::KNOBS.iter().find(|k| k.name == *key) {
            tunable.insert(
                (*key).to_owned(),
                serde_json::json!({
                    "default": def.clone(),
                    "min": k.min,
                    "max": k.max,
                    "unit": k.unit,
                    "rationale": k.rationale,
                }),
            );
        }
    }
    // The one bool knob (no numeric range).
    tunable.insert(
        "answer_corroborate".to_owned(),
        serde_json::json!({
            "default": s.answer_corroborate,
            "rationale": "C1 claim-level corroboration: count distinct independent evidence clusters whose top passage the cross-encoder finds states the same claim (suite-20: precision 0.97 / recall 1.0). Needs the cross-encoder — dormant in the scratch image.",
        }),
    );

    Ok(Json(serde_json::json!({
        "schema": 1,
        "version": crate::VERSION,
        "features": {
            "cross_encoder_present": ce,
            "deep_available": ce,
            "answer_available": ce,
            "corroboration_available": ce && s.answer_corroborate,
            "evidence_enabled": e.enabled,
            "analytics_enabled": a.enabled,
            "trends_available": state.analytics.is_some(),
            "decision_log_enabled": state.config.searx.decision_log,
            "contextual_policy": state.config.searx.contextual_policy,
            "anon_configured": state.config.searx.anon_url.is_some(),
            "anon_lane_enabled": state.config.lanes.anon_enabled,
            "regions_lane_enabled": state.config.lanes.regions_enabled,
        },
        "tunable": serde_json::Value::Object(tunable),
        "deploy": {
            "index": {
                "shingle_k": meridian_index::sketch::SHINGLE_K,
                "sketch_bins": meridian_index::sketch::BINS,
                "min_match_bins": meridian_index::sketch::MIN_MATCH_BINS,
                "merge_max_docs": state.config.index.merge_max_docs,
                "note": "INDEX-TIME: baked into stored sketches/segments at ingest — changing needs a RE-INGEST.",
                "env_prefix": "MERIDIAN_INDEX__",
            },
            "vector": {
                "connectivity": v.connectivity,
                "expansion_add": v.expansion_add,
                "expansion_search": v.expansion_search,
                "top_k": v.top_k,
                "binary_quantization": v.binary_quantization,
                "note": "connectivity/expansion_add are INDEX-REBUILD HNSW build params. expansion_search (ef) is query-time but applied store-wide in this build (not a per-request override — varying it per query would race shared index state); set it deploy-wide.",
                "env_prefix": "MERIDIAN_VECTOR__",
            },
            "server": {
                "concurrency_limit": state.config.server.concurrency_limit,
                "request_timeout_ms": state.config.server.request_timeout_ms,
                "rate_limit_per_sec": state.config.server.rate_limit_per_sec,
                "rate_limit_burst": state.config.server.rate_limit_burst,
                "note": "DEPLOY-TIME: loaded at startup (figment).",
                "env_prefix": "MERIDIAN_SERVER__",
            },
            "analytics": {
                "enabled": a.enabled,
                "pull_interval_secs": a.pull_interval_secs,
                "retention_days": a.retention_days,
                "note": "GDELT third-party feed (opt-in). Enables /v1/trends.",
                "env_prefix": "MERIDIAN_ANALYTICS__",
            },
            "lanes": {
                "anon_enabled": state.config.lanes.anon_enabled,
                "regions_enabled": state.config.lanes.regions_enabled,
                "allow_onion": state.config.lanes.allow_onion,
                "note": "DEPLOY-TIME egress lanes. Region lanes also need per-region WireGuard endpoints (see docs/region-lanes.md).",
                "env_prefix": "MERIDIAN_LANES__",
            },
            "searx": {
                "decision_log": state.config.searx.decision_log,
                "contextual_policy": state.config.searx.contextual_policy,
                "anon_configured": state.config.searx.anon_url.is_some(),
                "note": "contextual_policy is ADR-25-gated (≥10k logged decisions + DR-uplift CI excluding zero); stays OFF until GET /v1/decision-log/ope reports pass.",
                "env_prefix": "MERIDIAN_SEARX__",
            },
        },
    })))
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
