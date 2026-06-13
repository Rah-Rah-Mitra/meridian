# Meridian Developer Guide — research methodologies & implementation

How Meridian is built and, more importantly, *how decisions are made*. Meridian is
a privacy-preserving, geo-aware analytical search appliance for a Raspberry Pi 5
(8 GB) + Hailo-8L. Everything that ships clears a pre-registered, on-device gate;
everything that doesn't is recorded as a first-class NO. This guide is the map to
both the methodology and the code.

Companions: [`api.md`](api.md) / [`userguide.md`](userguide.md) (endpoints),
[`privacy.md`](privacy.md) (deletion/anon), `docs/plan/00-adr.md` (the ADR ledger),
`docs/plan/bench/` (every measurement), `docs/research/` (review workpapers).

---

## 1. The method: experiment-first, judge-first, recorded-NO

The repository's central discipline is that a feature does not exist until a
*judge* that can falsify it lands and passes. Concretely:

- **Judge-first.** Before a feature ships, an eval suite (`crates/meridian-eval`)
  that measures its claim is written and **passes**. The suite is the contract;
  the feature is the implementation of a thing already proven measurable.
- **Pre-registered gates with MDE₈₀.** Each gate states the bar *and* the minimum
  detectable effect at 80% power for the sample size, so "it passed" is meaningful
  rather than noise. (`mde80_*` metrics appear in suite output.)
- **Tuning → frozen → hold-out + style-shift (risk #21).** Any threshold is swept
  on a tuning seed, **frozen**, then judged on a disjoint hold-out *and* a
  deliberately distribution-shifted ("style") variant — the no-collapse falsifier
  that killed conformal bands (see §7). Distinct RNG seeds; no re-fitting on the
  hold-out.
- **Measure on-device, never simulate.** Latency gates run on the actual Pi at
  n ≥ 16 interpolated medians (`crates/meridian-eval/src/stats.rs`,
  `percentile_ms`). Budgets: fast ≤ 25 ms, deep p50 ≤ 2.5 s, answer p50 ≤ 3.0 s
  (`docs/plan/02-budgets.md`).
- **A recorded NO is a deliverable.** Methods that fail their gate are documented
  with the measurement and never silently retried. Examples in the ledger:
  `diversity=mmr` (suite 13b), conformal confidence bands (suite 16), seasonal
  DOW deseasonalisation (suite 19, `bench/seasonal.rs`), answer-mode embedding
  prune (suite 18b, `bench/answer_embed.rs`), and V1 filtered-ANN (this release).
- **Honest failure at runtime.** The service never silently degrades: `degraded:
  [...]` names skipped stages, fail-closed lanes return `503`, and every
  analytical block states its *basis* so thin evidence reads as low-confidence,
  not as a false claim.

### The eval harness
`crates/meridian-eval` is the bench/eval crate. Suites are individually
feature-gated so the dev-Pi `cargo check` stays light (ADR-D1); CI builds with
`--features bench`. Each suite is `bench/<name>.rs` with `pub fn run(&BenchConfig)
-> SuiteResult`, registered in `bench/mod.rs` (`compiled_suites`) and dispatched in
`bin/meridian-bench.rs`. Run one on-device:

```bash
cargo run -p meridian-eval --bin meridian-bench --release -- <suite> --out bench-out
```

Suites of note: `fusion` (RRF+LTR microbench), `ann`/`ann_filtered` (USearch +
filtered-dense), `synfarm`/`evidence` (ADR-18 clustering), `voi`/`answer`/
`answer_trust` (VoI answer mode + the trust layer), `spike`/`changepoint`/`seasonal`
(trends), `lexical`, `rerank`, `embed`, `thermal`, `divergence`/`anon` (privacy).

---

## 2. Retrieval — hybrid lexical + dense

The shipped retrieval path (`crates/meridian-query/src/planner.rs`) fuses two
lanes and reranks:

1. **Lexical (BM25)** — `crates/meridian-index` over tantivy; index schema v3 with
   `h3_r7`/`h3_r5`/`ts` fast fields for geo/time filtering.
2. **Dense (ANN)** — `crates/meridian-vector` over **usearch** HNSW,
   `MetricKind::Cos`, **int8** quantization (M=16, ef_add=128, ef_search=64).
   Embeddings from `crates/meridian-embed` (model2vec/potion static, 256-dim).
3. **Fusion** — Reciprocal Rank Fusion (`crates/meridian-query/src/rrf.rs`,
   RRF k=60), then a linear **LTR** rerank (`crates/meridian-rank`).
4. **Deep rerank (opt-in)** — INT8 MiniLM cross-encoder
   (`crates/meridian-rerank`, via `ort`) on `mode=deep`.

**ADR-07 — the ANN fallback ladder.** usearch+SIMD (rung 1, via `numkong`) →
usearch portable C++ (rung 2) → `hnsw_rs` (rung 3). Rung 1 is the only one built;
the P7 exit found the portable i8 cosine path collapses with scale (recall 0.52
@10k → 0.0 @1M — a representation defect, not graph quality), so the build
hard-drops ISA kernels the compiler rejects rather than silently degrading.

**ADR-10 — the geo/time filter tradeoff.** Under a geo/time filter the dense lane
is dropped (usearch has no native filtered search; post-filtering would smuggle
out-of-area docs into RRF), so constrained queries are exact-BM25. v0.6.5 re-tested
this (V1): the missing dense lane costs < 2 pp nDCG@10 (anchored to the measured
+0.85 pp *unfiltered* hybrid gain), so the tradeoff stands. The exact int8 scan
(`VectorStore::exact_scan`) and predicate-HNSW (`VectorStore::filtered_search`,
usearch's filter callback — no fork) ship unused, ready if a real geo-eval flips
the call.

---

## 3. Evidence / derivation clustering (ADR-18)

The `evidence` block answers "how many *independent* sources?" — the question
domain-grouping cannot (cross-domain syndication; suite-9 baseline F1 0.054).

- **Sketches.** `crates/meridian-index/src/sketch.rs`: a 64-byte
  one-permutation-hash (OPH, densified, b=8) MinHash over **4-word shingles**
  (`SHINGLE_K=4`), computed at ingest and stored *deletably* (ADR-19).
- **Clustering.** `crates/meridian-query/src/evidence.rs::cluster_tags`:
  union-find over pairwise **containment** ≥ `CONTAINMENT_TAU` (0.3). Containment
  (|A∩B|/min) — not Jaccard — is used because real syndication truncates and adds
  boilerplate asymmetrically. The parameters were chosen by the `synfarm` sweep
  (suite 9, F1 > 0.8 / false-merge < 5% on two generator variants).
- **Canonical member.** The most-shingled member of a cluster — the superset its
  copies derive from. A 75%-truncated copy can out-BM25 its original but cannot
  out-shingle it; this drives `diversity=evidence`, which dominated the withdrawn
  `diversity=mmr` on both alpha-nDCG and plain nDCG (suite 13b).

---

## 4. Answer mode — value-of-information + the trust layer

`answer=true` (`crates/meridian-query/src/planner.rs`, ADR-29) reads pages and
extracts a single best passage, choosing *what to read* by value of information.

- **VoI / pandora_walk.** `crates/meridian-fetch/src/voi.rs`: a Weitzman
  Pandora's-box selector. Answer mode runs it in passage-CE units — gain =
  novelty, realized value = the fetched doc's best passage CE, stop when the best
  reservation index falls below the value in hand. Validated in `bench/voi.rs` /
  `bench/answer.rs` (suite 18: +10.7 pp answer hit-rate over the no-fetch
  baseline at fewer fetches).
- **H3 selective abstention** (`search.answer_abstain_threshold`). Withhold a
  low-`ce_score` winner rather than ship a weak answer — selective prediction
  *without* a coverage guarantee (conformal bands died in suite 16; the honesty
  wording carries). Judged in suite 20 `answer_trust`.
- **C1 claim corroboration** (v0.6.5, `bench/answer_trust.rs` + planner). Count
  DISTINCT ADR-18 clusters (≠ the winner's) whose top passage the cross-encoder
  finds states the same claim; same-cluster copies excluded structurally. The
  kill criterion — *a false "k sources agree" badge is worse than none* — drove a
  judge built on the **real** signal: planted claim text → the shipped sketch
  clustering → the **real ort INT8 ms-marco CE**, with cluster precision ≥ 0.9 and
  zero same-cluster leakage on tuning + a style shift, τ_corr frozen on tuning.
  It cleared at precision 0.98 / recall 1.0 / zero leakage (MDE₈₀ 0.031). The
  measured CE-batch cost (312 ms p50 — not the design's ~10–20 ms guess) was
  recorded, not absorbed, and the deadline capped so an overrun thins the count
  rather than inflating it.

---

## 5. Geo-analytics (ADR-10, ADR-21)

- **H3 everywhere.** `crates/meridian-geo` (h3o). Docs index `h3_r7` (res-7) +
  `h3_r5` (its parent, precomputed so large-radius filters need no query-time
  math); k-ring filters become tantivy `TermSetQuery` MUST clauses, radius clamped
  to 250 km. Ingest geo-tags from an offline GeoNames gazetteer FST (no network).
- **Heatmap hot-spots.** `/v1/geo/heatmap` runs Getis-Ord **Gi\*** over each
  cell's k-ring-1 neighborhood with **Benjamini-Hochberg FDR** across returned
  cells. `significant` (q ≤ 0.05) — not raw count — is the defensible hot-spot
  flag.

---

## 6. Trends (ADR-15, ADR-21, ADR-28)

`/v1/trends` over an opt-in GDELT pull (`crates/meridian-analytics`). All trends
describe *media coverage*, not ground truth (ADR-15).

- **Movers.** Rank EventRootCodes by `z`: an **empirical-Bayes-shrunk**,
  **overdispersion-aware** (quasi-Poisson/NB) standardized excess of the latest
  day over the window baseline, with BH-FDR across roots. `ratio` is kept for
  explainability with a `label` honesty marker when elevated-but-not-significant.
- **Burst** (ADR-28, suite 17). A second detector for *sustained* multi-day
  elevation — the blind spot of point `z` (a slow ramp inflates the baseline the
  next day is judged against). `significant` and `burst.active` are independent
  flags. Constants frozen by the suite-17 study.

---

## 7. What the gates killed (and why that's the point)

- **`diversity=mmr`** — token-overlap MMR demotes canonical originals along with
  copies; alpha-nDCG gains only at >1 % plain-nDCG cost (suite 13b). Replaced by
  `diversity=evidence`.
- **Conformal confidence bands** — 19 pp coverage collapse under a query-style
  shift (suite 16). The QPP `confidence` block ships *uncalibrated* instead; bands
  return only with a materially stronger predictor.
- **Seasonal DOW deseasonalisation** — worsened FPR; the overdispersion model
  already absorbs weekly variance (suite 19, `bench/seasonal.rs`). Three
  adversarial verifiers couldn't refute the NO.
- **V1 filtered ANN** — harm < 2 pp (this release; §2).

---

## 8. Privacy architecture

- **Anon lane, fail-closed.** `crates/meridian-egress` (embedded Arti + SOCKS
  RFC1929 isolation). Type-level `AnonClient` with no direct transport; the anon
  searxng sits on an `internal: true` network whose only route out is the Tor
  SOCKS listener — Arti down ⇒ zero egress (topology fail-closure). Invariant
  tests run on every PR touching egress.
- **No query-text logging.** `crates/meridian-privacy/src/redact.rs`: a
  denylist-redacting tracing layer (q/url/ip/host…) is the only writer of log
  output; a CI privacy-smoke greps real output/metrics/data for canaries
  (merge-blocking).
- **Deletability (ADR-19).** Every derived structure either joins the atomic
  forget transaction or rebuilds in the background. `/v1/forget` removes the doc
  from the lexical index, the vector store, **and** its derivation sketch in one
  transaction, and tombstones the content hash. A hermetic forget test
  (`crates/meridian-query/src/ingest.rs`) is merge-blocking (risk #17): a sketch
  may never outlive its document.
- **Routing log + OPE (ADR-24/25).** An off-by-default per-decision routing log
  feeds an off-policy (doubly-robust) ship-gate report (`/v1/decision-log/ope`,
  `bench/ope.rs`) — reporting only, never changes routing; ε-greedy stays unless
  the CI excludes zero on ≥10k decisions.

---

## 9. Implementation map

| Crate | Responsibility |
|---|---|
| `meridian-common` | config (`SearchConfig`, `VectorConfig`, …), shared types |
| `meridian-index` | tantivy BM25 index (schema v3), MinHash **sketches** |
| `meridian-vector` | usearch int8 HNSW (`search`, `exact_scan`, `filtered_search`) |
| `meridian-embed` | model2vec/potion static embeddings |
| `meridian-rerank` | INT8 MiniLM cross-encoder via `ort` (gnu/`ort-backend` only) |
| `meridian-rank` | RRF + linear LTR, `diversity::cluster_diversify`, QPP |
| `meridian-fetch` | robots-respecting fetch ladder, passage split, **VoI** |
| `meridian-query` | the planner, `evidence` clustering, ingest, `rrf`, forget |
| `meridian-geo` | H3 ops, gazetteer, Gi\* heatmap rollups |
| `meridian-analytics` | GDELT counters, EB/overdispersion movers, burst |
| `meridian-egress` | anon (Arti) + region lanes, fail-closed |
| `meridian-privacy` | redaction layer, secret types |
| `meridian-searx` | SearXNG client, bandit arm routing |
| `meridian-api` | axum router (all endpoints), bearer auth, embedded UI |
| `meridian-eval` | the bench/eval harness + every suite |
| `bins/meridiand` | the server binary |

---

## 10. Build & release

- **Appliance image** — FROM-scratch, **musl arm64**, `cargo zigbuild
  --target aarch64-unknown-linux-musl` (RUSTFLAGS `target-cpu=cortex-a76`, the
  numkong NEON+sdot kernel matrix pinned to the A76). No `ort` (ADR-02: the deep
  CE needs glibc ≥ 2.39). `deploy/Dockerfile`, < 120 MB.
- **Deep variant** — native gnu + `ort` (`meridiand --features rerank-ort`,
  `deploy/Dockerfile.deep`), for deployments that want answer-mode + corroboration
  live. A CI build artifact, not a published release tag.
- **CI** (`.github/workflows/ci.yml`, on push to main) — `cargo fmt --all --check`
  + `cargo clippy --workspace --all-targets -- -D warnings` + tests + multi-arch
  builds + bench + privacy-smoke + GHCR scratch/deep images.
- **Release** (`.github/workflows/release.yml`, on `v*` tags) — builds the
  multi-arch scratch image, tags `:VERSION`/`:latest`, SBOMs (syft), and a GitHub
  release from `docs/release-notes/v*.md` (which must exist before tagging).

### Contributor workflow
One bet per branch; judge-first → feature → on-device measure → budget re-check →
`fmt`+`clippy` locally → push. Keep `main` green. Don't re-propose a method the
ledger killed; if you must, the burden is a new judge that falsifies the original
NO.
