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
    /// Whole-request timeout — must exceed the metasearch deadline.
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
            request_timeout_ms: 3_000,
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
    /// Domain diversity cap in the final ranking (SPEC §11: 3).
    pub max_per_domain: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            default_limit: 10,
            max_limit: 50,
            bm25_top_k: 1_000,
            searx_deadline_ms: 800,
            max_per_domain: 3,
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
}

impl Default for SearxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            url: "http://searxng:8080".to_owned(),
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
    }
}
