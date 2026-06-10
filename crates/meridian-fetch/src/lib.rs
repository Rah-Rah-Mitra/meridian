//! Fetch ladder (SPEC §3, §13.1): cache → HTTP GET on the requested egress lane →
//! readability extraction. SSRF guard runs on EVERY lane; robots.txt honored on every
//! lane; per-domain token buckets are SHARED ACROSS LANES so multiple lanes cannot
//! multiply pressure on one origin (SPEC §12.5 invariant 4).
//!
//! Extraction: Phase-1 bake-off (dom_smoothie vs readability-rs) decides the library.
//! Fetched bytes are capped (5MB), parsed, and DISCARDED — only url/title/snippet and
//! fast fields are stored.
//!
//! Status: Phase-0 scaffold — lands in Phase 1.
