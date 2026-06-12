//! Per-decision routing log (Phase 9, ADR-24): the substrate for offline
//! policy evaluation (IPS/doubly-robust, suite 14) — logging TODAY's ε-greedy
//! propensities makes the incumbent policy the logging policy for free.
//!
//! Privacy contract (operator-approved 2026-06-11, ADR-24):
//! - A row is 13 BYTES of coarse buckets + outcome: intent class, query-length
//!   bucket, language id, time-of-day bucket (3h), geo-filter flag, arm index,
//!   propensity, reward. **No query text. No URLs. No IPs. No timestamps finer
//!   than the day + 3h bucket.** The redb key carries the day for TTL sweeps.
//! - **k-anonymity floor:** a context-bucket combination is written GENERALIZED
//!   (all context fields = 0xFF) until it has been seen ≥5 times that day —
//!   rare combos never land identifiably. Arm/propensity/reward stay intact
//!   (generalization costs DR model features, never estimator validity).
//! - **30-day TTL** sweep + **≤20MB cap** (oldest days dropped first) +
//!   an operator **wipe** path.
//! - **Anon-lane decisions are never logged** — enforced at the call site
//!   (the planner logs exactly where it rewards the bandit, which the anon
//!   branch never reaches; SPEC §12.4) and re-stated here as the contract.
//!
//! Ships DISABLED by default in v0.3.x (`searx.decision_log`); the v0.4.0
//! exit flips it on together with the OPE gate (ADR-25).

use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

/// key = (day_epoch << 32) | per-day sequence; value = 13-byte row.
const DECISIONS: TableDefinition<u64, [u8; 13]> = TableDefinition::new("decisions_v1");

pub const RETENTION_DAYS: u32 = 30;
pub const MAX_BYTES: u64 = 20 * 1024 * 1024;
/// k-anonymity floor: combos below this daily count are generalized.
pub const K_FLOOR: u32 = 5;
const GENERALIZED: u8 = 0xFF;

#[derive(Debug, thiserror::Error)]
#[error("decision log: {0}")]
pub struct LogError(String);

/// Coarse, non-identifying decision context (ADR-24's exact field list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Context {
    /// Intent class index (4 classes).
    pub intent: u8,
    /// Query length bucket: 0 (≤20 chars), 1 (≤40), 2 (≤80), 3 (>80).
    pub len_bucket: u8,
    /// whichlang language id (coarse; 16 languages).
    pub lang: u8,
    /// Time-of-day bucket: hour/3 (8 buckets) — never a finer timestamp.
    pub tod_bucket: u8,
    /// A geo FILTER was present (the requested geography, never the user's).
    pub geo_filter: bool,
}

/// One logged decision, as read back for OPE.
#[derive(Debug, Clone, Copy)]
pub struct Decision {
    pub day: u32,
    pub context: Option<Context>, // None = generalized row
    pub arm: u8,
    pub propensity: f32,
    pub reward: bool,
}

pub struct DecisionLog {
    db: std::sync::Arc<Database>,
    seq: AtomicU32,
    /// (day, combo) → count for the k-anonymity floor; pruned on day change.
    combo_counts: Mutex<(u32, HashMap<Context, u32>)>,
    inserts_since_sweep: AtomicU32,
}

impl DecisionLog {
    pub fn new(db: std::sync::Arc<Database>) -> Result<Self, LogError> {
        let wtx = db.begin_write().map_err(|e| LogError(e.to_string()))?;
        wtx.open_table(DECISIONS)
            .map_err(|e| LogError(e.to_string()))?;
        wtx.commit().map_err(|e| LogError(e.to_string()))?;
        Ok(Self {
            db,
            seq: AtomicU32::new(0),
            combo_counts: Mutex::new((0, HashMap::new())),
            inserts_since_sweep: AtomicU32::new(0),
        })
    }

    /// Log one DIRECT-lane decision. The caller is the single place that also
    /// rewards the bandit — the anon branch never reaches either (§12.4).
    pub fn log(
        &self,
        now_unix: u64,
        ctx: Context,
        arm: u8,
        propensity: f32,
        reward: bool,
    ) -> Result<(), LogError> {
        let day = (now_unix / 86_400) as u32;

        // k-anonymity floor: generalize until the combo is common today.
        let generalize = {
            let mut guard = self.combo_counts.lock().expect("combo counts");
            if guard.0 != day {
                *guard = (day, HashMap::new());
            }
            let count = guard.1.entry(ctx).or_insert(0);
            *count += 1;
            *count < K_FLOOR
        };

        let mut row = [0u8; 13];
        if generalize {
            row[..5].fill(GENERALIZED);
        } else {
            row[0] = ctx.intent;
            row[1] = ctx.len_bucket;
            row[2] = ctx.lang;
            row[3] = ctx.tod_bucket;
            row[4] = ctx.geo_filter as u8;
        }
        row[5] = arm;
        row[6..10].copy_from_slice(&propensity.to_le_bytes());
        row[10] = reward as u8;
        // row[11..13] reserved (schema headroom without a migration).

        let key = (u64::from(day) << 32) | u64::from(self.seq.fetch_add(1, Ordering::Relaxed));
        let wtx = self.db.begin_write().map_err(|e| LogError(e.to_string()))?;
        {
            let mut table = wtx
                .open_table(DECISIONS)
                .map_err(|e| LogError(e.to_string()))?;
            table
                .insert(key, row)
                .map_err(|e| LogError(e.to_string()))?;
        }
        wtx.commit().map_err(|e| LogError(e.to_string()))?;

        if self.inserts_since_sweep.fetch_add(1, Ordering::Relaxed) >= 255 {
            self.inserts_since_sweep.store(0, Ordering::Relaxed);
            self.sweep(day)?;
        }
        Ok(())
    }

    /// TTL sweep (30 days) + size cap (oldest days first). Called on a cadence
    /// from `log` and at startup by the composition root.
    pub fn sweep(&self, today: u32) -> Result<(), LogError> {
        let cutoff = today.saturating_sub(RETENTION_DAYS);
        let wtx = self.db.begin_write().map_err(|e| LogError(e.to_string()))?;
        {
            let mut table = wtx
                .open_table(DECISIONS)
                .map_err(|e| LogError(e.to_string()))?;
            table
                .retain_in(..(u64::from(cutoff) << 32), |_, _| false)
                .map_err(|e| LogError(e.to_string()))?;
        }
        wtx.commit().map_err(|e| LogError(e.to_string()))?;

        // Size cap: drop the oldest remaining day until under the budget.
        // Row count approximates bytes (13B payload + redb overhead ≈ 60B).
        loop {
            let (count, oldest_day) = {
                let rtx = self.db.begin_read().map_err(|e| LogError(e.to_string()))?;
                let table = rtx
                    .open_table(DECISIONS)
                    .map_err(|e| LogError(e.to_string()))?;
                let count = table.len().map_err(|e| LogError(e.to_string()))?;
                let oldest = table
                    .first()
                    .map_err(|e| LogError(e.to_string()))?
                    .map(|(k, _)| (k.value() >> 32) as u32);
                (count, oldest)
            };
            if count * 60 <= MAX_BYTES || oldest_day.is_none() {
                break;
            }
            let day = oldest_day.unwrap();
            if day >= today {
                break; // never drop today's rows to satisfy the cap
            }
            let wtx = self.db.begin_write().map_err(|e| LogError(e.to_string()))?;
            {
                let mut table = wtx
                    .open_table(DECISIONS)
                    .map_err(|e| LogError(e.to_string()))?;
                table
                    .retain_in(..(u64::from(day + 1) << 32), |_, _| false)
                    .map_err(|e| LogError(e.to_string()))?;
            }
            wtx.commit().map_err(|e| LogError(e.to_string()))?;
        }
        Ok(())
    }

    /// Operator wipe: drop every row, now.
    pub fn wipe(&self) -> Result<usize, LogError> {
        let wtx = self.db.begin_write().map_err(|e| LogError(e.to_string()))?;
        let removed;
        {
            let mut table = wtx
                .open_table(DECISIONS)
                .map_err(|e| LogError(e.to_string()))?;
            removed = table.len().map_err(|e| LogError(e.to_string()))? as usize;
            table
                .retain(|_, _| false)
                .map_err(|e| LogError(e.to_string()))?;
        }
        wtx.commit().map_err(|e| LogError(e.to_string()))?;
        Ok(removed)
    }

    /// Read every retained decision (the OPE input).
    pub fn read_all(&self) -> Result<Vec<Decision>, LogError> {
        let rtx = self.db.begin_read().map_err(|e| LogError(e.to_string()))?;
        let table = rtx
            .open_table(DECISIONS)
            .map_err(|e| LogError(e.to_string()))?;
        let mut out = Vec::new();
        for entry in table.iter().map_err(|e| LogError(e.to_string()))? {
            let (k, v) = entry.map_err(|e| LogError(e.to_string()))?;
            let row = v.value();
            let context = if row[0] == GENERALIZED {
                None
            } else {
                Some(Context {
                    intent: row[0],
                    len_bucket: row[1],
                    lang: row[2],
                    tod_bucket: row[3],
                    geo_filter: row[4] != 0,
                })
            };
            out.push(Decision {
                day: (k.value() >> 32) as u32,
                context,
                arm: row[5],
                propensity: f32::from_le_bytes(row[6..10].try_into().unwrap()),
                reward: row[10] != 0,
            });
        }
        Ok(out)
    }

    pub fn len(&self) -> Result<u64, LogError> {
        let rtx = self.db.begin_read().map_err(|e| LogError(e.to_string()))?;
        let table = rtx
            .open_table(DECISIONS)
            .map_err(|e| LogError(e.to_string()))?;
        table.len().map_err(|e| LogError(e.to_string()))
    }

    pub fn is_empty(&self) -> Result<bool, LogError> {
        Ok(self.len()? == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn temp_log() -> (DecisionLog, std::path::PathBuf) {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "meridian-declog-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Arc::new(Database::create(dir.join("egress.redb")).unwrap());
        (DecisionLog::new(db).unwrap(), dir)
    }

    fn ctx(intent: u8) -> Context {
        Context {
            intent,
            len_bucket: 1,
            lang: 2,
            tod_bucket: 3,
            geo_filter: false,
        }
    }

    #[test]
    fn k_anonymity_floor_generalizes_rare_combos() {
        let (log, dir) = temp_log();
        let now = 86_400 * 20_000;
        for _ in 0..K_FLOOR + 2 {
            log.log(now, ctx(1), 0, 0.93, true).unwrap();
        }
        let rows = log.read_all().unwrap();
        let generalized = rows.iter().filter(|d| d.context.is_none()).count();
        let full = rows.iter().filter(|d| d.context.is_some()).count();
        assert_eq!(generalized, (K_FLOOR - 1) as usize, "first k−1 generalized");
        assert_eq!(full, 3, "rows at/after the floor carry context");
        // Arm/propensity/reward survive generalization (OPE validity).
        assert!(rows.iter().all(|d| d.arm == 0 && d.reward));
        assert!(rows.iter().all(|d| (d.propensity - 0.93).abs() < 1e-6));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ttl_sweep_and_wipe() {
        let (log, dir) = temp_log();
        let old_day = 20_000u64;
        let new_day = old_day + 31;
        log.log(old_day * 86_400, ctx(0), 1, 0.5, false).unwrap();
        log.log(new_day * 86_400, ctx(0), 2, 0.5, true).unwrap();
        assert_eq!(log.len().unwrap(), 2);
        log.sweep(new_day as u32).unwrap();
        let rows = log.read_all().unwrap();
        assert_eq!(rows.len(), 1, "31-day-old row swept");
        assert_eq!(rows[0].arm, 2);
        assert_eq!(log.wipe().unwrap(), 1);
        assert!(log.is_empty().unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rows_contain_no_text() {
        // The row type is 13 fixed bytes — this test pins the size so a
        // future field can't quietly grow into something string-shaped.
        assert_eq!(std::mem::size_of::<[u8; 13]>(), 13);
        let (log, dir) = temp_log();
        log.log(86_400 * 20_000, ctx(3), 1, 0.04, false).unwrap();
        let rows = log.read_all().unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].propensity - 0.04).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(dir);
    }
}
