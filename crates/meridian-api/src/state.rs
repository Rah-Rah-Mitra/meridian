//! Shared application state for the router.

use meridian_common::config::MeridianConfig;
use meridian_common::shed::ShedState;
use meridian_privacy::iphash::{IpHasher, RateKey};
use meridian_privacy::secret::SecretString;
use meridian_query::Fetcher;
use meridian_query::ingest::Ingestor;
use meridian_query::planner::Planner;
use std::num::NonZeroU32;
use std::sync::Arc;

type IpLimiter = governor::RateLimiter<
    RateKey,
    governor::state::keyed::DefaultKeyedStateStore<RateKey>,
    governor::clock::DefaultClock,
>;

pub struct AppState {
    pub config: MeridianConfig,
    pub planner: Arc<Planner>,
    pub ingestor: Arc<Ingestor>,
    pub fetcher: Arc<Fetcher>,
    pub shed: Arc<ShedState>,
    /// `/v1/trends` backend; None = analytics disabled (404s).
    pub analytics: Option<Arc<meridian_analytics::Analytics>>,
    /// ADR-24 decision-log admin surface; None = `searx.decision_log` off
    /// (the v0.3.x default) — the endpoints 404.
    pub decision_log: Option<Arc<meridian_searx::decision_log::DecisionLog>>,
    /// None = no token configured → mutating endpoints refuse outright.
    pub bearer: Option<SecretString>,
    pub iphash: IpHasher,
    pub ip_limiter: IpLimiter,
    /// In-flight request cap (SPEC §7.2) — explicit semaphore, shed with 429.
    pub inflight: tokio::sync::Semaphore,
    pub metrics: metrics_exporter_prometheus::PrometheusHandle,
    pub started: std::time::Instant,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: MeridianConfig,
        planner: Arc<Planner>,
        ingestor: Arc<Ingestor>,
        fetcher: Arc<Fetcher>,
        shed: Arc<ShedState>,
        analytics: Option<Arc<meridian_analytics::Analytics>>,
        decision_log: Option<Arc<meridian_searx::decision_log::DecisionLog>>,
        bearer: Option<SecretString>,
        metrics: metrics_exporter_prometheus::PrometheusHandle,
    ) -> Arc<Self> {
        let per_sec = NonZeroU32::new(config.server.rate_limit_per_sec.max(1)).expect(">0");
        let burst = NonZeroU32::new(config.server.rate_limit_burst.max(1)).expect(">0");
        let quota = governor::Quota::per_second(per_sec).allow_burst(burst);
        let inflight = tokio::sync::Semaphore::new(config.server.concurrency_limit.max(1));
        Arc::new(Self {
            planner,
            ingestor,
            fetcher,
            shed,
            analytics,
            decision_log,
            bearer,
            iphash: IpHasher::new(),
            ip_limiter: governor::RateLimiter::keyed(quota),
            inflight,
            metrics,
            started: std::time::Instant::now(),
            config,
        })
    }
}
