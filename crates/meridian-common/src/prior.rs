//! The LTR `domain_prior` seam (SPEC §11: nightly PageRank → feature).
//!
//! Lives in `meridian-common` so `meridian-analytics` (the producer) and
//! `meridian-query` (the consumer) need no edge between them — the composition
//! root wires the implementation in (SPEC §5 dependency rules).

/// Source of the per-domain prior in [0, 1]; 0 = unknown domain.
pub trait DomainPriorSource: Send + Sync {
    fn domain_prior(&self, domain_hash: u64) -> f32;
}

/// Cold default: no prior knowledge for any domain.
pub struct NoPrior;

impl DomainPriorSource for NoPrior {
    fn domain_prior(&self, _domain_hash: u64) -> f32 {
        0.0
    }
}
