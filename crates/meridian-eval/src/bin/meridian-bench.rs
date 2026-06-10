//! `meridian-bench` — the SPEC §15 on-device benchmark suite.
//!
//! Phase-0 stub: enumerates the suite and exits non-zero for unimplemented benches so
//! it can never masquerade as a passed gate. Implementations land right after the
//! planning sign-off; every SPEC §6.3 latency number stays provisional until this has
//! run on the Pi 5 and `docs/plan/02-budgets.md` is updated with measured values.

use std::process::ExitCode;

/// (name, gate, description) — SPEC §15 items 1–8.
const BENCHES: &[(&str, &str, &str)] = &[
    (
        "embed",
        ">2k docs/s",
        "model2vec embed throughput at batch 1/32/256",
    ),
    (
        "ann",
        "p99 < 40ms @ ef=64",
        "USearch build 1M synthetic 256-d int8; RAM, recall@10",
    ),
    (
        "lexical",
        "BM25 top-1000 p50 < 30ms",
        "Tantivy index 1M docs; docs/s, bytes, query p50/p99",
    ),
    (
        "rerank",
        "sets batch/depth",
        "ort INT8 cross-encoder ms/pair at batch 1/4/8",
    ),
    (
        "fusion",
        "<2ms / 1000+200",
        "RRF + LTR criterion microbench",
    ),
    (
        "thermal",
        "no throttle flags",
        "10-min all-stage loop, vcgencmd temp + throttle",
    ),
    (
        "disk",
        "scratch >= 1.0GB",
        "merge amplification: peak transient bytes during force-merge",
    ),
    (
        "anon",
        "leak test passes",
        "Arti bootstrap time, circuit p50/p99, anon RSS delta, Tor-only egress",
    ),
];

fn main() -> ExitCode {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "list".to_owned());

    match arg.as_str() {
        "list" | "--help" | "-h" => {
            println!(
                "meridian-bench — on-device suite (SPEC §15). Usage: meridian-bench <name|all>\n"
            );
            for (name, gate, desc) in BENCHES {
                println!("  {name:<8} gate: {gate:<28} {desc}");
            }
            ExitCode::SUCCESS
        }
        name => {
            let known = BENCHES.iter().any(|(n, ..)| *n == name) || name == "all";
            if known {
                eprintln!(
                    "bench '{name}' is not implemented yet (Phase 0 pending planning sign-off)"
                );
            } else {
                eprintln!("unknown bench '{name}' — run `meridian-bench list`");
            }
            ExitCode::FAILURE
        }
    }
}
