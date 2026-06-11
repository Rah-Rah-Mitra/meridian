//! GeoNames gazetteer as an fst `Map` (SPEC §3 Geo row): lowercase place name →
//! packed (lat, lon). Built OFFLINE from `cities15000.txt` (builder below, run
//! by CI / `deploy/fetch-gazetteer.sh`), shipped in the image (≤10MB budget,
//! SPEC §6.1); never built on the appliance.
//!
//! Ingest-time geo-tagging: extract capitalized 1..=3-token phrases from the
//! title + lead text, longest match wins. Zero network, microseconds per doc.

use fst::Map;
use std::io::BufRead;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum GazetteerError {
    #[error("gazetteer io: {0}")]
    Io(#[from] std::io::Error),
    #[error("gazetteer fst: {0}")]
    Fst(String),
    #[error("gazetteer source line malformed")]
    BadSource,
}

/// Pack (lat, lon) into the fst's u64 value: two f32 bit patterns.
fn pack(lat: f64, lon: f64) -> u64 {
    ((lat as f32).to_bits() as u64) << 32 | (lon as f32).to_bits() as u64
}

fn unpack(v: u64) -> (f64, f64) {
    let lat = f32::from_bits((v >> 32) as u32) as f64;
    let lon = f32::from_bits((v & 0xFFFF_FFFF) as u32) as f64;
    (lat, lon)
}

pub struct Gazetteer {
    map: Map<Vec<u8>>,
}

impl Gazetteer {
    /// Load a prebuilt fst. The whole file is read into memory — the shipped
    /// artifact is a few MB (cities15000), well under the 10MB budget.
    pub fn open(path: &Path) -> Result<Self, GazetteerError> {
        let bytes = std::fs::read(path)?;
        let map = Map::new(bytes).map_err(|e| GazetteerError::Fst(e.to_string()))?;
        Ok(Self { map })
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Exact lookup (case-insensitive).
    pub fn lookup(&self, name: &str) -> Option<(f64, f64)> {
        self.map.get(name.trim().to_lowercase()).map(unpack)
    }

    /// Geo-tag a document: scan `title` then the first ~500 chars of `text`
    /// for capitalized 1..=3-token phrases known to the gazetteer; the longest
    /// (most specific) match wins, title before body.
    pub fn scan(&self, title: &str, text: &str) -> Option<(f64, f64)> {
        let lead: String = text.chars().take(500).collect();
        for source in [title, lead.as_str()] {
            if let Some(hit) = self.scan_one(source) {
                return Some(hit);
            }
        }
        None
    }

    fn scan_one(&self, text: &str) -> Option<(f64, f64)> {
        let tokens: Vec<&str> = text
            .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
            .filter(|t| !t.is_empty())
            .collect();
        // Longest phrases first so "New York City" beats "York".
        for window in (1..=3).rev() {
            for chunk in tokens.windows(window) {
                // Candidate phrases start with an uppercase letter — cheap
                // filter that keeps the scan O(tokens) in practice.
                if !chunk[0].chars().next().is_some_and(char::is_uppercase) {
                    continue;
                }
                let phrase = chunk.join(" ").to_lowercase();
                if let Some(v) = self.map.get(&phrase) {
                    return Some(unpack(v));
                }
            }
        }
        None
    }
}

/// Offline builder: GeoNames `cities15000.txt` (tab-separated) → fst map.
/// Keys: lowercase `name` and `asciiname`; duplicate names keep the larger
/// population (the reader expects "Paris" to mean the big one).
pub fn build_from_geonames<R: BufRead>(reader: R, out: &Path) -> Result<usize, GazetteerError> {
    let mut best: std::collections::BTreeMap<String, (u64, u64)> =
        std::collections::BTreeMap::new();
    for line in reader.lines() {
        let line = line?;
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 15 {
            continue; // tolerate stray lines; GeoNames ships clean data
        }
        let (name, ascii) = (cols[1], cols[2]);
        let lat: f64 = cols[4].parse().map_err(|_| GazetteerError::BadSource)?;
        let lon: f64 = cols[5].parse().map_err(|_| GazetteerError::BadSource)?;
        let pop: u64 = cols[14].parse().unwrap_or(0);
        let packed = pack(lat, lon);
        for key in [name.to_lowercase(), ascii.to_lowercase()] {
            if key.is_empty() {
                continue;
            }
            let entry = best.entry(key).or_insert((0, packed));
            if pop >= entry.0 {
                *entry = (pop, packed);
            }
        }
    }
    let file = std::io::BufWriter::new(std::fs::File::create(out)?);
    let mut builder = fst::MapBuilder::new(file).map_err(|e| GazetteerError::Fst(e.to_string()))?;
    let count = best.len();
    for (key, (_pop, packed)) in best {
        builder
            .insert(key, packed)
            .map_err(|e| GazetteerError::Fst(e.to_string()))?;
    }
    builder
        .finish()
        .map_err(|e| GazetteerError::Fst(e.to_string()))?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "2950159\tBerlin\tBerlin\tBerlino\t52.52437\t13.41053\tP\tPPLC\tDE\t\t16\t00\t11000\t11000000\t3426354\t74\t43\tEurope/Berlin\t2023-10-12\n\
5128581\tNew York City\tNew York City\tNYC\t40.71427\t-74.00597\tP\tPPL\tUS\t\tNY\t061\t\t\t8804190\t10\t57\tAmerica/New_York\t2023-10-12\n\
4074267\tBerlin\tBerlin\t\t44.46867\t-71.18508\tP\tPPL\tUS\t\tNH\t007\t\t\t9367\t311\t317\tAmerica/New_York\t2023-10-12\n";

    fn sample_gazetteer() -> Gazetteer {
        // Unique file per CALL: both tests build an FST in parallel under the
        // same pid — a shared path lets the writers interleave and corrupt the
        // file (observed as a flaky `Fst("FST error")` in CI).
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("meridian-gaz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!(
            "test-{}.fst",
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let n = build_from_geonames(std::io::Cursor::new(SAMPLE), &path).unwrap();
        assert!(n >= 2);
        Gazetteer::open(&path).unwrap()
    }

    #[test]
    fn duplicate_names_resolve_to_larger_population() {
        let g = sample_gazetteer();
        let (lat, _lon) = g.lookup("Berlin").unwrap();
        assert!(
            (lat - 52.52).abs() < 0.1,
            "must be Berlin DE, not Berlin NH"
        );
        assert!(g.lookup("no such place").is_none());
    }

    #[test]
    fn scan_prefers_longest_match_and_title() {
        let g = sample_gazetteer();
        let (lat, lon) = g.scan("Transit history of New York City", "").unwrap();
        assert!((lat - 40.714).abs() < 0.01 && (lon + 74.006).abs() < 0.01);
        // Lowercase mentions don't trigger (capitalization filter).
        assert!(
            g.scan("no places here", "just berlin lowercase text")
                .is_none()
        );
        // Body text matches when the title has nothing.
        assert!(
            g.scan("Untitled", "A report from Berlin yesterday")
                .is_some()
        );
    }
}
