//! SearXNG JSON client (SPEC §5): two endpoints — the direct instance and the
//! Tor-proxied `searxng-anon` instance — normalized to a common result type; engine
//! health stats; ε-greedy bandit (ε=0.1) over engine subsets keyed by intent class,
//! arms persisted in redb. Anon-lane outcomes never feed shared bandit state
//! (SPEC §12.4 cache isolation).
//!
//! Status: Phase 1 — direct instance client live; anon endpoint + bandit
//! routing arrive in Phases 3–4.

pub mod bandit;
pub mod client;
pub mod decision_log;
