# Meridian Research Review & Research-Driven Roadmap

Status: REVIEW — proposes, does not decide. As-of: v0.6.0 / `812d3d4` / 2026-06-13.

Method: three parallel source-inspection passes over the repo at the pinned
commit; five track workpapers (Tracks A–K, [03-track-workpapers.md](03-track-workpapers.md))
with file:line citations; four competitive ledgers against current official
documentation ([02-competitive.md](02-competitive.md)); a claim-by-claim
re-baselining of the commissioning brief ([01-rebaseline.md](01-rebaseline.md)).
This document distills those into the fourteen sections the brief required.
Nothing here amends the plan of record: promotion happens only via operator
sign-off and a new ADR in `docs/plan/00-adr.md`.

---

## 1. Executive assessment

**The brief's baseline was partially stale.** It described the repo as of
roughly Phase 6; the repo finished Phase 10 and shipped v0.3.0–v0.6.0 on
2026-06-12/13. Several "limitations to verify" are now shipped features
(statistical trends, geo significance, answer mode, evidence diversity), and
three of the brief's suggested research directions were already implemented,
gated, and **killed by the repo's own experiments** (conformal bands, MMR,
embedding-coverage fetch combiner). The corrections ledger in §2.1 and
[01-rebaseline.md](01-rebaseline.md) re-baselines everything; every
recommendation below builds on the corrected baseline.

**Strongest differentiators** (verified in source, compared in §3):

1. **An evidence-and-honesty surface no competitor exposes.** Derivation
   clusters with a measured false-merge gate (`independent_source_count` vs
   `apparent_source_count`), explicit `evidence: null` when independence cannot
   be asserted, stop reasons (`search_stopped_because`,
   `estimated_marginal_gain_remaining`), an explicitly *uncalibrated*
   confidence score with honest wording, JSD vantage divergence judged against
   a *measured* noise floor, and trend significance with FDR control and
   "likely low-sample noise" labels. The C1/C2 ledgers confirm none of Exa,
   Parallel, Tavily, Brave, OpenAI Deep Research, or Perplexity publicly
   exposes stopping rationale or statistically honest uncertainty.
2. **Privacy as physics, not policy.** Competitors offer contractual ZDR;
   Meridian is local-only with a fail-closed anon lane whose isolation is
   type-level and "by construction" (`bandit.rs:5-7` — the planner cannot
   reward anon outcomes), a provable `/v1/forget` in one redb transaction, and
   ≥21 hermetic egress invariants. §10 reports the one place this promise has
   gaps (segment residuals; the domain-forget cap) — fixing them is roadmap
   Horizon 0.
3. **Experiment-first governance as a moat.** 18 suites, constants frozen by
   sweeps, tuning-margin/hold-out-no-collapse gate idiom, and a public record
   of features its own gates killed. This discipline is what makes the §6 bets
   cheap to adjudicate: most judges already exist.
4. **A statistical geo/trends stack on edge hardware**: Gi\* hot-spots with
   BH-FDR, EB-shrunk quasi-NB trend z, two-state burst decode — at 23.7ms
   heatmap p50 on a Pi 5.

**Largest technical gaps:**

1. **Cold-start adaptive layer.** A four-class lexical intent heuristic
   (`intent.rs:10-19`), three hand-curated engine arms with ε-greedy and a
   binary appeared-in-top-10 reward (`bandit.rs:30-43,176`), fixed RRF(k=60)
   (`rrf.rs:5`), linear LTR with no training data. The dark-shipped contextual
   policy (ADR-25) is gated on OPE power the deployment may structurally never
   accrue: the 30-day log TTL caps gate-visible decisions at 30× the daily
   rate, so the 10k gate needs ≥334 organic decisions/day (T1).
2. **No filtered ANN**: geo/time-constrained queries silently lose the dense
   lane (ADR-10 amendment) — on exactly the geo-aware queries Meridian claims
   as identity.
3. **Answer surface is one ≤500-char extractive passage** vs competitors'
   multi-hop structured research with per-field citations (Parallel Basis, Exa
   research) — and it currently ships with no abstention threshold and no
   corroboration signal.
4. **No entity-list ("find all") or monitoring workflows** — confirmed
   product surface at Exa (Websets/Monitors) and Parallel (FindAll/Monitor).
5. **An idle 13-TOPS NPU.** The deployment Pi carries a Hailo-8L the repo
   does not mention; the official model zoo (v2.19.0, 2026-06-01) now ships a
   MiniLM-L6 embedding HEF for this part, making transformer offload newly
   plausible (§6 bet 1, Track K).

**Where Meridian can and cannot outperform web-scale engines.** It cannot win
on index breadth, freshness-at-scale, generative synthesis, or sub-second
hundreds-of-sources research — those require web-scale infrastructure and
frontier models (C2 ledger). It can win wherever the unit of value is *trust
per result* rather than *results per second*: source independence,
geographic/vantage difference, statistically defensible trends, transparent
stopping economics, provable deletion, and zero marginal query cost on owned
hardware. The product thesis in one line: **competitors answer faster;
Meridian shows its evidence** — and (§14) the next phase should make the
extractive answer the place where all of that evidence converges.

## 2. Verified current architecture

### 2.1 Corrections ledger (brief → repo)

Full table with evidence: [01-rebaseline.md](01-rebaseline.md). Summary:

| Brief claim | Verdict |
|---|---|
| 3-arm ε-greedy engine routing, binary top-10 reward | **STILL TRUE** (`bandit.rs:30-43,176`; linear-TS alternative dark behind the ADR-25 gate, 0 organic rows) |
| Four-class lexical intent | **STILL TRUE** (`intent.rs:10-19,69-87`) |
| Fixed-parameter RRF | **STILL TRUE** (`rrf.rs:5`, k=60 locked) |
| LTR is identity cold-start | **PARTIAL** — hand-tuned *linear* scorer behind `Scorer`; GBDT awaits training data that does not exist |
| Domain PageRank over co-occurrence, not hyperlinks | **STILL TRUE** (GDELT co-occurrence, ADR-10; live LTR weight 0) |
| Web results can't take geo/time filters; ANN skipped under filters | **STILL TRUE** (ADR-10; `api.md` refuses `scope=web`+geo) |
| Trends = latest/mean ratio | **SUPERSEDED** — EB + quasi-NB z + BH-FDR (ADR-21) + two-state burst (ADR-28) |
| Geo = counts without significance | **SUPERSEDED** — Gi\*/BH `z`,`q_value`,`significant` (ADR-21) |
| Diversity = domain caps only | **SUPERSEDED** — `diversity=evidence` canonical-aware clusters (suite 13c) |
| Eval = nDCG/MRR/latency | **SUPERSEDED** — 18 suites, alpha-nDCG, ECE, OPE, VoI replay |
| No answer mode / uncertainty / stopping | **SUPERSEDED** — ADR-29 / ADR-23 / ADR-26 all shipped |

Already killed by repo gates (binding on this review; full list
[01-rebaseline.md §3](01-rebaseline.md)): conformal bands (suite 16, 19pp
coverage collapse), MMR (suite 13b), embedding-coverage page combiner (suite
15b), Pandora-for-page-objective (suite 15), Gi\* k-ring for movers (suite
10), pure-Poisson z (suite 10), BOCPD, neural routers/MCTS/knapsack bandits
(ADR-25), usearch portable-C++ rung (P7 recall defect).

### 2.2 Query pipeline (as implemented)

`bins/meridiand` (axum) → `meridian-query::planner` orchestrates per request
(`planner.rs:1-4` module doc; `SearchRequest` at `planner.rs:39-75`):

1. **Lane resolution** — direct / anon (fail-closed, `PlanError::AnonBusy`,
   `503` semantics in `api.md`) / region (refuses until egress-IP verified).
2. **Intent classification** — `intent::classify` (`intent.rs:69-87`), keys
   the bandit.
3. **Local retrieval** — Tantivy BM25 top-1000 (0.48ms p50 @100k measured) ∥
   usearch ANN (numkong rung, int8 cosine; 0.45ms p50 @100k; 1M: recall@10
   0.98 @ ef=128, p99 1.73ms, 506MB resident). Geo/ts filters: H3 r7/r5
   TermSet prefilter, ANN dropped (ADR-10).
4. **Metasearch fan-out** — SearXNG sidecar(s), engine subset from the
   ε-greedy bandit (`bandit.rs:150-172`), propensity captured at choice time
   (`bandit.rs:112-140`), 300ms fixed hedge on direct (T2 workpaper), deadline
   per stage.
5. **Fusion** — `rrf_fuse` k=60 (0.144ms measured) → domain-diversity cap →
   linear LTR (`meridian-rank`).
6. **Evidence stage** — ADR-18 MinHash-containment clustering (k=4 shingles,
   128 perms, τ=0.3, `MIN_MATCH_BINS=5`), canonical marking, ≤2ms budget;
   `diversity=evidence` reordering (v0.6.0).
7. **Deep mode** — ort INT8 cross-encoder (gnu image; musl degrades with
   `rerank_unavailable`), top-20/batch-4; VoI fetch ladder (`additive_walk`)
   with `analysis` block; **answer mode** swaps in `pandora_walk` + passage CE
   (≤32 passages, cap 8) → `best_passage` (ADR-29).
8. **Response assembly** — `rank_signals` (every surfaced signal,
   `planner.rs:83-98`), per-stage `timings`, `confidence` (NQC+Clarity),
   `evidence`, optional `divergence`/`analysis`/`best_passage` blocks —
   additive per ADR-20.

### 2.3 Analytics, geo, privacy, eval

**Analytics** (`meridian-analytics`): GDELT 15-min slices (HTTP+MD5, ADR-15) →
day-series counters (90-day TTL), EB+quasi-NB z+BH movers + burst decode
(`trends.rs`, suite-17 constants s=2.0, γ=1.0); domain co-occurrence graph →
nightly petgraph PageRank `domain_prior` (live weight 0). **Geo**
(`meridian-geo`): h3o cells r7/r5, gazetteer fst, Gi\* k-ring-1 + BH heatmap.
**Privacy** (`meridian-privacy` + stores): no query logging; forget =
delete-by-term + vector delete + sketch row + tombstone + cache purge in one
transaction; ADR-19 deletability standing rule; ADR-24 decision log
(13-byte bucketed rows, k-floor <5/24h, 30-day TTL, anon never logged,
default OFF). **Eval** (`meridian-eval`): suites 1–18 (§11), standing exit
gates (suite-2 recall re-run every exit; forget-correctness 100%; privacy
smoke; invariant count monotonic ≥20).

### 2.4 Device baseline (new to this review)

The deployment Pi 5 (8GB, 16K-page kernel, SD-only storage) carries a
**Hailo-8L NPU** (13 TOPS INT8, M.2; HailoRT 4.23.0; PCIe Gen2 x1 negotiated,
~400MB/s; Gen3 is a config knob) that no repo artifact mentions. Captured
facts and CPU baselines any offload must beat: [01-rebaseline.md §4](01-rebaseline.md).
The M.2 slot hosting the NPU is *why* the device has no NVMe — the storage
constraints in ADR-D2 and risk #7 are partly the price of compute the stack
does not yet use.

## 3. Competitive capability matrix

Marks: **C** confirmed in official sources · **I** inferred · **–** absent /
no public evidence · **?** unknown. Backing quotes and URLs:
[02-competitive.md](02-competitive.md). Columns: Meridian (MER), Exa (EXA),
Parallel (PAR), Tavily (TAV), Brave Search API (BRV), SearXNG (SXG), OpenAI
Deep Research (ODR), Perplexity Sonar/DR (PPX).

| Capability | MER | EXA | PAR | TAV | BRV | SXG | ODR | PPX |
|---|---|---|---|---|---|---|---|---|
| Own web index | – | C | ? | I | C (30B+, independent) | – (aggregator) | I (Bing-derived+) | I |
| Local index / hybrid retrieval | C | – | – | – | – | – | – | – |
| NL search objectives | – | C (deep search) | C (objective→queries) | I | – | – | C | C |
| Query decomposition / rewriting | – | C | C | I | – | – | C | C |
| Multi-hop research | – (single-hop + VoI fetch) | C (research API) | C (Task API) | – | – | – | C | C (deep research) |
| Similar-page discovery | – | C (findSimilar) | – | – | – | – | – | – |
| Focused excerpts / extraction | C (extractive passages, dom_smoothie) | C (contents) | C (excerpts) | C (extract) | C (snippets/grounding) | C (engine snippets) | C | C |
| Structured outputs + citations | – (blocks, not schemas) | C (outputSchema) | C (per-field Basis) | – | – | – | – (prose+spans) | C (JSON schema) |
| Confidence / uncertainty reporting | **C** (NQC+Clarity, honest wording) | – | C (Low/Med/High "calibrated") | – | – | – | – | – |
| Stopping transparency | **C** (`search_stopped_because`, marginal gain) | – | – | – | – | – | – | – |
| Source independence / dup detection | **C** (derivation clusters, measured gates) | – | – | – | – | I (hash dedup only) | – | – |
| Freshness controls / live fetch | C (VoI fetch, ts filters local) | C (maxAgeHours) | C | C | C (freshness param) | C (time-range param) | C | C (recency filters) |
| Monitoring / change detection | – | C (Monitors) | C (Monitor) | – | – | – | – | – |
| Entity-list "find all" | – | C (Websets) | C (FindAll) | – | – | – | – | – |
| Source-quality estimation | C (domain_prior, weight 0) | ? | I | ? | C (Goggles, operator-defined) | – | ? | ? |
| Agent APIs / MCP | – (REST only) | C (MCP) | C | C (hosted MCP) | C | – | C (MCP tools) | C |
| Result explainability (signals exposed) | **C** (rank_signals, timings) | – | – | – | – | partial (`/stats`) | – | – |
| Geographic filtering | C (local: H3/radius) | I | ? | I | C (country/locale) | I (locale) | – | – |
| Geographic comparison / divergence | **C** (JSD vs measured floor) | – | – | – | – | – | – | – |
| Statistical trend analytics | **C** (EB z, BH-FDR, burst) | – | – | – | – | – | – | – |
| Anonymous routing | **C** (fail-closed Tor lane) | – | – | – | – | C (proxy/Tor config) | – | – |
| Provable deletion | **C** (/v1/forget, txn + tombstones) | – | – | – | – | – | – | – |
| Local/self-host deployment | **C** (the product) | – | – | – | – | C | – | – |
| Privacy: no query retention | C (physics: never stored) | I (ZDR enterprise) | I | C (SOC2+ZDR claim) | C (≤90d, no linkage) | C | C† (ZDR conflicts w/ background mode) | C (ZDR default claims) |
| Per-query marginal cost | **C** ($0 on owned hardware) | $7–15/1k | $5/1k+ | credits | $5/1k | $0 | $$/task | ~$1/task |

Two honest readings of this matrix: (a) the entire left-column cluster of
**C**s that no other column has — stopping transparency, independence
counts, divergence, statistical analytics, deletion, explainability — is one
coherent product surface (trust), not scattered features; (b) the rows where
Meridian shows "–" against four or more **C**s — NL objectives,
decomposition, multi-hop, structured outputs, monitoring, find-all, MCP — are
all *orchestration* layers above retrieval, several of which are feasible
without a generative model (§5 B2, §6, §14; MCP is an API-surface decision,
not research).

## 4. Unique Meridian advantages → defensible product positions

1. **Stopping economics as API contract.** No vendor states *why* search
   stopped or what marginal value remains (C1/C2 ledgers; vendors sell depth
   tiers instead). Productize: keep `search_stopped_because` +
   `estimated_marginal_gain_remaining` on every deep/answer response and add
   the §8 trust fields; document it as "the engine that tells you when more
   searching is a waste".
2. **Source-independence counts with a measured error gate.** Syndication
   collapse (suite-9 F1 1.0/0.918, false-merge 0) is invisible to every
   compared system (SearXNG dedups only by hash). Productize via §6 bet 3:
   make the answer cite *k independent clusters*, not one URL.
3. **Vantage divergence against a measured noise floor.** `compare=vantages`
   with the p90-0.30 floor is a capability with no public competitor analog;
   it generalizes to region lanes when (and only when) the Phase-11 sidecar
   budget exists.
4. **Privacy by construction.** Anon-lane isolation enforced by types and
   call-graph absence (`bandit.rs:5-7`), not policy. Brave's privacy page is
   best-in-class for a cloud API; it is still a policy. Position Meridian as
   the only AI-oriented search system whose privacy claims are *auditable in
   its own CI* (privacy smoke + invariant tests).
5. **Honest uncertainty.** The repo *withdrew* a confidence feature its own
   suite falsified (ADR-27) and documented why. As a market position: every
   confidence number Meridian ships survived an adversarial gate; competitors
   ship none (Parallel's Low/Med/High "calibrated" Basis is the only peer
   attempt, with no published calibration evidence — C1 ledger).
6. **Zero marginal cost + data sovereignty.** $5–15/1k-result cloud pricing
   vs owned-hardware $0 changes which analytical workloads are economically
   sane (continuous monitoring of a local corpus, high-volume geo probes).

## 5. Research opportunity matrix

Full rows (math, costs, kill criteria, file:line):
[03-track-workpapers.md](03-track-workpapers.md). Priorities: P0 = do
regardless; P1 = top-bet material; P2 = worthwhile if adjacent work lands;
P3 = recorded, not advised now. Verdict ∈ {ADVANCE, CONDITIONAL, REJECT}.

| # | Proposal | Meridian problem | Gain (expected) | Pi-5 cost | Privacy | Judge | Priority / verdict |
|---|---|---|---|---|---|---|---|
| A1 | DR-OPE power analysis + verdict protocol | ADR-25 verdict is calendar-bound with structurally capped n | Decision quality at ~2026-08-11; MDE₈₀=6.7pp @10k known *now* | analysis only | none | suite 14 + `GET /v1/decision-log/ope` | **P0 ADVANCE** |
| A2 | Graded (rank-weighted) reward in decision log | Binary reward wastes estimator power | ~2× sd reduction → MDE₈₀@3k ≈ 6pp | 1 reserved byte/row | ADR-24 re-sign-off | suite 14 extension | **P1 ADVANCE (time-critical: before accrual)** |
| A3 | Sliding-window bandit via log re-replay | Engine drift (risk #12) vs stationary stats | drift robustness, zero new constants | <10ms amortized | none | replay | P2 |
| B1 | Answer-mode embedding-redundancy pruning | Paraphrase copies have no reveal reward in single-best regime | hit-rate via fetch redirection; ~700–900ms/avoided CE batch tail | ~0 (embeddings exist) | none | standing `voi-embed` + suite 18b | **P1 ADVANCE (bet 2)** |
| B2 | Non-generative query decomposition / aspect coverage | No decomposition vs all C2 competitors | aspect coverage on multi-aspect queries | fan-out × engines (risk #12) | none | new harness; gate behind aspect-collapse counter | P2 CONDITIONAL |
| B3 | Adaptive hedging | Fixed 300ms hedge, no telemetry | tail latency | counters first | none | metrics | P3 (telemetry first) |
| C1 | Corroboration scoring for `best_passage` | Answer cites one page; independence unused at claim level | trust signal; uses in-RAM passage CE + sketches | ≪1ms vs ~500ms headroom | none | suite 18b planted truth | **P1 ADVANCE (bet 3b)** |
| C2 | HITS vs PageRank vs degree on co-occurrence graph | Is PageRank even right on this substrate? | popularity-bias audit; future GBDT feature | nightly batch | none | offline comparison | P3 (no live consumer) |
| C3 | Ingest-time outlink retention (`links_v1`) + citation chains | Links are *discarded* at extraction (`extract.rs:17-28`) — irreversible data loss daily | enables true citation graph later | +rows in dedup.redb; ADR-19 delete path designed | forget-audited | retention first, analytics later | P2 ADVANCE (retention part) |
| D1 | Multi-lane generalized JSD spec | Two-lane JSD only; region divergence is the identity claim | spec frozen for Phase-11 sidecars | blocked: sidecars NOT BUDGETED | operator probes only (no organic-query summaries) | suite 12 extension | P2 (spec), blocked (code) |
| D2 | Entropy-gain value model + entropy stopping in VoI frame | Is information gain better than reservation values? | likely small (headroom −0.0023 nDCG); closes the question | replay only | none | suite 15 harness | P2 (cheap, run once) |
| E1 | Moran's I / LISA diagnostic | Global clustering question Gi\* doesn't answer | marginal beyond Gi\*+BH | nightly minutes (perm inference) | aggregate only | suite 10 extension | P3 |
| E2 | Kulldorff space-time scan | joint space-time clusters | overlaps z+burst+Gi\* | ~30–60s MC, nightly only | aggregate | — | REJECT (query-time); E2a: Gi\* on GDELT day-slice, ~25ms | P2 (E2a) |
| E3 | Seasonal (day-of-week) baseline adjustment | DOW bias inflates EB baseline, correlated across roots — BH can't absorb | FPR reduction on seasonal nulls; ~30 lines | ~0 | none | suites 10/17 + seasonal generators | **P1 ADVANCE (bet 5)** |
| F1 | Forget-path remediation (3 gaps) | Segment residuals till merge; `forget_domain` 10k cap vs privacy.md; tombstone inference | closes the gap between promise and bytes | merge scheduling; loop or doc fix | **positive** | extended hermetic forget test | **P0 ADVANCE (H0 blocker)** |
| F2 | CMS/HLL/heavy-hitters adoption | — | none at this scale: CMS at needed accuracy ≈5.4GB vs ≤700MB exact | — | deletability risk | arithmetic in T4 | REJECT |
| F3 | KLL/t-digest for latency telemetry | — | exporter already DDSketch-backed | — | — | — | REJECT (no row) |
| G1 | Spectral clustering of domain graph | — | shingle clusters already crisp (F1 0.918 hold-out); no eigengap evidence | O(n³)/Lanczos | — | — | REJECT |
| G2 | RMT / Marchenko-Pastur denoising | — | matrix is 20 roots × ≤90 days; bursts violate iid/stationarity *by construction* | — | — | — | REJECT |
| G3 | Low-rank embedding compression | — | potion already 256-d int8; BQ recorded for >1.5M docs | — | — | — | REJECT (BQ trigger stands) |
| H1 | Bootstrap rank stability | Fusion-input resampling → head churn score | new signal; B=20 ≈ 2.9ms | 2.9–7.2ms | none | suite 13/16 harness, beat NQC ρ 0.234 | P2 (feeds H2) |
| H2 | QPP ensemble (NQC+Clarity+lane-agreement+score-gap) | Single predictors are weak (ρ≈0.23) | ρ ≥0.30 target; style-shift no-collapse | ≤1ms | none | suites 13+16 | P2 ADVANCE |
| H3 | Answer abstention threshold (selective prediction, no coverage claim) | `best_passage` ships at any ce_score; risk #26 | precision/coverage curve from data on disk | one comparison | none | suite 18 corpus | **P1 ADVANCE (bet 3a)** |
| I1 | DP release for published aggregates (tree aggregation, ε≤2 gate) | No publish boundary exists yet (SPEC §16) | contingent design; doc-heatmap-first (GDELT is public data) | noise calibration | **positive when boundary exists** | suite-10 noise injection | P2 (paired with export only) |
| I2 | Decision-log k-floor vs DP | — | k-floor suffices at single-operator scale; DP noise would break OPE validity | — | — | attack analysis in T4 | REJECT (keep k-floor) |
| I3 | Pan-privacy audit of analytics state | EB priors etc. retaining forgotten-doc influence | clean — verified | — | — | — | done (T4) |
| J1 | Knob-level Pareto synthesis | Measured curves sit in separate bench files | one frontier doc; flags dominated defaults (ef=64@1M) | doc work | none | existing bench JSON | **P1 ADVANCE** |
| J2 | Class-aware admission control (heavy-cap anchored to rayon×4) | fast queries queue behind deep/answer | tail latency under mix | gauges first | none | new suite 19 | P2 |
| J3 | MDE pre-registration for every gate | Gates don't state detectable effect sizes | suite honesty; feeds A1 | doc work | none | — | P1 ADVANCE |
| K1 | Hailo CE rerank offload (`HailoReranker` behind `Reranker` trait) | 204ms CPU rerank stage; CPU contention | 2–7× stage speedup (3–6× plausible); frees A76 cores | +50–100MB host RAM; ~2W NPU | none (local; lane-blindness invariant) | suites 4/13 parity + 18 | **P1 ADVANCE (bet 1)** |
| K2 | Transformer embedder upgrade (NPU-hosted MiniLM) | potion static embeddings cap dense recall | recall step-change *if* measured | 384-d → index rebuild, busts 600MB row | none | suites 2/13 | P2 CONDITIONAL (schema cost) |
| K3 | Answer-passage scorer on NPU | CE ≈ ~700ms of answer p50 | answer p50 2502→~1900–2100ms or cap-16 restored ≤3.0s | rides K1 | as K1 | suite 18 + cap study | P1 (with K1) |
| K4 | NPU brute-force filtered dense scan | "speed up ANN directly" | **negative**: 25.6MB transfer ≈64ms vs whole CPU NEON scan 2–4ms; no matmul-as-a-service in HailoRT | — | — | arithmetic in T5 | REJECT |
| K5 | Power/joules-per-result measurement | Track J metric lever | data for the §11 energy metric | PMIC reads | none | suite 6 extension | P3 (inside K1 exit) |
| V1 | Filtered ANN (exact-scan tier + predicate HNSW beyond) | Geo/time queries lose the dense lane (R9) | hybrid restored on identity queries | exact scan ≤50k cands ≈2–4ms CPU | none | suites 2/12 filtered extension | **P1 ADVANCE (bet 4)** |

## 6. Top five research bets

Selection rule: (repo-evidenced gap) × (falsifiability on an existing or
cheaply-extended judge) × (competitive differentiation); ≥1 bet killable in
<2 Pi-days; ≤1 design-only (none made the cut — the DP design I1 stays a
contingent appendix). The NPU bet leads per the operator's directive; its
step-0 probe is deliberately zero-risk.

### Bet 1 — Hailo-8L cross-encoder offload (K1+K3)

- **Hypothesis.** The INT8 MiniLM-L6 cross-encoder runs on the Hailo-8L at
  ≥2× the CPU stage's effective throughput at score parity, cutting answer
  p50 from 2502ms toward ~2.0s (or buying rerank depth 20→40+ / passage cap
  8→16 at flat latency) while freeing all four A76 cores during the rerank
  phase.
- **Why it may beat simpler alternatives.** The CPU path is already
  optimized (ort, NEON dotprod, INT8); the remaining lever is silicon that is
  *already installed and idle*. The decisive new fact: Hailo model zoo
  v2.19.0 (2026-06-01) ships an official `all_minilm_l6_v2` HEF for HAILO8L —
  the same backbone — proving attention/LayerNorm/GELU compile for this part
  (C4 ledger; this overturned Hailo's 2024 "too large for 8L" position).
- **Baseline.** Shipped ort CPU path: rerank stage 204ms p50 (top-20/batch-4,
  P3 exit); answer p50 2502ms @ cap 8 (`2026-06-13-pi5-answer-cap-study.md`).
- **Falsifying experiment.** (0) `hailortcli run` the official MiniLM HEF
  on-device — measures the real PCIe Gen2-x1 penalty vs Hailo's Gen3-x4
  published numbers (121/492 FPS b1/b8); hours, no code. (1) DFC-compile the
  CE export (x86_64 host required — unverified dependency, the bet's hardest
  blocker; embedding lookup stays host-side, fixed-length window). (2)
  Suite-4 score-parity gate (Hailo-INT8 vs ort-INT8: rank correlation on the
  100-query eval within noise) + suite-13 ρ no-regression + suite-18 replay +
  suite-6 thermal re-run.
- **Pi cost.** +50–100MB host RAM (HailoRT), ~2W NPU under load; T5
  arithmetic: ~6.04 GOPs/pair at seq-256 → 23–47ms batch-20 at 20–40%
  utilization; multi-context weight streaming is the swing factor (could
  compress the win to ~2×).
- **Integration.** `HailoReranker` behind the existing `Reranker` trait
  (`meridian-rerank/src/lib.rs`); ships in the gnu image variant as a
  `hailo` feature (the ADR-02 ort precedent); **fail-open to CPU** with a
  `rerank_npu_unavailable`-style degraded marker; candidate-ADR
  "heterogeneous compute policy" (T5 §5) incl. the lane-blindness invariant
  (NPU usage must not distinguish anon from direct timing) and the
  `/dev/hailo0` world-writable permission fix.
- **Accept / kill.** Accept: ≥2× stage speedup at parity on all three
  judges. Kill: DFC cannot compile the CE; or parity fails (score drift
  beyond suite-4 gate); or measured end-to-end win <1.5× (multi-context
  streaming, PCIe). A kill still leaves K2's query-side embedder HEF as the
  salvage path.

### Bet 2 — Answer-mode embedding-redundancy pruning (B1)

- **Hypothesis.** In the single-best (answer) regime — where a paraphrase
  copy has *no* reveal reward — discounting fetch candidates by embedding
  similarity to already-fetched text redirects fetches to novel sources and
  raises answer hit-rate at equal or lower fetch count.
- **Why now.** The repo's own ADR-26 suite-15b addendum nominates exactly
  this use and pre-assigned its judge; the signal is measured real (0.716 vs
  0.052 cosine separation) and the page-objective failure mode (paraphrase
  reveals still buy rank mass) does not exist here.
- **Baseline.** Shipped `pandora_walk` with MinHash-novelty value model
  (suite-18: 0.700 hit-rate).
- **Experiment.** Suite **18b**: 15b paraphrase corpus × suite-18 hit-rate
  semantics; gain′ = min(sketch-novelty, embed-novelty); τ swept in
  [0.3, 0.6]. Accept: hit-rate non-degrading AND ≥15% fetch reduction (or
  measurable tail-latency cut ~700–900ms per avoided CE batch). Kill: the
  15b failure shape reappears (savings only at >1pp hit-rate cost).
- **Cost.** ~0 RAM (embeddings already computed); replay-only experiment.
  Integration: `meridian-fetch::voi` value model + `answer_candidate` path.

### Bet 3 — Answer trust layer: abstention + corroboration (H3+C1)

- **Hypothesis.** (a) A ce_score abstention threshold τ chosen by selective
  risk on the suite-18 corpus yields a precision/coverage curve that
  dominates always-show (target: ≥+5pp hit-rate precision at ≤20%
  abstention); (b) a corroboration block — does the winning passage's claim
  appear in passages from *distinct* ADR-18 clusters among fetched/ingested
  docs — separates planted-true from planted-false answers.
- **Why.** Risk #26 (confident wrong passage) is registered with wording as
  its only mitigation; the suite-18 n=1000 data needed to calibrate
  abstention is *already on disk*. C1 reuses per-doc passage CE scores and
  in-RAM sketches (`planner.rs:1231,1180-82`) — ≪1ms against ~500ms answer
  headroom. This is selective prediction *without* conformal coverage claims
  (those died in suite 16; the honesty wording carries).
- **Baseline.** Always-show `best_passage`; single-URL citation.
- **Experiment.** <2 Pi-days: τ-sweep on existing replay data; suite-18b
  planted true/false corroboration variant. Accept: curve dominates; planted
  separation AUC materially >0.5 with same-cluster copies never counted.
  Kill: abstention curve is flat (score uninformative — would also be
  evidence against K3's value); corroboration fires on syndicated copies.
- **API.** Additive `best_passage.corroboration: {independent_clusters,
  supporting_urls}` + `abstained: true` marker (§8).

### Bet 4 — Filtered ANN: restore the dense lane under geo/time (V1)

- **Hypothesis.** A two-tier filtered dense path — (i) exact int8 scan when
  the filtered candidate set ≤N (T5 arithmetic: ≤50k candidates ≈2–4ms on
  A76 NEON, *within* the fast-path budget), (ii) predicate-aware HNSW
  (usearch filter callbacks / ACORN-style) beyond — restores hybrid quality
  on geo/time queries at ≤+10ms p50.
- **Why.** R9 is the worst remaining mismatch between identity ("geo-aware
  analytical search") and implementation (geo queries silently degrade to
  BM25-only, ADR-10). The K4 rejection sharpens this: the cheap exact-scan
  tier is a *CPU* solution; no NPU needed.
- **Baseline.** Lexical-only under filters (shipped).
- **Experiment.** First *measure the harm*: extend suites 2/12 with
  geo-filtered queries at 100k/1M and record the nDCG delta of the missing
  dense lane. If <2pp, deprioritize (the recorded tradeoff was right). Else
  implement tier (i) (trivial: H3 TermSet → doc ids → exact scan) and gate
  tier (ii) on usearch API support without a fork (ADR-07 ladder discipline).
- **Accept / kill.** Accept: ≥95% of unfiltered dense contribution recovered
  at ≤+10ms p50 @100k. Kill: harm <2pp; or usearch predicate path requires a
  fork; or RAM exceeds the vector budget row.

### Bet 5 — Seasonal baselines for trends (E3)

- **Hypothesis.** GDELT day-series carry day-of-week structure that inflates
  the EB baseline and is *correlated across root codes* — so BH-FDR cannot
  absorb it — and a ~30-line DOW adjustment (per-root weekday factors from
  the window, quasi-NB z on adjusted counts) reduces null FPR at matched TPR.
- **Why.** T4 verified the absence in `trends.rs`/`stats.rs`/burst AND in
  both suite generators — the one place the otherwise rigorous trends stack
  has an untested systematic bias. Cheapest bet on the list; killable in
  hours.
- **Baseline.** Shipped EB+quasi-NB z (suite-10 constants).
- **Experiment.** Extend suite-10/17 generators with planted weekly
  seasonality (tuning) + a different seasonal shape (hold-out); run the
  shipped detector vs the adjusted detector. Accept: FPR reduction on
  seasonal nulls at matched TPR, margin on tuning, no collapse on hold-out,
  spike/ramp parity preserved. Kill: shipped z already absorbs DOW within
  noise (then record the no and close).

**Honorable mentions (not bets, but time-sensitive):** A2's reserved
reward byte should be decided *before* organic accrual starts (T1: it
halves the MDE; it is worthless retroactively); F1's forget remediations are
Horizon-0 correctness work, above research; J1's Pareto synthesis is a
documentation win sitting on measured data.

---

## 7. Routing as a constrained, privacy-aware policy problem

This section gives the **formal object** the ADR-25 verdict (~2026-08-11) is a
decision about — not a fresh design. The repo already implements the policy, the
estimator, and the gate; the contribution here is to write the formulation down
precisely, derive the gate's statistical power, and convert "wait for the
sunset" into a pre-registered protocol. Full derivations live in
`03-track-workpapers.md` §T1; the binding source anchors are restated inline.

### 7.1 The decision process

- **Context** `x`: the 26-dim one-hot of (intent class × coarse bucket), the
  exact featurization the dark contextual policy already builds
  (`contextual.rs:33-37`). Intent is the 4-class lexical heuristic
  (`intent.rs:69-87`); there is **no query text, no URL, no fine timestamp** in
  `x` — the ADR-24 privacy envelope is the state-space boundary, not a bolt-on.
- **Action** `a ∈ {fast, broad, reference}`: K=3 curated engine subsets
  (`bandit.rs:30-43`). The action set is **hard-bounded by the shed ladder**
  (`shed.rs`): cost is a constraint, not a reward term — which is why no
  latency-penalized reward is proposed (ADR-25's own argument,
  `00-adr.md:622-625`).
- **Policy** `π_ε`: ε-greedy, ε=0.1 (`main.rs:152`), with exact logged
  propensities `(1−ε)+ε/K ≈ 0.9333` greedy / `ε/K ≈ 0.0333` explore
  (`bandit.rs:133-138`). Logging today's propensities makes the incumbent its
  own logging policy — the precondition for off-policy evaluation.
- **Reward** `r ∈ {0,1}`: "the chosen arm contributed a result to the final
  top-10" (`bandit.rs:1-2,176`). Bernoulli, computed from rank attribution the
  planner already holds.
- **Constraints** (hard, non-negotiable): (i) anon-lane outcomes **never** update
  policy state — the planner does not call `reward()` on the anon branch
  (`bandit.rs:5-7`), so isolation is a property of the call graph, not a runtime
  check that could regress; (ii) the decision log is 13 bytes/row, k-floored at
  5, 30-day TTL, anon never logged (`decision_log.rs:8,29,31-34,144-158`); (iii)
  `/v1/forget` is untouched because the log holds no document references.

The candidate policy is linear Thompson sampling (DIM=26, λ=1, v²=0.25, 64-draw
MC propensities floored at 1/64), in-memory, **shipped DARK** —
`searx.contextual_policy` defaults `false` (`config.rs:343`) and the module is
"constructed only when [it] is on" (`contextual.rs:2`); when on it is rebuilt at
boot by replaying the decision log (`contextual.rs:24-28,132-143`), so the 30-day
TTL is already a sliding window on the posterior, a fact §7.4 exploits.

### 7.2 Off-policy evaluation and the gate

The verdict statistic is the doubly-robust uplift Δ = DR(candidate) − incumbent
realized mean, with a paired percentile bootstrap CI (`ope.rs:117-167`); the gate
passes iff the 95% CI excludes zero on ≥10k logged decisions, sunsetting at 60
days (`00-adr.md:611-641`); the verdict surface is `GET /v1/decision-log/ope`
with {pass, inconclusive, negative, insufficient_data}. DR uses a per-(bucket,
arm) empirical-mean model with prior 0.5 (`ope.rs:52-71,96`); the suite-14 anchor
measured DR relative bias 0.74% and **DR replicate sd 0.024 at n=10⁴**
(`bench/2026-06-12-pi5-p9-ope.json:21-30`).

### 7.3 Power — what the gate can and cannot certify

From the measured anchor, sd(n) = 0.024·√(10⁴/n), the gate's minimum detectable
uplift at 80% power is **MDE₈₀ = (1.96+0.84)·0.024 = 6.7pp at n=10k**, rising to
12.3pp at n=3k and 21pp at n=1k. A variance decomposition (T1 §A1.1) refines this
by the candidate's disagreement rate `d` with the logger's greedy arm: at the
realistic d≈0.25, MDE₈₀ is 3.6pp @10k / 6.6pp @3k; the dominating term is the
explore-arm IPS correction (weight 1/(ε/K)=30), which is exactly why DR's model
term matters (IPS sd 0.0384 vs DR 0.024, json:24,27). Because the realized uplift
is Δ = d·g (g = per-row reward gap on re-routed traffic), passing at organic
volume requires a **≥14pp per-context arm gap** — the gate can only ever certify
a *large, obvious* win. For an appliance that is the correct bar, and the verdict
note should say so.

**A structural finding (T1 §A1.2), load-bearing for the roadmap:** the OPE input
is `read_all()` over *retained* rows, and the TTL drops rows older than 30 days
(`decision_log.rs:180-191,247-276`). So `n_max = 30 × decisions/day`: reaching
10k needs **≥334 organic direct-lane decisions/day sustained over the trailing 30
days**. A single-operator appliance at 20–100 searches/day yields n∈[600, 3000],
MDE₈₀ between 6.6pp and 27pp. Risk #23 (inconclusive-forever,
`03-risk-register.md:33`) is therefore not merely likely — under the TTL it is
**structural**: extending the calendar deadline is mathematically useless unless
the *rate* rises. The modal 2026-08-11 outcome is `insufficient_data → sunset`.

### 7.4 The recommended protocol (process, not code)

1. **Pre-register four verdict branches before 2026-08-11** (T1 §A1.3): PASS →
   flip `searx.contextual_policy=true` (`config.rs:333,343`), amend ADR-25, re-run
   OPE with the TS MC propensities as the new logging policy (ADR-25 already
   requires this re-validation), keep ε-greedy behind the flag one release as
   rollback. INCONCLUSIVE / NEGATIVE / INSUFFICIENT_DATA execute ADR-25 verbatim:
   ε-greedy retained, log TTLs out, module stays dark (<1MB, `02-budgets.md:144`).
2. **Extend-once rule** (the only principled one given the TTL cap): extend 60
   days iff the trailing-14-day direct-lane rate ≥334/day; otherwise sunset, no
   second extension. **Do not** synthesize traffic to feed the gate — replayed
   rewards measure the generator, not the operator (the risk-#21 pathology), and
   "organic" is load-bearing in the ADR-25 record.
3. **Graded reward, before accrual starts (A2 — the one time-critical change).**
   Replace the binary reward's *logged value* with a rank-weighted graded reward
   `r = Σ_{i∈arm hits} w_i / Σ_{i=1..10} w_i`, `w_i = 1/log₂(i+1)`, u8-quantized
   into the **reserved** `row[11]` (`decision_log.rs:158`) — zero row growth, zero
   migration, DR unchanged on r∈[0,1] (`ope.rs:25`). Expected ~2× sd reduction ⇒
   MDE₈₀ @3k from 6.6–12.3pp to ~3.3–6.2pp. It does **not** rescue the 10k bar
   alone, but it is the single highest-leverage move against risk #23 and it pays
   only if it lands before rows are written (they are immutable). The cost is one
   ADR-24 re-sign-off: the row's field list is operator-approved verbatim, and
   populating a reserved byte changes that list even though the value is derived
   purely from ranks (no query content, privacy delta ≈ nil). Keep live ε-greedy
   on the binary reward; **log both**; switch the gate estimand only if
   var(graded)/var(binary) < 0.5 on the first 500 organic rows.
4. **Drift guard (A3).** Lifetime arm counts make "auto-demote a failed engine"
   slow (months at organic rates). Pick the **zero-constant** option: re-replay
   the decision log every 256 inserts (the existing sweep cadence) so the
   posterior window *is* the already-signed-off 30-day TTL (~20M flops, <10ms
   amortized on one core); plus a two-line exponential-forgetting cap on the live
   ε-greedy (halve pulls and reward_sum at pulls≥2000). Both are Pi-free,
   constant-free, and leave the propensity contract exact. Rejected alternatives:
   discounted TS (new frozen γ needs its own suite), changepoint-restart reusing
   ADR-28 Viterbi (machinery in search of a consumer — the BOCPD rejection
   ground).
5. **Optional, only if the operator wants the gate winnable at organic rates
   (A2-ext):** retain per-(bucket, arm, day) sufficient statistics
   (n, Σr, Σr/p, Σ(r/p)²) beyond the row TTL; IPS/DR are linear in rows so
   aggregates reconstruct the estimates exactly, and a delta-method normal CI
   replaces the bootstrap. This decouples the gate horizon from the TTL. It is a
   new retention surface ⇒ explicit ADR-24 sign-off; the minimal version is a
   single cumulative u64 counter so "≥10k" is at least *measurable* across TTL
   windows.

**What stays rejected** (T1 rejections, ADR-25 record unchanged and now on
*worse* data — 0 organic rows): neural routers, MCTS, bandits-with-knapsacks
(no training data, NP-hard, unexplainable; shed ladder already gates the action
set); M/G/1 optimization (every assumption fails on single-operator bursty,
closed-loop, bimodal-service traffic); RL scheduling (per-user state forbidden,
no reward in telemetry). The Hailo-8L changes nothing here: routing is
data-bound, not compute-bound, and the DFC is x86-only regardless (T1 §A1
rejection note).

### 7.5 Operations adjuncts (Track J, routing-adjacent)

- **J2 class-aware admission** (P2, gauges-first): two atomic counters
  (c_fast, c_heavy); admit heavy iff c_heavy<4 ∧ c_total<8, else serve the
  fast-path with `degraded:["admission_shed"]`. H=4 is *anchored* to the
  4-thread rayon pool, not tuned. Gate: new suite-19 `admission`, margin (mixed
  fast-p99 ≤2× solo) + no-collapse (held-out mix ≤4×). Precondition: add
  class-split latency gauges first; if measured mixes never inflate fast p99,
  **do not ship**.
- **J1 Pareto synthesis** (P1) and **J3 MDE pre-registration** (P1) are near-free
  process bets feeding every future suite — detailed in §11.

---

## 8. Analytical search: evidence structure and information-gain design

Meridian's product thesis (§1) is *"competitors answer faster; Meridian shows its
evidence."* This section specifies the two systems that make that thesis
load-bearing on the highest-stakes surface (answer mode): a **claim-level
corroboration layer** and a **calibrated abstention layer**, plus the
information-theoretic value model that governs *which* documents get fetched.
Everything here composes already-shipped, gate-validated machinery; nothing
re-opens a killed method. Detail in `03-track-workpapers.md` §T2 (B/D) and §T3
(C/H).

### 8.1 The value model is settled; only fetch savings remain

The fetch selector is already at its objective's optimum, measured from both
sides, and this bounds every "smarter retrieval" proposal:

- **Page objective** (additive nDCG): `additive_walk` opens while
  `p·gain − cost > 0` (`voi.rs:104-143`); greedy is near-optimal because the
  novelty-discounted gains are diminishing by construction. Suite-15 hold-out:
  nDCG@10 **0.7411 vs 0.7434 fetch-all at −30.5% fetches**
  (`bench/2026-06-12-pi5-p9-voi.json:22-31`). The residual quality headroom for
  *any* better page-value model is −0.0023 nDCG — essentially zero. Only fetch
  savings are winnable, and they are capped by `deep_fetch_max=2` (`config.rs:303`).
- **Single-best / answer objective**: Pandora's-box (Weitzman) `pandora_walk` is
  *exactly* optimal for "best passage in hand"; suite-18 hold-out (n=1000) gives
  answer hit-rate **0.700 vs 0.593** no-fetch (+10.7pp), **3.8×** the additive
  page selector (`bench/2026-06-12-pi5-p10-answer.md:15-28`).

The consequence: the analytical value-add is **not a better ranker** — that
ground is mined out. It is *trustworthy evidence structure around the answer*,
where competitors have no offering at all (§3).

### 8.2 Evidence-graph corroboration for answer mode (C1, **HIGH**)

**The gap.** `best_passage` cites exactly one page (`api.md:307-331`), and api.md
itself warns "a confidently relevant passage can still be wrong, and the engine
cannot tell" (risk #26). The existing `independent_source_count`
(`evidence.rs:37`) is *page-set-level* — how many apparent origins are in the
result list — and says nothing about whether a second origin supports *the claim
in the winning passage*. Claim-level vs page-set-level is the entire gap.

**The mechanism.** Let `w` be the winning passage and `C(w)` the ADR-18 evidence
cluster of its source page. Score, cheapest test first:

```
(i)  textual support:   c(w, d) ≥ τ        (MinHash containment, MIN_MATCH_BINS=5, 00-adr.md:459-465)
(ii) relevance support: |s_d − ce(w)| ≤ δ  (top-passage CE already in doc_ce, planner.rs:1231)
independent_clusters = | { cluster(d) : (i)∨(ii), cluster(d) ≠ C(w) } |   ← same-cluster copies NEVER count
```

Same-cluster copies are excluded *by construction* — syndication is exactly what
ADR-18 exists to discount, and counting a copy as corroboration would re-import
the suite-9 baseline failure (F1 0.054, `evidence.rs:6-8`). Inputs are all in
RAM: `fetched_sketches` (`planner.rs:1180-1182`, currently used only for VoI
novelty and never surfaced), the per-doc CE scores (`planner.rs:1231`), and the
ingested result-set sketches (`planner.rs:1349-1356`). The pure cluster function
`evidence.rs:71` already takes any `HashMap<u64, Sketch>`. Cost: **≪1ms** for the
textual+relevance variant; an optional paraphrase-support variant (CE-score `w`
against other clusters' top passages) adds ~10-20ms — <5% of the ~500ms answer
headroom (3.0s row, measured 2502ms @ cap 8, `02-budgets.md:148`).

**API shape (additive, ADR-20 pattern, own `schema`):**

```json
"best_passage": {
  "schema": 2, "url": "https://a.example/x", "text": "…", "ce_score": 4.1,
  "corroboration": {
    "schema": 1,
    "independent_clusters": 2,
    "supporting_urls": ["https://b.example/y", "https://c.example/z"],
    "basis": { "candidates_checked": 7, "method": "containment+ce" }
  }
}
```

`basis` carries the honesty payload (how many candidates were checkable — the
`sketched_results` idiom, `evidence.rs:39-41`), so the badge degrades gracefully
when coverage is thin rather than silently claiming "1 source."

**Judge.** Suite-18 harness extension: plant, beside each decisive original,
(a) genuinely independent second originals (true corroboration), (b) syndicated
copies (the trap — must not count), (c) uncorroborated singletons. **Accept:**
corroboration-label precision ≥0.9 / recall ≥0.6 on the hold-out variant, AND
hit-rate(corroborated) − hit-rate(uncorroborated) > 0 with planted-truth margin,
AND answer p50 row holds (n≥16 interpolated median). **Kill:** precision <0.9 on
either variant (a false "2 independent sources agree" badge is worse than none —
the conformal lesson at claim level), or any same-cluster leakage.

### 8.3 Calibrated abstention for answer mode (H3, **HIGH**)

**The gap.** The engine today shows the best passage it found *no matter how bad*;
`answer_unavailable` fires only on mechanical failure (`planner.rs:1267-1269`).
Suite-18 measured hit-rate 0.700 — **30% of shipped passages are misses**, some
fraction sitting at low `ce_score` and cheaply refusable. Risk #26 is mitigated
today by wording alone.

**The mechanism.** From suite-18 replay, per-query pairs `(ce_score, hit∈{0,1})`.
Publish the full coverage-vs-selective-hit-rate curve — *the tradeoff is the
deliverable* — and choose `τ* = max{τ : h(τ) ≥ h_target}` on tuning, verify on
hold-out **and** the style-shift variant. When `ce(w) < τ`, withhold
`best_passage` and emit `degraded:["answer_below_threshold"]` (distinct from
`answer_unavailable`). Serving cost ~0ms (scores already computed).

**The honesty framing, written into api.md:** this is selective prediction
*without* distribution-free coverage guarantees — those are **killed** (ADR-27
REFUTED; 19pp absolute-coverage collapse under style shift,
`2026-06-12-pi5-p10-conformal.md:37-43`). The wording is "tuned on the repo eval
set; the threshold filters low-relevance passages, it does not certify shown
ones." The asymmetry vs bands is the whole reason this is allowed where bands
were not: a band's failure mode is a *false certificate on shown results*;
abstention's failure under shift is *mis-set coverage* (refusing too much/little)
— a cheaper failure, but **only if no coverage number is ever advertised**.
**Gate:** `h(τ*) ≥ h_target` on hold-out AND within 10pp on the style variant
(no-collapse); ship default-OFF with the curve published if the style variant
moves >10pp. **Kill:** risk-coverage curve ~flat (ce carries no selective signal
on the answer corpus) → keep mechanical-only abstention.

### 8.4 The confidence block: a stronger predictor, no new certificate (H1+H2)

The shipped confidence block is NQC + Clarity squashed 0.5/0.5 — explicitly "NOT
calibration, just bounded blending" (`qpp.rs:49-50`), ρ=0.256 barely clearing the
0.25 gate (`2026-06-12-pi5-p8-gates.md:48-49`). Two cheap, orthogonal predictors
are unused and are *structurally different* from the score-curve family that
failed suite 16:

- **H1 bootstrap rank-stability**: resample the RRF inputs B times with Poisson(1)
  weights, `rank_stability = mean_b RBO_{0.9}(top10(π₀), top10(π_b))`. Fusion is
  pure and measured at 0.144ms (`02-budgets.md:84`); B=20 ≈ 2.9ms on the fast
  path, B=50 ≈ 7ms on deep. It measures the *ranking's own variance*, plausibly
  the predictor least sensitive to query style — exactly the property suite 16
  punishes the absence of.
- **H2 QPP ensemble**: a small linear model over NQC, Clarity, score-gap,
  **lane-agreement** `1 − JSD(local ‖ web)` computed within one response (reusing
  `compare.rs:171` `jsd`, zero new egress), and optionally stability. Fit by OLS
  on the suite-13 set, frozen in-repo.

**Gate (both):** suite-13 ρ ≥ 0.30 tuning / ≥ 0.25 hold-out AND > each single
feature; suite-16 harness for style-shift no-collapse of the *relative* lift. The
output stays "uncalibrated, ranking-comparable" wording (`api.md:133-136`); the
conformal door reopens **only** through the standing suite-16 judge if these ever
constitute a materially stronger predictor (`00-adr.md:733-735`) — that is the
documented path, not part of this proposal's acceptance.

### 8.5 Growing the pool, not just re-ordering it (B1, B2 — conditional)

- **B1 answer-mode embedding-redundancy pruning (HIGH).** Explicitly nominated by
  the ADR-26 suite-15b addendum (`00-adr.md:691-693`); note `voi.rs:57-58` already
  reserves the seam ("Embedding coverage joins when the planner wiring lands…").
  In the single-best regime a *paraphrase* of an already-fetched page carries high
  sketch-novelty (low exact overlap) yet ~zero reveal reward — the asymmetry the
  page model could not exploit but the answer objective can. Add
  `ν_emb(i)=1−max_{j∈S}cos(e_i,e_j)` to `pandora_walk`'s gain (soft
  `min(novelty_sketch, ν_emb)` or hard prune at τ∈[0.3,0.6], the 15b-measured
  band). Microsecond cost (potion at 42.9k docs/s). Judge: new suite-18b composing
  the 15b paraphrase generator × suite-18 outcome semantics; the standing
  `voi-embed` judge. **Kill:** if every τ trades hit-rate >1pp for its fetch
  savings (the exact 15b failure shape), record the no and close the 15b
  nomination permanently.
- **B2 non-generative query decomposition (LOW-MEDIUM, conditional).** The only
  candidate that *grows* the result pool (k-means pseudo-aspects over the RRF
  head, submodular coverage allocation), but the prior is genuinely weak (PRF
  topic-drift), it needs a new harness, and there is **no measured pain signal**.
  Run the zero-cost precondition first: count the fraction of deep queries whose
  head collapses to one evidence cluster (computable from in-RAM clusters, no
  logging). Full candidate only if aspect-deficient pools are real. Hard guards:
  ≤2 extra sub-queries, deep-only, never anon, per-arm reward attribution
  unchanged (SearXNG rate tolerance, risk #12).

### 8.6 A second independence axis (C3 citation graph, MEDIUM — do the irreversible part early)

Extraction keeps only `{title, text}` and discards HTML by hard rule
(`extract.rs:1-9`); **no outlink survives ingest**. ADR-18 sees *textual*
derivation (copies); a hyperlink graph would see *attributed* derivation — ten
differently-worded articles all citing one primary source are 10 ADR-18 clusters
but one citation origin. Retain ≤64 registered-domain outlink hashes per doc in a
new `links_v1` table **written in the same forget-coupled transaction** as
`sketch_v1` (ADR-18 pattern), ≤512B/doc (4× the sketch — record the ceiling). The
analytics (domain-level directed fold, citation-root detection) defer until
intra-corpus density is *measured* (kill if <0.05 edges/doc after re-ingest), but
the **link-retention schema change is the unrecoverable part** — links discarded
today are gone — so it lands early or not at all. Forget story: per-doc row joins
the atomic transaction (class (a)); domain aggregate is provably rebuildable on
the nightly schedule (class (b)); the hermetic forget test extends to assert both.
This is the lawful, ingest-only answer to "source graph" — full web-graph
PageRank is rejected (§13): no crawl exists or may exist.

---

## 9. Geo-analytics: statistically valid regional comparison

Two data surfaces must not be conflated (the brief does): **`/v1/geo/heatmap`
counts operator-ingested docs** (Tantivy fast-field scan, `lexical.rs:368-465`),
while **`/v1/trends` consumes GDELT counters** (`trends.rs:65-72`). Gi*+BH runs on
the doc surface (`stats.rs:158-207`); EB quasi-NB z + BH + burst run on the GDELT
surface (`stats.rs:48-141`, `burst.rs:76-134`). Crucially, **there is no spatial
statistic over the GDELT surface today** — trends take one optional `h3_r5` cell
at a time. That asymmetry, not exotic spatial methods, is where the genuine
greenfield is. Detail in `03-track-workpapers.md` §T4 (E).

### 9.1 The shortlist item: seasonal baselines (E3, **MEDIUM-HIGH**)

**The verified gap.** The mover baseline is the unweighted window mean minus the
latest day (`stats.rs:61-71`); the burst baseline is head-60% moments
(`burst.rs:82-89`). **No weekly-periodicity handling exists anywhere** in
`trends.rs`/`stats.rs`/`burst.rs`, and neither suite generator models it (suite 10
plants Poisson/NB+ramp, `spike.rs:10-18`; suite 17 stationary-baseline ramps). GDELT
media volume has a strong weekend dip, so a Monday latest-day judged against a
weekend-containing baseline gets an inflated z. Two aggravators make this worse
than a per-series nuisance: (1) the error is **correlated across all ~20 root
codes simultaneously** (a common day-of-week factor), so **BH-FDR cannot absorb
it** — every p-value shifts together; (2) the pooled quasi-NB dispersion partly
eats weekly variance as overdispersion, deflating power on *every* day rather than
fixing the bias on the wrong days. This is the one place the otherwise-rigorous
trends stack carries an untested systematic bias.

**The fix.** A multiplicative day-of-week pre-adjustment before the EB fit:
`f_d = (n_d·r_d + λ)/(n_d + λ)` (ratio-to-mean seasonal index, shrunk toward 1),
`y′_t = y_t / f_{dow(t)}`, `dow(t) = (day_epoch+4) mod 7`; skip when the window is
<21 days (<3 obs/weekday); the same adjustment feeds the burst head-window
moments. ~30 lines in `stats.rs`, O(n) per report, <1ms against the ≤60ms row.
Privacy: none (GDELT-only). **Judge:** extend suites 10 and 17 with a
multiplicative weekly cycle (weekend factor swept 0.6–1.4), tuning/hold-out
variants per risk #21. **Accept:** FPR reduction ≥1.5× vs unadjusted at matched
TPR on seasonal variants AND no regression on non-seasonal variants (TPR within
2pp, FPR ≤ unadjusted). **Kill:** if the pooled dispersion already holds seasonal
FPR within 1.5× — a legitimate "the existing machinery was adequate" outcome.

### 9.2 The first spatial view of the GDELT surface (E2a, **LOW-MEDIUM**)

`heatmap_stats` (`stats.rs:158`) is data-source-agnostic `&[(u64,u32)]`. Feeding
it `store.scan(day,day,root,None)` (`store.rs:117`) yields the *first* spatial
hot-spot view of the GDELT surface at the already-measured ~24ms cost class
(heatmap+Gi* 23.7ms p50, `02-budgets.md:123`) — Getis-Ord Gi* with BH-FDR, whose
constants are *already* suite-10-judged, so the method risk is ≈0. Ship as a
`/v1/trends?spatial=day` additive block. **Accept:** p50 ≤60ms. **Kill:** no
operator consumption after one release (the ADR-25 sunset discipline). This is the
cheapest way to test whether anyone wants GDELT-spatial analytics *before*
building anything heavier on top.

### 9.3 The honest spatial-outlier story (E1 LISA, **LOW**)

Global Moran's I enables no defensible user statement on a media-coverage surface
— "clustering exists at all" is vacuous and will be significant on essentially any
day (population geography dominates). LISA's *only* non-redundant statement is the
**HL/LH spatial outlier**: a cell anomalously quiet relative to a hot neighborhood
— on GDELT, a *media-coverage hole*, an honest-signals statement in the ADR-15
spirit. But the inference cost is the killer: at n≈12k res-5 cells the BH rank-1
threshold is q/n ≈ 4.2e-6, so permutation p-values need P ≥ ~240k replications to
clear it (~1.7e10 ops, **nightly-batch only**); P=999 yields a 1e-3 p-floor,
useless under BH at this family size. Ship **only if** a consumer for the
coverage-hole label exists; otherwise it stays a documented possibility. The
normal-approximation escape hatch just recreates the moment-reliability
compromises Gi* already makes.

### 9.4 Uncertainty and privacy controls for the geo layer

- **Sparse-count honesty is already the repo idiom and must scale.** Plug-in
  divergence/entropy on 10–50 results is upward-biased (Miller–Madow-type
  ≈ support/(2n ln2)); the established answer is empirical — *measure the noise
  floor per configuration* (`compare.rs:32-38`, the same-lane p90 0.30 floor,
  ADR-22). Any multi-vantage or spatial extension gets its own suite-12-style
  floor probe (≥300 same-config pairs) before any "exceeds floor" claim means
  anything. Never mix a smoothed statistic with an unsmoothed floor.
- **Region-by-topic summaries must not become a query log.** A standing
  region×topic divergence report built from *organic* queries would be
  query-derived persistent state — it collides with the no-query-logging promise.
  Clean resolution: topic summaries are an **operator batch probe** over a
  *published canned query list* (the suite-12b shape), producing population-level
  claims the per-request block explicitly disclaims (`compare.rs:35-38`), and
  bounding Tor/lane load to scheduled windows.
- **Multi-lane JSD math, frozen now / blocked in code (D1, MEDIUM-spec).** For m
  vantages, ship the omnibus `GJS_π = H(ΣπᵢPᵢ) − Σπᵢ H(Pᵢ) ∈ [0, log₂ m]`
  normalized by log₂ m, **plus** the pairwise JSD matrix (each entry on today's
  [0,1] floor-comparable scale) for attribution, **plus** per-lane one-vs-rest
  for "who is the outlier." This is the flagship differentiator (no SaaS API
  exposes local-first geo divergence), but the *code* is blocked on the Phase-11
  region-sidecar row, which is **NOT BUDGETED** (`02-budgets.md:145`, risk #19,
  384–512MB each) — design-complete, implementation-gated on operator sign-off.

What is **rejected** in the geo track (§13): query-time Kulldorff scan
(~30–60s/999-replicate, three orders over the 60ms row; defer the nightly batch
until E2a proves demand), global Moran's I as a user statement, and (Track G)
spectral clustering / RMT denoising of the 20×90 root-day matrix — the bursts and
weekly cycles E3 fixes *are* the non-stationarity those methods would erase.

---

## 10. Data management: bounded structures, compaction, and provable deletion

The brief frames this track as design; the repo's state turns it mostly into
**audit**. `/v1/forget` provability is the appliance's single strongest claim
("forget-correctness 100%", `SPEC.md:774`), so the primary duty is to *certify it
with cites or surface the gap*, not to invent new streaming machinery. The full
trace is in `03-track-workpapers.md` §T4 (F); the result is one certification, one
P0 remediation bundle, and a documented rejection of premature sketches.

### 10.1 Forget-correctness — traced end to end (F1, CERTIFIED at the result surface)

The forget transaction (`POST /v1/forget` → `forget_keys`,
`ingest.rs:307-362`) is one redb write txn covering: tombstone insert (re-ingest
refused, `ingest.rs:167-175,332-334`), dedup row + url_key→hash removal
(`:335-341`), **sketch row removed unconditionally in the same txn** (`:343-347`,
ADR-19/risk-17), lexical delete-by-term staged + committed (`:348,357-359`),
vector remove + persist (`:349-352,360`); both query caches and the fetch/extract
cache purge default-true (`api/lib.rs:790-792`, `ladder.rs:66-67`); the audit line
carries counts, never the selector (`api/lib.rs:795-797`). Every derived aggregate
classifies cleanly under ADR-19 (`00-adr.md:467-483`):

| Structure | Class | Certification |
|---|---|---|
| Lexical docs / vectors / dedup / sketches | (a) joins the atomic txn | `ingest.rs:335-360` |
| Evidence/cluster annotations | (b) computed per query from the live SketchReader; nothing persisted; deleted sketches cannot contribute; cached SERPs die in the purge | certified |
| Geo heatmap counts | (b) computed on read from live fast fields (`lexical.rs:368-465`); no materialized counts; propagates at the next query | certified |
| Trends counters + edges, PageRank priors | (c) GDELT-only; the ingest path never references `AnalyticsStore`; nightly wholesale replace | certified |
| Decision log / bandit arm stats | no doc data (13 fixed bytes; per-intent means) — reward-bit *influence* tracked separately (I3) | certified |
| Moka caches | purged in-call; anon cache additionally 5-min TTL | certified |

**Verdict: CERTIFIED — no enumerated structure can resurface a forgotten document
in any response.** This is the review's strongest single confirmation and should
be quoted as such.

### 10.2 Three gaps surfaced (F1 remediation bundle — Horizon-0, **P0 correctness**)

None are the risk-17 resurfacing class; all three are honesty/coverage gaps that
sit *above* research priority because they protect the flagship claim:

- **F1-G1 (bytes-at-rest residual, MEDIUM).** Lexical deletion is
  delete-term + commit; the doc's bytes remain in immutable segments until
  LogMergePolicy merges (`lexical.rs:195-197`). The `SegmentStore` trait
  *anticipates* "`/v1/forget` compactions" (`meridian-index/src/lib.rs:43`) **but
  no code forces a merge/GC on forget.** SPEC's literal promise is only "delete
  term + commit" (`SPEC.md:609-610`), yet `privacy.md:162` ("removed from the
  lexical index") may read as erasure-at-rest. The persisted usearch file after
  `remove` has the same question (slot-marking semantics — **verify, do not
  assert**). Fix: a forced merge/GC hook post-forget OR one honest sentence in
  privacy.md stating the residual and the merge schedule. Cheap either way.
- **F1-G2 (doc-vs-code mismatch, LOW-MEDIUM).** `forget_domain` enumerates via
  `TopDocs::with_limit(10_000)` (`lexical.rs:479`); a domain with >10k docs is
  only partially forgotten per call, while `privacy.md:159` promises "every
  currently indexed document of the domain." Fix: loop until the enumeration
  drains, or document "repeat until removed=0."
- **F1-G3 (membership inference, LOW — deliberate).** Tombstones retain a 16-byte
  content hash forever (`privacy.md:137`); an offline attacker with disk access
  and a candidate document can confirm "this content was ingested and forgotten."
  Accepted by design; optional hardening = keyed tombstones HMAC(node-secret,
  hash). Document-only.

### 10.3 Pan-privacy: does any internal state retain a forgotten doc's influence? (I3, joint with F1)

All refit-per-request or nightly-wholesale state is clean (EB prior, quasi-NB
dispersion, burst moments, PageRank priors, the dark TS posterior — all rebuilt,
nothing persisted). **One residual, accepted:** bandit means / decision-log reward
bits retain the *statistical influence* of a forgotten doc (a forgotten local doc
may have flipped a past top-10 reward bit) — but this is **influence, not content
or identity**: one non-invertible bit, no doc reference, 30-day TTL / rolling.
Recommendation: a one-line ADR-19 clarification distinguishing *content/identity
retention* (prohibited) from *statistical influence on coarse aggregates*
(accepted), so the classification is recorded policy rather than reviewer
judgment. The tombstone (F1-G3) is the one structure that deliberately retains a
derivative of forgotten content forever — already on the record.

### 10.4 Exact beats sketch at this scale (F2, REJECT premature)

The deletability rule already confines non-deletable sketches to GDELT aggregates;
the arithmetic shows they buy nothing even there:

- **Per-(day,cell,root) counters.** Measured 128MB/14 simulated days against a
  700MB gate (`02-budgets.md:121`, `store.rs:352-356`). A CMS holding error below
  the *single-digit per-cell-day* signal (the whole point of suite 10/ADR-21)
  needs ε≈1.3e-5 ⇒ ~4MB **per deletable (day,root) unit** ⇒ 90×15 ≈ **5.4GB vs
  ≤700MB exact** — sketches lose ~8× at granularity parity, before the +ε·N bias
  destroys the EB z outright.
- **Per-domain counters:** ~1.5MB exact vs ~74MB CMS. **Distinct domains:** 240KB
  exact set vs HLL saving 224KB — pointless. **Heavy hitters:** 20 root codes,
  exact is free.

**Verdict: REJECT — sketches are premature at 100k–1M docs / single operator.**
The only justified sketch in the system remains MinHash/SimHash (ADR-18), which
earns its place answering a *similarity* question, not a counting one. Re-open
only if key cardinality grows ~100× (multi-tenant or per-URL analytics, neither
planned).

### 10.5 Latency telemetry (F3, NO ROW — file as a chore)

Quantile sketching is already present: `metrics-exporter-prometheus` backs
histograms with DDSketch, and `meridian_request_ms` is recorded per route
(`api/lib.rs:118`). Per-stage `timings` exist per response (`planner.rs:474`) but
are not exported; adding `meridian_stage_ms{stage=…}` is a one-line
`metrics::histogram!` within the existing bounded-cardinality labels. KLL's
deterministic guarantees are irrelevant at single-node sample volumes — an
engineering chore if per-stage p99s are wanted on dashboards, not a research row.

### 10.6 What this track is *not*

No CMS/HLL/KLL adoption, no new persistent query-derived state, no DP on
GDELT-derived series (the protected unit is a *public event* — DP there protects
nothing; the only aggregate with a real protected unit is the operator-doc
heatmap, deferred to §12/H3 with the export boundary). The design discipline is:
**every new persisted structure must name its ADR-19 deletion class and extend the
hermetic forget test before it ships** — that is the standing rule §10.2's
`links_v1` (C3, §8.6) and any future structure must satisfy.

---

## 11. Evaluation plan

The repo already runs an experiment-first protocol — 16 numbered suites, the
tuning-margin / hold-out-no-collapse gate idiom, on-device measurement (risk #8's
bench-first rule), and recorded NOs (suites 13b/15b/16) treated as first-class
outcomes. This section specifies the **datasets, synthetic workloads, statistical
discipline, and acceptance gates** that every §6–§10 proposal must clear, in that
same idiom, and adds two cross-cutting process bets.

### 11.1 Datasets and synthetic workloads (per proposal)

All judges are hermetic generators in `meridian-eval/src/bench/` — no web-scale
training data, no query logs — extended, never replaced:

| Proposal | Generator (existing → extension) | Qrels / ground truth |
|---|---|---|
| C1 corroboration (§8.2) | `answer.rs` + `synfarm.rs` → plant {independent 2nd original, syndicated copy, singleton} beside each decisive original | planted corroboration labels; planted answer hit |
| H3 abstention (§8.3) | `answer.rs` → + style-shift variant (suite-16 phrase trick on the answer corpus) | per-query (ce_score, hit∈{0,1}) |
| H1/H2 confidence (§8.4) | `qpp`/suite-13 set + suite-16 variant generator | per-query nDCG@10 (ρ target) |
| B1 embedding-prune (§8.5) | `voi_embed.rs` 15b paraphrase generator × `answer.rs` outcomes (new suite 18b) | planted decisive original + paraphrase/copy |
| E3 seasonal (§9.1) | `spike.rs`/`changepoint.rs` → + multiplicative weekly cycle, weekend factor 0.6–1.4 | planted spike/ramp truth on seasonal baseline |
| V1 filtered ANN (§6 Bet 4) | suites 2/12 → + geo/time-filtered query set at 100k/1M | nDCG@10 vs unfiltered dense contribution |
| K1/K3 Hailo (§6 Bets 1) | suites 4/13/18 unchanged; new CPU-vs-NPU arm | ms/pair (batch 1/4/8); nDCG@10 parity |
| J2 admission (§7.5) | new suite 19 `admission`: closed-loop mixed fast/heavy | fast p99 inflation ratio; heavy completion rate |

### 11.2 Geo and adversarial scenarios

- **Geo scenarios.** E3 seasonal nulls (weekend-dip series that must *not* fire);
  planted spatio-temporally compact moderate elevations (the one shape z+burst
  jointly miss, the E2/Kulldorff motivation — used to *bound* the value, then
  reject query-time scan); planted cold-cell-in-hot-ring for the E1 LISA recall
  gate; multi-vantage floor probes (≥300 same-config pairs per lane-set before any
  "exceeds floor" claim).
- **Adversarial scenarios (the risk-#21 "measure the generator, not the operator"
  discipline).** Syndication farms for C1 (the copy must *never* count as
  corroboration — same-cluster leakage is an automatic kill); query-style shift
  for H1/H2/H3 (the standing falsifier that killed conformal — relative lift must
  not collapse); aspect-deficient pools for B2 (decisive aspect-2 docs retrievable
  only by the sub-query); threshold-nudging tripwire — re-fitting a failed ρ on
  the variant data is *prohibited* (the risk-#24 precedent), a failed style
  variant is a recorded NO, not a re-roll.

### 11.3 Statistical discipline — gates, power, and pre-registration (J3, **P1**)

The single highest-leverage process change: **every bench gate pre-registers its
MDE₈₀ in one line of the bench doc; an observed margin below the gate's MDE is
recorded "underpowered," never "pass."** The arithmetic the repo's existing gate
types reduce to (T1 §J3):

- **Proportion gates** (suite-18 style): MDE₈₀ ≈ 2.8·√(2p̄(1−p̄)/n). At p̄=0.65,
  n=1000 → **6.0pp** (suite-18's +10.7pp was adequately powered); certifying a 3pp
  effect needs n≈3,970.
- **Paired nDCG gates** (suite-15's ±0.01 bar): with paired sd≈0.1, MDE₈₀ =
  2.8·0.1/√n ⇒ the ±0.01 bar needs **n≥~780 paired queries**; 200-query suites can
  only honestly claim ±0.02.
- **Latency medians at n=16** (the probes idiom): replicate medians imply run-sd
  ≲30ms ⇒ deltas ≳~100ms detectable; gates targeting <50ms deltas at n=16 are
  underpowered (record, don't claim).
- **The OPE gate** is the proportion case inflated by the IPS correction (×1.15 to
  ×30 per-row variance by disagreement rate) — the bridge that makes §7.3's MDE
  the same arithmetic as every other gate.

### 11.4 On-device measurement protocol

Bench-first, on the Pi, never simulated (risk #8). Latency arms use **n≥16
interpolated medians** (the probes idiom; n<16 medians are interpolated and
flagged). Every latency-touching proposal re-measures its budget row on device
(fast ≤25ms working target, deep p50 ≤2.5s, answer p50 ≤3.0s — measured 2502ms @
cap 8, `02-budgets.md:148`). Hailo arms additionally instrument suite 6 (thermal)
with PMIC power sampling (`vcgencmd pmic_read_adc`), CPU-CE vs NPU-CE, reporting
°C / throttle flags / J/query (K5), and **must prove fail-open**: yank
`/dev/hailo0` mid-run → the path degrades to CPU, never errors. The amd64/CI lane
hosts only the x86-only Hailo DFC compile step (the HEF artifact), never a serving
measurement.

### 11.5 Simple-baseline comparisons (mandatory, the repo's own rule)

Every proposal names the trivial baseline it must beat, and the review is prepared
to record **simple-baseline-wins**: E3 vs unadjusted z (kill if dispersion already
absorbs DOW); C2 PageRank vs raw weighted degree (predicted ρ>0.9 → replace the
30-iteration power method with an O(E) degree pass); E2a Gi* vs per-cell z; H2
ensemble vs each single feature (ship the single feature if the fit's gain is
one-feature); V1 filtered ANN vs lexical-only under filters (deprioritize if
measured harm <2pp — the recorded tradeoff was right); B2 decomposition vs
single-query (dead at step one if planted aspect-2 recall <50%). A baseline that
wins is a *result*, not a failure.

### 11.6 J1 Pareto synthesis (P1) — the documentation win on measured data

Knobs with measured curves already on disk (`answer_passage_cap`, ef/expansion,
VoI `deep_fetch_max`, rerank depth/batch — T1 §J1) get one operator-facing
frontier table normalizing each to (quality, latency) and marking *dominated*
settings (e.g. ef=64 @1M is dominated: −4pp recall to save 0.95ms p99 against a
40ms gate). Gaps to fill with bench-only sweeps (no production code): rerank depth
{10,20,40}×batch{4,8}; answer cap 12; the never-swept `deep_fetch_deadline_ms`.
Cheap, high-value, and it gives J2 and every future shed-ladder tuning their cost
curves.

---

## 12. Phased roadmap

**This section amends nothing.** It proposes an *ordering* of candidate work;
promotion to plan-of-record happens only via operator sign-off and a new ADR in
`docs/plan/00-adr.md`. No ADR numbers are minted here; the latest shipped is
ADR-29. The horizons map to the repo's own structure: SPEC §16 records Phase-11
candidates "recorded, not planned" (`SPEC.md:837-841`), and the budget table marks
the region-sidecar row **NOT BUDGETED** (`02-budgets.md:145`). The experiment-first
protocol holds throughout: **the judge suite lands before the feature** (the
Phase-10 protocol), so each H2/H3 item below names a suite that must pass first.

### H0 — v0.6.x hardening (honestly near-empty)

v0.6.0 was the carry-closure release: all three post-v0.5.0 carries closed
(diversity=evidence, answer-cap study, hybrid healed-lane re-eval — PRs #26/#28/#27),
21 invariants green, both profiles measured at scale. There is **no backlog of
deferred correctness work** here, and the review will not pad one. The single
exception worth pulling forward is the **F1 forget-remediation bundle** (§10.2):
it is Horizon-0 correctness, not research, because it protects the appliance's
strongest claim — a forced-merge/GC hook *or* one honest privacy.md sentence
(G1), the `forget_domain` >10k loop (G2), and the usearch persisted-file
slot-marking *verification* (not yet asserted). Small, above research, do first.

### H1 — Phase-11 recorded candidates + the calendar-bound ADR-25 verdict

The items SPEC §16 already records, with a recommended promotion order:

1. **ADR-25 contextual-routing verdict (~2026-08-11, calendar-bound).** Execute
   §7.4: pre-register the four branches *now*; land the **A2 graded reward before
   any organic accrual** (it is worthless retroactively — rows are immutable); set
   the extend-once rule (trailing-14-day rate ≥334/day). Modal outcome:
   `insufficient_data → sunset`, ε-greedy retained, module stays dark. This is the
   only date-bound obligation in the whole roadmap.
2. **Region-lane metasearch sidecars (ADR-22).** Unlocks the multi-lane JSD
   flagship (§9.4 D1), whose *math is design-complete now*. Blocked on the **NOT
   BUDGETED** row (`02-budgets.md:145`, 384–512MB each) + operator sign-off + the
   risk-#19 compose RAM ceiling (>6.5GB committed ⇒ feature stays off). Promote
   only when the operator wants regional vantages enough to accept the budget row.
3. **Binary quantization** — trigger-bound (>1.5M docs, `SPEC.md:840`); the BQ row
   already encodes the correct future decision point (§13 rejects the premature
   PCA rung between int8 and BQ).
4. **crates.io publication** — follows the operator's repo-visibility decision
   (orthogonal to research).

### H2 — research prototypes (the top-five bets, experiment-first)

Each bet's judge suite is built and must pass before the feature ships
(§11.1). Recommended sequence by *evidence-readiness × cost*, not just priority:

| Order | Bet | Why this slot | First gate |
|---|---|---|---|
| 1 | **E3 seasonal baselines** (§9.1) | cheapest, killable in hours, fixes a correlated bias in a *shipped* statistic | suite-10/17 seasonal variant: FPR ↓1.5× at matched TPR |
| 2 | **Answer trust layer** = H3 abstention + C1 corroboration (§8.2–8.3) | data on disk, ~0 marginal compute, mitigates registered risk #26 on the flagship block | suite-18 ext: precision ≥0.9; selective h(τ*) holds on style variant |
| 3 | **B1 embedding-redundancy pruning** (§8.5) | evidence-nominated (15b addendum), `voi.rs:57` seam reserved, `voi-embed` judge exists | suite-18b: hit-rate +2pp on paraphrase-heavy OR fetches −15% within ±1pp |
| 4 | **V1 filtered ANN** (§6 Bet 4) | the worst identity/implementation mismatch (geo queries lose the dense lane, `planner.rs:624`); *measure the harm first* | suites 2/12 geo-filtered: recover ≥95% dense contribution at ≤+10ms |
| 5 | **Hailo CE offload** = K1 (+K3, +K5) (§6 Bet 1) | largest hardware lever, but gated on the **unverified x86 DFC host + BERT-class compilability** (C4); fail-open-to-CPU is the design | suite-4 NPU arm: ms/pair ≤0.5× CPU AND nDCG@10 parity AND fail-open proven |

H1/H2 confidence-predictor work (§8.4) rides alongside as a MEDIUM-HIGH adjunct
(ship H1+H2 together so stability enters the ablation); C3 link-retention (§8.6)
should land its *irreversible* extraction change early even though its analytics
defer. J1/J3 (§11) are process bets with cross-track payoff and should land first
of all — they cost bench time and a doc rule, nothing else.

### H3 — evidence-gated futures (trigger-bound, not scheduled)

- **Confidence bands** — return *only* through the standing suite-16 judge with a
  materially stronger predictor (ADR-27's documented re-entry path); H1/H2 may
  eventually constitute that evidence. Until they measurably do, no band ships.
- **DP aggregate release** (§10.6, I1) — top-bet ranking *only* paired with an
  operator-facing publish/export feature (the boundary that creates a protected
  unit); standalone it is a contingency file with a pre-registered utility gate
  (BH-FDR detections survive ε≤2 on the suite-10 extension) and the honest
  expectation that res-5/day granularity will not survive it.
- **C3 citation analytics** — gated on measured intra-corpus citation density
  ≥0.05 edges/doc after re-ingest.
- **Hailo K2 embedding upgrade** — only after K1 proves the toolchain, and only
  with the 256→384-d schema-migration cost priced (the 600MB disk row busts).

### Roadmap honesty notes

- **Nothing here is on a release critical path that depends on the NPU.** Hailo
  enters at H2 with a candidate-ADR for heterogeneous-compute policy
  (accelerator-never-dependency, fail-open invariant inv22), never as a gate.
- **The only date-bound item is the ADR-25 sunset.** Everything else is
  trigger-bound or evidence-gated — consistent with an appliance that ships when
  the measurement says so, not on a calendar.

---

## 13. Rejection list

Two classes: methods the **repo already killed through its own gates** (binding
evidence — must not be re-proposed without new evidence through the same judge),
and methods **this review rejects** after engaging them. Full arguments in the
track workpapers; the one-line verdict and the *re-entry condition* are recorded
here so a future proposer knows exactly what would have to change.

### 13a — Empirically killed by the repo's own gates (stay killed)

| Method | Killed by | Re-entry condition |
|---|---|---|
| **Conformal coverage bands** | suite 16: 19pp absolute-coverage collapse under query-style shift; ADR-27 REFUTED | a materially stronger predictor through the *same* suite-16 judge (the H1/H2 path) — never a re-fit on variant data |
| **MMR diversity rerank** | suite 13b (both seeds): demotes canonical originals with their copies; superseded by diversity=evidence, dominant on BOTH metrics | new evidence through suite 13b; none exists (diversity=evidence already owns the ground) |
| **VoI embedding-coverage combiner (page objective)** | suite 15b: signal real (0.716 vs 0.052 cosine) but no profitable combiner — paraphrase copies still buy page-reveal rank mass | the **answer** objective, where the asymmetry flips (B1, §8.5) — explicitly the addendum's nominated path |
| **Pandora's-box for the page objective** | suite 15: starves 2/3 subtopic clusters after its first decisive find | none for page; validated and shipped for the single-best/answer regime |
| **Neural routers / MCTS / bandits-with-knapsacks** | ADR-25 a-priori (no training data, NP-hard, unexplainable) — and now on *worse* data (0 organic rows) | training data that does not exist + an explainability story the appliance does not have |

### 13b — Rejected by this review (engaged, then rejected with the bar each must clear)

| Method | Track | Verdict and re-entry condition |
|---|---|---|
| **NPU brute-force filtered dense scan** (the literal "Hailo speeds up ANN" test) | K4 | **REJECT with numbers**: per-query vectors are data not weights → 25.6MB must cross PCIe ≈ 64ms transfer (Gen2 x1) vs the *entire* CPU scan at 2–4ms; HailoRT exposes no generic-matmul API. The surviving R9 attack is index-side filtered ANN on CPU (V1). Re-open only if a runtime exposes resident generic GEMM with on-device data updates |
| **Signed-graph contradiction detection** | C/R1 | REJECT: requires NLI — no model in-tree (the `ort` *runtime* exists, but adding an NLI model breaks the "no neural cost" idiom), O(k²) CE-class inferences blow the answer headroom, no contradiction-labeled corpus, and false "sources contradict" is the conformal lesson at max stakes. The Hailo-8L cannot rescue it (x86-only DFC, no retrieval HEF). C1 ships the unsigned (support-only) half. Re-entry: an offline-compiled model + a planted-contradiction suite |
| **Full web-graph PageRank** | C/R2 | REJECT: no crawl exists or may exist (egress is metasearch + the bounded fetch ladder, 1 req/2s/domain, SSRF-guarded, hermetic egress-invariant gate). C3 is the lawful version — links over *ingested* docs only |
| **Min-cut / spectral splitting of evidence clusters** | C/R3, G1 | REJECT: fixes zero observed failures (suite-9 false-merge 0.0, F1 0.918 hold-out); union-find is already conservative. Re-opens only if the false-merge tripwire fires on organic data |
| **Spectral clustering of domain co-occurrence** | G1 | REJECT: the graph topology is *manufactured* by clique construction, the only ranking consumer (`domain_prior`) is weighted 0, eigengap unmeasured. Re-entry: `domain_prior` earns nonzero LTR weight from real training data AND a measured eigengap |
| **RMT / Marchenko-Pastur trend denoising** | G2 | REJECT by construction: the only matrix the store yields is 20×90 (roots×days), p=20 ≪ asymptopia, and the bursts/weekly-cycles E3 fixes *are* the signal MP would erase. No re-entry at this store schema |
| **Low-rank (PCA) embedding compression** | G3 | REJECT premature: int8 256-d is not a binding constraint at any profiled scale; the BQ trigger (>1.5M docs) already encodes the correct future decision point |
| **Exact→sketch (CMS/HLL/KLL) for analytics counters** | F2/F3 | REJECT: exact wins 8–50× at single-operator scale and sketch bias destroys the EB z (§10.4); DDSketch already backs latency histograms. Re-open only if key cardinality grows ~100× |
| **Query-time Kulldorff scan statistics** | E2 | REJECT for query-time (~30–60s/999-replicate, three orders over the 60ms row); defer the nightly batch until E2a proves the GDELT-spatial surface is consumed at all |
| **Global Moran's I as a user statement** | E1 | REJECT: vacuous on a media-coverage surface (significant on any day). Only the LISA HL/LH *outlier* is defensible, and only if a consumer exists |
| **DPP diversity rerank** | B/D | REJECT: a symmetric kernel re-imports the MMR failure; given cluster structure it collapses to diversity=evidence, already shipped and dominant. Nothing left to buy |
| **MI feature selection / NCD / information-bottleneck excerpts** | D | REJECT: no labeled training data (MI selects nothing); MinHash owns lexical derivation and potion owns semantic register (NCD adds no third signal); IB's relevance variable is exactly what the CE already scores |
| **Deep/Bayesian uncertainty, ranker ensembles** | H/R5,R6 | REJECT: need neural forward passes the budgets don't have, and answer a question H1's 7ms bootstrap already answers over the *actual* production fusion |
| **Local DP / federated / MPC / secure aggregation** | I | REJECT: single-node single-operator — no honest threat model in which the node randomizes against itself. Re-enters only with the multi-node/region-sidecar contingency and its ADR |
| **LLM / decoder models on Hailo-8L; neural intent classifier** | K | REJECT: wrong architecture class (static dataflow, no KV-cache) and on-chip memory excludes decoder-class models; the 4-class lexical intent has no training data and the NPU round-trip is ~1000× its µs budget |

---

## 14. Final recommendation

### 14.1 One product and research identity

**Meridian is a privacy-preserving, geo-aware analytical search appliance whose
differentiator is not answer speed but *shown evidence under resource and
deletion constraints*.** Competitors (Exa, Parallel, Tavily, Brave, deep-research
agents) answer faster and at web scale; none of them expose evidence structure,
source dependence, geographic divergence, or provable deletion, and none run on a
private 8GB edge box. The review's through-line: the ranker ground is mined out
(VoI within −0.0023 nDCG of fetch-all; answer mode 3.8× its nearest wrong
objective), so the value-add is **trustworthy structure around results** —
claim-level corroboration, calibrated abstention, regional divergence, and an
honest "we don't know" — built on machinery the repo already shipped and gated.
The Hailo-8L is a *force multiplier on that identity* (it buys rerank depth and
answer-latency headroom), **not a new identity** and never a dependency.

### 14.2 The first three experiments (each killable in ≤2 Pi-days, judge before feature)

1. **E3 seasonal baselines** (§9.1) — extend the suite-10/17 generators with a
   weekly cycle; run shipped-z vs DOW-adjusted-z. *Decides:* whether a shipped
   statistic carries a correlated bias BH-FDR cannot absorb. Cheapest possible
   start; kill in hours if the pooled dispersion already absorbs it.
2. **Answer trust layer** (§8.2–8.3) — extend suite-18 with planted
   {independent / syndicated / singleton} corroboration and a style-shift variant;
   measure corroboration precision and the selective risk-coverage curve. *Decides:*
   whether the flagship block can carry a claim-level trust signal and a principled
   abstention without a false certificate. Data is already on disk.
3. **The J3/J1 process pair** (§11.3, §11.6) — pre-register every gate's MDE and
   synthesize the knob Pareto frontier from bench data already on disk. *Decides
   nothing experimentally* but makes every subsequent experiment honest and
   correctly powered — the cheapest leverage in the document.

In parallel, two **non-experiment** obligations: land the **A2 graded reward
before any ADR-25 organic accrual** (§7.4 — time-critical, worthless
retroactively), and ship the **F1 forget-remediation bundle** (§10.2 — Horizon-0
correctness protecting the strongest claim).

### 14.3 What evidence would change this roadmap

- **The ADR-25 verdict (~2026-08-11).** A `pass` flips contextual routing on and
  re-orders H2 toward routing; the modal `insufficient_data` confirms the
  sunset-and-stay-dark recommendation. Either way the date forces the decision.
- **A measured filtered-query harm <2pp** (V1's first gate) retires Bet 4 — the
  recorded geo/dense tradeoff (ADR-10) was right, and the "geo-aware" identity is
  satisfied by lexical filtering alone. A harm ≥2pp promotes it.
- **C4's DFC answer.** "Cannot compile a BERT-class encoder on the 8L" reduces the
  entire Hailo track to a documented no-go with the §T5 arithmetic on the record;
  "compiles single-context" promotes K1/K3 toward the top of H2.
- **An operator export/publish feature** turns DP release (I1) from a contingency
  file into a top bet with a crisp gate; absent that boundary it stays deferred.
- **An organic decision rate ≥334/day** is the *only* thing that makes the routing
  gate winnable under the 30-day TTL; below it, no amount of calendar time helps.

### 14.4 Closing note on method

Every recommendation here is a hypothesis with a named baseline, a complexity and
Pi-5 cost, a privacy impact, a judge suite, and an explicit kill criterion; the
review re-proposes no method the repo killed, and it is prepared to record
*simple-baseline-wins* and *recorded-NOs* as first-class outcomes — the same
discipline that produced suites 13b/15b/16. The document **proposes; it does not
decide.** Promotion to plan-of-record stays with operator sign-off and a new ADR.

---

*End of review. As-of: v0.6.0 / `812d3d4` / 2026-06-13. Companion files:
`01-rebaseline.md` (claim ledger), `02-competitive.md` (competitor ledger),
`03-track-workpapers.md` (Tracks A–K, full math and Pi-5 costs).*
