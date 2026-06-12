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

## 6. Post-v0.1.0 suites (Phases 7–9, added 2026-06-11)

Experiment-first protocol: suites 9, 10, and the suite-12 noise-floor probe run
**before** their features are implemented — their results fix the ADR-18/21
constants and the Phase-8 divergence gate. Statistical metrics (F1, FPR, JSD) are
build-profile-independent; latency suites still require the release-bench build.

| # | Suite | Method | Gate | Where it runs |
|---|---|---|---|---|
| 9 | `synfarm` | Generate syndication farms (1 original + N derived copies + M independents; primary AND held-out generator variants — risk #21), sweep shingle size × MinHash perms × cluster threshold, score pairwise F1 + false-merge vs ground truth; domain-dedup baseline reported | **F1 >0.8 AND false-merge <5% on both variants** (for the params chosen for ADR-18) | CI (pure in-process, deterministic) |
| 10 | `spike` | Hex-lattice cells (topology-equivalent to H3 res-5; production uses h3o grid_disk), Gamma-heterogeneous Poisson counts, planted last-day spikes at ×2/×4/×8; ratio baseline vs EB-shrinkage (quasi-NB variance) + Gi* + BH-FDR; FPR at matched TPR; detector constants fixed on the tuning variant, judged on an overdispersed NB hold-out | **≥3× FPR reduction at equal TPR** vs thresholded ratio (Poisson variant) AND ≥1× with held FDR on the NB hold-out | CI |
| 11 | `evidence-latency` | Micro-bench of sketch+cluster stage on 1000-candidate sets @100k corpus (P7, with the production sketch module) | ≤2ms added p50 fast-path | CI + device re-run at P7 exit |
| 12 | `divergence` | (a) noise-floor probe (P7): repeated identical queries same-lane (direct/direct, anon/anon) against a running meridiand → bootstrap CI of within-lane JSD over domain distributions; (b) cross-lane mode (P8): curated region-sensitive query set, direct vs anon | (a) informational — the floor IS the deliverable; (b) cross-lane JSD > floor at p<0.05 | device only, feature `bench-divergence` (network-touching; never CI) |
| 13 | `qpp` | NQC/Clarity vs measured per-query nDCG@10 on the eval set; ECE of the fused confidence (P8) | Spearman ρ ≥0.25; ECE reported (gate set at P8 exit) | CI |
| 14 | `ope` | IPS/DR estimators vs synthetic logged ground truth with known true arm values (P9) | estimator bias <5% | CI |
| 15 | `voi` | Replay deep-mode fetch traces with/without VoI: fetch count vs nDCG@10 frontier + evidence-diversity guard (P9) | ≥25% fewer fetches at equal nDCG@10 (±1%); median `independent_source_count` non-degrading | CI replay + device spot-check |

**Quality-eval extension (P7–P8):** the §2 100-query set gains a duplicate-heavy
subset (for alpha-nDCG@10) and a curated region-sensitive subset (for suite 12b);
a small BEIR subset is added for ranking sanity (task choice = operator Q9).
New metrics in `meridian-eval::metrics`: alpha-nDCG@10, ECE.

**Suite 15 result (2026-06-12): PASS on hold-out with frozen constants** —
VoI (additive-objective selector; see the ADR-26 amendment: Pandora's rule
measured wrong for page-level nDCG) reaches nDCG@10 within 0.0023 of
fetch-all at **30.5% fewer fetches**, median fetched clusters 3 vs
rank-greedy 2; two interim harness defects documented in module docs
(novelty-only value model opened chaff; per-round best-reset disabled the
stop). bench/2026-06-12-pi5-p9-voi.json. Planner wiring re-validates the
deep 2.5s gate on-device.

**Suite 13b result (2026-06-12, seeds 42 + 1337):** the duplicate-heavy MMR
gate **FAILED** on both seeds at every λ in a 0.5–0.95 sweep (alpha-nDCG gain
only at 7–11% plain-nDCG cost; symmetric token similarity demotes canonical
originals with their copies). `diversity=mmr` withdrawn pre-release; see
bench/2026-06-12-pi5-p8-gates.md. **Suite 14 result (2026-06-12): PASS** —
IPS rel. bias 2.0%, DR 0.74% (gate <5%), DR tighter (sd 0.024 vs 0.038);
bench/2026-06-12-pi5-p9-ope.json.

## 7. Phase-10 suites (added 2026-06-12)

Same experiment-first protocol: suites 16–18 run and fix their constants
BEFORE the features they gate merge; each carries a held-out generator
variant per the risk-#21 discipline. All three are statistical replays —
CI-runnable, build-profile-independent; the answer-mode latency row is
device-only at the exit.

| # | Suite | Method | Gate | Where it runs |
|---|---|---|---|---|
| 16 | `conformal` | Generator-built query sets (calibration + disjoint-seed hold-out + held-out generator variant) through the suite-13 harness → per-query (confidence score, nDCG@10); fit band thresholds λ_high/λ_med with the (n+1) finite-sample correction against targets (τ_high=0.5 @90%, τ_med=0.3 @80%, operator Q12); report hold-out selective coverage, band-quality monotonicity, ECE | **Hold-out coverage within 5pp of target per band AND strictly monotone mean nDCG across bands, on both generator variants** — else bands don't ship (risk #24) | CI |
| 17 | `changepoint` | Null (stationary noise), single-day spikes (suite-10 parity), and **multi-day ramps** (the case latest-day z is structurally blind to); (s, γ) sweep on the Poisson/linear-ramp tuning variant, judged FROZEN on an overdispersed-NB/convex-ramp hold-out; baseline = the shipped EB+z+BH detector over the same BH family | **Null FPR ≤ z on BOTH variants (no slack); ramp TPR ≥ z+0.2 on tuning AND ≥1.25×z on the hold-out (margin-on-tuning / no-collapse-on-hold-out, the suite-10 pattern); median delay past the 2× crossing ≤1d tuning / ≤2d hold-out; z spike parity on tuning; burst must add detections z misses or it does not ship** (ADR-28) | CI |
| 18 | `answer` | Suite-15 replay corpus extended: decisive originals carry an answer-bearing passage (copies carry truncated/paraphrased versions); compare best-passage extraction (pandora_walk + fetched-text CE) vs the snippet-head baseline (no fetch, CE over snippets); measure pandora-vs-additive fetches-to-best-find on the single-best objective | **Hit-rate (best_passage from a decisive original) ≥ baseline + 10pp on the hold-out; pandora fetches-to-best ≤ additive's** (its theoretical regime — measured, not assumed) | CI |
| 15b | `voi` ext. | Embedding-coverage study (P9 carry): real potion embeddings of the replay corpus; candidate value model = frozen v0.4.0 + coverage term; same hold-out protocol | **Amend-or-record:** ships ONLY if it beats the frozen selector on fetches-saved at equal nDCG with non-degrading clusters; a measured no closes the carry | CI |

**Suite 17 result (2026-06-12): PASS with frozen s = 2.0, γ = 1.0**
(`bench/2026-06-12-pi5-p10-changepoint.md`). The suite falsified its own
candidate twice before passing (lower-trimmed moments truncate the dispersion
evidence → hold-out null FPR 0.104; two-pass re-estimation is circular on
null series → 0.055); the shipped head-window estimator holds FPR at 0.025
≤ z's 0.0375 with ramp TPR 0.692 vs z's 0.408 (tuning) and +62% relative on
the hold-out. Gate constructs amended to the suite-10 margin/no-collapse
pattern, recorded in the bench doc; the FPR condition never bent. The
harness's single-slot `gate()` (a later call overwrites an earlier one) is
also recorded there — multi-condition suites must emit ONE combined gate.

**Standing gates at every post-v0.1.0 phase exit:** suites 1–8 re-run (no
regression vs the Phase-6 baseline: BM25 0.48ms p50, ANN 0.45ms, fusion 0.144ms,
heatmap 27ms, RSS ~250MB plateau); forget-correctness 100% (incl. ADR-19
structures); privacy smoke green; hermetic egress-invariant count monotonically
non-decreasing (13 → ≥16 at P8 → ≥17 at P9; as built at the P9 exit: 20, so the
floor is now ≥20).
