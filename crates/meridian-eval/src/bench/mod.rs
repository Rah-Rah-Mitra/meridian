//! `meridian-bench` framework (SPEC §15): suite registry, shared config, and the
//! markdown+JSON report emitter. Suites are feature-gated; a suite that is not
//! compiled in reports `skipped`, and a gated suite that fails its gate makes the
//! whole run exit non-zero so a partial run can never masquerade as a passed gate.

#[cfg(any(feature = "bench-embed", feature = "bench-rerank"))]
use crate::stats::Rng;
use serde::Serialize;
use std::path::PathBuf;

pub mod evidence;
pub mod fusion;
pub mod ope;
pub mod spike;
pub mod synfarm;

#[cfg(feature = "bench-divergence")]
pub mod divergence;

#[cfg(feature = "bench-embed")]
pub mod embed;

#[cfg(feature = "bench-ann")]
pub mod ann;

#[cfg(feature = "bench-lexical")]
pub mod lexical;

#[cfg(feature = "bench-rerank")]
pub mod rerank;

#[cfg(feature = "bench-embed")]
pub mod thermal;

#[cfg(feature = "bench-anon")]
pub mod anon;

/// Shared run configuration, filled from CLI flags.
#[derive(Debug, Clone)]
pub struct BenchConfig {
    /// Directory with model artifacts (potion-base-8M/, ms-marco CE files).
    pub models_dir: PathBuf,
    /// JSONL corpus file ({"title":..,"body":..} per line) for the lexical suite.
    pub corpus: Option<PathBuf>,
    /// Lexical corpus size cap (Profile R default 100k; --full → 1M).
    pub max_docs: usize,
    /// Vector count for the ANN suite (1M in both profiles per the bench plan).
    pub ann_vectors: usize,
    /// Scratch directory for indexes / serialized stores (must be on the real disk,
    /// not tmpfs, so the disk suite and the 16K mmap smoke test are honest).
    pub scratch_dir: PathBuf,
    /// Thermal loop duration in seconds (600 per SPEC; shorter for smoke runs).
    pub thermal_secs: u64,
    /// Base URL of a RUNNING meridiand for the divergence probe (suite 12).
    pub api_base: String,
    /// Optional query list (one per line) for the divergence probe; defaults to
    /// the embedded region-sensitive set.
    pub queries: Option<PathBuf>,
    /// Repeats per query per lane for the divergence probe.
    pub repeats: usize,
    /// Suite 12b: run the cross-lane Phase-8 gate instead of the floor probe.
    pub cross_lane: bool,
    /// Suite 12b: the committed same-lane floor MEAN the gate tests against
    /// (default = the 2026-06-11 anon-lane probe, docs/plan/bench).
    pub floor_mean: f64,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            models_dir: PathBuf::from("models"),
            corpus: None,
            max_docs: 100_000,
            ann_vectors: 1_000_000,
            scratch_dir: PathBuf::from("bench-scratch"),
            thermal_secs: 600,
            api_base: "http://127.0.0.1:8080".to_owned(),
            queries: None,
            repeats: 8,
            cross_lane: false,
            floor_mean: 0.096,
        }
    }
}

/// Outcome of one suite.
#[derive(Debug, Serialize)]
pub struct SuiteResult {
    pub name: &'static str,
    /// Human description of the gate, if the suite has one.
    pub gate: Option<String>,
    /// None = informational / not compiled in; Some(bool) = gate verdict.
    pub gate_passed: Option<bool>,
    pub skipped: bool,
    pub metrics: serde_json::Map<String, serde_json::Value>,
    pub notes: Vec<String>,
    pub duration_ms: f64,
}

impl SuiteResult {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            gate: None,
            gate_passed: None,
            skipped: false,
            metrics: serde_json::Map::new(),
            notes: Vec::new(),
            duration_ms: 0.0,
        }
    }

    pub fn skipped(name: &'static str, why: &str) -> Self {
        let mut r = Self::new(name);
        r.skipped = true;
        r.notes.push(why.to_owned());
        r
    }

    pub fn metric(&mut self, key: &str, value: impl Into<serde_json::Value>) {
        self.metrics.insert(key.to_owned(), value.into());
    }

    pub fn note(&mut self, s: impl Into<String>) {
        self.notes.push(s.into());
    }

    pub fn gate(&mut self, description: &str, passed: bool) {
        self.gate = Some(description.to_owned());
        self.gate_passed = Some(passed);
    }
}

/// Whole-run report: environment header + suite results, emitted as md + json.
#[derive(Debug, Serialize)]
pub struct Report {
    pub version: &'static str,
    pub kernel: String,
    pub page_size: u64,
    pub nproc: usize,
    pub compiled_suites: Vec<&'static str>,
    pub profile_note: &'static str,
    pub suites: Vec<SuiteResult>,
}

impl Report {
    pub fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            kernel: std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .unwrap_or_default()
                .trim()
                .to_owned(),
            page_size: crate::probe::page_size(),
            nproc: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(0),
            compiled_suites: compiled_suites(),
            profile_note: if cfg!(debug_assertions) {
                "DEBUG BUILD — numbers are meaningless; use the CI release-bench artifact"
            } else {
                "release-bench (thin LTO, target-cpu=cortex-a76); product profile is \
                 fat LTO — gates re-validated under it at Phase-1 exit"
            },
            suites: Vec::new(),
        }
    }

    pub fn all_gates_pass(&self) -> bool {
        self.suites.iter().all(|s| s.gate_passed != Some(false))
    }

    pub fn to_markdown(&self) -> String {
        use std::fmt::Write;
        let mut md = String::new();
        let _ = writeln!(md, "# meridian-bench report\n");
        let _ = writeln!(
            md,
            "- version: {} · kernel: `{}` · page size: {} · cores: {}",
            self.version, self.kernel, self.page_size, self.nproc
        );
        let _ = writeln!(md, "- compiled suites: {}", self.compiled_suites.join(", "));
        let _ = writeln!(md, "- build: {}\n", self.profile_note);
        for s in &self.suites {
            let verdict = match (s.skipped, s.gate_passed) {
                (true, _) => "SKIPPED".to_owned(),
                (_, Some(true)) => "GATE PASS".to_owned(),
                (_, Some(false)) => "**GATE FAIL**".to_owned(),
                (_, None) => "info".to_owned(),
            };
            let _ = writeln!(
                md,
                "## {} — {} ({:.1}s)\n",
                s.name,
                verdict,
                s.duration_ms / 1e3
            );
            if let Some(g) = &s.gate {
                let _ = writeln!(md, "Gate: {g}\n");
            }
            if !s.metrics.is_empty() {
                let _ = writeln!(md, "| metric | value |\n|---|---|");
                for (k, v) in &s.metrics {
                    let _ = writeln!(md, "| {k} | {v} |");
                }
                let _ = writeln!(md);
            }
            for n in &s.notes {
                let _ = writeln!(md, "- {n}");
            }
            let _ = writeln!(md);
        }
        md
    }
}

impl Default for Report {
    fn default() -> Self {
        Self::new()
    }
}

/// Words sampled to build deterministic synthetic sentences (the embed cost of a
/// static model is tokenizer+lookup+mean-pool, so realistic word shapes suffice).
#[cfg(any(feature = "bench-embed", feature = "bench-rerank"))]
const WORDS: &[&str] = &[
    "search",
    "engine",
    "latency",
    "geography",
    "river",
    "history",
    "protocol",
    "network",
    "privacy",
    "model",
    "vector",
    "index",
    "battery",
    "climate",
    "village",
    "music",
    "theorem",
    "compiler",
    "harbor",
    "election",
    "museum",
    "galaxy",
    "enzyme",
    "railway",
    "festival",
    "border",
    "currency",
    "volcano",
    "library",
    "treaty",
];

#[cfg(any(feature = "bench-embed", feature = "bench-rerank"))]
pub(crate) fn synthetic_sentences(n: usize, rng: &mut Rng) -> Vec<String> {
    (0..n)
        .map(|_| {
            let len = 8 + rng.below(12);
            let words: Vec<&str> = (0..len).map(|_| WORDS[rng.below(WORDS.len())]).collect();
            words.join(" ")
        })
        .collect()
}

fn compiled_suites() -> Vec<&'static str> {
    // `mut` is unused only when every bench feature is off.
    #[allow(unused_mut)]
    let mut v = vec!["fusion", "synfarm", "spike", "evidence", "ope"];
    #[cfg(feature = "bench-embed")]
    v.extend(["embed", "thermal"]);
    #[cfg(feature = "bench-ann")]
    v.push("ann");
    #[cfg(feature = "bench-lexical")]
    v.extend(["lexical", "disk"]);
    #[cfg(feature = "bench-rerank")]
    v.push("rerank");
    #[cfg(feature = "bench-anon")]
    v.push("anon");
    #[cfg(feature = "bench-divergence")]
    v.push("divergence");
    v
}
