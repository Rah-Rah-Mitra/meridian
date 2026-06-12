//! `meridian.toml` configuration model (SPEC §14: figment file + env, single source).
//!
//! Every SPEC §6 budget and every lane is a typed, documented key. Defaults MUST
//! stay secure-by-default (SPEC §13.4): localhost bind, anon/regions disabled,
//! debug query logging off, CORS closed (no config key — it is simply never
//! enabled in Phase 1). Secrets are NOT part of this model — only paths/env names
//! that point at them (SPEC §13.5); see `meridian_privacy::secret`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Root configuration. Loaded by [`MeridianConfig::load`] from `meridian.toml`
/// (path overridable via `MERIDIAN_CONFIG`) merged with `MERIDIAN_*` env vars
/// (nested keys split on `__`, e.g. `MERIDIAN_SERVER__PORT=9090`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MeridianConfig {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub privacy: PrivacyConfig,
    pub lanes: LanesConfig,
    pub index: IndexConfig,
    pub search: SearchConfig,
    pub searx: SearxConfig,
    pub fetch: FetchConfig,
    pub ingest: IngestConfig,
    pub models: ModelsConfig,
    pub vector: VectorConfig,
    pub analytics: AnalyticsConfig,
    pub evidence: EvidenceConfig,
}

impl MeridianConfig {
    /// figment: defaults ← `meridian.toml` (optional) ← `MERIDIAN_*` env.
    pub fn load() -> Result<Self, crate::MeridianError> {
        use figment::Figment;
        use figment::providers::{Env, Format, Serialized, Toml};
        let path = std::env::var("MERIDIAN_CONFIG").unwrap_or_else(|_| "meridian.toml".to_owned());
        // BEARER_TOKEN is a secret and must never transit the config layer
        // (SPEC §13.5); CONFIG/LOG steer the loader itself.
        Figment::from(Serialized::defaults(Self::default()))
            .merge(Toml::file(path))
            .merge(
                Env::prefixed("MERIDIAN_")
                    .ignore(&["bearer_token", "config", "log"])
                    .split("__"),
            )
            .extract()
            .map_err(|e| crate::MeridianError::Config(e.to_string()))
    }
}

/// HTTP server settings (SPEC §7.2 concurrency, §13.2 limits, §13.4 bind).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ServerConfig {
    /// Bind address. Secure default: loopback only (SPEC §13.4).
    pub bind: String,
    pub port: u16,
    /// In-flight request cap; queueing beyond it sheds with 429 (SPEC §7.2).
    pub concurrency_limit: usize,
    /// Whole-request ceiling — a last-resort guard, NOT the enforcement
    /// mechanism (per-stage deadlines are). Must exceed the SLOWEST lane
    /// budget: anon metasearch is 8s (SPEC §6.3), so 3s (the Phase-1 fast-path
    /// value) clipped legitimate anon searches with API-level 504s.
    pub request_timeout_ms: u64,
    /// Request body cap (SPEC §13.2: 1MB).
    pub body_limit_bytes: usize,
    /// Public-endpoint rate limit per hashed client IP (SPEC §13.2: 5 rps burst 20).
    pub rate_limit_per_sec: u32,
    pub rate_limit_burst: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_owned(),
            port: 8080,
            concurrency_limit: 8,
            request_timeout_ms: 12_000,
            body_limit_bytes: 1_048_576,
            rate_limit_per_sec: 5,
            rate_limit_burst: 20,
        }
    }
}

/// Bearer-auth wiring (SPEC §10). The token VALUE lives in the env var or file —
/// never in this struct, never serialized (SPEC §13.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AuthConfig {
    /// Env var read for the bearer token.
    pub token_env: String,
    /// Optional 0600 file holding the token (takes precedence over env if set).
    pub token_file: Option<PathBuf>,
    /// Mutating endpoints always require the bearer; this additionally gates
    /// `/v1/search` (recommended when exposed beyond localhost).
    pub require_bearer_for_search: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            token_env: "MERIDIAN_BEARER_TOKEN".to_owned(),
            token_file: None,
            require_bearer_for_search: false,
        }
    }
}

/// Privacy guardrail switches (SPEC §13.4). Shipped defaults are the strict ones.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PrivacyConfig {
    /// Raw query text at TRACE level only, behind this flag. OFF in all shipped
    /// configs; logs a startup warning when enabled.
    pub debug_query_logging: bool,
}

/// Egress lane enablement (SPEC §12). Only `direct` ships enabled.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LanesConfig {
    pub anon_enabled: bool,
    pub regions_enabled: bool,
    /// `.onion` rejected on every lane while false (SPEC §12.4).
    pub allow_onion: bool,
    pub direct: DirectLaneConfig,
    pub anon: AnonLaneConfig,
    /// `region:<id>` lanes, keyed by id (SPEC §12.3). Each is an operator-managed
    /// WireGuard interface's source IP; see `deploy/wg-lane-templates/`.
    pub regions: std::collections::BTreeMap<String, RegionConfig>,
}

/// Anon lane (Tor via embedded Arti) settings — SPEC §12.4. The lane is OFF by
/// default; these keys only take effect with `anon_enabled = true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AnonLaneConfig {
    /// In-process SOCKS5 listener for `searxng-anon` AND Meridian's own anon
    /// fetches. Default loopback; the compose anon profile widens it to the
    /// container's `back`-network interface (never published to the host).
    pub socks_listen: String,
    /// Concurrent Tor circuit cap (SPEC §12.4 rate-limit citizenship).
    pub max_circuits: usize,
    /// Concurrent anon *searches* admitted by the planner (excess → 503).
    pub max_concurrent_searches: usize,
    pub connect_timeout_ms: u64,
    /// Generous total deadline (SPEC §13.1: 30s on anon).
    pub total_timeout_ms: u64,
    /// Arti state+cache root. Defaults to `<index.data_dir>/arti` (the single
    /// mutable volume, SPEC §9.7).
    pub state_dir: Option<PathBuf>,
}

impl Default for AnonLaneConfig {
    fn default() -> Self {
        Self {
            socks_listen: "127.0.0.1:9150".to_owned(),
            max_circuits: 6,
            max_concurrent_searches: 2,
            connect_timeout_ms: 10_000,
            total_timeout_ms: 30_000,
            state_dir: None,
        }
    }
}

/// One `region:<id>` lane (SPEC §12.3): bind outbound sockets to the WireGuard
/// interface's source IP; policy routing (operator-applied) does the steering.
/// Meridian never sees WireGuard keys — only this source address.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RegionConfig {
    /// Source IP of the operator's wg interface for this region.
    pub source_ip: std::net::IpAddr,
    /// IP-echo endpoint fetched through the lane on bring-up; the observed
    /// egress IP must match `expected_ip` or the lane goes Degraded and refuses
    /// traffic (routing-leak tripwire, SPEC §12.3).
    pub verify_url: String,
    /// Expected egress IP. Exact match, or prefix match when the value ends in
    /// `.` or `:` (e.g. `203.0.113.` for a /24). `None` = only verify the lane
    /// can egress at all (the echo fetch must succeed).
    pub expected_ip: Option<String>,
}

impl Default for RegionConfig {
    fn default() -> Self {
        Self {
            source_ip: std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            verify_url: "https://checkip.amazonaws.com/".to_owned(),
            expected_ip: None,
        }
    }
}

/// Direct-lane client settings (SPEC §12.2, §13.2 honest UA).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DirectLaneConfig {
    /// Appended to the UA: `MeridianSearch/<ver> (+<contact_url>)`. Operators
    /// MUST set a real contact URL before fetching third-party sites.
    pub contact_url: String,
    /// Hedge a duplicate metasearch request after this delay (direct lane ONLY,
    /// SPEC §11). 0 disables hedging.
    pub hedge_after_ms: u64,
    pub connect_timeout_ms: u64,
    pub total_timeout_ms: u64,
}

impl Default for DirectLaneConfig {
    fn default() -> Self {
        Self {
            contact_url: "https://example.invalid/meridian-bot".to_owned(),
            hedge_after_ms: 300,
            connect_timeout_ms: 3_000,
            total_timeout_ms: 10_000,
        }
    }
}

/// Lexical index settings (SPEC §7.2, §9.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct IndexConfig {
    /// Single mutable state volume (SPEC §9.7). Index, redb stores, everything.
    pub data_dir: PathBuf,
    pub writer_threads: usize,
    pub writer_heap_bytes: usize,
    /// LogMergePolicy doc cap — derived from SPEC §9.1's 256MB via measured
    /// bytes/doc (bench: 527 B/doc → ~500k docs ≈ 256MB).
    pub merge_max_docs: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("data"),
            writer_threads: 2,
            writer_heap_bytes: 256 * 1024 * 1024,
            merge_max_docs: 500_000,
        }
    }
}

/// Query planning knobs (SPEC §11).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SearchConfig {
    pub default_limit: usize,
    pub max_limit: usize,
    /// BM25 candidate depth (SPEC §11: top-1000).
    pub bm25_top_k: usize,
    /// SearXNG fan-out deadline on the direct lane (SPEC §6.3: 800ms).
    pub searx_deadline_ms: u64,
    /// SearXNG-anon fan-out deadline — generous, Tor circuits add seconds
    /// (SPEC §6.3 / Phase-4 exit gate: anon metasearch p50 ≤8s).
    pub anon_searx_deadline_ms: u64,
    /// Domain diversity cap in the final ranking (SPEC §11: 3).
    pub max_per_domain: usize,
    /// Compare-vantages (Phase 8, ADR-22): max randomized delay before the
    /// anon-side dispatch (risk #18 timing decorrelation). Default ON (30s
    /// window); 0 = operator explicitly accepts the correlation risk.
    pub compare_jitter_ms_max: u64,
    /// Same-lane JSD noise floor (p90) from this deployment's suite-12 probe —
    /// the per-request `exceeds_floor` reference. Measured, not invented.
    pub compare_noise_floor_p90: f64,
    /// VoI deep-mode fetching (Phase 9, ADR-26): hard cap on the per-request
    /// `fetch_budget` param. Fetching at query time creates per-query egress
    /// to result domains — opt-in per request, direct lane only.
    pub deep_fetch_max: usize,
    /// Wall-clock ceiling for the whole fetch phase inside one deep request —
    /// sized so the deep p50 ≤2.5s gate holds (SPEC §16 P9).
    pub deep_fetch_deadline_ms: u64,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 10,
            max_limit: 50,
            bm25_top_k: 1_000,
            searx_deadline_ms: 800,
            anon_searx_deadline_ms: 8_000,
            max_per_domain: 3,
            compare_jitter_ms_max: 30_000,
            // 2026-06-11 probe (docs/plan/bench/, anon lane p90).
            compare_noise_floor_p90: 0.30,
            deep_fetch_max: 2,
            deep_fetch_deadline_ms: 1_200,
        }
    }
}

/// SearXNG sidecar endpoints (SPEC §4).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SearxConfig {
    pub enabled: bool,
    /// Direct instance, reachable on the `back` network only.
    pub url: String,
    /// Tor-proxied instance (`searxng-anon`, compose anon profile). `None` =
    /// anon searches fail closed with "no metasearch backend" even when the
    /// anon lane itself is up.
    pub anon_url: Option<String>,
    /// Per-decision routing log (Phase 9, ADR-24): 13-byte coarse-bucket rows
    /// feeding offline policy evaluation. OFF by default through v0.3.x —
    /// flipping it on is the v0.4.0 exit decision (ADR-25). Anon-lane
    /// decisions are never logged regardless of this flag.
    pub decision_log: bool,
    /// Linear Thompson-sampling contextual routing (Phase 9, ADR-25).
    /// EXPERIMENTAL and OFF by default: the ship gate (doubly-robust uplift
    /// CI excluding zero on ≥10k logged decisions) has not been evaluated;
    /// enabling this early means routing on an unvalidated policy. Requires
    /// `decision_log = true` (the log is both its training data and its
    /// persistence).
    pub contextual_policy: bool,
}

impl Default for SearxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            url: "http://searxng:8080".to_owned(),
            anon_url: None,
            decision_log: false,
            contextual_policy: false,
        }
    }
}

/// Fetch ladder limits (SPEC §13.1, §13.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FetchConfig {
    /// Streamed response cap (SPEC §13.1: 5MB).
    pub max_body_bytes: usize,
    pub max_redirects: usize,
    /// Per-domain budget, GLOBAL ACROSS LANES (SPEC §13.2: 1 req / 2s sustained).
    pub per_domain_interval_ms: u64,
    pub per_domain_burst: u32,
    pub robots_ttl_secs: u64,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            max_body_bytes: 5 * 1024 * 1024,
            max_redirects: 3,
            per_domain_interval_ms: 2_000,
            per_domain_burst: 2,
            robots_ttl_secs: 86_400,
        }
    }
}

/// Model artifact location (baked into the image at /models; SPEC §9.6).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ModelsConfig {
    pub dir: PathBuf,
    /// Gazetteer fst for ingest geo-tagging. Default: `<dir>/gazetteer.fst`,
    /// loaded only if present (geo-tagging silently off otherwise).
    pub gazetteer_file: Option<PathBuf>,
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            dir: PathBuf::from("models"),
            gazetteer_file: None,
        }
    }
}

impl ModelsConfig {
    /// The gazetteer path to probe: explicit config, or `<dir>/gazetteer.fst`.
    pub fn gazetteer_path(&self) -> PathBuf {
        self.gazetteer_file
            .clone()
            .unwrap_or_else(|| self.dir.join("gazetteer.fst"))
    }
}

/// Evidence layer (Phase 7, ADR-18): source-independence clustering over
/// search results. ON by default (operator decision 2026-06-11) — pure local
/// computation over already-held data, no privacy surface; this is the
/// kill-switch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EvidenceConfig {
    pub enabled: bool,
}

impl Default for EvidenceConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// GDELT analytics (SPEC §9.4). OFF by default — pulling third-party feeds is
/// an operator choice, not a surprise (SPEC §13.4 no-phone-home posture).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AnalyticsConfig {
    pub enabled: bool,
    /// Plain HTTP by upstream necessity (ADR-15: invalid TLS cert; manifest
    /// MD5 verifies integrity).
    pub gdelt_base: String,
    /// GDELT publishes every 15 minutes.
    pub pull_interval_secs: u64,
    /// Counter TTL (SPEC §9.4: 90 days).
    pub retention_days: u32,
    /// Co-occurrence edge cap (PageRank substrate size bound).
    pub max_edges: usize,
}

impl Default for AnalyticsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gdelt_base: "http://data.gdeltproject.org/gdeltv2".to_owned(),
            pull_interval_secs: 900,
            retention_days: 90,
            max_edges: 200_000,
        }
    }
}

/// ANN vector store knobs (SPEC §8.2: M=16, efc=128, ef=64, int8).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VectorConfig {
    pub connectivity: usize,
    pub expansion_add: usize,
    pub expansion_search: usize,
    /// ANN candidate depth in the fusion (SPEC §11: top-200).
    pub top_k: usize,
    /// Persist the vector store after this many newly ingested docs (plus on
    /// graceful shutdown). Full-file save — keep coarse on SD storage.
    pub persist_every_docs: usize,
    /// Binary-quantization + rescore path (SPEC §8.2): for >1.5M-doc corpora.
    /// Config exists per spec; implementation deferred until a corpus needs it
    /// (Profile R tops out at 100k — see Phase-2 exit note).
    pub binary_quantization: bool,
}

impl Default for VectorConfig {
    fn default() -> Self {
        Self {
            connectivity: 16,
            expansion_add: 128,
            expansion_search: 64,
            top_k: 200,
            persist_every_docs: 25_000,
            binary_quantization: false,
        }
    }
}

/// Ingest pipeline limits (SPEC §6.1 hard rules, §10 batch cap).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct IngestConfig {
    pub batch_max: usize,
    /// Snippet generated at ingest so queries never need the body (SPEC §9.2).
    pub snippet_max_chars: usize,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            batch_max: 100,
            snippet_max_chars: 240,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MeridianConfig;

    #[test]
    fn defaults_are_secure() {
        let c = MeridianConfig::default();
        assert_eq!(c.server.bind, "127.0.0.1");
        assert!(!c.privacy.debug_query_logging);
        assert!(!c.lanes.anon_enabled);
        assert!(!c.lanes.regions_enabled);
        assert!(!c.lanes.allow_onion);
        // Anon defaults: loopback-only SOCKS, modest circuit + search budgets.
        assert_eq!(c.lanes.anon.socks_listen, "127.0.0.1:9150");
        assert_eq!(c.lanes.anon.max_circuits, 6);
        assert_eq!(c.lanes.anon.max_concurrent_searches, 2);
        assert!(c.lanes.regions.is_empty());
        assert!(c.searx.anon_url.is_none());
    }
}
