//! Analytics (SPEC §5): GDELT v2 15-min slice puller (stream-parse, NEVER store raw),
//! H3×topic×day counters in redb (zstd, 90-day TTL with daily compaction), nightly
//! petgraph PageRank → domain_prior for the LTR features, `/v1/trends` queries.
//!
//! Note (verified 2026-06-10): data.gdeltproject.org serves an invalid TLS cert —
//! fetch over HTTP and verify the manifest MD5 instead.
//!
//! Status: Phase-0 scaffold — lands in Phase 5.
