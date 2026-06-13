# 01 — Re-baselining ledger: the commissioning brief vs the repo

Status: REVIEW — proposes, does not decide. As-of: v0.6.0 / `812d3d4` / 2026-06-13.

The research-review brief ("analysis date June 11, 2026") listed fourteen "known
baseline limitations to verify". The repo moved through Phases 8–10 and four
releases (v0.3.0–v0.6.0) on 2026-06-12/13, so the brief is partially stale. This
ledger is the claim-by-claim correction; `00-review.md` §2.1 carries the summary
table and every downstream section builds on the corrected baseline, not the
brief's.

## 1. Claims that are STILL TRUE (the live gaps)

| # | Brief's claim | Verdict | Evidence |
|---|---|---|---|
| R1 | Engine routing = three manually defined engine-subset arms, ε-greedy | **STILL TRUE** (as the *live default*; a linear-TS contextual policy is implemented dark behind a ship gate) | `crates/meridian-searx/src/bandit.rs` (3 arms, ε-greedy); ADR-25 status: linear TS in `crates/meridian-searx/src/contextual.rs`, `searx.contextual_policy` default OFF, flip gated on DR-OPE ≥10k organic decisions / 60-day sunset (~2026-08-11), currently 0 organic rows |
| R2 | Routing reward is binary "arm contributed a result to final top-10" | **STILL TRUE** | bandit reward site in `meridian-searx`; ADR-24 row schema carries a single `reward` field captured at the bandit-reward site |
| R3 | Query intent = four-class lexical heuristic | **STILL TRUE** | `crates/meridian-query/src/intent.rs` (103 lines, µs-scale heuristic per ADR-02 Phase-3 resolution: "no neural cost") |
| R4 | Anonymous-lane outcomes do not update shared routing state | **STILL TRUE — and strengthened since** | ADR-22 (compare halves pin engines, no bandit reads/rewards); ADR-24 ("anon-lane decisions are never logged", enforced "by construction" at the reward site); anon cache ephemeral (SPEC §12.4) |
| R5 | Fusion is fixed-parameter RRF | **STILL TRUE** | `crates/meridian-query/src/rrf.rs` (`RRF_K = 60`); `planner.rs:3` module doc "RRF(k=60)"; ADR-09 |
| R6 | Cold-start LTR is RRF-identity until operator training data exists | **PARTIAL** — cold-start is a hand-tuned *linear* scorer (pure-Rust Gemm behind the `Scorer` trait), not literal identity; GBDT→ONNX still awaits training data that does not exist | ADR-02 Phase-3 resolution; ADR-09 status note; `meridian-rank::LinearLtr` (imported at `planner.rs:14`); `domain_prior` deliberately weighted 0 in cold-start (ADR-10 P5 amendment) |
| R7 | Domain PageRank is over domain co-occurrence, not a hyperlink/citation graph | **STILL TRUE** | ADR-10 Phase-5 amendment: GDELT carries no hyperlink graph; `domain_prior` = petgraph `page_rank` over domains co-reporting the same event-root class in the same 15-min slice, ≤30-domain cliques, weight-1 edges dropped, 200k-edge cap, nightly recompute |
| R8 | Web results cannot receive exact geo/time filters from upstream engines | **STILL TRUE** | `docs/api.md` (geo/time filters apply to local results only; `scope=web` + geo filter → `400`; `scope=both` → `degraded: ["geo_web_unfiltered"]`); ADR-10 recorded tradeoff |
| R9 | Local ANN is skipped under geo/time filtering (no filtered vector search) | **STILL TRUE** | ADR-10 P5 amendment ("ANN under a geo/ts filter is dropped … usearch has no filtered search; post-filtering would smuggle out-of-area docs into RRF"); `docs/api.md` says the same in user-facing words |

## 2. Claims that are SUPERSEDED (the brief is stale)

| # | Brief's claim | What actually ships | Evidence |
|---|---|---|---|
| S1 | Trends rank movers by latest-count / window-mean | Movers rank by **empirical-Bayes-shrunk, quasi-negative-binomial z** with **Benjamini-Hochberg FDR** (q=0.05); raw ratio retained for explainability with a "likely low-sample noise" honesty label; plus a **two-state Kleinberg-style burst decode** (`s=2.0, γ=1.0`, exact Viterbi, head-window moment estimator) for sustained multi-day ramps | ADR-21 (suite-10 run: 3.76× FPR reduction at matched TPR; quasi-NB scale forced by an overdispersed hold-out failure; k-ring 0 for movers); ADR-28 + suite 17 (ramp TPR 0.692 vs z 0.408, null FPR 0.025 ≤ z's 0.0375, two falsified estimators on the record); `docs/api.md` /v1/trends |
| S2 | Geo analytics expose counts/trends but not significance, uncertainty, or spatial structure | Heatmap cells carry **Getis-Ord Gi\*** z over k-ring-1 neighborhoods with BH-FDR `q_value` and a `significant` flag; trends carry `shrunk_rate`/`z`/`q_value`/`significant` + honesty labels | ADR-21; `docs/api.md` /v1/geo/heatmap, /v1/trends; measured heatmap+Gi\* 23.7ms p50 (P7 exit) |
| S3 | Diversity is domain-cap based only | Domain cap remains in the fast path, and **`diversity=evidence`** (v0.6.0) reorders by ADR-18 derivation clusters: each cluster's **canonical** member (most-shingled superset) takes the cluster's best slot, copies defer; gate: alpha-nDCG@10 +0.19/+0.23 AND plain nDCG@10 +0.14/+0.20 (tuning/hold-out) — dominates MMR on both metrics | `docs/api.md` `diversity` param; suite 13c (`bench/2026-06-13-pi5-13c-evidence-diversity.md`); `planner.rs` `diversity_evidence` field |
| S4 | Evaluation focuses on nDCG/MRR/Recall, latency, thermal, privacy smoke | **18 numbered suites** + standing exit gates: synfarm (9), spike (10), evidence-latency (11), divergence/noise-floor (12), QPP (13, 13b MMR, 13c evidence-diversity), OPE (14), VoI (15, 15b), conformal (16, judge for withdrawn bands), changepoint (17), answer (18); metrics extended with alpha-nDCG@10 and ECE; experiment-first protocol (suite runs before the feature merges; constants frozen from sweeps; tuning-margin / hold-out-no-collapse gate idiom) | `docs/plan/04-bench-plan.md` §6–7; `crates/meridian-eval` |
| S5 | No answer mode / extraction beyond snippets | **Answer mode** (`answer=true`, v0.5.0): Weitzman `pandora_walk` fetch selection + extractive `best_passage` block (≤500-char sentence-aligned passage, CE-scored, `ce_score` explicitly "relevance, not correctness"); suite-18 hold-out: hit-rate 0.700 vs 0.593 no-fetch baseline (+10.7pp), 3.8× the additive page-selector at fewer fetches; `answer_passage_cap` 16→8 study won the 3.0s p50 row back (2502ms measured) | ADR-29; `bench/2026-06-12-pi5-p10-answer.md`; `bench/2026-06-13-pi5-answer-cap-study.md`; `docs/api.md` `best_passage` |
| S6 | No uncertainty exposure | Every response carries a `confidence` block: **NQC + Clarity** (KL of top-k language model vs collection) + an explicitly *uncalibrated* blended score, honest-use wording in api.md; plus per-request **JSD divergence vs a measured same-lane noise floor** in compare mode | ADR-23; `docs/api.md` confidence + divergence blocks; ADR-22 P7-exit amendment (noise floor p90 0.30, n=336 pairs) |
| S7 | No fetch economics / stopping | Deep mode has a **value-of-information fetch ladder** (ADR-26): additive-objective greedy (`additive_walk`) for page nDCG — suite-15 hold-out: −0.0023 nDCG at −30.5% fetches; `analysis` block emits `search_stopped_because` + `estimated_marginal_gain_remaining` | ADR-26; `bench/2026-06-12-pi5-p9-voi.json`; `docs/api.md` analysis block |

## 3. Methods already KILLED by the repo's own gates (re-proposal requires new evidence through the same judge)

| Method | Killed by | Record |
|---|---|---|
| Conformal/selective confidence bands | Suite 16 (2026-06-12): Q12 target unachievable outright (top-5% coverage 65% vs 51% base rate); **19pp absolute-coverage collapse** under a held-out query-STYLE shift; relative lift (+12–14pp) survives — i.e. the score is exactly the ranking-comparable signal ADR-23 already ships | ADR-27 **REFUTED**, bands withdrawn pre-ship; risk #24 tripwire fired as registered; the `conformal` subcommand (frontier rule, finite-sample fit, variant generator) is the standing judge for any stronger predictor |
| MMR diversity rerank | Suite 13b (seeds 42+1337): token-overlap MMR demotes canonical originals with their copies; alpha-nDCG gains only at 7–11% plain-nDCG cost at every λ ∈ 0.5–0.95 | `diversity=mmr` withdrawn pre-release; replaced by `diversity=evidence` (suite 13c) which dominates it on both metrics |
| Embedding-coverage term in the page-objective VoI value model | Suite 15b (2026-06-13): the signal is REAL (potion separates same-field paraphrase registers at 0.716 cosine vs 0.052 cross-field — what MinHash novelty cannot see) but **no swept combiner is profitable**: up to 41% fetch savings always at page-nDCG cost beyond the ±0.01 bar, because a paraphrase reveal still buys rank mass | ADR-26 suite-15b addendum: "RECORDED NO, carry closed"; `voi-embed` is the standing judge; answer-mode pruning explicitly nominated as the future use (redundant copies have no reveal reward there) |
| Pandora's-box stopping for the page objective | Suite 15: Weitzman optimizes the single best find; page nDCG is additive; the walk starved 2 of 3 subtopic clusters | ADR-26 amendment; `pandora_walk` retained for the single-best regime and *validated* there by suite 18 |
| Gi\* k-ring smoothing for trend movers | Suite 10: k-ring-1 smoothing dilutes isolated spikes, lost on both variants | ADR-21: k-ring 0 (per-cell EB z + BH) for movers; Gi\* k=1 retained for spatially-clustered heatmap hot-spots only |
| Pure-Poisson z scale | Suite 10: failed the overdispersed hold-out (0.83× — worse than the ratio baseline) | ADR-21: quasi-NB scale shipped |
| BOCPD (Bayesian online change-point detection) | A-priori in ADR-28: run-length posteriors + hazard zoo for ≤90-day small-count windows is machinery without a consumer; 2-state Viterbi is exact, deterministic, O(2n), sweepable | ADR-28 "Rejected" |
| Neural/deep routers, MCTS planning, full bandits-with-knapsacks | A-priori in ADR-25 (no training data, no GPU, unexplainable; NP-hard; the shed ladder already hard-gates the action set) | ADR-25 "Rejected alternatives" |
| usearch portable-C++ rung (ADR-07 rung 2) | P7 exit: i8 cosine recall collapses at scale (0.52 @10k → 0.0 @1M) vs numkong 0.98 | Risk #3 fired twice; rung 1 (numkong 7.7.0 per-kernel probes) restored; suite-2 RECALL gate now re-runs at every exit |

## 4. Hardware baseline the brief did not know about

The deployment Pi 5 carries a **Hailo-8L NPU** the repo does not use or mention:

| Fact | Value | Source (captured live on the device, 2026-06-13) |
|---|---|---|
| Device | Hailo-8L AI ACC M.2 B+M KEY MODULE EXT TMP, Device Architecture `HAILO8L` (13 TOPS INT8) | `hailortcli fw-control identify` |
| PCIe | `0001:04:00.0`; device capable 8GT/s x4; link **negotiated 5GT/s x1 (downgraded)** ≈ ~400–450 MB/s effective; Pi 5 exposes one lane; Gen3 (8GT/s ≈ ~900 MB/s) is a config.txt knob | `lspci -vv` LnkCap/LnkSta |
| Runtime | HailoRT 4.23.0 + PCIe driver 4.23.0 + TAPPAS core 5.1.0 + `python3-hailort`, `/dev/hailo0` present | `dpkg -l`, `ls /dev` |
| Models on disk | Vision CNNs only (yolo/resnet/scrfd HEFs) — no retrieval-relevant model is compiled | `/usr/share/hailo-models/` |
| Toolchain constraint | Retrieval models (cross-encoder, embedder) require offline compilation to HEF via the Hailo Dataflow Compiler, which is **x86_64-only** — it cannot run on this Pi; HEFs are version-coupled to HailoRT | Hailo DFC documentation (see `02-competitive.md` C4 ledger) |
| Repo awareness | **Zero.** No mention of Hailo/NPU in any crate, doc, ADR, or plan file | `grep -ri hailo` over the tree |
| Storage interplay | The M.2 slot hosts the NPU — this is *why* the device has no NVMe (ADR-D2's "no NVMe" Profile-R constraint and the SD-endurance risk #7 are partially the *price* of the NPU) | ADR-D2; device inspection |

CPU baselines any NPU proposal must beat (Profile R, measured): CE deep rerank
p50 204ms @ top-20/batch-4 (P3 exit), deep p50 2173ms, answer p50 2502ms @ cap 8;
embed 42.9k docs/s batch-32 (potion static embeddings — *not* a transformer);
ANN 0.45ms p50 @100k / p99 1.73ms @1M ef=128; BM25 0.48ms p50 @100k; fusion
0.144ms; RSS plateau ~250–261MB inside a 3GB cgroup; thermal max 75.7°C no
throttle (`docs/plan/02-budgets.md` §3, §6–7).
