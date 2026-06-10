//! ANN vector store (SPEC §5, §8.2): USearch HNSW, cosine, int8-quantized
//! storage, RAM-resident with file persistence. Bench-validated on this device
//! (2026-06-10): 506MB @1M vectors, recall@10 0.98 @ ef=64, p99 0.95ms, and the
//! 16K-page mmap view() smoke passes (ADR-01).
//!
//! Fallback ladder (ADR-07): usearch+SIMD → usearch portable C++ → hnsw_rs
//! (offset-u8). Only the first rung is built; the others activate if the musl
//! cross-build tripwire (risk #3) ever fires.

use meridian_common::config::VectorConfig;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use usearch::Index;
use usearch::ffi::{IndexOptions, MetricKind, ScalarKind};

#[derive(Debug, thiserror::Error)]
#[error("vector store: {0}")]
pub struct VectorError(String);

fn err(e: cxx::Exception) -> VectorError {
    VectorError(e.to_string())
}

pub struct VectorStore {
    index: Index,
    path: PathBuf,
    /// Serializes add/reserve/save sequences; searches go lock-free (usearch is
    /// concurrent by design).
    write_lock: Mutex<()>,
    dims: usize,
}

impl VectorStore {
    pub fn open_or_create(
        data_dir: &Path,
        dims: usize,
        cfg: &VectorConfig,
    ) -> Result<Self, VectorError> {
        std::fs::create_dir_all(data_dir).map_err(|e| VectorError(e.to_string()))?;
        let path = data_dir.join("vectors.usearch");
        let options = IndexOptions {
            dimensions: dims,
            metric: MetricKind::Cos,
            quantization: ScalarKind::I8,
            connectivity: cfg.connectivity,
            expansion_add: cfg.expansion_add,
            expansion_search: cfg.expansion_search,
            multi: false,
        };
        let index = Index::new(&options).map_err(err)?;
        if path.exists() {
            index
                .load(path.to_string_lossy().as_ref())
                .map_err(|e| VectorError(format!("load: {e}")))?;
        }
        index
            .reserve((index.size() + 4096).max(8192))
            .map_err(err)?;
        Ok(Self {
            index,
            path,
            write_lock: Mutex::new(()),
            dims,
        })
    }

    /// Add (or re-add) one embedding. Growth is amortized under the write lock.
    pub fn add(&self, key: u64, vector: &[f32]) -> Result<(), VectorError> {
        if vector.len() != self.dims {
            return Err(VectorError(format!(
                "dimension mismatch: got {}, store is {}",
                vector.len(),
                self.dims
            )));
        }
        let _guard = self.write_lock.lock().expect("vector write lock");
        if self.index.size() + 1 > self.index.capacity() {
            let grown = (self.index.capacity() * 3 / 2).max(self.index.capacity() + 4096);
            self.index.reserve(grown).map_err(err)?;
        }
        // One vector per URL key: re-ingest of changed content under the same
        // URL replaces rather than appends (multi=false still appends on add).
        if self.index.contains(key) {
            let _ = self.index.remove(key);
        }
        self.index.add(key, vector).map_err(err)
    }

    /// Top-k cosine neighbors: `(url_key, similarity)` with similarity in
    /// [-1, 1] (usearch returns cosine DISTANCE = 1 - sim).
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>, VectorError> {
        let matches = self.index.search(query, k).map_err(err)?;
        Ok(matches
            .keys
            .iter()
            .zip(matches.distances.iter())
            .map(|(&key, &dist)| (key, 1.0 - dist))
            .collect())
    }

    /// Atomic full-file persist (tmp + rename). Coarse by design — SD-friendly
    /// cadence is the caller's job (`vector.persist_every_docs`).
    pub fn persist(&self) -> Result<(), VectorError> {
        let _guard = self.write_lock.lock().expect("vector write lock");
        let tmp = self.path.with_extension("usearch.tmp");
        self.index
            .save(tmp.to_string_lossy().as_ref())
            .map_err(err)?;
        std::fs::rename(&tmp, &self.path).map_err(|e| VectorError(e.to_string()))?;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.index.size()
    }

    pub fn is_empty(&self) -> bool {
        self.index.size() == 0
    }

    pub fn memory_usage_bytes(&self) -> usize {
        self.index.memory_usage()
    }

    pub fn disk_bytes(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIMS: usize = 512;
    const N: u64 = 500;

    /// Well-separated unit vector: a dominant component at a per-key dimension
    /// (unique for keys < DIMS) plus small deterministic noise. Separation is
    /// robust to int8 quantization and the portable (non-SIMD) kernels — the
    /// property under test is store/search/persist correctness, not ANN recall
    /// on hard data (that is the §15.2 bench's job).
    fn unit(seed: u64) -> Vec<f32> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).max(1);
        let mut v = vec![0.0f32; DIMS];
        v[(seed as usize) % DIMS] = 4.0;
        for slot in v.iter_mut() {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            *slot += ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32)
                * 0.1;
        }
        let n = v.iter().map(|a| a * a).sum::<f32>().sqrt().max(1e-9);
        v.iter_mut().for_each(|a| *a /= n);
        v
    }

    #[test]
    fn add_search_persist_reload_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "meridian-vec-test-{}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let cfg = VectorConfig::default();
        let store = VectorStore::open_or_create(&dir, DIMS, &cfg).unwrap();
        for key in 0..N {
            store.add(key, &unit(key)).unwrap();
        }
        // Self-retrieval: the vector's own key is its nearest neighbor.
        let hits = store.search(&unit(42), 5).unwrap();
        assert_eq!(hits[0].0, 42, "{hits:?}");
        assert!(hits[0].1 > 0.9, "self-sim should be ~1: {}", hits[0].1);

        // Replacement keeps one vector per key.
        store.add(42, &unit(9999)).unwrap();
        assert_eq!(store.len(), N as usize);

        store.persist().unwrap();
        drop(store);
        let reopened = VectorStore::open_or_create(&dir, DIMS, &cfg).unwrap();
        assert_eq!(reopened.len(), N as usize);
        let hits = reopened.search(&unit(7), 3).unwrap();
        assert_eq!(hits[0].0, 7);
        let _ = std::fs::remove_dir_all(dir);
    }
}
