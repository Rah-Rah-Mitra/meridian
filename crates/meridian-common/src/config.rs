//! `meridian.toml` configuration model (SPEC §14: figment file + env, single source).
//!
//! Phase-0 skeleton: section structs exist so every budget in SPEC §6 and every lane
//! lands as a typed, documented config key. Defaults here MUST stay secure-by-default
//! (SPEC §13.4): localhost bind, anon/regions disabled, debug query logging off.

use serde::{Deserialize, Serialize};

/// Root configuration. Loaded by figment (file + `MERIDIAN_` env overrides) in Phase 1.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MeridianConfig {
    pub server: ServerConfig,
    pub privacy: PrivacyConfig,
    pub lanes: LanesConfig,
}

/// HTTP server settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ServerConfig {
    /// Bind address. Secure default: loopback only (SPEC §13.4).
    pub bind: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_owned(),
            port: 8080,
        }
    }
}

/// Privacy guardrail switches (SPEC §13.4). Shipped defaults are the strict ones
/// (`false` everywhere, hence derived).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PrivacyConfig {
    /// Raw query text at TRACE level only, behind this flag. OFF in all shipped configs.
    pub debug_query_logging: bool,
}

/// Egress lane enablement (SPEC §12). Only `direct` ships enabled; everything else
/// defaults to `false` (hence derived).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LanesConfig {
    /// Enable the Tor (Arti) anon lane. Default off.
    pub anon_enabled: bool,
    /// Enable WireGuard region lanes (requires the `regions` compose profile). Default off.
    pub regions_enabled: bool,
    /// Allow fetching `.onion` URLs (anon lane only). Default off; when off, `.onion`
    /// is rejected on every lane (SPEC §12.4).
    pub allow_onion: bool,
}
