//! Geo subsystem (SPEC §5): H3 ops via `h3o` (cell, grid_disk/k-ring, parents),
//! GeoNames gazetteer as an fst, remote geocoder client with a redb persistent cache,
//! heatmap aggregation, optional `maxminddb`/GeoLite2 enrichment (operator-supplied
//! DB; also used for region-lane egress verification).
//!
//! Status: Phase-0 scaffold — lands in Phase 5 (H3 fast fields appear in the index
//! schema from Phase 1 so no reindex is needed).
