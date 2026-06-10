//! The analytics store (`analytics.redb`, SPEC §9.4): H3×topic×day counters,
//! the domain co-occurrence edge accumulator, and the PageRank prior table.
//!
//! Key layout puts the DAY FIRST so retention is one range-remove and trend
//! queries are window-range scans: `(day_epoch u32 BE, h3_r5 u64 BE, root u8)`.
//! Counters are u32 saturating. Never stores GDELT raw rows (SPEC §9.4).

use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::path::{Path, PathBuf};

/// (day, h3_r5, root) → event count. 13-byte big-endian packed key.
const COUNTERS: TableDefinition<&[u8], u32> = TableDefinition::new("counters_v1");
/// (domain_a, domain_b) sorted-pair → co-occurrence weight.
const EDGES: TableDefinition<(u64, u64), u32> = TableDefinition::new("edges_v1");
/// domain_hash → PageRank prior in [0,1] (f32 bits).
const PRIOR: TableDefinition<u64, f32> = TableDefinition::new("prior_v1");
/// Singleton metadata: last processed slice URL hash, etc.
const META: TableDefinition<&str, u64> = TableDefinition::new("meta_v1");

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("analytics store: {0}")]
    Redb(String),
}

fn err<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError::Redb(e.to_string())
}

/// One observed (geo, topic) event bucket increment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CounterKey {
    /// Days since the unix epoch (NOT YYYYMMDD — TTL math stays arithmetic).
    pub day: u32,
    pub h3_r5: u64,
    /// GDELT EventRootCode (1..=20); 0 = unknown.
    pub root: u8,
}

impl CounterKey {
    fn pack(&self) -> [u8; 13] {
        let mut k = [0u8; 13];
        k[..4].copy_from_slice(&self.day.to_be_bytes());
        k[4..12].copy_from_slice(&self.h3_r5.to_be_bytes());
        k[12] = self.root;
        k
    }

    fn unpack(k: &[u8]) -> Option<Self> {
        if k.len() != 13 {
            return None;
        }
        Some(Self {
            day: u32::from_be_bytes(k[..4].try_into().ok()?),
            h3_r5: u64::from_be_bytes(k[4..12].try_into().ok()?),
            root: k[12],
        })
    }
}

pub struct AnalyticsStore {
    db: Database,
    path: PathBuf,
}

impl AnalyticsStore {
    pub fn open(data_dir: &Path) -> Result<Self, StoreError> {
        let path = data_dir.join("analytics.redb");
        let db = Database::create(&path).map_err(err)?;
        let txn = db.begin_write().map_err(err)?;
        {
            txn.open_table(COUNTERS).map_err(err)?;
            txn.open_table(EDGES).map_err(err)?;
            txn.open_table(PRIOR).map_err(err)?;
            txn.open_table(META).map_err(err)?;
        }
        txn.commit().map_err(err)?;
        Ok(Self { db, path })
    }

    /// Apply one slice's aggregates in a single transaction (SD-friendly).
    pub fn apply(
        &self,
        counters: &std::collections::HashMap<CounterKey, u32>,
        edges: &std::collections::HashMap<(u64, u64), u32>,
    ) -> Result<(), StoreError> {
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut table = txn.open_table(COUNTERS).map_err(err)?;
            for (key, n) in counters {
                let packed = key.pack();
                let current = table
                    .get(packed.as_slice())
                    .map_err(err)?
                    .map(|v| v.value())
                    .unwrap_or(0);
                table
                    .insert(packed.as_slice(), current.saturating_add(*n))
                    .map_err(err)?;
            }
            let mut table = txn.open_table(EDGES).map_err(err)?;
            for (&(a, b), n) in edges {
                let current = table
                    .get(&(a, b))
                    .map_err(err)?
                    .map(|v| v.value())
                    .unwrap_or(0);
                table
                    .insert(&(a, b), current.saturating_add(*n))
                    .map_err(err)?;
            }
        }
        txn.commit().map_err(err)?;
        Ok(())
    }

    /// Counters in `[from_day, to_day]`, optionally filtered by root/h3_r5.
    pub fn scan(
        &self,
        from_day: u32,
        to_day: u32,
        root: Option<u8>,
        h3_r5: Option<u64>,
    ) -> Result<Vec<(CounterKey, u32)>, StoreError> {
        let txn = self.db.begin_read().map_err(err)?;
        let table = txn.open_table(COUNTERS).map_err(err)?;
        let lo = CounterKey {
            day: from_day,
            h3_r5: 0,
            root: 0,
        }
        .pack();
        let hi = CounterKey {
            day: to_day,
            h3_r5: u64::MAX,
            root: u8::MAX,
        }
        .pack();
        let mut out = Vec::new();
        for entry in table.range(lo.as_slice()..=hi.as_slice()).map_err(err)? {
            let (k, v) = entry.map_err(err)?;
            let Some(key) = CounterKey::unpack(k.value()) else {
                continue;
            };
            if root.is_some_and(|r| r != key.root) {
                continue;
            }
            if h3_r5.is_some_and(|h| h != key.h3_r5) {
                continue;
            }
            out.push((key, v.value()));
        }
        Ok(out)
    }

    /// Retention (SPEC §9.4): drop counter days older than `cutoff_day` and
    /// thin the edge table when it outgrows `max_edges` (drop weight-1 edges,
    /// then lowest weights). Returns (counters_dropped, edges_dropped).
    pub fn compact(&self, cutoff_day: u32, max_edges: usize) -> Result<(u64, u64), StoreError> {
        let txn = self.db.begin_write().map_err(err)?;
        let mut counters_dropped = 0u64;
        let mut edges_dropped = 0u64;
        {
            let mut table = txn.open_table(COUNTERS).map_err(err)?;
            let hi = CounterKey {
                day: cutoff_day,
                h3_r5: 0,
                root: 0,
            }
            .pack();
            let stale = table
                .extract_from_if(..hi.as_slice(), |_, _| true)
                .map_err(err)?;
            for entry in stale {
                entry.map_err(err)?;
                counters_dropped += 1;
            }

            let mut edges = txn.open_table(EDGES).map_err(err)?;
            let total = edges.len().map_err(err)? as usize;
            if total > max_edges {
                // Pass 1: weight-1 edges (the long noise tail).
                let removed = edges
                    .extract_from_if::<(u64, u64), _>(.., |_, w| w <= 1)
                    .map_err(err)?;
                for entry in removed {
                    entry.map_err(err)?;
                    edges_dropped += 1;
                }
                // Pass 2 (rare): still over cap → raise the floor.
                let mut floor = 2u32;
                while edges.len().map_err(err)? as usize > max_edges && floor < 1024 {
                    let removed = edges
                        .extract_from_if::<(u64, u64), _>(.., |_, w| w <= floor)
                        .map_err(err)?;
                    for entry in removed {
                        entry.map_err(err)?;
                        edges_dropped += 1;
                    }
                    floor *= 2;
                }
            }
        }
        txn.commit().map_err(err)?;
        Ok((counters_dropped, edges_dropped))
    }

    /// All edges (the nightly PageRank input).
    pub fn edges(&self) -> Result<Vec<(u64, u64, u32)>, StoreError> {
        let txn = self.db.begin_read().map_err(err)?;
        let table = txn.open_table(EDGES).map_err(err)?;
        let mut out = Vec::new();
        for entry in table.iter().map_err(err)? {
            let (k, v) = entry.map_err(err)?;
            let (a, b) = k.value();
            out.push((a, b, v.value()));
        }
        Ok(out)
    }

    /// Replace the prior table wholesale (nightly job output).
    pub fn store_priors(&self, priors: &[(u64, f32)]) -> Result<(), StoreError> {
        let txn = self.db.begin_write().map_err(err)?;
        txn.delete_table(PRIOR).map_err(err)?;
        {
            let mut table = txn.open_table(PRIOR).map_err(err)?;
            for &(domain, p) in priors {
                table.insert(domain, p).map_err(err)?;
            }
        }
        txn.commit().map_err(err)?;
        Ok(())
    }

    pub fn prior(&self, domain_hash: u64) -> f32 {
        let Ok(txn) = self.db.begin_read() else {
            return 0.0;
        };
        let Ok(table) = txn.open_table(PRIOR) else {
            return 0.0;
        };
        table
            .get(domain_hash)
            .ok()
            .flatten()
            .map(|v| v.value())
            .unwrap_or(0.0)
    }

    pub fn meta(&self, key: &str) -> Option<u64> {
        let txn = self.db.begin_read().ok()?;
        let table = txn.open_table(META).ok()?;
        table.get(key).ok().flatten().map(|v| v.value())
    }

    pub fn set_meta(&self, key: &str, value: u64) -> Result<(), StoreError> {
        let txn = self.db.begin_write().map_err(err)?;
        {
            let mut table = txn.open_table(META).map_err(err)?;
            table.insert(key, value).map_err(err)?;
        }
        txn.commit().map_err(err)?;
        Ok(())
    }

    pub fn disk_bytes(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }

    /// redb only reuses free pages; compaction returns bytes to the OS
    /// (weekly job, SPEC §9.3).
    pub fn compact_file(&mut self) -> Result<bool, StoreError> {
        self.db.compact().map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static DIR_SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn temp_store() -> AnalyticsStore {
        let dir = std::env::temp_dir().join(format!(
            "meridian-analytics-{}-{}",
            std::process::id(),
            DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        AnalyticsStore::open(&dir).unwrap()
    }

    #[test]
    fn counters_accumulate_scan_and_compact() {
        let store = temp_store();
        let k1 = CounterKey {
            day: 100,
            h3_r5: 42,
            root: 14,
        };
        let k2 = CounterKey {
            day: 101,
            h3_r5: 42,
            root: 14,
        };
        let k3 = CounterKey {
            day: 101,
            h3_r5: 99,
            root: 3,
        };
        let mut counters = std::collections::HashMap::new();
        counters.insert(k1, 5);
        counters.insert(k2, 7);
        counters.insert(k3, 1);
        store.apply(&counters, &Default::default()).unwrap();
        // Second apply accumulates.
        store.apply(&counters, &Default::default()).unwrap();

        let all = store.scan(0, u32::MAX, None, None).unwrap();
        assert_eq!(all.len(), 3);
        let day101 = store.scan(101, 101, None, None).unwrap();
        assert_eq!(day101.len(), 2);
        let root14 = store.scan(0, u32::MAX, Some(14), None).unwrap();
        assert_eq!(root14.iter().map(|(_, n)| n).sum::<u32>(), 24);

        // Retention: cutoff at day 101 drops day-100 rows only.
        let (dropped, _) = store.compact(101, usize::MAX).unwrap();
        assert_eq!(dropped, 1);
        assert_eq!(store.scan(0, u32::MAX, None, None).unwrap().len(), 2);
    }

    #[test]
    fn edges_accumulate_and_thin() {
        let store = temp_store();
        let mut edges = std::collections::HashMap::new();
        edges.insert((1u64, 2u64), 1u32);
        edges.insert((1u64, 3u64), 5u32);
        store.apply(&Default::default(), &edges).unwrap();
        store.apply(&Default::default(), &edges).unwrap();
        let all = store.edges().unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&(1, 3, 10)));

        // Cap 1 forces thinning; the weight-2 edge goes first.
        let (_, dropped) = store.compact(0, 1).unwrap();
        assert!(dropped >= 1);
        assert!(store.edges().unwrap().len() <= 1);
    }

    /// SPEC §16 Phase-5 exit gate: analytics steady-state disk ≤700MB over two
    /// simulated weeks. GDELT-realistic cardinality: ~12k distinct res-5 cells
    /// × ~15 active roots × 14 days plus a 200k-edge graph. Run explicitly:
    ///   cargo test -p meridian-analytics disk_gate -- --ignored --nocapture
    #[test]
    #[ignore = "exit-gate measurement; ~1 min on the Pi"]
    fn disk_gate_two_simulated_weeks_under_700mb() {
        let store = temp_store();
        let mut edges = std::collections::HashMap::new();
        for i in 0..200_000u64 {
            edges.insert((i, i + 1), 2u32);
        }
        for day in 0..14u32 {
            let mut counters = std::collections::HashMap::new();
            for cell in 0..12_000u64 {
                for root in 1..=15u8 {
                    counters.insert(
                        CounterKey {
                            day,
                            h3_r5: 0x0850_0000_0000_0000 + cell,
                            root,
                        },
                        (cell % 50 + 1) as u32,
                    );
                }
            }
            store.apply(&counters, &Default::default()).unwrap();
        }
        store.apply(&Default::default(), &edges).unwrap();
        // Retention pass like the nightly job (nothing >90d here, edges capped).
        store.compact(0, 200_000).unwrap();
        let bytes = store.disk_bytes();
        println!(
            "analytics.redb after 14 simulated days: {} MB",
            bytes / 1_048_576
        );
        assert!(bytes <= 700 * 1_048_576, "disk gate: {bytes} > 700MB");
    }

    #[test]
    fn priors_roundtrip() {
        let store = temp_store();
        store.store_priors(&[(7, 1.0), (8, 0.25)]).unwrap();
        assert_eq!(store.prior(7), 1.0);
        assert_eq!(store.prior(8), 0.25);
        assert_eq!(store.prior(9), 0.0, "unknown domain = no prior");
        // Wholesale replacement drops stale entries.
        store.store_priors(&[(7, 0.5)]).unwrap();
        assert_eq!(store.prior(8), 0.0);
    }
}
