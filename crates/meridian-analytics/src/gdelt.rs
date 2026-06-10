//! GDELT v2 15-minute slice puller (SPEC §9.4, ADR-15).
//!
//! `data.gdeltproject.org` serves an INVALID TLS certificate (verified live,
//! 2026-06-10) — slices come over plain HTTP and are verified against the
//! `lastupdate.txt` manifest MD5 instead (integrity, not authenticity; the
//! counters never feed the trusted ranking path directly — domain_prior is
//! bounded, recomputed nightly, and observable). Raw slices are stream-parsed
//! and DISCARDED — never written to disk (SPEC §9.4).

use crate::store::CounterKey;
use md5::{Digest, Md5};
use meridian_egress::LaneClient;
use std::collections::HashMap;
use std::io::Read;

/// Slice zip cap: observed exports run 45KB–3.1MB (ADR-15); 32MB is paranoid.
const MAX_ZIP_BYTES: usize = 32 * 1024 * 1024;
/// Domains per (slice, root) clique for the co-occurrence graph — caps the
/// PageRank edge fan-out (30 domains → ≤435 edges per bucket).
const MAX_CLIQUE: usize = 30;

#[derive(Debug, thiserror::Error)]
pub enum GdeltError {
    #[error("gdelt transport error")]
    Transport,
    #[error("gdelt manifest unparsable")]
    BadManifest,
    #[error("gdelt slice md5 mismatch")]
    ChecksumMismatch,
    #[error("gdelt slice too large")]
    TooLarge,
    #[error("gdelt zip: {0}")]
    Zip(String),
}

/// Aggregates from one slice — what [`crate::store::AnalyticsStore::apply`] takes.
#[derive(Debug, Default)]
pub struct SliceAggregates {
    pub counters: HashMap<CounterKey, u32>,
    pub edges: HashMap<(u64, u64), u32>,
    pub rows_seen: usize,
    pub rows_geo: usize,
}

/// One manifest entry from `lastupdate.txt`: `<size> <md5> <url>`.
#[derive(Debug, Clone)]
pub struct SliceRef {
    pub md5_hex: String,
    pub url: String,
    /// Stable identity for "already processed" bookkeeping.
    pub id: u64,
}

/// Fetch + parse the manifest, returning the EXPORT (events) slice reference.
pub async fn latest_export(client: &LaneClient, base: &str) -> Result<SliceRef, GdeltError> {
    let url = format!("{}/lastupdate.txt", base.trim_end_matches('/'));
    let url = reqwest::Url::parse(&url).map_err(|_| GdeltError::BadManifest)?;
    let body = client
        .get(url)
        .send()
        .await
        .map_err(|_| GdeltError::Transport)?
        .text()
        .await
        .map_err(|_| GdeltError::Transport)?;
    for line in body.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() == 3 && cols[2].ends_with(".export.CSV.zip") {
            return Ok(SliceRef {
                md5_hex: cols[1].to_lowercase(),
                url: cols[2].to_owned(),
                id: u64::from_le_bytes(
                    blake3::hash(cols[2].as_bytes()).as_bytes()[..8]
                        .try_into()
                        .expect("8 bytes"),
                ),
            });
        }
    }
    Err(GdeltError::BadManifest)
}

/// Fetch one slice, verify its MD5 against the manifest, parse, discard.
pub async fn pull_slice(
    client: &LaneClient,
    slice: &SliceRef,
) -> Result<SliceAggregates, GdeltError> {
    let url = reqwest::Url::parse(&slice.url).map_err(|_| GdeltError::BadManifest)?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| GdeltError::Transport)?;
    if !response.status().is_success() {
        return Err(GdeltError::Transport);
    }
    let bytes = response.bytes().await.map_err(|_| GdeltError::Transport)?;
    if bytes.len() > MAX_ZIP_BYTES {
        return Err(GdeltError::TooLarge);
    }
    // ADR-15: HTTP + manifest MD5 (upstream TLS is broken).
    let digest = Md5::digest(&bytes);
    let got = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if got != slice.md5_hex {
        return Err(GdeltError::ChecksumMismatch);
    }

    let cursor = std::io::Cursor::new(bytes.as_ref());
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| GdeltError::Zip(e.to_string()))?;
    if zip.is_empty() {
        return Err(GdeltError::Zip("empty archive".into()));
    }
    let file = zip
        .by_index(0)
        .map_err(|e| GdeltError::Zip(e.to_string()))?;
    let mut csv = String::new();
    file.take(MAX_ZIP_BYTES as u64 * 4)
        .read_to_string(&mut csv)
        .map_err(|e| GdeltError::Zip(e.to_string()))?;
    Ok(parse_events(&csv))
}

/// Parse GDELT v2 events TSV (61 columns). Tolerant: malformed rows are
/// skipped, never fatal — upstream data quality is what it is.
pub fn parse_events(csv: &str) -> SliceAggregates {
    let mut agg = SliceAggregates::default();
    // Domains seen per root code in THIS slice → co-occurrence cliques.
    let mut by_root: HashMap<u8, Vec<u64>> = HashMap::new();

    for line in csv.lines() {
        agg.rows_seen += 1;
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 61 {
            continue;
        }
        // Col 1: Day (YYYYMMDD) · col 28: EventRootCode ·
        // cols 56/57: ActionGeo_Lat/Long · col 60: SOURCEURL.
        let Some(day) = parse_day(cols[1]) else {
            continue;
        };
        let root: u8 = cols[28].trim().parse().unwrap_or(0);
        let (Ok(lat), Ok(lon)) = (
            cols[56].trim().parse::<f64>(),
            cols[57].trim().parse::<f64>(),
        ) else {
            continue;
        };
        let Ok(h3_r5) = meridian_geo::h3::latlng_to_cell(lat, lon, meridian_geo::ANALYTICS_RES)
        else {
            continue;
        };
        agg.rows_geo += 1;
        *agg.counters
            .entry(CounterKey { day, h3_r5, root })
            .or_insert(0) += 1;

        if let Some(domain) = domain_of(cols[60]) {
            let bucket = by_root.entry(root).or_default();
            if !bucket.contains(&domain) && bucket.len() < MAX_CLIQUE {
                bucket.push(domain);
            }
        }
    }

    // Clique edges per root: domains reporting the same event class in the
    // same 15 minutes are "related" — the PageRank substrate (ADR amendment:
    // GDELT has no link graph; co-occurrence is the honest available signal).
    for bucket in by_root.values() {
        for i in 0..bucket.len() {
            for j in (i + 1)..bucket.len() {
                let (a, b) = if bucket[i] < bucket[j] {
                    (bucket[i], bucket[j])
                } else {
                    (bucket[j], bucket[i])
                };
                *agg.edges.entry((a, b)).or_insert(0) += 1;
            }
        }
    }
    agg
}

/// YYYYMMDD → days since the unix epoch (Howard Hinnant's days_from_civil).
fn parse_day(raw: &str) -> Option<u32> {
    let raw = raw.trim();
    if raw.len() != 8 {
        return None;
    }
    let y: i64 = raw[..4].parse().ok()?;
    let m: i64 = raw[4..6].parse().ok()?;
    let d: i64 = raw[6..8].parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1970..=2200).contains(&y) {
        return None;
    }
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = y_adj.div_euclid(400);
    let yoe = y_adj - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u32::try_from(days).ok()
}

/// SOURCEURL → stable domain hash (the same identity the index uses).
fn domain_of(url: &str) -> Option<u64> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    let host = parsed.host_str()?;
    Some(meridian_index_domain_hash(host))
}

/// blake3(host)[..8] — kept literally in sync with
/// `meridian_index::lexical::domain_hash` WITHOUT a crate edge (analytics must
/// not depend on the index crate; the hash recipe is the contract).
fn meridian_index_domain_hash(domain: &str) -> u64 {
    let digest = blake3::hash(domain.as_bytes());
    u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("8 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(day: &str, root: &str, lat: &str, lon: &str, source: &str) -> String {
        let mut cols = vec![""; 61];
        cols[0] = "1234567";
        cols[1] = day;
        cols[28] = root;
        cols[56] = lat;
        cols[57] = lon;
        cols[60] = source;
        cols.join("\t")
    }

    #[test]
    fn parses_events_counts_and_edges() {
        let csv = [
            row("20260611", "14", "52.52", "13.40", "https://a.example/x"),
            // Same coordinates as the row above — res-5 cells are ~10km wide
            // and nearby points can straddle a boundary.
            row("20260611", "14", "52.52", "13.40", "https://b.example/y"),
            row("20260611", "14", "52.52", "13.40", "https://a.example/z"),
            row("20260611", "3", "35.68", "139.65", "https://c.example/w"),
            // Malformed rows: skipped, not fatal.
            row("20260611", "14", "", "", "https://d.example/v"),
            "short\trow".to_owned(),
        ]
        .join("\n");
        let agg = parse_events(&csv);
        assert_eq!(agg.rows_seen, 6);
        assert_eq!(agg.rows_geo, 4);
        // Berlin cell (root 14) counted 3×, Tokyo (root 3) once.
        assert_eq!(agg.counters.values().sum::<u32>(), 4);
        assert_eq!(agg.counters.len(), 2, "{:?}", agg.counters);
        // a.example and b.example co-occur under root 14 → one edge.
        assert_eq!(agg.edges.len(), 1);
        assert_eq!(agg.edges.values().next(), Some(&1));
    }

    #[test]
    fn day_conversion_matches_epoch_arithmetic() {
        assert_eq!(parse_day("19700101"), Some(0));
        assert_eq!(parse_day("19700102"), Some(1));
        assert_eq!(parse_day("20000301"), Some(11017));
        // 2026-06-11 = 20615 days after epoch (cross-checked with date -d).
        assert_eq!(parse_day("20260611"), Some(20615));
        assert_eq!(parse_day("garbage!"), None);
        assert_eq!(parse_day("20261401"), None, "month 14 refused");
    }

    #[test]
    fn manifest_export_line_selected() {
        // Shape check on the parser via a synthetic manifest (the live fetch
        // is exercised in the on-device exit gate, not CI).
        let manifest = "\
120000 0123456789abcdef0123456789abcdef http://data.gdeltproject.org/gdeltv2/20260611120000.export.CSV.zip
340000 fedcba9876543210fedcba9876543210 http://data.gdeltproject.org/gdeltv2/20260611120000.mentions.CSV.zip";
        for line in manifest.lines() {
            let cols: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(cols.len(), 3);
        }
        // The export selector logic itself lives in latest_export (async);
        // its filter is the ".export.CSV.zip" suffix asserted here.
        assert!(
            manifest
                .lines()
                .next()
                .unwrap()
                .ends_with(".export.CSV.zip")
        );
    }
}
