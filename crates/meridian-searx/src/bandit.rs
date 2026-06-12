//! ε-greedy engine-subset routing (SPEC §11): pick which SearXNG engine subset
//! to query for a given intent class. Reward = "appeared in the final top-10".
//! Arms are persisted per intent in redb so learning survives restarts.
//!
//! Anonymity firewall (SPEC §12.4): anon-lane outcomes NEVER update arm stats —
//! the planner simply doesn't call [`Bandit::reward`] for anon requests, so
//! anon behavior cannot leak into shared routing state.

use redb::{Database, ReadableTable, TableDefinition};
use std::collections::HashMap;
use std::sync::Mutex;

/// `intent\0arm` → (pulls, reward_sum) packed as two LE u64s.
const ARMS_TABLE: TableDefinition<&str, [u8; 16]> = TableDefinition::new("bandit_arms_v1");

#[derive(Debug, thiserror::Error)]
#[error("bandit: {0}")]
pub struct BanditError(String);

/// One engine subset (an "arm"). The engine list is what gets passed to SearXNG
/// as the `engines=` parameter; an empty list means "SearXNG defaults".
#[derive(Debug, Clone)]
pub struct Arm {
    pub id: &'static str,
    pub engines: &'static [&'static str],
}

/// The candidate arms. Curated subsets balance coverage vs latency (the §11
/// latency floor is the slowest engine in the chosen subset).
pub const ARMS: &[Arm] = &[
    Arm {
        id: "fast",
        engines: &["duckduckgo", "brave", "mojeek"],
    },
    Arm {
        id: "broad",
        engines: &["duckduckgo", "brave", "qwant", "startpage", "wikipedia"],
    },
    Arm {
        id: "reference",
        engines: &["wikipedia", "wikidata", "arxiv", "crossref"],
    },
];

#[derive(Clone, Copy, Default)]
struct Stat {
    pulls: u64,
    reward_sum: u64,
}

impl Stat {
    fn mean(&self) -> f64 {
        if self.pulls == 0 {
            0.5 // optimistic prior so every arm gets tried
        } else {
            self.reward_sum as f64 / self.pulls as f64
        }
    }
}

pub struct Bandit {
    db: std::sync::Arc<Database>,
    epsilon: f64,
    /// In-memory mirror of the redb arm stats; redb is the durable backing.
    stats: Mutex<HashMap<String, Stat>>,
}

impl Bandit {
    pub fn open(data_dir: &std::path::Path, epsilon: f64) -> Result<Self, BanditError> {
        std::fs::create_dir_all(data_dir).map_err(|e| BanditError(e.to_string()))?;
        let db = std::sync::Arc::new(
            Database::create(data_dir.join("egress.redb"))
                .map_err(|e| BanditError(e.to_string()))?,
        );
        let mut stats = HashMap::new();
        {
            let wtx = db.begin_write().map_err(|e| BanditError(e.to_string()))?;
            {
                let table = wtx
                    .open_table(ARMS_TABLE)
                    .map_err(|e| BanditError(e.to_string()))?;
                for entry in table.iter().map_err(|e| BanditError(e.to_string()))? {
                    let (k, v) = entry.map_err(|e| BanditError(e.to_string()))?;
                    let bytes = v.value();
                    stats.insert(
                        k.value().to_owned(),
                        Stat {
                            pulls: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
                            reward_sum: u64::from_le_bytes(bytes[8..].try_into().unwrap()),
                        },
                    );
                }
            }
            wtx.commit().map_err(|e| BanditError(e.to_string()))?;
        }
        Ok(Self {
            db,
            epsilon: epsilon.clamp(0.0, 1.0),
            stats: Mutex::new(stats),
        })
    }

    /// Shared handle on `egress.redb` — the decision log (Phase 9, ADR-24)
    /// lives in the same database (one file, one durability story).
    pub fn database(&self) -> std::sync::Arc<Database> {
        self.db.clone()
    }

    /// [`Bandit::choose`] plus the chosen arm's PROPENSITY under this ε-greedy
    /// policy: (1−ε)+ε/K for the greedy arm, ε/K otherwise (Phase 9, ADR-24 —
    /// logging today's propensities makes the incumbent the logging policy).
    pub fn choose_with_propensity(&self, intent: &str, salt: u64) -> (&'static Arm, f32) {
        let chosen = self.choose(intent, salt);
        let greedy = {
            let stats = self.stats.lock().expect("bandit stats");
            ARMS.iter()
                .max_by(|a, b| {
                    let ma = stats
                        .get(&Self::arm_key(intent, a.id))
                        .copied()
                        .unwrap_or_default()
                        .mean();
                    let mb = stats
                        .get(&Self::arm_key(intent, b.id))
                        .copied()
                        .unwrap_or_default()
                        .mean();
                    ma.partial_cmp(&mb).unwrap()
                })
                .map(|a| a.id)
                .unwrap_or(ARMS[0].id)
        };
        let k = ARMS.len() as f64;
        let p = if chosen.id == greedy {
            (1.0 - self.epsilon) + self.epsilon / k
        } else {
            self.epsilon / k
        };
        (chosen, p as f32)
    }

    fn arm_key(intent: &str, arm_id: &str) -> String {
        format!("{intent}\0{arm_id}")
    }

    /// Choose an arm for `intent`. With probability ε explore a pseudo-random
    /// arm; otherwise exploit the highest mean reward. `salt` provides the
    /// exploration randomness without a global RNG (SPEC: scripts/this code stay
    /// deterministic given inputs) — the planner passes a per-request nonce.
    pub fn choose(&self, intent: &str, salt: u64) -> &'static Arm {
        let roll = ((salt.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f64) / (1u64 << 24) as f64;
        let explore = roll < self.epsilon;
        if explore {
            return &ARMS[(salt as usize) % ARMS.len()];
        }
        let stats = self.stats.lock().expect("bandit stats");
        ARMS.iter()
            .max_by(|a, b| {
                let ma = stats
                    .get(&Self::arm_key(intent, a.id))
                    .copied()
                    .unwrap_or_default()
                    .mean();
                let mb = stats
                    .get(&Self::arm_key(intent, b.id))
                    .copied()
                    .unwrap_or_default()
                    .mean();
                ma.partial_cmp(&mb).unwrap()
            })
            .unwrap_or(&ARMS[0])
    }

    /// Record an outcome for (intent, arm): `appeared` = the arm's results
    /// reached the final top-10. Updates the in-memory mean and persists.
    pub fn reward(&self, intent: &str, arm_id: &str, appeared: bool) -> Result<(), BanditError> {
        let key = Self::arm_key(intent, arm_id);
        let updated = {
            let mut stats = self.stats.lock().expect("bandit stats");
            let stat = stats.entry(key.clone()).or_default();
            stat.pulls += 1;
            stat.reward_sum += appeared as u64;
            *stat
        };
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&updated.pulls.to_le_bytes());
        bytes[8..].copy_from_slice(&updated.reward_sum.to_le_bytes());
        let wtx = self
            .db
            .begin_write()
            .map_err(|e| BanditError(e.to_string()))?;
        {
            let mut table = wtx
                .open_table(ARMS_TABLE)
                .map_err(|e| BanditError(e.to_string()))?;
            table
                .insert(key.as_str(), bytes)
                .map_err(|e| BanditError(e.to_string()))?;
        }
        wtx.commit().map_err(|e| BanditError(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exploit_converges_and_persists() {
        let dir = std::env::temp_dir().join(format!("meridian-bandit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bandit = Bandit::open(&dir, 0.0).unwrap(); // ε=0 → pure exploit

        // Teach: for "keyword" intent, only "reference" ever appears in top-10.
        for _ in 0..20 {
            bandit.reward("keyword", "reference", true).unwrap();
            bandit.reward("keyword", "fast", false).unwrap();
            bandit.reward("keyword", "broad", false).unwrap();
        }
        // Exploit must now pick "reference" regardless of salt.
        for salt in 0..5u64 {
            assert_eq!(bandit.choose("keyword", salt).id, "reference");
        }

        // Persisted: a fresh open sees the learned arm.
        drop(bandit);
        let reopened = Bandit::open(&dir, 0.0).unwrap();
        assert_eq!(reopened.choose("keyword", 7).id, "reference");
        let _ = std::fs::remove_dir_all(dir);
    }
}
