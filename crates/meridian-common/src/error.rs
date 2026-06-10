//! Workspace-wide error taxonomy. Crate-local errors convert into `MeridianError`
//! at the API boundary, where they map onto RFC 7807 problem responses (SPEC §10).

/// Top-level error type returned across crate boundaries.
///
/// Variants are intentionally coarse; per-crate error enums carry the detail and
/// convert via `#[from]` as the crates gain real implementations.
#[derive(Debug, thiserror::Error)]
pub enum MeridianError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("requested capability is not yet implemented: {0}")]
    Unimplemented(&'static str),
}
