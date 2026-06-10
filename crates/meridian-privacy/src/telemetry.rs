//! Telemetry initialization. Lives here (not meridian-common) because the FIRST
//! layer installed must be the redaction layer — the §5 dependency rules make
//! privacy the lowest crate that can own it (privacy → common only).

use meridian_common::config::PrivacyConfig;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Install the global subscriber: env-filtered (`MERIDIAN_LOG`, default `info`)
/// with the redacting writer as the ONLY output. Call once, before anything logs.
pub fn init(cfg: &PrivacyConfig) {
    let filter = tracing_subscriber::EnvFilter::try_from_env("MERIDIAN_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(crate::redact::RedactLayer::stdout(cfg.debug_query_logging))
        .init();

    if cfg.debug_query_logging {
        // SPEC §13.4: explicit, loud, and off in every shipped config.
        tracing::warn!(
            "privacy.debug_query_logging is ENABLED — raw query text will appear at TRACE level"
        );
    }
}
