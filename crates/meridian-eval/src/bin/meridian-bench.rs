//! `meridian-bench` — the SPEC §15 on-device benchmark suite.
//!
//! Suites are feature-gated (see meridian-eval's Cargo features); a suite that is
//! requested but not compiled in, or that fails its gate, makes the run exit
//! non-zero — a partial run can never masquerade as a passed gate.
//!
//! Usage:
//!   meridian-bench list
//!   meridian-bench <suite>|all [--models-dir D] [--corpus F] [--docs N]
//!                  [--ann-vectors N] [--scratch D] [--thermal-secs S] [--out D]

use meridian_eval::bench::{BenchConfig, Report, SuiteResult};
use std::path::PathBuf;
use std::process::ExitCode;

const SUITES: &[(&str, &str)] = &[
    (
        "embed",
        "model2vec throughput at batch 1/32/256 — gate >2k docs/s",
    ),
    (
        "ann",
        "USearch 1M×256d int8 build + search + recall + 16K mmap view — gate p99 <40ms",
    ),
    (
        "lexical",
        "tantivy index + BM25 top-1000 latency curve — gate p50 <30ms",
    ),
    (
        "rerank",
        "INT8 cross-encoder ms/pair at batch 1/4/8 (tract) — informational",
    ),
    ("fusion", "RRF + linear LTR microbench — gate <2ms p50"),
    (
        "thermal",
        "sustained all-stage loop, temp + throttle flags — gate: no throttling",
    ),
    (
        "disk",
        "merge amplification during lexical suite — gate transient ≤0.75GB",
    ),
    (
        "anon",
        "Arti bootstrap + isolated circuit timing + RSS delta — informational",
    ),
    (
        "synfarm",
        "suite 9: syndication-farm sketch sweep — gate F1 >0.8 + false-merge <5% (both variants)",
    ),
    (
        "spike",
        "suite 10: planted-spike trends study, ratio vs EB+Gi*+BH — gate ≥3× FPR reduction",
    ),
    (
        "divergence",
        "suite 12 probe: same-lane JSD noise floor vs a running meridiand — informational (device)",
    ),
    (
        "evidence",
        "suite 11: sketch-lookup + containment clustering latency — gate ≤2ms p50 @ limit 50",
    ),
    (
        "ope",
        "suite 14: IPS/DR offline-policy-evaluation vs synthetic truth — gate bias <5%",
    ),
    (
        "voi",
        "suite 15: VoI fetch replay — gate ≥25% fewer fetches at equal nDCG@10 + diversity guard",
    ),
    (
        "changepoint",
        "suite 17: multi-day-ramp burst study, two-state Viterbi vs EB-z — gate FPR≤z + TPR+0.2 + delay≤1d",
    ),
    (
        "answer",
        "suite 18: best-passage replay, pandora vs additive vs snippet baseline — gate +10pp hit-rate",
    ),
    (
        "voi-embed",
        "suite 15b: embedding-coverage study vs the frozen v0.4.0 selector — amend-or-record (needs models/)",
    ),
    (
        "answer_trust",
        "suite 20: answer-mode trust layer — C1 corroboration (precision≥0.9, no same-cluster leakage) + H3 abstention (selective hit ≥+5pp at ≤20% abstain, no style collapse)",
    ),
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut suite = String::from("list");
    let mut cfg = BenchConfig::default();
    let mut out_dir = PathBuf::from("bench-out");

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let take = |i: &mut usize| -> Option<String> {
            *i += 1;
            args.get(*i).cloned()
        };
        match arg.as_str() {
            "--models-dir" => cfg.models_dir = take(&mut i).map(PathBuf::from).unwrap_or_default(),
            "--corpus" => cfg.corpus = take(&mut i).map(PathBuf::from),
            "--docs" => cfg.max_docs = take(&mut i).and_then(|v| v.parse().ok()).unwrap_or(100_000),
            "--ann-vectors" => {
                cfg.ann_vectors = take(&mut i)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1_000_000);
            }
            "--scratch" => cfg.scratch_dir = take(&mut i).map(PathBuf::from).unwrap_or_default(),
            "--thermal-secs" => {
                cfg.thermal_secs = take(&mut i).and_then(|v| v.parse().ok()).unwrap_or(600);
            }
            "--api-base" => {
                cfg.api_base = take(&mut i).unwrap_or_else(|| cfg.api_base.clone());
            }
            "--queries" => cfg.queries = take(&mut i).map(PathBuf::from),
            "--cross-lane" => cfg.cross_lane = true,
            "--floor-mean" => {
                cfg.floor_mean = take(&mut i).and_then(|v| v.parse().ok()).unwrap_or(0.096);
            }
            "--repeats" => cfg.repeats = take(&mut i).and_then(|v| v.parse().ok()).unwrap_or(8),
            "--out" => out_dir = take(&mut i).map(PathBuf::from).unwrap_or_default(),
            s if !s.starts_with("--") => suite = s.to_owned(),
            s => {
                eprintln!("unknown flag {s}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    if suite == "list" || suite == "--help" {
        println!("meridian-bench — on-device suite (SPEC §15). Usage: meridian-bench <name|all>\n");
        for (name, desc) in SUITES {
            println!("  {name:<8} {desc}");
        }
        return ExitCode::SUCCESS;
    }
    if suite != "all" && !SUITES.iter().any(|(n, _)| *n == suite) {
        eprintln!("unknown suite '{suite}' — run `meridian-bench list`");
        return ExitCode::FAILURE;
    }

    let mut report = Report::new();
    let wants = |name: &str| suite == "all" || suite == name;
    // disk is produced by the lexical pass; requesting either runs both.
    let wants_lexical = wants("lexical") || wants("disk");

    println!(
        ">> meridian-bench v{} — kernel {}, page size {}, {} cores",
        report.version, report.kernel, report.page_size, report.nproc
    );

    if wants("fusion") {
        run_and_print(&mut report, meridian_eval::bench::fusion::run(&cfg));
    }

    if wants("synfarm") {
        run_and_print(&mut report, meridian_eval::bench::synfarm::run(&cfg));
    }

    if wants("spike") {
        run_and_print(&mut report, meridian_eval::bench::spike::run(&cfg));
    }

    if wants("evidence") {
        run_and_print(&mut report, meridian_eval::bench::evidence::run(&cfg));
    }

    if wants("ope") {
        run_and_print(&mut report, meridian_eval::bench::ope::run(&cfg));
    }

    if wants("voi") {
        run_and_print(&mut report, meridian_eval::bench::voi::run(&cfg));
    }

    if wants("changepoint") {
        run_and_print(&mut report, meridian_eval::bench::changepoint::run(&cfg));
    }

    if wants("answer") {
        run_and_print(&mut report, meridian_eval::bench::answer::run(&cfg));
    }

    if wants("answer_trust") {
        run_and_print(&mut report, meridian_eval::bench::answer_trust::run(&cfg));
    }

    if wants("voi-embed") {
        run_and_print(&mut report, meridian_eval::bench::voi_embed::run(&cfg));
    }

    if wants("embed") {
        #[cfg(feature = "bench-embed")]
        run_and_print(&mut report, meridian_eval::bench::embed::run(&cfg));
        #[cfg(not(feature = "bench-embed"))]
        run_and_print(
            &mut report,
            SuiteResult::skipped("embed", "not compiled in (bench-embed)"),
        );
    }

    if wants("rerank") {
        #[cfg(feature = "bench-rerank")]
        run_and_print(&mut report, meridian_eval::bench::rerank::run(&cfg));
        #[cfg(not(feature = "bench-rerank"))]
        run_and_print(
            &mut report,
            SuiteResult::skipped("rerank", "not compiled in (bench-rerank)"),
        );
    }

    if wants("ann") {
        #[cfg(feature = "bench-ann")]
        run_and_print(&mut report, meridian_eval::bench::ann::run(&cfg));
        #[cfg(not(feature = "bench-ann"))]
        run_and_print(
            &mut report,
            SuiteResult::skipped("ann", "not compiled in (bench-ann)"),
        );
    }

    if wants_lexical {
        #[cfg(feature = "bench-lexical")]
        for r in meridian_eval::bench::lexical::run(&cfg) {
            run_and_print(&mut report, r);
        }
        #[cfg(not(feature = "bench-lexical"))]
        run_and_print(
            &mut report,
            SuiteResult::skipped("lexical", "not compiled in (bench-lexical)"),
        );
    }

    if wants("thermal") {
        #[cfg(feature = "bench-embed")]
        run_and_print(&mut report, meridian_eval::bench::thermal::run(&cfg));
        #[cfg(not(feature = "bench-embed"))]
        run_and_print(
            &mut report,
            SuiteResult::skipped("thermal", "not compiled in (needs bench-embed)"),
        );
    }

    if wants("anon") {
        #[cfg(feature = "bench-anon")]
        run_and_print(&mut report, meridian_eval::bench::anon::run(&cfg));
        #[cfg(not(feature = "bench-anon"))]
        run_and_print(
            &mut report,
            SuiteResult::skipped("anon", "not compiled in (bench-anon)"),
        );
    }

    if wants("divergence") {
        #[cfg(feature = "bench-divergence")]
        run_and_print(&mut report, meridian_eval::bench::divergence::run(&cfg));
        #[cfg(not(feature = "bench-divergence"))]
        run_and_print(
            &mut report,
            SuiteResult::skipped("divergence", "not compiled in (bench-divergence)"),
        );
    }

    // Emit report (md + json).
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("cannot create out dir: {e}");
        return ExitCode::FAILURE;
    }
    let md = report.to_markdown();
    let json = serde_json::to_string_pretty(&report).unwrap_or_default();
    let md_path = out_dir.join("report.md");
    let json_path = out_dir.join("report.json");
    if std::fs::write(&md_path, &md).is_err() || std::fs::write(&json_path, &json).is_err() {
        eprintln!("failed writing report files");
        return ExitCode::FAILURE;
    }
    println!(
        "\n>> report: {} / {}",
        md_path.display(),
        json_path.display()
    );

    let skipped_requested = report.suites.iter().any(|s| s.skipped);
    if report.all_gates_pass() && !skipped_requested {
        println!(">> all gates passed");
        ExitCode::SUCCESS
    } else {
        println!(">> GATE FAILURES or skipped suites — see report");
        ExitCode::FAILURE
    }
}

fn run_and_print(report: &mut Report, result: SuiteResult) {
    let verdict = match (result.skipped, result.gate_passed) {
        (true, _) => "SKIPPED",
        (_, Some(true)) => "gate PASS",
        (_, Some(false)) => "gate FAIL",
        (_, None) => "info",
    };
    println!(
        ">> suite {:<8} {} ({:.1}s)",
        result.name,
        verdict,
        result.duration_ms / 1e3
    );
    for (k, v) in &result.metrics {
        println!("     {k}: {v}");
    }
    for n in &result.notes {
        println!("     note: {n}");
    }
    report.suites.push(result);
}
