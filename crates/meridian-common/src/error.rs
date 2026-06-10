//! Workspace-wide error taxonomy. Crate-local errors convert into `MeridianError`
//! at the API boundary, where they map onto RFC 7807 problem responses (SPEC §10).

/// Top-level error type returned across crate boundaries.
///
/// Variants are deliberately coarse; per-crate error enums carry the detail and
/// convert via `From` impls. NOTE (SPEC §13.4/§13.5): messages must never embed
/// query text, client IPs, or secret material — wrap user data in
/// `meridian_privacy::Redacted` before formatting.
#[derive(Debug, thiserror::Error)]
pub enum MeridianError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("index error: {0}")]
    Index(String),

    #[error("ingest rejected: {0}")]
    Ingest(String),

    #[error("fetch refused: {0}")]
    FetchRefused(String),

    #[error("upstream error: {0}")]
    Upstream(String),

    #[error("lane unavailable: {0}")]
    Lane(String),

    #[error("requested capability is not yet implemented: {0}")]
    Unimplemented(&'static str),
}
