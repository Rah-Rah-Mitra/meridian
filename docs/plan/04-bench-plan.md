# 04 — On-Device Benchmark Plan (BUILD & RUN FIRST)

> SPEC §1.5 / §15. **EXECUTED 2026-06-10** — results in
> [bench/2026-06-10-pi5-report.md](bench/2026-06-10-pi5-report.md); budgets
> re-issued. All gates pass; rerank unmeasured (tract op gap, ADR-02). The plan
> below remains the reference for re-runs at later phase exits.

## 0. Device & invariants captured in every report header

`uname -r`, `getconf PAGESIZE` (currently **16384** — ADR-01), `vcgencmd
measure_clock arm`, `vcgencmd get_throttled`, storage medium (**SD card** on the
first device — Profile R; SPEC numbers assume NVMe), free disk, zram state, ambient
notes (active cooling present), and the binary's build flags
(`target-cpu=cortex-a76`, musl/gnu per ADR-02).

## 1. Suite (order matters — cheap gates first)

| # | Bench | Method | Gate (Profile F) | Profile R note |
|---|---|---|---|---|
| 1 | `embed` | model2vec potion-base-8M, batch 1/32/256, 10k synthetic sentences | >2k docs/s | same gate — CPU-bound, storage-independent |
| 2 | `ann` | USearch 1M×256-d int8 synthetic (Gaussian mixture), M=16, efc=128; measure build RAM, p50/p99 @ ef=64, recall@10 vs brute force on 1k queries | p99 <40ms, recall@10 ≥0.95 | run at 1M even on Profile R (synthetic vectors fit: ~600MB transient — gate on free disk first) |
| 3 | `lexical` | Tantivy: index Wikipedia slice (1M docs F / 100k R), measure docs/s, bytes, BM25 top-1000 p50/p99 over 1k queries | p50 <30ms | R uses 100k + extrapolation curve (also measure at 10k/50k/100k to fit the curve) |
| 4 | `rerank` | ort INT8 CE (`model_qint8_arm64.onnx`), 256-token pairs, batch 1/4/8 | informational → sets default depth/batch | verify dotprod kernels active (`ort` profiling + `cargo asm` spot check) |
| 5 | `fusion` | criterion: RRF (1000+200 lists) + LTR features+inference (100 docs) | <2ms total | — |
| 6 | `thermal` | 10-min loop of stages 1–5, log `vcgencmd measure_temp` + throttle flags every 5s | no throttle bits; temp <80°C | active cooler assumed but verified here |
| 7 | `disk` | ingest 1M (F) / 100k (R), force merges, sample `statvfs` at 1Hz → peak transient bytes, write amplification | transient ≤1.0GB | R: also record total bytes written (SD endurance datum) |
| 8 | `anon` | Arti: cold + warm bootstrap time, circuit-build p50/p99 (20 circuits), end-to-end anon fetch p50 (10 fetches of a known endpoint), RSS delta; **leak test**: run anon-only load with all other egress firewalled (nftables counter on non-Tor destinations = 0) | bootstrap <60s warm; zero non-Tor packets | requires `anon` profile enabled for the bench run only |

## 2. Quality eval (Phase 2, not Phase 0)

100-query labeled set (graded 0–3, trec qrels) from the operator's corpus domain
(default assumption: research/technical-web — see 06-operator-questions Q1).
Report nDCG@10, MRR@10, Recall@100 for BM25 / hybrid / hybrid+LTR / deep.
Acceptance: hybrid ≥ BM25; no stage regresses its predecessor.

## 3. Load & soak

- `oha` fast-mode at 2/5/10/20 rps ×10min (Profile R stops at 10 rps); record
  p50/p99/error%/temp/RSS per step.
- Anon lane separately at 1/2 rps.
- Privacy smoke test (SPEC §15) runs in CI **and** on-device: canary query string +
  canary client IP driven through search/ingest/fetch/anon paths → zero occurrences
  in logs, /metrics, or on disk; no Set-Cookie; `/v1/forget` erases the canary doc;
  egress capture shows no non-allowlisted host.

## 4. What each result re-derives

| Measurement | Re-derives |
|---|---|
| embed throughput | ingest pipeline sizing (never the bottleneck — confirm) |
| ANN p99 vs ef | `ef_search` default; whether BQ+rescore is needed earlier than 1.5M |
| BM25 p50 @ corpus sizes | fast-mode local budget; Profile R doc ceiling |
| CE ms/pair | rerank depth (20?) and batch (4?); deep-mode deadline |
| thermal curve | shed thresholds (78/82°C) sanity |
| merge transient | scratch floor (1.0GB F / 0.75GB R) |
| Arti bootstrap/circuit | anon deadline (8s?), circuit cap (6?), bootstrap UX |

## 5. Bench harness engineering notes

- Single binary `meridian-bench` (already stubbed) — subcommand per suite, `all`
  runs in the order above, exits non-zero if any *gated* suite fails its gate.
- Emits markdown (human) + JSON (tracked in-repo for regression diffing).
- Corpus downloader streams + caps disk; never leaves >1.5GB of scratch.
- Suites 1–5 are pure-Rust/in-process; 6–8 shell out (`vcgencmd`, `nft`) and are
  feature-gated `bench-device` so CI (x86) builds but skips them.
