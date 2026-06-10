//! `/v1/trends` queries (SPEC §10): per-day time series for a topic and/or H3
//! res-5 cell over a window, plus "top movers" — topics whose latest-day volume
//! most exceeds their window mean.

use crate::store::{AnalyticsStore, StoreError};
use std::collections::HashMap;

#[derive(Debug, serde::Serialize)]
pub struct TrendsReport {
    /// (day_epoch, count) ascending.
    pub series: Vec<(u32, u32)>,
    /// (root_code, latest_count, window_mean, ratio) descending by ratio.
    pub top_movers: Vec<Mover>,
}

#[derive(Debug, serde::Serialize)]
pub struct Mover {
    pub root: u8,
    pub latest: u32,
    pub mean: f32,
    pub ratio: f32,
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

    // Movers: latest-day volume vs window mean per root code.
    let mut roots: HashMap<u8, Vec<u32>> = HashMap::new();
    for ((root, _day), n) in &by_root_day {
        roots.entry(*root).or_default().push(*n);
    }
    let mut top_movers: Vec<Mover> = roots
        .into_iter()
        .filter_map(|(root, counts)| {
            let latest = *by_root_day.get(&(root, latest_day))?;
            let mean = counts.iter().sum::<u32>() as f32 / counts.len() as f32;
            if mean <= 0.0 || counts.len() < 2 {
                return None;
            }
            Some(Mover {
                root,
                latest,
                mean,
                ratio: latest as f32 / mean,
            })
        })
        .collect();
    top_movers.sort_by(|a, b| {
        b.ratio
            .partial_cmp(&a.ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
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

        // Topic filter narrows the series.
        let only14 = trends(&store, 100, 104, Some(14), None).unwrap();
        assert!(only14.series.iter().all(|(_, n)| *n == 5));
    }
}
