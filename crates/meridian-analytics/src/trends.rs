//! `/v1/trends` queries (SPEC §10): per-day time series for a topic and/or H3
//! res-5 cell over a window, plus "top movers" — topics whose latest-day
//! volume significantly exceeds their window baseline.
//!
//! Phase 7 (ADR-21): movers are ranked by an EB-shrunk quasi-NB z-score with
//! BH-FDR across root codes — the raw latest/mean ratio fired on
//! single-digit-count noise (suite-10 study). The raw values stay in the
//! payload for explainability; `significant` is the defensible flag.

use crate::burst::{BurstParams, decode as burst_decode, summarize as burst_summarize};
use crate::stats::{MoverInput, mover_stats};
use crate::store::{AnalyticsStore, StoreError};
use std::collections::HashMap;

#[derive(Debug, serde::Serialize)]
pub struct TrendsReport {
    /// (day_epoch, count) ascending.
    pub series: Vec<(u32, u32)>,
    /// Movers descending by z (Phase 7: statistical ordering, ADR-21).
    pub top_movers: Vec<Mover>,
}

#[derive(Debug, serde::Serialize)]
pub struct Mover {
    pub root: u8,
    pub latest: u32,
    pub mean: f32,
    /// Raw latest/window-mean ratio — kept for explainability (pre-P7 metric).
    pub ratio: f32,
    /// EB-shrunk latest-day rate (ADR-21).
    pub shrunk_rate: f32,
    /// Quasi-NB standardized excess over the shrunk baseline.
    pub z: f32,
    /// BH q-value across all root codes in this report.
    pub q_value: f32,
    /// q ≤ 0.05 — the defensible "this moved" flag.
    pub significant: bool,
    /// "likely low-sample noise" when the ratio looks elevated but the
    /// statistics cannot back it (ADR-21 honesty label).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<&'static str>,
    /// Two-state burst decode over this root's window (Phase 10, ADR-28):
    /// answers the question z cannot — "is this series inside a SUSTAINED
    /// elevation, and since when" (a multi-day ramp never makes any single
    /// day extreme, so the latest-day z is structurally blind to it). An
    /// independent flag alongside `significant`, never a replacement. Absent
    /// when the window is too short to decode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub burst: Option<MoverBurst>,
}

/// Burst summary with the onset converted to an absolute day epoch.
#[derive(Debug, serde::Serialize)]
pub struct MoverBurst {
    /// The latest day is inside an elevated run.
    pub active: bool,
    /// Day epoch where the trailing elevated run began (present when active).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_day: Option<u32>,
    /// Length of the trailing elevated run in days.
    pub days_active: usize,
}

/// `topic` = GDELT EventRootCode; `h3_r5` = a res-5 cell (both optional).
pub fn trends(
    store: &AnalyticsStore,
    from_day: u32,
    to_day: u32,
    topic: Option<u8>,
    h3_r5: Option<u64>,
) -> Result<TrendsReport, StoreError> {
    let rows = store.scan(from_day, to_day, topic, h3_r5)?;

    let mut by_day: HashMap<u32, u32> = HashMap::new();
    let mut by_root_day: HashMap<(u8, u32), u32> = HashMap::new();
    let mut latest_day = from_day;
    for (key, n) in &rows {
        *by_day.entry(key.day).or_insert(0) += n;
        *by_root_day.entry((key.root, key.day)).or_insert(0) += n;
        latest_day = latest_day.max(key.day);
    }
    let mut series: Vec<(u32, u32)> = by_day.into_iter().collect();
    series.sort_unstable();

    // Movers (ADR-21): dense per-day vectors per root over the observed window
    // (missing days are real zeros), then EB + quasi-NB z + BH across roots.
    let first_day = by_root_day
        .keys()
        .map(|(_, day)| *day)
        .min()
        .unwrap_or(latest_day);
    let window_len = (latest_day.saturating_sub(first_day) + 1) as usize;
    let mut root_ids: Vec<u8> = by_root_day.keys().map(|(root, _)| *root).collect();
    root_ids.sort_unstable();
    root_ids.dedup();

    let mut candidates: Vec<(u8, MoverInput)> = Vec::new();
    if window_len >= 2 {
        for &root in &root_ids {
            let days: Vec<u32> = (0..window_len)
                .map(|i| {
                    by_root_day
                        .get(&(root, first_day + i as u32))
                        .copied()
                        .unwrap_or(0)
                })
                .collect();
            // Same admission rule as the pre-P7 detector: a baseline must exist.
            let baseline_sum: u32 = days[..window_len - 1].iter().sum();
            if baseline_sum > 0 {
                candidates.push((root, MoverInput { days }));
            }
        }
    }
    let inputs: Vec<MoverInput> = candidates
        .iter()
        .map(|(_, input)| MoverInput {
            days: input.days.clone(),
        })
        .collect();
    let stats = mover_stats(&inputs);

    let mut top_movers: Vec<Mover> = candidates
        .iter()
        .zip(stats)
        .map(|((root, input), s)| {
            let latest = *input.days.last().unwrap_or(&0);
            let mean = input.days.iter().sum::<u32>() as f32 / input.days.len() as f32;
            let burst = burst_decode(&input.days, &BurstParams::default()).map(|states| {
                let sum = burst_summarize(&states);
                MoverBurst {
                    active: sum.active,
                    onset_day: sum.onset_index.map(|i| first_day + i as u32),
                    days_active: sum.days_active,
                }
            });
            Mover {
                root: *root,
                latest,
                mean,
                ratio: if mean > 0.0 {
                    latest as f32 / mean
                } else {
                    0.0
                },
                shrunk_rate: s.shrunk_rate,
                z: s.z,
                q_value: s.q_value,
                significant: s.significant,
                label: s.label,
                burst,
            }
        })
        .collect();
    top_movers.sort_by(|a, b| {
        b.z.partial_cmp(&a.z)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.root.cmp(&b.root))
    });
    top_movers.truncate(10);

    Ok(TrendsReport { series, top_movers })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::CounterKey;

    #[test]
    fn series_and_movers() {
        let dir = std::env::temp_dir().join(format!("meridian-trends-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = AnalyticsStore::open(&dir).unwrap();

        let mut counters = std::collections::HashMap::new();
        // Root 14 steady (5/day), root 3 spiking on the last day.
        for day in 100..=104u32 {
            counters.insert(
                CounterKey {
                    day,
                    h3_r5: 1,
                    root: 14,
                },
                5,
            );
            counters.insert(
                CounterKey {
                    day,
                    h3_r5: 1,
                    root: 3,
                },
                if day == 104 { 50 } else { 2 },
            );
        }
        store.apply(&counters, &Default::default()).unwrap();

        let report = trends(&store, 100, 104, None, None).unwrap();
        assert_eq!(report.series.len(), 5);
        assert_eq!(report.series[0], (100, 7));
        assert_eq!(report.series[4], (104, 55));
        assert_eq!(report.top_movers[0].root, 3, "{:?}", report.top_movers);
        assert!(report.top_movers[0].ratio > 3.0);
        // Phase 7 (ADR-21): the spike is statistically backed; steady is not.
        assert!(
            report.top_movers[0].significant,
            "z={} q={}",
            report.top_movers[0].z, report.top_movers[0].q_value
        );
        let steady = report.top_movers.iter().find(|m| m.root == 14).unwrap();
        assert!(!steady.significant, "steady root must not be flagged");
        assert!(
            steady.label.is_none(),
            "steady root is not 'elevated' either"
        );

        // Topic filter narrows the series.
        let only14 = trends(&store, 100, 104, Some(14), None).unwrap();
        assert!(only14.series.iter().all(|(_, n)| *n == 5));

        // 5-day window is below the burst decode minimum — field absent.
        assert!(report.top_movers.iter().all(|m| m.burst.is_none()));
    }

    #[test]
    fn sustained_ramp_gets_a_burst_flag() {
        let dir = std::env::temp_dir().join(format!("meridian-burst-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = AnalyticsStore::open(&dir).unwrap();

        let mut counters = std::collections::HashMap::new();
        // 14 days: root 14 steady at 5; root 3 quiet at 2 then SUSTAINED at 10
        // for the last 5 days — the multi-day shape ADR-28 exists for.
        for day in 200..=213u32 {
            counters.insert(
                CounterKey {
                    day,
                    h3_r5: 1,
                    root: 14,
                },
                5,
            );
            counters.insert(
                CounterKey {
                    day,
                    h3_r5: 1,
                    root: 3,
                },
                if day >= 209 { 10 } else { 2 },
            );
        }
        store.apply(&counters, &Default::default()).unwrap();

        let report = trends(&store, 200, 213, None, None).unwrap();
        let ramp = report.top_movers.iter().find(|m| m.root == 3).unwrap();
        let burst = ramp.burst.as_ref().expect("14-day window decodes");
        assert!(burst.active, "{burst:?}");
        let onset = burst.onset_day.unwrap();
        assert!((208..=210).contains(&onset), "onset {onset}");
        assert!(burst.days_active >= 4, "{burst:?}");

        let steady = report.top_movers.iter().find(|m| m.root == 14).unwrap();
        let sb = steady.burst.as_ref().unwrap();
        assert!(!sb.active, "steady root must not burst: {sb:?}");
    }
}
