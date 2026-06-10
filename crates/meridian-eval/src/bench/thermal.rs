//! Suite 6 — sustained-load thermal check (SPEC §15.6).
//! Loops the most CPU-bound stage (embedding) for `thermal_secs`, sampling SoC
//! temperature and the firmware throttle flags every 5s, and tracking throughput
//! drift (a softer throttling indicator than the flags).
//! Gate: no throttle bits set and temperature stays <80°C.

use super::{BenchConfig, SuiteResult};
use crate::probe::{soc_temp_c, throttled_flags};
use crate::stats::Rng;
use std::time::{Duration, Instant};

pub fn run(cfg: &BenchConfig) -> SuiteResult {
    let mut result = SuiteResult::new("thermal");
    let start = Instant::now();

    let model = match super::embed::load(&cfg.models_dir) {
        Ok(m) => m,
        Err(e) => {
            result.note(e);
            result.gate("no throttling, temp < 80°C", false);
            return result;
        }
    };
    if soc_temp_c().is_none() {
        return SuiteResult::skipped("thermal", "vcgencmd unavailable (not a Pi?)");
    }

    let mut rng = Rng::new(0x7E81);
    let sentences = super::synthetic_sentences(2_000, &mut rng);

    let deadline = Instant::now() + Duration::from_secs(cfg.thermal_secs);
    let mut max_temp: f64 = 0.0;
    let mut throttle_union: u32 = 0;
    let mut last_sample = Instant::now() - Duration::from_secs(10);
    let mut window_docs = 0usize;
    let mut window_start = Instant::now();
    let mut first_window_rate = 0.0f64;
    let mut last_window_rate = 0.0f64;

    while Instant::now() < deadline {
        for chunk in sentences.chunks(64) {
            window_docs += model.encode(chunk).len();
            if last_sample.elapsed() >= Duration::from_secs(5) {
                last_sample = Instant::now();
                if let Some(t) = soc_temp_c() {
                    max_temp = max_temp.max(t);
                }
                if let Some(f) = throttled_flags() {
                    throttle_union |= f;
                }
            }
            if window_start.elapsed() >= Duration::from_secs(30) {
                let rate = window_docs as f64 / window_start.elapsed().as_secs_f64();
                if first_window_rate == 0.0 {
                    first_window_rate = rate;
                }
                last_window_rate = rate;
                window_docs = 0;
                window_start = Instant::now();
            }
            if Instant::now() >= deadline {
                break;
            }
        }
    }

    result.metric("duration_secs", cfg.thermal_secs);
    result.metric("max_temp_c", max_temp);
    result.metric("throttle_flags_union", format!("0x{throttle_union:x}"));
    if first_window_rate > 0.0 && last_window_rate > 0.0 {
        result.metric(
            "throughput_drift_pct",
            ((last_window_rate - first_window_rate) / first_window_rate * 100.0).round(),
        );
    }
    result.gate(
        "no throttle flags and max temp < 80°C",
        throttle_union == 0 && max_temp < 80.0,
    );
    result.duration_ms = start.elapsed().as_secs_f64() * 1e3;
    result
}
