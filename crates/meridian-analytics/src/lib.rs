//! Analytics (SPEC §5, §9.4): GDELT v2 15-min slice puller (stream-parse,
//! NEVER store raw), H3-res-5 × event-root × day counters in `analytics.redb`
//! (90-day TTL, daily compaction), nightly co-occurrence PageRank →
//! `domain_prior` for the LTR (served through
//! `meridian_common::prior::DomainPriorSource` — no crate edge to the query
//! path; the composition root wires it).

pub mod gdelt;
pub mod graph;
pub mod stats;
pub mod store;
pub mod trends;

use meridian_egress::LaneClient;
use std::path::Path;
use std::sync::Arc;

pub use store::{AnalyticsStore, CounterKey, StoreError};
pub use trends::{Mover, TrendsReport, trends};

pub struct Analytics {
    store: Arc<AnalyticsStore>,
    retention_days: u32,
    max_edges: usize,
}

impl Analytics {
    pub fn open(
        data_dir: &Path,
        retention_days: u32,
        max_edges: usize,
    ) -> Result<Self, StoreError> {
        Ok(Self {
            store: Arc::new(AnalyticsStore::open(data_dir)?),
            retention_days,
            max_edges,
        })
    }

    pub fn store(&self) -> &Arc<AnalyticsStore> {
        &self.store
    }

    /// One puller tick: manifest → (skip if already processed) → fetch+verify →
    /// parse → apply. Returns rows applied; `Ok(0)` = nothing new.
    pub async fn pull_once(
        &self,
        client: &LaneClient,
        base: &str,
    ) -> Result<usize, gdelt::GdeltError> {
        let slice = gdelt::latest_export(client, base).await?;
        if self.store.meta("last_slice") == Some(slice.id) {
            return Ok(0);
        }
        let agg = gdelt::pull_slice(client, &slice).await?;
        let rows = agg.rows_geo;
        self.store
            .apply(&agg.counters, &agg.edges)
            .map_err(|e| gdelt::GdeltError::Zip(e.to_string()))?;
        let _ = self.store.set_meta("last_slice", slice.id);
        metrics::counter!("meridian_gdelt_rows_total").increment(rows as u64);
        metrics::gauge!("meridian_analytics_disk_bytes").set(self.store.disk_bytes() as f64);
        Ok(rows)
    }

    /// Daily retention (SPEC §9.4): drop counters older than the TTL, thin the
    /// edge table to its cap. Returns (counters_dropped, edges_dropped).
    pub fn compact(&self) -> Result<(u64, u64), StoreError> {
        let today = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
            / 86_400) as u32;
        let cutoff = today.saturating_sub(self.retention_days);
        let result = self.store.compact(cutoff, self.max_edges);
        metrics::gauge!("meridian_analytics_disk_bytes").set(self.store.disk_bytes() as f64);
        result
    }

    /// Nightly PageRank → prior table (SPEC §11 domain_prior).
    pub fn recompute_priors(&self) -> Result<usize, StoreError> {
        let edges = self.store.edges()?;
        let priors = graph::compute_priors(&edges);
        let n = priors.len();
        self.store.store_priors(&priors)?;
        metrics::gauge!("meridian_domain_priors").set(n as f64);
        Ok(n)
    }
}

/// The LTR `domain_prior` feed (wired into the planner by the composition root).
pub struct StorePrior(pub Arc<AnalyticsStore>);

impl meridian_common::prior::DomainPriorSource for StorePrior {
    fn domain_prior(&self, domain_hash: u64) -> f32 {
        self.0.prior(domain_hash)
    }
}

#[cfg(test)]
mod hash_contract {
    /// The gdelt module re-derives `domain_hash` to avoid an analytics→index
    /// crate edge. This test (dev-dep only) keeps the two recipes identical —
    /// if it ever fails, the prior table and the planner disagree on identity.
    #[test]
    fn domain_hash_recipes_agree() {
        for host in ["example.com", "sub.domain.co.uk", "xn--idn.test"] {
            let ours = {
                let digest = blake3::hash(host.as_bytes());
                u64::from_le_bytes(digest.as_bytes()[..8].try_into().unwrap())
            };
            assert_eq!(ours, meridian_index::lexical::domain_hash(host));
        }
    }
}
