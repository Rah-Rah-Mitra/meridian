//! H3 operations (SPEC §3 Geo row, §11): res-7 cells as the index's u64 fast
//! field, `grid_disk` k-rings as the search prefilter, parents for heatmap
//! rollups. Thin, total wrappers over `h3o` — callers never see h3o types.

use h3o::{CellIndex, LatLng, Resolution};

/// The index stores cells at this resolution (SPEC §3: res-7, ~5km²/cell).
pub const INDEX_RES: u8 = 7;
/// Analytics counters aggregate at res-5 (~250km²/cell) — SPEC §9.4.
pub const ANALYTICS_RES: u8 = 5;
/// Upper bound on prefilter cell sets (TermSetQuery size; beyond this the
/// caller should coarsen the resolution or shrink the radius).
pub const MAX_FILTER_CELLS: usize = 4096;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GeoError {
    #[error("coordinates out of range")]
    BadCoords,
    #[error("h3 resolution out of range (0..=15)")]
    BadResolution,
    #[error("radius produces too many cells; reduce radius_km")]
    TooManyCells,
}

fn resolution(res: u8) -> Result<Resolution, GeoError> {
    Resolution::try_from(res).map_err(|_| GeoError::BadResolution)
}

/// (lat, lon) → H3 cell at `res`, as the index's u64 representation.
/// Strict range validation at this boundary — h3o silently normalizes
/// longitude, which would mask caller bugs.
pub fn latlng_to_cell(lat: f64, lon: f64, res: u8) -> Result<u64, GeoError> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return Err(GeoError::BadCoords);
    }
    let ll = LatLng::new(lat, lon).map_err(|_| GeoError::BadCoords)?;
    Ok(u64::from(ll.to_cell(resolution(res)?)))
}

/// Cell centroid as (lat, lon) — heatmap responses.
pub fn cell_to_latlng(cell: u64) -> Option<(f64, f64)> {
    let cell = CellIndex::try_from(cell).ok()?;
    let ll = LatLng::from(cell);
    Some((ll.lat(), ll.lng()))
}

/// k-ring 1 of `cell` INCLUDING the cell itself (≤7 cells; fewer near
/// pentagons). The Gi* neighborhood primitive for the Phase-7 heatmap
/// statistics (ADR-21).
pub fn k_ring1(cell: u64) -> Vec<u64> {
    let Ok(cell) = CellIndex::try_from(cell) else {
        return Vec::new();
    };
    cell.grid_disk::<Vec<_>>(1)
        .into_iter()
        .map(u64::from)
        .collect()
}

/// Parent of `cell` at `res` (no-op when `res` ≥ the cell's own resolution).
/// Heatmap rollup primitive.
pub fn parent_at(cell: u64, res: u8) -> Option<u64> {
    let cell = CellIndex::try_from(cell).ok()?;
    let res = resolution(res).ok()?;
    if res >= cell.resolution() {
        return Some(u64::from(cell));
    }
    cell.parent(res).map(u64::from)
}

/// Which indexed H3 field a filter set targets (the index stores BOTH res-5
/// and res-7 as indexed fast fields — SPEC §11 "k-ring at res 5–7").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterRes {
    R5,
    R7,
}

/// The search prefilter set (SPEC §11): k-ring around (lat, lon) covering
/// `radius_km`. Small radii produce res-7 term sets (fine precision); large
/// radii fall back to res-5 term sets directly (no child expansion — that is
/// what keeps the set bounded). Radius is clamped to 250km, the largest the
/// res-5 path covers inside [`MAX_FILTER_CELLS`].
pub fn filter_cells(lat: f64, lon: f64, radius_km: f64) -> Result<(FilterRes, Vec<u64>), GeoError> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return Err(GeoError::BadCoords);
    }
    let radius_km = radius_km.clamp(0.1, 250.0);
    // Average hex edge lengths: res5 ≈ 9.85km, res6 ≈ 3.72km, res7 ≈ 1.41km.
    let (filter_res, disk_res, edge_km) = if radius_km <= 8.0 {
        (FilterRes::R7, 7u8, 1.41)
    } else if radius_km <= 30.0 {
        (FilterRes::R7, 6u8, 3.72) // disk at res-6, expanded ×7 to res-7
    } else {
        (FilterRes::R5, 5u8, 9.85) // res-5 term set, no expansion
    };
    let ll = LatLng::new(lat, lon).map_err(|_| GeoError::BadCoords)?;
    let center = ll.to_cell(resolution(disk_res)?);
    let k = (radius_km / edge_km).ceil() as u32;

    let target_res = match filter_res {
        FilterRes::R7 => resolution(INDEX_RES)?,
        FilterRes::R5 => resolution(ANALYTICS_RES)?,
    };
    let mut out = Vec::new();
    for cell in center.grid_disk::<Vec<_>>(k) {
        if cell.resolution() >= target_res {
            out.push(u64::from(cell));
        } else {
            out.extend(cell.children(target_res).map(u64::from));
        }
        if out.len() > MAX_FILTER_CELLS {
            return Err(GeoError::TooManyCells);
        }
    }
    Ok((filter_res, out))
}

/// Filter set for an EXPLICIT H3 cell (the API's `h3` parameter): the cell's
/// res-7 children when it is coarse (6..=7 → direct children; ≤5 → its res-5
/// self/children to stay bounded), or its res-7 parent when finer than res-7.
pub fn cells_for_cell(cell: u64) -> Result<(FilterRes, Vec<u64>), GeoError> {
    let cell = CellIndex::try_from(cell).map_err(|_| GeoError::BadCoords)?;
    let res = u8::from(cell.resolution());
    if res > INDEX_RES {
        // Finer than the index stores: match the covering res-7 cell.
        let parent = cell
            .parent(resolution(INDEX_RES)?)
            .ok_or(GeoError::BadResolution)?;
        return Ok((FilterRes::R7, vec![u64::from(parent)]));
    }
    if res >= 6 {
        let cells: Vec<u64> = if res == INDEX_RES {
            vec![u64::from(cell)]
        } else {
            cell.children(resolution(INDEX_RES)?)
                .map(u64::from)
                .collect()
        };
        return Ok((FilterRes::R7, cells));
    }
    // Coarse cells go through the res-5 indexed field.
    let cells: Vec<u64> = if res == ANALYTICS_RES {
        vec![u64::from(cell)]
    } else {
        let children: Vec<u64> = cell
            .children(resolution(ANALYTICS_RES)?)
            .map(u64::from)
            .collect();
        if children.len() > MAX_FILTER_CELLS {
            return Err(GeoError::TooManyCells);
        }
        children
    };
    Ok((FilterRes::R5, cells))
}

/// Great-circle distance between a cell centroid and (lat, lon), in km —
/// the LTR `geo_distance_km` feature.
pub fn distance_km(cell: u64, lat: f64, lon: f64) -> Option<f64> {
    let (clat, clon) = cell_to_latlng(cell)?;
    let target = LatLng::new(lat, lon).ok()?;
    let cell_ll = LatLng::new(clat, clon).ok()?;
    Some(cell_ll.distance_km(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Geo property tests (SPEC §16 Phase-5 exit criterion).

    #[test]
    fn cell_roundtrip_and_containment() {
        // A known point lands in a res-7 cell whose centroid is nearby.
        let (lat, lon) = (52.5200, 13.4050); // Berlin
        let cell = latlng_to_cell(lat, lon, INDEX_RES).unwrap();
        let (clat, clon) = cell_to_latlng(cell).unwrap();
        let d = distance_km(cell, lat, lon).unwrap();
        assert!(d < 2.0, "res-7 centroid within ~edge length: {d}km");
        assert!((clat - lat).abs() < 0.1 && (clon - lon).abs() < 0.1);
    }

    #[test]
    fn parent_hierarchy_is_consistent() {
        let cell = latlng_to_cell(35.6762, 139.6503, INDEX_RES).unwrap(); // Tokyo
        let p5 = parent_at(cell, 5).unwrap();
        let p3 = parent_at(cell, 3).unwrap();
        // Parent at coarser res contains the finer parent's parent.
        assert_eq!(parent_at(p5, 3).unwrap(), p3);
        // Parent at own res is identity.
        assert_eq!(parent_at(cell, INDEX_RES).unwrap(), cell);
        // Distance from coarse parents' centroids grows but stays bounded.
        assert!(distance_km(p3, 35.6762, 139.6503).unwrap() < 100.0);
    }

    #[test]
    fn filter_cells_cover_center_and_respect_cap() {
        let (lat, lon) = (40.7128, -74.0060); // NYC
        for radius in [1.0, 10.0, 50.0, 200.0] {
            let (res, cells) = filter_cells(lat, lon, radius).unwrap();
            assert!(!cells.is_empty());
            assert!(cells.len() <= MAX_FILTER_CELLS, "radius {radius}");
            // The center cell (at the filter's resolution) must be in the set.
            let center_res = match res {
                FilterRes::R7 => INDEX_RES,
                FilterRes::R5 => ANALYTICS_RES,
            };
            let center = latlng_to_cell(lat, lon, center_res).unwrap();
            assert!(
                cells.contains(&center),
                "radius {radius}: center cell missing from filter set"
            );
        }
        // Resolution switches with radius: fine close-in, coarse far out.
        assert_eq!(filter_cells(lat, lon, 2.0).unwrap().0, FilterRes::R7);
        assert_eq!(filter_cells(lat, lon, 100.0).unwrap().0, FilterRes::R5);
    }

    #[test]
    fn explicit_cell_filters_by_resolution() {
        let r7 = latlng_to_cell(52.52, 13.40, 7).unwrap();
        let (res, cells) = cells_for_cell(r7).unwrap();
        assert_eq!(res, FilterRes::R7);
        assert_eq!(cells, vec![r7]);

        let r6 = latlng_to_cell(52.52, 13.40, 6).unwrap();
        let (res, cells) = cells_for_cell(r6).unwrap();
        assert_eq!(res, FilterRes::R7);
        assert_eq!(cells.len(), 7, "res-6 → its 7 res-7 children");
        assert!(cells.contains(&r7));

        let r5 = latlng_to_cell(52.52, 13.40, 5).unwrap();
        let (res, cells) = cells_for_cell(r5).unwrap();
        assert_eq!(res, FilterRes::R5);
        assert_eq!(cells, vec![r5]);

        let r3 = latlng_to_cell(52.52, 13.40, 3).unwrap();
        let (res, cells) = cells_for_cell(r3).unwrap();
        assert_eq!(res, FilterRes::R5);
        assert!(cells.contains(&r5), "coarse cell expands to res-5 children");

        let r9 = latlng_to_cell(52.52, 13.40, 9).unwrap();
        let (res, cells) = cells_for_cell(r9).unwrap();
        assert_eq!(res, FilterRes::R7);
        assert_eq!(cells, vec![r7], "finer than index → covering res-7 cell");
    }

    #[test]
    fn bigger_radius_never_shrinks_coverage_area() {
        let (lat, lon) = (48.8566, 2.3522); // Paris
        let small = filter_cells(lat, lon, 2.0).unwrap().1.len();
        let large = filter_cells(lat, lon, 30.0).unwrap().1.len();
        assert!(
            large > small,
            "30km ({large}) must cover more res-7 cells than 2km ({small})"
        );
    }

    #[test]
    fn invalid_inputs_refused() {
        assert_eq!(latlng_to_cell(91.0, 0.0, 7), Err(GeoError::BadCoords));
        assert_eq!(latlng_to_cell(0.0, 181.0, 7), Err(GeoError::BadCoords));
        assert_eq!(latlng_to_cell(0.0, 0.0, 16), Err(GeoError::BadResolution));
        assert!(cell_to_latlng(0).is_none());
    }
}
