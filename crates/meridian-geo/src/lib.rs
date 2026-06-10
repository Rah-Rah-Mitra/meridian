//! Geo subsystem (SPEC §5): H3 ops via `h3o` (cell, grid_disk/k-ring, parents),
//! GeoNames gazetteer as an fst, remote geocoder client with a redb persistent
//! cache. Heatmap *aggregation* lives in `meridian-index` (it scans fast
//! fields); this crate provides the cell math the rollup uses.
//!
//! Optional `maxminddb`/GeoLite2 enrichment (operator-supplied DB) is deferred
//! until an operator actually supplies one — the `expected_ip` region-lane
//! verification (Phase 4) covers the original use case without it.

pub mod gazetteer;
pub mod geocode;
pub mod h3;

pub use gazetteer::Gazetteer;
pub use geocode::Geocoder;
pub use h3::{ANALYTICS_RES, FilterRes, GeoError, INDEX_RES, MAX_FILTER_CELLS};
