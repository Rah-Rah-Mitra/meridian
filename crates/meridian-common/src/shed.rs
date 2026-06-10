//! Backpressure & shedding ladder (SPEC §8.6) — ordered, observable, never
//! silent. A background monitor samples RSS/temperature/free-disk and flips
//! atomic flags; hot paths read the flags for free. Every shed state is a
//! gauge in `/metrics`.
//!
//! Ordered ladder (SPEC §8.6):
//!   RSS  >2.5GB → disable rerank (flag exists now; rerank lands Phase 3)
//!   RSS  >2.7GB → drop caches
//!   RSS  >2.9GB → pause ingest + fast-mode only
//!   temp >78°C  → pause ingest
//!   temp >82°C  → local-only (shed all metasearch fan-out)
//!   free disk < floor → pause ingest

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const RSS_DISABLE_RERANK: u64 = 2_500 * 1024 * 1024;
pub const RSS_DROP_CACHES: u64 = 2_700 * 1024 * 1024;
pub const RSS_PAUSE_INGEST: u64 = 2_900 * 1024 * 1024;
pub const TEMP_PAUSE_INGEST_C: f64 = 78.0;
pub const TEMP_LOCAL_ONLY_C: f64 = 82.0;
/// Profile R scratch floor (02-budgets.md); Profile F uses 1.0GB.
pub const DISK_FLOOR_BYTES: u64 = 768 * 1024 * 1024;

#[derive(Default)]
pub struct ShedState {
    pub rerank_disabled: AtomicBool,
    pub caches_dropped: AtomicBool,
    pub ingest_paused: AtomicBool,
    pub metasearch_shed: AtomicBool,
}

impl ShedState {
    pub fn ingest_allowed(&self) -> bool {
        !self.ingest_paused.load(Ordering::Relaxed)
    }

    pub fn metasearch_allowed(&self) -> bool {
        !self.metasearch_shed.load(Ordering::Relaxed)
    }
}

/// Evaluate the ladder once against sampled values. Split from the loop for
/// testability; returns human-readable transitions for logging.
pub fn evaluate(state: &ShedState, rss: u64, temp_c: Option<f64>, free_disk: u64) -> Vec<String> {
    let mut transitions = Vec::new();
    let temp = temp_c.unwrap_or(0.0);

    let mut set = |flag: &AtomicBool, on: bool, name: &str| {
        let was = flag.swap(on, Ordering::Relaxed);
        if was != on {
            transitions.push(format!("{name}={}", if on { "SHED" } else { "ok" }));
        }
        metrics::gauge!("meridian_shed_state", "stage" => name.to_owned()).set(if on {
            1.0
        } else {
            0.0
        });
    };

    set(&state.rerank_disabled, rss > RSS_DISABLE_RERANK, "rerank");
    set(&state.caches_dropped, rss > RSS_DROP_CACHES, "caches");
    set(
        &state.ingest_paused,
        rss > RSS_PAUSE_INGEST || temp > TEMP_PAUSE_INGEST_C || free_disk < DISK_FLOOR_BYTES,
        "ingest",
    );
    set(
        &state.metasearch_shed,
        temp > TEMP_LOCAL_ONLY_C,
        "metasearch",
    );

    metrics::gauge!("meridian_rss_bytes").set(rss as f64);
    metrics::gauge!("meridian_free_disk_bytes").set(free_disk as f64);
    if let Some(t) = temp_c {
        metrics::gauge!("meridian_soc_temp_c").set(t);
    }
    transitions
}

/// Spawn the 5s monitor loop (call once from the composition root).
pub fn spawn_monitor(state: Arc<ShedState>, data_dir: PathBuf) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let rss = crate::sysprobe::rss_bytes();
            let temp = crate::sysprobe::soc_temp_c();
            let free = crate::sysprobe::free_disk_bytes(&data_dir);
            for t in evaluate(&state, rss, temp, free) {
                tracing::warn!(transition = %t, "shed state change");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_orders_and_recovers() {
        let s = ShedState::default();
        // Healthy.
        evaluate(&s, 1 << 30, Some(50.0), 10 << 30);
        assert!(s.ingest_allowed());
        assert!(s.metasearch_allowed());

        // Hot SoC: ingest pauses; metasearch still up at 79°C.
        evaluate(&s, 1 << 30, Some(79.0), 10 << 30);
        assert!(!s.ingest_allowed());
        assert!(s.metasearch_allowed());

        // Hotter: metasearch sheds too.
        evaluate(&s, 1 << 30, Some(83.0), 10 << 30);
        assert!(!s.metasearch_allowed());

        // Disk floor breached independently of temp.
        evaluate(&s, 1 << 30, Some(50.0), 100 << 20);
        assert!(!s.ingest_allowed());
        assert!(s.metasearch_allowed());

        // Full recovery.
        evaluate(&s, 1 << 30, Some(50.0), 10 << 30);
        assert!(s.ingest_allowed());
        assert!(s.metasearch_allowed());

        // RSS ladder.
        evaluate(&s, 2_950 * 1024 * 1024, Some(50.0), 10 << 30);
        assert!(!s.ingest_allowed());
        assert!(s.rerank_disabled.load(std::sync::atomic::Ordering::Relaxed));
        assert!(s.caches_dropped.load(std::sync::atomic::Ordering::Relaxed));
    }
}
