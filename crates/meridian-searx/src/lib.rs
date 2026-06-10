//! SearXNG JSON client (SPEC §5): two endpoints — the direct instance and the
//! Tor-proxied `searxng-anon` instance — normalized to a common result type; engine
//! health stats; ε-greedy bandit (ε=0.1) over engine subsets keyed by intent class,
//! arms persisted in redb. Anon-lane outcomes never feed shared bandit state
//! (SPEC §12.4 cache isolation).
//!
//! Status: Phase-0 scaffold — lands in Phase 1 (direct) / Phase 4 (anon endpoint).
