# T4 — Tracks E (geo analytics), F (streaming/deletion), G (spectral/RMT), I (privacy-preserving analytics)

Status: DRAFT workpaper. As-of: v0.6.0 / 2026-06-13. Binding baseline:
`01-rebaseline.md` (corrected claims R1–R9, superseded S1–S7, killed list §3).
Hard constraints inherited unchanged: forget-correctness 100%, anon firewall,
no query logging, Profile-R budgets (trends/heatmap ≤60ms p50,
`docs/plan/02-budgets.md:140`; burst stage ≤+10ms, `:147`), experiment-first
protocol with tuning-margin / hold-out-no-collapse gates.

A data-flow fact both E and F hinge on, established up front because the brief
conflates the two surfaces: **/v1/geo/heatmap counts operator-ingested docs**
(Tantivy fast-field scan, `crates/meridian-index/src/lexical.rs:368–465`;
"Heatmaps are computed from the operator's own indexed documents",
`docs/privacy.md:147–148`), while **/v1/trends consumes GDELT counters**
(`crates/meridian-analytics/src/trends.rs:65–72` over
`store.rs:117–154`; "never stores GDELT raw rows", `store.rs:6`). Gi*+BH runs
on the doc surface (`crates/meridian-analytics/src/stats.rs:158–207`); EB
quasi-NB z + BH + burst run on the GDELT surface (`stats.rs:48–141`,
`burst.rs:76–134`). There is **no spatial statistic over the GDELT surface
today** — trends accept one optional `h3_r5` filter cell at a time
(`trends.rs:65–71`). That asymmetry reorders Track E below.

---

## Track E — spatial structure beyond what shipped

### E1 — Moran's I / LISA over H3 res-5 counts

| Field | Content |
|---|---|
| Proposal | Global Moran's I (diagnostic scalar) + local Moran (LISA) with quadrant labels (HH/LL/HL/LH), conditional-permutation inference, BH across cells |
| Meridian problem | Gi*+BH answers "this cell(+ring) is hot" (`stats.rs:158–207`). LISA's only non-redundant statement is the **HL/LH spatial outlier**: "this cell is anomalously quiet/hot relative to its neighbors" — on GDELT a LL/LH cell inside a hot cluster is a *media-coverage hole*, an honest-signals statement in the ADR-15 spirit |
| Math | I = (n/W)·(zᵀWz/zᵀz); local Iᵢ = zᵢ·Σⱼwᵢⱼzⱼ; inference by conditional permutation (hold cell i, permute the rest), p̂ = (1+#{I* ≥ I})/(P+1) |
| Expected gain | Global I: ~zero — on event counts it is dominated by population geography and will be significant on essentially any day ("clustering exists at all" is vacuous for a news surface). LISA: one new label class (coverage holes) |
| Complexity | O(n·P·k̄), k̄≈6 neighbors (`crates/meridian-geo/src/h3.rs:50–58`) |
| Pi-5 cost | The killer is inference granularity, not raw ops: at the realistic GDELT surface n≈12k res-5 cells (`store.rs:352` test comment), the BH rank-1 threshold is q/n ≈ 4.2e-6, so permutation p-values need P ≥ ~240k to be able to clear it (min p = 1/(P+1)). 12k×2.4e5×6 ≈ 1.7e10 ops ⇒ **minutes-scale, nightly only**. P=999 (~7e7 ops, tens of ms) yields a p-floor of 1e-3 — useless under BH at this family size. The normal-approximation escape hatch recreates exactly the moment-reliability compromises Gi* already makes, shrinking LISA's marginal value further |
| Privacy impact | None (GDELT-only, or operator-doc counts already exposed by the heatmap) |
| Required data | Existing counters/heatmap cells; no new state |
| Evaluation | Suite-10 lattice extended with planted cold-cell-in-hot-ring shapes; gate: HL/LH recall ≥0.8 at FDR ≤ q on both variants |
| Implementation | `meridian-analytics/src/stats.rs` (sibling of `heatmap_stats`), nightly job |
| Priority | **LOW** |
| Acceptance / kill | Ship only if a consumer for the coverage-hole label exists (api.md block + operator request); kill if suite shows Gi* low-z cells already coincide with planted LISA outliers ≥80% (i.e. redundant in practice) |

Note the shipped Gi* caveat LISA would inherit: population = present (non-zero)
cells only, absent neighbors contribute 0 but are not population members
(`stats.rs:152–157`) — on the sparse operator-doc surface both statistics see
a distorted neighborhood.

**Verdict: LOW PRIORITY.** The user-facing statement Gi*+BH cannot make is
narrow (spatial outliers), the inference cost at BH-compatible resolution is
nightly-batch-only, and global Moran's I enables no defensible user statement
at all on media-coverage data.

### E2 — Space-time scan statistics (Kulldorff)

**Cost first, honestly.** Candidate cylinders ≈ n cells × radii × temporal
windows ≈ 12k × 4 (k-ring 0–3) × 7 (1–7 days) ≈ 336k zones; Poisson LRT each
is O(1) given prefix sums. 999 Monte-Carlo replications: ~3.4e8 LRT evals
(ln-dominated) **plus** regenerating the null surface per replicate (~12k×90
Poisson draws × 999 ≈ 1e9 draws), totaling **~30–60s on 4 A76 cores. Reject
for query-time** (3 orders over the 60ms row, `02-budgets.md:140`); feasible
nightly.

**Does it detect anything z+burst jointly miss?** Yes, one shape: a
*spatio-temporally compact moderate elevation* — spread over a ~7-cell disk
and 2–4 days where no single cell-day is extreme (per-root z is latest-day
only, `stats.rs:111–120`; burst is per-root whole-scope temporal,
`trends.rs:129–136`; Gi* is spatial-only and on the other data surface). But
the cheaper first rung exists and is nearly free:

**E2a (recommended instead): reuse the shipped Gi*+BH verbatim on a GDELT
day-slice.** `heatmap_stats` (`stats.rs:158`) is data-source-agnostic
`&[(u64, u32)]`; feeding it `store.scan(day, day, root, None)`
(`store.rs:117`) yields the *first* spatial view of the GDELT surface at the
measured ~24ms cost class (heatmap+Gi* 23.7ms p50, `02-budgets.md:123`).
Row: Proposal = `/v1/trends?spatial=day` additive block | Problem = no
GDELT spatial structure exists at all | Gain = new surface at ~zero method
risk (constants already suite-10-judged) | Pi-5 cost ≈ 25ms | Privacy = none
(GDELT) | Suite = suite-10 lattice as-is | Location = `meridian-api` trends
handler + `stats.rs` | Priority = **LOW-MEDIUM** | Accept: p50 ≤60ms;
kill: no operator consumption after one release (same sunset discipline as
ADR-25).

**Kulldorff verdict: REJECT for query-time; defer the nightly batch until
E2a proves the GDELT-spatial surface is consumed at all.** Building a
999-replicate scanner in front of zero demonstrated demand inverts the
repo's own consumer-first rule (the ADR-28 BOCPD rejection logic,
`00-adr.md:750–753`).

### E3 — Seasonal (day-of-week) baselines for GDELT day-series

**Verified gap.** The mover baseline is the unweighted mean of the window
minus the latest day (`stats.rs:61–71`); the burst baseline is head-60%
moments (`burst.rs:82–89`). **No weekly periodicity handling exists anywhere
in `trends.rs`/`stats.rs`/`burst.rs`**, and neither generator models it:
suite 10 plants Poisson/NB+ramp only (`crates/meridian-eval/src/bench/spike.rs:10–18`),
suite 17 stationary-baseline ramps (`changepoint.rs:196–298`). GDELT media
volume has a strong weekend dip; a Monday latest-day judged against a
weekend-containing baseline gets an inflated z. Two aggravators: (1) the
error is **correlated across all ~20 root codes simultaneously** (a common
day-of-week factor), so BH cannot absorb it — all p-values shift together;
(2) the pooled quasi-NB dispersion (`stats.rs:73–94`) partially eats weekly
variance as overdispersion, which deflates power on every day rather than
fixing the bias on the wrong days.

| Field | Content |
|---|---|
| Proposal | Multiplicative day-of-week pre-adjustment before the EB fit: f_d = mean(counts on weekday d)/overall mean, shrunk toward 1; y′_t = y_t/f_{dow(t)}; dow(t) = (day_epoch+4) mod 7. Skip when window <21 days (<3 obs/weekday). Same adjustment feeds the burst head-window moments |
| Meridian problem | Correlated DOW bias in the shipped latest-day z; power loss from DOW variance absorbed as dispersion |
| Math | Ratio-to-mean seasonal index with shrinkage f̂_d = (n_d·r_d + λ)/(n_d + λ); EB + quasi-NB unchanged on adjusted series |
| Expected gain | FPR reduction on seasonal series at matched TPR; some power recovery via smaller pooled 1/r̂ |
| Complexity | O(n) per report; ~30 lines in `stats.rs` |
| Pi-5 cost | Negligible (<1ms against the ≤60ms row; the whole suite-17 sweep ran <0.1s, `02-budgets.md:147`) |
| Privacy impact | None (GDELT-only) |
| Required data | Existing counters; ≥21-day windows |
| Evaluation | Suite-10 and suite-17 generators extended with a multiplicative weekly cycle (weekend factor sweep 0.6–1.4), tuning/hold-out variants per risk #21 (`03-risk-register.md:31`) |
| Implementation | `meridian-analytics/src/stats.rs` (+`burst.rs` head moments), constants frozen by the sweep |
| Priority | **MEDIUM-HIGH — shortlist candidate** (cheapest correctness fix on a shipped statistic) |
| Acceptance | On seasonal variants: FPR reduction ≥1.5× vs unadjusted at matched TPR; on the existing non-seasonal variants: no regression (TPR within 2pp, FPR ≤ unadjusted) — the no-collapse idiom |
| Kill | If the pooled quasi-NB dispersion already holds seasonal-variant FPR within 1.5× of the adjusted detector, record the no and close — the existing machinery would have proven adequate, which is a legitimate suite outcome |

### E4 — Hierarchical H3 consistency

Relevant only under a DP release; folded into I1 (consistency post-processing
paragraph there). No standalone row.

---

## Track F — audit first, design second

### F1 — Forget-correctness audit (traced end-to-end)

**The forget transaction** (`POST /v1/forget`,
`crates/meridian-api/src/lib.rs:734–802` → `forget_keys`,
`crates/meridian-query/src/ingest.rs:307–362`), one redb write txn covering:
tombstone insert (`ingest.rs:332–334`; re-ingest refused, `ingest.rs:167–175`),
dedup row + url_key→hash removal (`:335–341`), **sketch row removed
unconditionally in the same txn** (`:343–347`, ADR-19/risk-17), lexical
delete-by-term staged (`:348` → `lexical.rs:469–472`) and committed
(`:357–359`), vector remove (`:349–352` → `meridian-vector/src/lib.rs:113–121`,
idempotent) + full-file persist (`:360`). Cache purge default-true:
both query caches (`api/lib.rs:790–792` → `planner.rs:436–441`) and the
fetch/extract cache wholesale (`ladder.rs:66–67`). Audit line carries counts,
never the selector (`api/lib.rs:795–797`).

**Every derived aggregate, classified per ADR-19 (`00-adr.md:467–483`):**

| Structure | Class | Evidence |
|---|---|---|
| Lexical docs | (a) joins txn | `ingest.rs:348,357–359` |
| Vectors | (a) | `ingest.rs:349–352,360` |
| Dedup rows / reverse map | (a) | `ingest.rs:335–341` |
| Sketches (`sketch_v1`) | (a) | `ingest.rs:343–347` |
| Evidence/cluster annotations | (b)-equivalent: computed per query from the live SketchReader (`planner.rs:1349`, `ingest.rs:445`); nothing persisted; deleted sketches cannot contribute. Cached SERPs that embedded clusters die in the cache purge | certified |
| Geo heatmap counts | (b)-equivalent: **computed on read** from live fast fields (`lexical.rs:368–465`) — no materialized counts; forget propagates at the next query after commit | certified |
| Trends counters + edges | (c) GDELT-only: written solely by the slice puller (`gdelt.rs:128–184`); the ingest path never touches `AnalyticsStore` (no analytics reference in `ingest.rs`); `privacy.md:173–175` states it | certified |
| PageRank priors | (c): nightly wholesale replace from GDELT edges (`store.rs:222–233`, `graph.rs:17–37`) | certified |
| Decision log | no doc data: 13 fixed bytes (`decision_log.rs:144–157`), size pinned by test (`:374–378`) | certified (reward-bit influence → I3) |
| Bandit arm stats | no doc data: per-intent arm means (`bandit.rs:64`), rolling per SPEC §13.4 (`SPEC.md:608`) | certified (influence → I3) |
| Moka caches | purged in-call; anon cache additionally 5-min TTL (`privacy.md:167–168`) | certified |

**Verdict: CERTIFIED at the result-surface level** — no enumerated structure
can resurface a forgotten document in any response. **Three gaps surfaced
loudly, none of the risk-17 resurfacing class:**

- **F1-G1 (bytes-at-rest residual, MEDIUM).** Lexical deletion is delete-term
  + commit; the doc's bytes remain in immutable segments until LogMergePolicy
  merges (`lexical.rs:195–197`). The `SegmentStore` trait *anticipates*
  "`/v1/forget` compactions" (`meridian-index/src/lib.rs:43`) **but no code
  path forces a merge or segment GC on forget**. SPEC's literal promise is
  only "delete term + commit" (`SPEC.md:609–610`); `privacy.md:162` says
  "removed from the lexical index", which an operator may read as erasure at
  rest. Same question mark for the persisted usearch file after `remove`
  (slot-marking semantics — **to verify**, not asserted). Bet candidate:
  forced merge/GC hook post-forget OR one honest sentence in privacy.md
  stating the residual and the merge schedule. Cheap either way.
- **F1-G2 (doc-vs-code mismatch, LOW-MEDIUM).** `forget_domain` enumerates
  via `TopDocs::with_limit(10_000)` (`lexical.rs:479`); a domain with >10k
  docs is only partially forgotten in one call, while `privacy.md:159`
  promises "every currently indexed document of the domain". Fix: loop until
  the enumeration drains, or document "repeat until removed=0".
- **F1-G3 (deliberate, document-only).** Tombstones retain a 16-byte content
  hash forever (`privacy.md:137`) — an offline attacker with disk access and
  a candidate document can confirm "this content was ingested and forgotten"
  (membership inference on forgotten content). Accepted by design; optional
  hardening: keyed tombstones HMAC(node-secret, hash). LOW.

### F2 — Exact-vs-sketch arithmetic at Profile R/F scale

The deletability rule (ADR-19) already confines non-deletable sketches to
GDELT aggregates; the question is whether they buy anything even there. They
do not — the arithmetic:

- **Per-(day,cell,root) counters.** Cardinality ≈ 12k cells × 15 roots × 90
  days ≈ 16M keys worst case; measured 128MB/14 simulated days (P5 exit,
  `02-budgets.md:121`) against a 700MB gate (`store.rs:352–356`). A CMS
  replacement must keep error below the signal, and the signal is
  *single-digit per-cell-day counts* (the entire point of suite 10 / ADR-21).
  CMS error is ε·N over the stream mass: at ~150k geo rows/day, a ±2-count
  target needs ε≈1.3e-5 ⇒ width e/ε ≈ 2e5 × depth 5 × 4B ≈ 4MB **per
  deletable (day,root) unit** (TTL = drop-by-day, `store.rs:159–177`, forces
  per-day sketches) ⇒ 90×15 ≈ 1350 sketches ≈ **5.4GB vs ≤700MB exact**.
  Sketches lose by ~8× at granularity parity — before noting that +ε·N bias
  on single-digit counts would destroy the EB z outright.
- **Per-domain counters** (hypothetical): ~30k GDELT domains × ~50B redb row
  ≈ **1.5MB exact**. A CMS at ±10 counts over the 90-day stream (N≈13.5M)
  needs ε≈7.4e-7 ⇒ ~74MB. Exact wins 50×.
- **HLL distinct domains:** exact u64 set ≈ 240KB; HLL saves ~224KB. Pointless.
- **Heavy hitters:** the mover universe is 20 root codes (`store.rs:36–37`).
  Exact is free.

**Verdict: REJECT — sketches are premature at 100k–1M docs / single
operator; the only justified sketch in the system remains MinHash/SimHash
(ADR-18), which earns its place by answering a similarity question, not a
counting one.** Re-open only if key cardinality grows ~100× (multi-tenant or
per-URL analytics, neither planned).

### F3 — Quantile sketches for latency telemetry

Already effectively present: `metrics-exporter-prometheus` backs histogram
summaries with DDSketch (`Cargo.lock:3482–3510`, `sketches-ddsketch`), and
`meridian_request_ms` is recorded per route (`api/lib.rs:118`). Per-stage
timings exist per response (`planner.rs:474` `timings` map) but are not
exported as histograms. Adding `meridian_stage_ms{stage=…}` is a one-line
`metrics::histogram!` per stage inside the existing bounded-cardinality
labels (`privacy.md:12–13`) — an engineering chore, not a research row; KLL's
deterministic guarantees over DDSketch's relative-error are irrelevant at
single-node sample volumes. **Verdict: NO ROW; file as a chore if per-stage
p99s are wanted on dashboards.**

---

## Track G — rejection workpaper (argued, not asserted)

First, the standing fact: the repo **already ships a spectral method** —
power-iteration PageRank, 30 iterations, damping 0.85
(`graph.rs:13–31`). The question for each candidate is what *additional*
spectrum buys.

### G1 — Spectral clustering of domain co-occurrence

- **Assumptions:** a graph whose Laplacian has a usable eigengap; a consumer
  for global domain communities; cost headroom for eigensolves.
- **Repo reality:** the graph is capped at 200k edges with weight-1 edges
  dropped first (`store.rs:179–202`), built from ≤30-domain cliques per
  (slice, root) bucket (`gdelt.rs:19–20,171–183`) — a topology *manufactured*
  by the clique construction, so community structure partly reflects the
  bucketing, not the domains. Node count ~10–30k: dense O(n³) is impossible
  (~2.7e13 flops); Lanczos top-50 on 200k nnz is ~1e10 flops ≈ tens of
  seconds nightly — affordable, but: the eigengap is unknown and unmeasured;
  the user-facing independence question is already answered at the *document*
  level by ADR-18 shingle clusters (suite-9 F1 0.918 hold-out,
  `00-adr.md:459–465`); and the only ranking consumer of domain structure,
  `domain_prior`, is **deliberately weighted 0 in the live cold-start scorer**
  (`01-rebaseline.md:21`, R6). Building richer structure on a signal weighted
  zero is decoration.
- **Verdict: REJECT.** Re-entry condition: domain_prior earns nonzero LTR
  weight from real training data AND a measured eigengap on the live graph.

### G2 — RMT / Marchenko-Pastur denoising of trend covariance

- **Assumptions:** MP needs iid, stationary entries with p,n → ∞ at fixed
  p/n; "noise" eigenvalues inside the MP bulk are discardable.
- **Repo reality, actual dimensions:** the only day-series matrix the store
  can produce is roots×days ≈ **20×90** (`store.rs:32–38` keys on root, not
  domain — a domains×days matrix does not exist and would require new state).
  p=20 is nowhere near asymptopia; and the entries are overdispersed counts
  whose bursts and (E3) weekly cycles are **the signal** — non-stationarity
  is what ADR-21/ADR-28 exist to detect. Denoising toward the MP bulk would
  subtract exactly what the product sells. The violated-by-construction
  argument is decisive independent of cost.
- **Verdict: REJECT.** No re-entry condition at this store schema.

### G3 — Low-rank embedding compression

- **Assumptions:** embedding storage or ANN throughput is a binding constraint.
- **Repo reality:** potion static embeddings are already 256-d int8 (~256B/doc
  ⇒ 25.6MB @100k, 256MB @1M); ANN measured 0.45ms p50 @100k and 0.98
  recall@1M ef=128 (`01-rebaseline.md:66–70`); binary quantization is the
  *recorded* Phase-11 candidate with an explicit trigger >1.5M docs
  (`SPEC.md:840–841`). A PCA rung between int8 and BQ would spend recall to
  relieve a constraint that is not binding at any profiled scale.
- **Verdict: REJECT (premature); the BQ trigger row already encodes the
  correct future decision point.**

---

## Track I — privacy-preserving analytics

### I1 — DP trend/heatmap release design (contingent plan for the export boundary)

SPEC §16 defers DP "until an operator-facing publish/export feature exists —
today's aggregates are unreleased derivatives of public GDELT data"
(`SPEC.md:837–840`). Threat-model honesty first: for **GDELT-derived** series
the protected unit is a public event — DP there adds noise and protects
nothing. The aggregate with a real protected unit is the **operator-doc
heatmap** (publishing it leaks corpus membership: which documents/regions the
operator ingested) and any decision-log export (→ I2). The design below is
therefore doc-heatmap-first, GDELT-optional.

| Field | Content |
|---|---|
| Proposal | Continual-observation release: per-(cell) day-count streams released through the binary-tree mechanism (Chan–Shi–Song); event = one document (heatmap) or one GDELT row (trends, if ever wanted) |
| Math | T=90 leaves ⇒ L=⌈log₂T⌉≈7 levels; each event touches ≤L+1≈8 nodes; per-node Laplace(b=(L+1)/ε); leaf error std ≈ √2·8/ε ≈ 11.3/ε, prefix-sum error O(L^{1.5}/ε) |
| ε accounting | Event-level ε per 90-day window per released resolution; parallel composition across disjoint cells; sequential across resolutions unless the hierarchy is released once and made consistent by post-processing |
| H3 consistency (E4) | Release res-3/4/5 jointly, then constrained least-squares (Hay-et-al consistency) on the H3 tree — children sum to parents exactly; linear in cells; post-processing costs no ε |
| Suppression interplay | k-threshold suppression applied **post-noise** is pure utility hygiene (DP already protects); suppressed cells must carry an ADR-20-style honesty marker, mirroring the "likely low-sample noise" label discipline (`stats.rs:137`) — never silently dropped |
| Pi-5 cost | Negligible (noise + tree maintenance is O(cells·logT)) |
| Required data | The export feature itself — which does not exist |
| Evaluation — the falsifiable utility gate | Extend suite 10: inject tree-mechanism noise post-aggregation at calibrated ε. **Gate: BH-FDR mover/hot-spot detections survive ε ≤ 2** (event-level, doc neighbor, 90-day window): TPR within 5pp of noiseless at FPR ≤ q on BOTH generator variants. **Kill: TPR drop >15pp even at ε=4** ⇒ the finest publishable granularity is coarser; the suite then sweeps granularity (res-4/weekly/root-marginals) until the gate passes, and THAT granularity is what the export ships |
| Pre-registered expectation | At res-5/day, per-cell counts are single-digit (the suite-10 regime) while leaf noise at ε=2 has std ≈5.7 — detections likely die; the experiment exists to *discover* the coarsening, not to confirm feasibility |
| Implementation | New `meridian-analytics/src/dp.rs` + the export endpoint; constants frozen by the suite run |
| Priority | **Top-bet ONLY paired with the publish/export feature; standalone LOW** (a design without a boundary is a contingency file, and this is it) |

### I2 — Decision-log k-floor adequacy vs DP

The shipped floor: first k−1 occurrences of a context combo per day are
written with context blanked (0xFF), rows ≥5 carry context
(`decision_log.rs:33–35,133–153`, test `:339–355`); 13-byte rows, no
text/IP/fine timestamps (`:5–18`); 30-day TTL + 20MB cap + wipe
(`:180–244`); anon never logged (`:16–18`, `00-adr.md:589`).

**Attack analysis (risk #20, `03-risk-register.md:30`):**

- *Differencing across days:* rows are day-keyed; cross-day snapshots reveal
  only what the day key already states. Differencing against the floor leaks
  one bit ("combo C occurred ≥5 vs ≤4 times that day") — bounded and coarse.
- *Rare-bucket inference:* blanked below the floor; the residual is that a
  combo occurring exactly 5 times yields 1 readable row — the k-property
  ("≥5 events that day share this context") still holds, so no row pins a
  specific request. The in-memory combo counter resets on restart
  (`decision_log.rs:103,134–142`), which *over*-generalizes after a restart —
  conservative direction.
- *One real residual:* the redb key embeds a per-day **sequence number**
  (`:159`), so within-day row ORDER is finer than the 3h bucket — an
  observer who knows when they issued a query can partially de-bucket time.
  One-line fix candidates: randomize insertion order per sweep, or batch
  writes per bucket. LOW.
- *DP alternative:* randomized response on context buckets would corrupt the
  DR reward-model features, and noising propensities/rewards breaks the only
  consumer outright — OPE validity requires them exact (ADR-25 gate;
  `decision_log.rs:13` records that generalization "costs DR model features,
  never estimator validity"). At single-operator scale (currently **0 organic
  rows**, ADR-25 sunset 2026-08-11) the realistic adversary is device
  compromise, who also holds the corpus and caches; the 13-byte log is the
  least of their take.

**Verdict: k-floor suffices at this scale and consumer; DP noise is
unwarranted (and actively harmful to ADR-25). Revisit only if a log
export/publish path appears — at which point it joins I1's boundary.** File
the seq-ordering side channel as a LOW hygiene chore.

### I3 — Pan-privacy / forget-interaction of analytics internal state (joint with F1)

Does any internal state retain influence of forgotten docs even after rebuild?

| State | Finding |
|---|---|
| EB Gamma prior + pooled quasi-NB 1/r̂ | Refit per request from the current window scan (`trends.rs:121`, `stats.rs:98–107`) — nothing persisted; GDELT-only besides. CLEAN |
| Burst (s,γ) + head-window moments | Constants frozen from synthetic sweeps (`burst.rs:33–39`); moments per-call (`burst.rs:82–89`). CLEAN |
| PageRank priors | Wholesale nightly replacement (`store.rs:222–233`) from GDELT edges. CLEAN |
| Contextual TS posterior (dark) | Fit from decision-log rows; wipe path exists ⇒ rebuildable-on-wipe. CLEAN |
| Bandit means / decision-log reward bits | Retain the *influence* of forgotten docs (a forgotten local doc may have displaced web results in a past top-10, flipping a reward bit — reward def per `01-rebaseline.md:17` R2). Influence-not-content: 1-bit, non-invertible, no doc identity, 30d TTL / rolling. **No action**; recommend a one-line ADR-19 clarification distinguishing content/identity retention (prohibited) from statistical influence on coarse aggregates (accepted), so this classification is recorded policy rather than reviewer judgment |
| Tombstones | The one structure that *deliberately* retains a derivative of forgotten content forever (F1-G3 above). Accepted by design; optional HMAC hardening, LOW |

**Verdict: CLEAN except the two recorded residuals (reward-bit influence —
accept with ADR wording; tombstone membership inference — accepted by design,
optional hardening).**

### Standing rejections

**Local DP:** single-node, single-operator — there is no honest threat model
in which the node randomizes against itself. **Federated learning / MPC /
secure aggregation:** no multi-node deployment exists (SPEC's scale-out seam
is a trait, `meridian-index/src/lib.rs:25–44`, not a deployment); if the
region-sidecar or multi-node contingency ever materializes, secure
aggregation re-enters with that ADR. One sentence each, as commissioned.

---

## Shortlist deltas proposed to 00-review

1. **E3 seasonal DOW adjustment** — enters the top-5 candidates: a correlated
   error in a shipped statistic, ~30 lines, falsifiable by extending two
   existing suites, with a clean kill criterion.
2. **F1 gap remediation bundle** (forced merge/GC or privacy.md residual
   sentence; `forget_domain` >10k loop; usearch persisted-file verification)
   — hygiene bet, small, protects the repo's strongest claim
   (forget-correctness 100%, `SPEC.md:774`).
3. **I1 DP release** — top-bet ranking *only* as a pair with the
   publish/export feature; recorded here as the contingent design with a
   pre-registered utility gate (ε ≤ 2, suite-10 extension) and the honest
   expectation that res-5/day granularity will not survive it.
4. E2a (Gi* over a GDELT day-slice) — cheap optional row, LOW-MEDIUM.
5. G1/G2/G3, F2 sketches, F3 KLL, query-time Kulldorff, local-DP/FL/MPC —
   rejected with the arguments above; none needs carry tracking.
