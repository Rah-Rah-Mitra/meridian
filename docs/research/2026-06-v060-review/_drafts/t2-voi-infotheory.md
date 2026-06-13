# T2 — Tracks B (parallel information finding) & D (information theory)

Status: DRAFT workpaper — proposes, does not decide. Baseline: `01-rebaseline.md`
(binding), v0.6.0 / 2026-06-13. Hard constraints applied throughout: no query
logging, anon-lane isolation, forget provability, Profile-R budgets
(deep p50 ≤2.5s, answer p50 ≤3.0s — measured 2502ms @ `answer_passage_cap` 8,
`bench/2026-06-13-pi5-answer-cap-study.md:8-13`), and the §3 killed list stays
killed (`01-rebaseline.md:38-50`).

## 0. What is ALREADY ANSWERED — the evidence floor this track builds on

This is the most evidence-saturated corner of the repo. Four questions that a
fresh literature review would "propose" are already measured, with frozen
constants and standing judges:

1. **Pandora's-box stopping is the wrong objective for page ranking — measured,
   not argued.** Weitzman optimizes the single best find; page nDCG is additive;
   the suite-15 replay measured the walk starving 2 of 3 subtopic clusters after
   its first decisive find (`docs/plan/00-adr.md:661-666`,
   `crates/meridian-fetch/src/voi.rs:44-48`). Killed for the page objective
   (`01-rebaseline.md:45`).
2. **Additive greedy is near-optimal for the page objective and shipped.**
   `additive_walk` opens while expected net marginal value `p·gain − cost > 0`
   (`voi.rs:104-143`); greedy is near-optimal because the novelty-discounted
   gains are diminishing by construction (`voi.rs:99-101`). Suite-15 hold-out
   with frozen β0=0.2/β2=0.3: nDCG@10 **0.7411 vs fetch-all 0.7434 at −30.5%
   fetches**, median fetched clusters 3 vs rank-greedy 2
   (`docs/plan/bench/2026-06-12-pi5-p9-voi.json:22-31`). The gap to fetch-all is
   **−0.0023 nDCG** — the residual quality headroom for ANY better page-value
   model is essentially zero; only fetch savings remain to be won.
3. **Embedding-coverage: signal real, page model unprofitable.** Potion
   separates same-field paraphrase registers at cosine 0.716 vs 0.052
   cross-field — redundancy MinHash cannot see — but every swept gain discount
   (min / product / mean) saves fetches (up to 41%) only at page-nDCG cost
   beyond the ±0.01 bar, because in the page-reveal model a paraphrase copy
   still buys rank mass (`docs/plan/bench/2026-06-13-pi5-p10-voi-embed.md:10-34`;
   ADR-26 suite-15b addendum, `00-adr.md:683-693`). RECORDED NO; `voi-embed` is
   the standing judge for any future use of the signal, and the addendum
   **explicitly nominates answer-mode pruning** as the candidate future use
   (`p10-voi-embed.md:42-45`, `00-adr.md:691-693`).
4. **Pandora is validated for the single-best regime.** Suite-18 hold-out
   (n=1000): answer hit-rate **0.700 vs 0.593** no-fetch baseline (+10.7pp,
   gate +10pp), **3.8× the additive page selector at fewer fetches** (0.183 @
   2.00 fetches vs 0.700 @ 1.65) — the regime split measured from both sides
   (`docs/plan/bench/2026-06-12-pi5-p10-answer.md:15-28`). `ANSWER_FETCH_COST
   = 0.1` frozen in passage-CE units, a deliberate separate unit system
   (`voi.rs:165-173`).

Production envelope every candidate below must respect: `deep_fetch_max = 2`
(`crates/meridian-common/src/config.rs:303`), fetch-phase deadlines 1200ms
(deep) / 1800ms (answer) (`config.rs:304-305`), head = top-20
(`crates/meridian-query/src/planner.rs:1045`), per-fetch CE realization is the
dominant answer-mode latency term (~700ms of the cap-16→8 delta,
`answer-cap-study.md:14-16`).

---

## 1. B1 — Answer-mode embedding-redundancy pruning  **[PRIORITY: HIGH]**

**Proposal.** Down-weight (or hard-prune) answer-mode fetch candidates whose
embedding similarity to already-fetched text exceeds a calibrated threshold,
inside `pandora_walk`'s value model.

**Meridian problem.** The answer candidate's gain is pure sketch novelty:
`gain = novelty = 1 − max sketch-containment vs fetched` (`voi.rs:178-185`,
computed at `planner.rs:1087-1095` over title+snippet). A *paraphrase* copy of
an already-fetched page carries high sketch novelty (low exact-token overlap,
`p10-voi-embed.md:13-16`) yet has ~zero reveal reward in the single-best
regime: the suite-18 corpus construction itself states that "a copy is a worse
place to read the answer than its original" (`p10-answer.md:10-12`). The page
model could not price paraphrases out profitably because a paraphrase reveal
still buys rank mass; the answer objective has no such consolation prize —
this is precisely the asymmetry the 15b addendum recorded.

**Math.** Let fetched set S with potion embeddings {e_j}; candidate i has
embedding e_i (title+snippet; full extracted text for members of S, RAM-only
like sketches). Define embedding-novelty `ν_emb(i) = 1 − max_{j∈S} cos(e_i,
e_j)` (07-voi-design.md:46-48 — the term the design always specified). Answer
candidate becomes

```
gain'_i = min(novelty_sketch(i), ν_emb(i))        (soft form), or
gain'_i = novelty_sketch(i) · 1[max_j cos(e_i,e_j) < τ]   (hard prune)
z_i     = gain'_i − ANSWER_FETCH_COST / p_i        (reservation index, voi.rs:37-39)
```

Justification in the Weitzman frame: if doc i derives from fetched doc j whose
best passage realized CE v_j, then E[best passage CE of i] ≤ v_j + ε (register
noise; copies carry truncated passages), so the box's true two-point gain is
≈0 and its honest reservation index is below any incumbent — the model should
say so. τ is swept on the tuning seed inside the band 15b measured: same-field
registers 0.716, cross-field 0.052 → τ ∈ [0.3, 0.6].

**Expected gain.** Bounded honestly by `deep_fetch_max = 2`: mean answer
fetches are already 1.65/2 (`p10-answer.md:17`). Two distinct wins: (a)
*redirection* — when slot 2 would have gone to a paraphrase of slot 1, the
walk opens a genuinely different doc instead → hit-rate gain on
syndication-heavy queries (the measurable claim); (b) *earlier optimal stops*
when all remaining candidates are copies → saves one fetch + one passage-CE
batch ≈ 700–900ms on affected requests (tail, not p50, of the 2502ms
baseline). Claim (a) is the gate; (b) is the latency bonus.

**Complexity.** O(|head| · |S|) cosines per re-pose, |S| ≤ 2. **Pi-5 cost:**
potion embeds at 42.9k docs/s (`01-rebaseline.md:68-69`) — microseconds;
requires `models/` present (the `voi-embed` suite already skips honestly
without it, `crates/meridian-eval/src/bench/voi_embed.rs:349-356`). No new RAM
class (embeddings of fetched text are transient, same lifetime as
`fetched_sketches`, `planner.rs:1066`).

**Privacy.** None: pure local geometry over text already in RAM; no new
egress, no logging; forget-provability untouched (nothing persisted).

**Required data.** None beyond the repo — hermetic replay; the 15b
paraphrase-register generator and the 18 answer-outcome semantics both exist.

**Evaluation (existing judges, composed).** New suite **18b `answer-embed`**:
the 15b corpus semantics (each decisive original gets one token copy AND one
paraphrase copy, `voi_embed.rs:215-269` generator machinery) × the suite-18
outcome semantics (hit-rate, `bench/answer.rs:278-330`). Idiom: τ swept on
the tuning seed (a-priori rule: max hit-rate, fewer fetches tie-break — the
suite-18 rule verbatim); frozen winner judged on a disjoint hold-out seed.

**Implementation location.** `voi.rs::answer_candidate` gains an
embedding-novelty argument; `planner.rs:1097-1103` computes `ν_emb` beside
sketch novelty; new `meridian-eval/src/bench/answer_embed.rs` (or an arm in
`answer.rs`).

**Acceptance:** tuning hit-rate ≥ frozen answer mode AND (hit-rate +2pp on the
paraphrase-heavy generator OR mean fetches −15% at hit-rate within ±1pp);
hold-out no-collapse (hit-rate ≥ frozen −1pp); answer p50 does not regress
(device arm, n≥16 interpolated median). **Kill:** if every τ trades hit-rate
beyond 1pp for its fetch savings — the exact 15b failure shape — record no
through the same judge and close the 15b nomination permanently.

---

## 2. B2 — Non-generative query decomposition / aspect-coverage allocation  **[PRIORITY: LOW-MEDIUM, conditional]**

**Proposal.** Extract pseudo-aspects WITHOUT an LLM — embedding/term clusters
over the retrieved top-k pool (pseudo-relevance-feedback style), or
engine-category fan-out — then allocate fetches and/or SearXNG sub-queries
across aspects by submodular coverage.

**Meridian problem.** One query string goes upstream
(`crates/meridian-searx/src/client.rs:93-105`); aspect coverage is handled
only *post hoc* by the additive walk's novelty discount. The walk can only
cover clusters that retrieval surfaced: suite-15 shows the additive walk
recovers 3/3 median clusters **from a pool that contains them**
(`p9-voi.json:28`). Decomposition only pays when the pool is
aspect-deficient — a condition no current telemetry detects and no current
suite generates.

**Math.** Aspects A_1..A_m from k-means (m ≤ 3) over potion embeddings of the
RRF head (k≈50), weights w_j = pool mass fraction. Coverage objective
`F(S) = Σ_j w_j · max_{d∈S} rel(d, A_j)` — monotone submodular; greedy gets
(1−1/e). Fetch allocation reduces to the existing frame: candidate gain
becomes the marginal coverage `Σ_j w_j · max(0, rel(i,A_j) − cov_j(S))`,
i.e., aspect structure replacing pairwise containment in the same
`additive_walk`. The fan-out variant issues m sub-queries (query + top
discriminative aspect terms) concurrently within `searx_deadline_ms` (800ms,
`config.rs:297`).

**Honesty about lexical decomposition.** A short query has nothing to
decompose; the aspects must come from the result pool, which imports PRF's
classic failure mode — topic drift amplifies whatever the first retrieval got
wrong. Non-generative aspect labels are noisy term clusters, not intents. The
prior here is genuinely weak, and the workpaper says so.

**SearXNG rate tolerance (risk #12) and the 2.5s budget.** Each sub-query
multiplies upstream load; the direct lane already duplicates slow requests
(hedging, `client.rs:121-138`), so m sub-queries ⇒ up to 2m upstream hits per
user query against engines that suspend under pressure (risk #12 alert: engine
error rate >50%/1h, `docs/plan/03-risk-register.md:22`). Concurrent dispatch
fits the deep 2.5s wall-clock but the rate guard must be structural: m ≤ 2
extra sub-queries, deep-mode only, never on anon (no multi-circuit fan-out),
and the bandit reward attribution must stay per-arm (sub-queries pin the
chosen arm's engines — no new bandit surface).

**Expected gain.** Unknown sign. **Complexity:** moderate (planner fan-out +
merge into RRF as extra lists — `planner.rs:755-758` already fuses per-engine
lists, so merging is natural). **Pi-5 cost:** k-means over ≤50 potion vectors
is microseconds; the cost is upstream. **Privacy:** sub-queries are derived
from query text and sent upstream exactly as the query itself is — no new
disclosure class, nothing logged.

**Required data.** A new generator: aspect-deficient pools (decisive docs for
aspect 2 retrievable only by the sub-query, planted below the head cut). No
telemetry exists to show this happens organically — flag for operator
sign-off: a *counting-only* aggregate (fraction of deep queries whose head
collapses to 1 evidence cluster) would justify or kill the candidate before
any implementation; it is computable from existing in-RAM evidence clusters
with no logging.

**Evaluation.** New suite (margin/no-collapse idiom): on the aspect-deficient
generator, decomposed retrieval must beat the single-query baseline on
alpha-nDCG@10 by margin on tuning AND not regress plain nDCG@10 on hold-out
(the 13b double-axis lesson, `bench/2026-06-12-pi5-p8-gates.md:72-95`); on the
standard suite-15 corpus it must be a measured no-op; upstream requests ≤2×
per user query.

**Acceptance:** both axes pass + no rate-guard breach. **Kill (crisp):** if
clustered pseudo-aspects on the tuning seed cannot produce sub-queries that
retrieve the planted aspect-2 docs into the head at all (retrieval-recall of
planted docs < 50%), the mechanism is dead at step one — record no without
sweeping the allocator. Priority LOW-MEDIUM: the only candidate that grows
the pool rather than re-ordering it, but weak prior, new harness required,
and no measured pain signal. Run the cheap cluster-collapse count first.

---

## 3. B3 — Tail-latency hedging / optimal stopping for fan-out  **[PRIORITY: LOW, telemetry-first]**

**What exists (verified).** Metasearch hedging is direct-lane only: one
duplicate request after a **fixed** `hedge_after_ms = 300`
(`config.rs:207,216`), first success wins inside the 800ms deadline
(`client.rs:121-138`); never on anon (`client.rs:83-85` — correct: duplicate
Tor circuits double the observable fingerprint and burn the anon budget).
The deep fetch phase is strictly sequential with a phase deadline
(`planner.rs:1138-1142`, per-fetch timeout `planner.rs:1172-1179`) and no
hedging. Compare mode is deliberately sequential — direct, then jitter (≤30s,
`config.rs:300`), then anon (`crates/meridian-query/src/compare.rs:66-101`) —
it is a decorrelation path, not a latency path, and must stay one.

**Math.** Hedged request at delay t with latency CDF F: completion
`T = min(T₁, t + T₂)`, so `P(T > x) = (1−F(x))·(1−F(x−t))` for x > t.
Choosing t = F⁻¹(q) costs expected extra load `1 − q` and pulls the tail
toward `t + F⁻¹(p)` for moderate p — the standard "hedge at p95 for ≤5%
extra load" result. Queueing check: duplicates raise sidecar utilization ρ
with response inflation ∝ 1/(1−ρ); at the deployment's 5rps/8-in-flight caps
the LOCAL sidecar ρ is low — the binding constraint is upstream engine rate
tolerance (risk #12), same as B2.

**The gap.** Whether 300ms is a sensible quantile of this deployment's searx
latency is unmeasured: `EngineHealth` counts responses/results only
(`client.rs:55-59`), and no hedge-fired counter exists anywhere in
`meridian-searx`. Proposal in two stages: (1) **telemetry only** — an
aggregate hedge-fired counter + a searx_ms histogram (route/lane labels only,
within the §13.4 cardinality rule); (2) only if p95−p50 ≫ 300ms or hedge-fire
rate is pathological (≈0% = delay too long; >30% = too short), make the delay
quantile-adaptive: `delay = clamp(300, EMA-p95̂, deadline/2)`.

**Fetch-phase hedging — argued and declined.** Opening the top-2 reservation
candidates in parallel sacrifices the re-pose adaptivity
(`planner.rs:1079-1082`) for latency; with production budget 2 it degenerates
to fetch-all-2, erasing the stop rule whose measured value IS the feature
(`analysis` honesty block, `docs/api.md:294-305`). And the answer-mode
latency budget is dominated by CE realization, not fetch wait
(`answer-cap-study.md:14-16`) — hedging fetches does not touch the binding
term. **Expected gain:** small tail improvement on searx_ms only.
**Pi-5 cost / privacy:** nil / nil (aggregate counters). **Required data:**
the stage-(1) counters — existing telemetry style, no sign-off needed.
**Evaluation:** soak-style A/B on device (suite-12-pacing idiom), gate =
searx_ms p95 improvement at ≤10% extra upstream requests.
**Implementation:** `client.rs` + a config formula. **Acceptance:** measured
p95 cut at bounded duplicate rate. **Kill:** if p95−p50 < 300ms on live
telemetry, the fixed delay is already past the knee — close with the
measurement, change nothing.

---

## 4. D1 — JSD regional-divergence productization  **[PRIORITY: MEDIUM as spec; BLOCKED as code]**

**What exists.** Two-lane JSD over registered-domain distributions, log-2,
bounded [0,1], NaN-free with honest degenerate cases
(`compare.rs:171-194`; domain extraction `compare.rs:137-166`); measured
same-lane noise floor p90 0.30 (n=336 pairs, ADR-22 P7-exit amendment,
`00-adr.md:555-564`); suite 12b: cross-lane JSD mean 0.7127 vs floor 0.096,
bootstrap p≈0 (`bench/2026-06-12-pi5-p8-divergence-gate.json:20-33`,
`p8-gates.md:13-23`). Region lanes have **no metasearch backend** — the
planner refuses (`planner.rs:540-543`); the architecture for regional
vantages is ADR-22 option (b): per-region SearXNG sidecars on WireGuard,
384–512MB each, **Phase-11 and NOT BUDGETED** — explicit budget row + operator
sign-off required (`00-adr.md:541-551`, risk #19). **Flagged: everything below
is design-complete/implementation-blocked until that row exists.**

**Multi-lane generalization (the math to freeze now).** For m lanes with
domain distributions P₁..Pₘ and weights π (uniform default):

```
GJS_π(P₁..Pₘ) = H(Σᵢ πᵢ Pᵢ) − Σᵢ πᵢ H(Pᵢ)   ∈ [0, log₂ m]
```

Ship `gjs_normalized = GJS/log₂ m` as the omnibus statistic, PLUS the
pairwise JSD matrix (m(m−1)/2 entries, each on today's [0,1] scale and
comparable to today's floor) for attribution, PLUS per-lane one-vs-rest
`JSD(Pᵢ ‖ mean of others)` as the "which lane is the outlier" score.
Pairwise-vs-one-vs-rest is not either/or: omnibus answers "do vantages
disagree", one-vs-rest answers "who", pairwise answers "about what" (the
`domains_only_in_*` lists generalize per pair, `compare.rs:39-41`).

**Sparse-counts honesty.** Plug-in JSD on 10–50 results over a large domain
support is biased upward (Miller–Madow-type bias ≈ support/(2n ln 2) per
entropy term). The repo's established answer is empirical, not analytic:
measure the noise floor per configuration (`compare.rs:32-38` — "configured,
not invented"). That discipline must scale: the floor is a function of m and
per-lane result counts, so each lane-set gets its own suite-12-style probe
(same-lane repeats, ≥300 pairs) before `exceeds_floor` means anything.
Add-λ smoothing is an alternative ONLY if the floor is re-measured under the
same smoothing — never mix smoothed statistics with an unsmoothed floor.

**Region-by-topic summaries.** A standing region×topic divergence report
built from *organic* queries would be query-derived persistent state — it
collides with the no-query-logging promise. Clean resolution: topic summaries
are an **operator batch probe** over a published canned query list (the suite
12b shape: 12 region-sensitive queries × repeats, bootstrap CI per topic),
producing population-level claims the per-request block explicitly disclaims
(`compare.rs:35-38`). This also bounds Tor/lane load to scheduled windows.

**Expected gain:** the flagship differentiating capability (no SaaS API
exposes it) extended from 2 to m vantages. **Complexity:** small math, large
ops (sidecars, WG, floors). **Pi-5 cost:** RAM is the binding term (risk
#19); compute is trivial. **Privacy:** per-request unchanged; batch probes
operator-initiated; compare stays opt-in, jittered, fail-closed
(`compare.rs:73-108`). **Required data:** none until Phase-11; then new floor
probes per lane-set. **Evaluation:** suite-12/12b machinery generalized
(`meridian-eval/src/bench/divergence.rs`). **Implementation:** `compare.rs`
(GJS + matrix), `meridian-eval` floors. **Acceptance:** m-lane GJS > measured
m-lane floor at p<0.05 on the probe set. **Kill:** if the m-lane floor
overlaps the cross-lane signal (sparse-count noise swamps vantage signal at
realistic result counts), ship pairwise-only and say so.

---

## 5. D2 — Information-gain value models inside the VoI frame  **[PRIORITY: MEDIUM — one cheap suite arm]**

**Proposal.** Replace the page model's gain term with a non-generative
expected-entropy-reduction estimate, and compare entropy-threshold stopping
against the shipped net-value stopping — judged by the existing suite-15
harness.

**Math.** Let q_c = head score mass per evidence cluster c (clusters exist at
query time, ADR-18). Cluster-coverage entropy `H(q) = −Σ q_c log q_c`.
Candidate i in cluster c(i): fetching re-scores i on full text; model the
expected post-fetch mass shift Δ̂ᵢ = p_i · dcg_headroom(i) into c(i) and define
`gain_IG(i) = H(q) − H(q after Δ̂ᵢ)`. Stopping: stop when `max_i p_i·gain_IG(i)
− cost < h` (h=0 recovers the shipped rule's shape). Honest structural
observation: the shipped novelty-discounted additive gain and the entropy gain
are both monotone concave set functions of the same cluster-coverage vector —
greedy is near-optimal for both, and D2 is a re-parameterization of the
family, not a new theory. The score-distribution-entropy variant (ΔH of
normalized top-k fused scores, NQC-adjacent — `qpp.rs` already owns that
machinery) joins as a second arm.

**Why expected gain is small — said before the experiment.** The frozen page
model is within −0.0023 nDCG of fetch-all (`p9-voi.json:23,29`); the only
winnable axis is fetch savings beyond 30.5% without breaching ±0.01 — and
suite 15b just measured that aggressive gain discounts break exactly that bar
(`p10-voi-embed.md:23-34`). Production impact is further capped by
`deep_fetch_max = 2`. For **answer mode**, information-gain acquisition is
a-priori the wrong objective: the reward is the best passage *in hand*, not
knowledge of which doc is best — Weitzman is exactly optimal for that reward
structure and suite 18 measured the 3.8× gap against the nearest wrong
objective; no entropy arm is proposed there.

**Entropy stopping vs reservation stopping.** The shipped stop is calibrated:
cost and βs are "a unit system, not independent knobs" (`voi.rs:160-163`),
and the honesty payload (`est_gain_remaining`, `voi.rs:119-136`;
`docs/api.md:294-305`) falls out of the model. An entropy threshold h is one
new knob that must be frozen by the same sweep AND must define its own honest
remainder (max remaining p·ΔH) or it regresses the `analysis` block contract.
That asymmetry is the real bar.

**Complexity:** ~50 lines (gain closure + h sweep). **Pi-5 cost:** O(k·C)
per re-pose, negligible; suite runs in ~1s even in debug
(`p9-voi.json:37`). **Privacy:** none (in-RAM head statistics).
**Required data:** none — hermetic. **Evaluation:** the suite-15 harness
takes a gain-model arm exactly as `voi_embed.rs` does (`GainModel` enum,
`voi_embed.rs:215,267-269`); tuning sweep over h, frozen hold-out judgment.
**Implementation:** `meridian-eval/src/bench/voi.rs` arm first; `voi.rs`
only if it wins. **Acceptance:** fetch savings >35% at hold-out nDCG within
±0.01 AND median fetched clusters ≥3 (the risk-#22 diversity guard,
`00-adr.md:652-655`) AND a defined honest remainder. **Kill:** no admissible
point on tuning → RECORDED NO through suite 15, the 15b path verbatim. Worth
running because it is nearly free and permanently closes the
"information-theoretic value model" question with a measured row.

---

## 6. Engaged rejections

- **DPP diversity rerank.** k-DPP MAP is greedy O(k²·n) per step with kernel
  construction O(n²) (and exact sampling O(n³) eigendecomposition); but cost
  is not the real objection. A DPP kernel built from text/embedding similarity
  is symmetric — the exact property that made MMR demote canonical originals
  with their copies (suite 13b mechanism, `p8-gates.md:83-88`). Given cluster
  structure to fix that, the DPP collapses to "one representative per
  cluster, best first" — which is `diversity=evidence`, already shipped and
  **dominant on both alpha-nDCG and plain nDCG** (`01-rebaseline.md:32`,
  `docs/api.md:46`). Nothing left to buy; killed list stays killed.
- **MMR / Pandora-for-page re-proposals.** Killed by suites 13b and 15
  respectively (`01-rebaseline.md:43,45`); re-proposal requires new evidence
  through the same judges (`01-rebaseline.md:38`). None exists; none offered.
- **MCTS fetch planning.** ADR-25/26 a-priori rejection stands and is
  *stronger* post-suite-15: the inspection-cost structure has a provably
  optimal index policy per regime (`00-adr.md:657-659`), and both regimes are
  now measured at or near their objective's optimum. Tree search would burn
  Pi-5 CPU to approximate what closed forms already achieve.
- **MI-based feature selection for LTR.** There is no training data: GBDT→ONNX
  awaits operator data that does not exist (`01-rebaseline.md:21`; WBS 3.1
  train/ scripts are scaffolding, `docs/plan/01-wbs.md:102`); the cold-start
  scorer is hand-tuned linear. MI selection without labels selects nothing.
  Revisit only if the ADR-24 log ever yields a labeled set — different track.
- **Compression-based similarity (NCD).** MinHash sketches own lexical
  derivation (ADR-18, query-time evidence clusters) and potion owns the
  semantic register (15b finding 1). NCD is slower than both and adds no
  third signal class; no row.
- **Information bottleneck for excerpt selection.** A bounded non-neural IB
  needs p(passage, relevance) — the relevance variable is exactly what the CE
  already scores, and a term-distribution surrogate (clarity-style KL per
  passage) is strictly weaker than the CE that powers the measured 0.700
  hit-rate. The one plausible non-neural use — a KL pre-filter to cut CE
  pairs below the cap — is already dominated by the positional cap: 8/9
  winning passages live in the first 8 (`answer-cap-study.md:19-24`), and
  cap 8 won the 3.0s row back. No bounded version worth a row; rejected.

## 7. Verdict table

| # | Candidate | Priority | One-line verdict |
|---|---|---|---|
| B1 | Answer-mode embedding-redundancy pruning | **HIGH** | Evidence-nominated, judge exists, cost ≈0; gain honest-but-bounded by budget 2 |
| D2 | Entropy gain/stopping arm in suite 15 | MEDIUM | Nearly-free experiment; small expected upside; closes the question either way |
| D1 | Multi-lane JSD productization | MEDIUM (spec) / BLOCKED (code) | Freeze the GJS + floor math now; implementation gated on the unbudgeted Phase-11 sidecar row |
| B3 | Quantile-adaptive hedging | LOW | Telemetry first (hedge-fire counter + searx_ms histogram); adapt only if the data says 300ms is mis-set |
| B2 | Non-generative query decomposition | LOW-MEDIUM (conditional) | Run the zero-cost cluster-collapse count first; full candidate only if aspect-deficient pools are real |
