//! Per-domain politeness budget (SPEC §13.2: 1 req/2s sustained), enforced at ONE
//! chokepoint shared by every lane — using three lanes cannot triple the pressure
//! on an origin (SPEC §12.5 invariant 4).

use governor::clock::DefaultClock;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::{Quota, RateLimiter};
use std::num::NonZeroU32;
use std::time::Duration;

pub struct DomainBudget {
    limiter: RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock>,
}

impl DomainBudget {
    pub fn new(interval_ms: u64, burst: u32) -> Self {
        let interval = Duration::from_millis(interval_ms.max(1));
        let burst = NonZeroU32::new(burst.max(1)).expect("burst >= 1");
        let quota = Quota::with_period(interval)
            .expect("nonzero period")
            .allow_burst(burst);
        Self {
            limiter: RateLimiter::keyed(quota),
        }
    }

    /// Wait until `domain` has budget. All lanes call this — the limiter neither
    /// knows nor cares which lane is asking.
    pub async fn acquire(&self, domain: &str) {
        self.limiter.until_key_ready(&domain.to_owned()).await;
    }

    /// Non-blocking probe (ingest planning).
    pub fn try_acquire(&self, domain: &str) -> bool {
        self.limiter.check_key(&domain.to_owned()).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn burst_then_throttle_per_domain() {
        let budget = DomainBudget::new(50, 2);
        // Burst of 2 passes immediately…
        assert!(budget.try_acquire("a.example"));
        assert!(budget.try_acquire("a.example"));
        // …third in the same instant is throttled…
        assert!(!budget.try_acquire("a.example"));
        // …while an unrelated domain is unaffected.
        assert!(budget.try_acquire("b.example"));

        // And the async path eventually admits the throttled domain.
        tokio::time::timeout(Duration::from_millis(500), budget.acquire("a.example"))
            .await
            .expect("budget must replenish within the interval");
    }
}
