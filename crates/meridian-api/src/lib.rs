//! HTTP API surface (SPEC §10): axum router + tower layers (timeout, concurrency
//! limit, governor rate limit, compression, request-id), `/metrics`, `/healthz`.
//!
//! Privacy-by-default: no query text or client IPs in logs/metrics; `x-request-id`
//! is random per request; no cookies; CORS closed (SPEC §13.4).
//!
//! Status: Phase-0 scaffold — router lands in Phase 1.
