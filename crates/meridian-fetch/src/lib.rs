//! Fetch ladder (SPEC §3, §13.1): cache → HTTP GET on the requested egress lane →
//! readability extraction. SSRF guard runs on EVERY lane; robots.txt honored on
//! every lane; per-domain token buckets are SHARED ACROSS LANES (SPEC §12.5
//! invariant 4). Fetched bytes are capped, parsed, and DISCARDED — only
//! url/title/clean-text survive toward the index (SPEC §6.1).
//!
//! Phase 1: direct lane. The fetch-result cache rung arrives with Moka in
//! Phase 2; the Browser rung is out of scope (SPEC §2).

pub mod budget;
pub mod extract;
pub mod ladder;
pub mod robots;
pub mod ssrf;
pub mod voi;

pub use ladder::{FetchedDoc, Fetcher};

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("invalid url")]
    BadUrl,

    #[error("refused by ssrf guard: {0}")]
    Ssrf(#[from] ssrf::SsrfError),

    #[error("robots.txt could not be checked")]
    RobotsUnavailable,

    #[error("disallowed by robots.txt")]
    RobotsDenied,

    #[error("lane error: {0}")]
    Lane(meridian_egress::EgressError),

    #[error("upstream returned status {0}")]
    Http(u16),

    #[error("response exceeded the size cap")]
    TooLarge,

    #[error("unsupported media type")]
    UnsupportedMediaType,

    #[error("invalid redirect")]
    BadRedirect,

    #[error("too many redirects")]
    TooManyRedirects,

    #[error("network: {0}")]
    Network(String),

    #[error("extraction failed: {0}")]
    Extract(String),
}
