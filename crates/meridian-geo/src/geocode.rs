//! Remote geocoder client (SPEC §3: "remote geocoding APIs + persistent cache";
//! operator question Q8 default = public Nominatim at 1 rps). OFF by default —
//! ingest geo-tagging is gazetteer-first and the gazetteer needs no network;
//! the geocoder is the operator-opt-in fallback for places the fst misses.
//!
//! Politeness: a process-global 1-request-per-second budget (Nominatim usage
//! policy), honest UA via the direct lane, and a persistent `geo.redb` cache so
//! a place is asked about ONCE. Negative results are cached too.

use crate::h3::{INDEX_RES, latlng_to_cell};
use meridian_egress::LaneClient;
use redb::{Database, ReadableDatabase, TableDefinition};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const CACHE: TableDefinition<&str, (f64, f64)> = TableDefinition::new("geocode_v1");
/// Negative cache (misses), kept separate so hits stay a simple (lat, lon).
const MISSES: TableDefinition<&str, ()> = TableDefinition::new("geocode_miss_v1");

#[derive(Debug, thiserror::Error)]
pub enum GeocodeError {
    #[error("geocode store: {0}")]
    Store(String),
    #[error("geocode transport error")]
    Transport,
    #[error("geocode response unparsable")]
    BadResponse,
}

pub struct Geocoder {
    db: Arc<Database>,
    db_path: std::path::PathBuf,
    /// Read-through hot cache (SPEC §8.4: fetch/geo Moka 128MB shared budget —
    /// geocode answers are tiny, 32MB is generous).
    hot: moka::sync::Cache<String, Option<(f64, f64)>>,
    /// 1 rps politeness gate (Nominatim usage policy).
    budget: tokio::sync::Mutex<tokio::time::Instant>,
    endpoint: String,
}

impl Geocoder {
    pub fn open(data_dir: &Path, endpoint: &str) -> Result<Self, GeocodeError> {
        let db_path = data_dir.join("geo.redb");
        let db = Database::create(&db_path).map_err(|e| GeocodeError::Store(e.to_string()))?;
        // Ensure tables exist so first reads don't error.
        let txn = db
            .begin_write()
            .map_err(|e| GeocodeError::Store(e.to_string()))?;
        {
            txn.open_table(CACHE)
                .map_err(|e| GeocodeError::Store(e.to_string()))?;
            txn.open_table(MISSES)
                .map_err(|e| GeocodeError::Store(e.to_string()))?;
        }
        txn.commit()
            .map_err(|e| GeocodeError::Store(e.to_string()))?;
        Ok(Self {
            db: Arc::new(db),
            db_path,
            hot: moka::sync::Cache::builder()
                .max_capacity(32 * 1024 * 1024)
                .weigher(|k: &String, _| (k.len() + 24) as u32)
                .build(),
            budget: tokio::sync::Mutex::new(tokio::time::Instant::now()),
            endpoint: endpoint.trim_end_matches('/').to_owned(),
        })
    }

    /// Cache-only lookup (no network) — what ingest uses when the remote
    /// geocoder is disabled.
    pub fn cached(&self, place: &str) -> Option<Option<(f64, f64)>> {
        let key = place.trim().to_lowercase();
        if let Some(hit) = self.hot.get(&key) {
            return Some(hit);
        }
        let txn = self.db.begin_read().ok()?;
        if let Ok(table) = txn.open_table(CACHE) {
            if let Ok(Some(v)) = table.get(key.as_str()) {
                let coords = v.value();
                self.hot.insert(key, Some(coords));
                return Some(Some(coords));
            }
        }
        if let Ok(table) = txn.open_table(MISSES) {
            if let Ok(Some(_)) = table.get(key.as_str()) {
                self.hot.insert(key, None);
                return Some(None);
            }
        }
        None
    }

    /// Resolve a place name to (lat, lon): cache, then ONE polite remote
    /// query on the direct lane. `Ok(None)` = the service doesn't know it
    /// (cached negatively — never asked again).
    pub async fn resolve(
        &self,
        client: &LaneClient,
        place: &str,
    ) -> Result<Option<(f64, f64)>, GeocodeError> {
        if let Some(cached) = self.cached(place) {
            return Ok(cached);
        }
        let key = place.trim().to_lowercase();

        // Politeness: serialize remote calls at ≥1s spacing, process-wide.
        {
            let mut next_allowed = self.budget.lock().await;
            let now = tokio::time::Instant::now();
            if *next_allowed > now {
                tokio::time::sleep(*next_allowed - now).await;
            }
            *next_allowed = tokio::time::Instant::now() + Duration::from_secs(1);
        }

        let url = reqwest::Url::parse_with_params(
            &format!("{}/search", self.endpoint),
            &[("q", key.as_str()), ("format", "jsonv2"), ("limit", "1")],
        )
        .map_err(|_| GeocodeError::BadResponse)?;
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|_| GeocodeError::Transport)?;
        if !response.status().is_success() {
            return Err(GeocodeError::Transport);
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|_| GeocodeError::BadResponse)?;
        let coords = body.as_array().and_then(|a| a.first()).and_then(|hit| {
            let lat = hit.get("lat")?.as_str()?.parse::<f64>().ok()?;
            let lon = hit.get("lon")?.as_str()?.parse::<f64>().ok()?;
            Some((lat, lon))
        });

        let txn = self
            .db
            .begin_write()
            .map_err(|e| GeocodeError::Store(e.to_string()))?;
        {
            match coords {
                Some(c) => {
                    let mut table = txn
                        .open_table(CACHE)
                        .map_err(|e| GeocodeError::Store(e.to_string()))?;
                    table
                        .insert(key.as_str(), c)
                        .map_err(|e| GeocodeError::Store(e.to_string()))?;
                }
                None => {
                    let mut table = txn
                        .open_table(MISSES)
                        .map_err(|e| GeocodeError::Store(e.to_string()))?;
                    table
                        .insert(key.as_str(), ())
                        .map_err(|e| GeocodeError::Store(e.to_string()))?;
                }
            }
        }
        txn.commit()
            .map_err(|e| GeocodeError::Store(e.to_string()))?;
        self.hot.insert(key, coords);
        Ok(coords)
    }

    /// Convenience: resolved place → index cell.
    pub fn to_cell(coords: (f64, f64)) -> Option<u64> {
        latlng_to_cell(coords.0, coords.1, INDEX_RES).ok()
    }

    /// Retention sweep hook (SPEC §6.1: geocode shares the 600MB redb cap):
    /// drop the negative cache wholesale when the store outgrows `max_bytes` —
    /// misses are the unbounded-growth side; hits are precious.
    pub fn sweep(&self, max_bytes: u64) -> Result<u64, GeocodeError> {
        let size = std::fs::metadata(&self.db_path)
            .map(|m| m.len())
            .unwrap_or(0);
        if size <= max_bytes {
            return Ok(size);
        }
        let txn = self
            .db
            .begin_write()
            .map_err(|e| GeocodeError::Store(e.to_string()))?;
        txn.delete_table(MISSES)
            .map_err(|e| GeocodeError::Store(e.to_string()))?;
        {
            txn.open_table(MISSES)
                .map_err(|e| GeocodeError::Store(e.to_string()))?;
        }
        txn.commit()
            .map_err(|e| GeocodeError::Store(e.to_string()))?;
        self.hot.invalidate_all();
        tracing::info!(size_bytes = size, "geocode sweep: negative cache dropped");
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_roundtrip_without_network() {
        let dir = std::env::temp_dir().join(format!("meridian-geocode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let g = Geocoder::open(&dir, "https://nominatim.example/").unwrap();
        assert!(g.cached("Berlin").is_none(), "cold cache knows nothing");

        // Simulate a stored hit + a stored miss directly through the tables.
        let txn = g.db.begin_write().unwrap();
        {
            let mut t = txn.open_table(CACHE).unwrap();
            t.insert("berlin", (52.52, 13.40)).unwrap();
            let mut m = txn.open_table(MISSES).unwrap();
            m.insert("atlantis", ()).unwrap();
        }
        txn.commit().unwrap();

        assert_eq!(g.cached("Berlin"), Some(Some((52.52, 13.40))));
        assert_eq!(g.cached("  ATLANTIS "), Some(None), "negative cache works");
        assert!(Geocoder::to_cell((52.52, 13.40)).is_some());
    }
}
