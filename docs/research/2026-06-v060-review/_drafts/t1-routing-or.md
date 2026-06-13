# T1 — Track A (resource-aware routing) + Track J (evaluation & operations research)

Status: DRAFT workpaper for the v0.6.0 research review. As-of: v0.6.0 / 2026-06-13.
Builds on the corrected baseline in `01-rebaseline.md`; the §3 killed list is binding.
Framing rule honored: Track A is **what to do at the ADR-25 verdict (~2026-08-11)** —
no fresh routing design is proposed anywhere in this paper.

## 0. Ground truth from the repo (anchors for everything below)

- **Arms**: K=3 curated engine subsets (`fast`/`broad`/`reference`),
  `crates/meridian-searx/src/bandit.rs:30-43`.
- **ε**: 0.1, set at the single construction site `bins/meridiand/src/main.rs:152`
  (`Bandit::open(&config.index.data_dir, 0.1)`).
- **Propensities** (exact, emitted at choice time): greedy arm
  `(1−ε)+ε/K = 0.9 + 0.1/3 ≈ 0.9333`; each explore arm `ε/K ≈ 0.0333`
  (`bandit.rs:133-138`).
- **Reward**: binary "arm contributed a result to the final top-10"
  (`bandit.rs:1-2`, reward site `bandit.rs:176`; one `reward` byte in the row,
  `decision_log.rs:156`). Rebaseline R2 confirms this is still the live definition.
- **Log row**: 13 bytes, coarse buckets, k-floor K_FLOOR=5, **RETENTION_DAYS=30**,
  ≤20MB cap, wipe path, anon never logged
  (`decision_log.rs:8,29,31-34,144-158,180-226`; ADR-24 `docs/plan/00-adr.md:581-609`).
  Bytes `row[11..13]` are **reserved schema headroom** (`decision_log.rs:158`).
- **Candidate policy**: linear TS, DIM=26 one-hot, λ=1, v²=0.25, 64 MC propensity
  draws floored at 1/64; state is in-memory, **rebuilt at boot by replaying the
  decision log** (`crates/meridian-searx/src/contextual.rs:24-28,37,41-44,132-143,195-215`).
- **Gate**: DR uplift 95% CI excludes zero on ≥10k logged decisions, 60-day sunset
  (~2026-08-11), inconclusive ⇒ ε-greedy retained, log TTLs out — ADR-25
  (`00-adr.md:611-641`); verdict surface `GET /v1/decision-log/ope` with
  pass/inconclusive/negative/insufficient_data (`00-adr.md:638-640`). Current organic
  rows: **0** (`01-rebaseline.md:16`).
- **Estimator anchor**: suite 14 (`docs/plan/bench/2026-06-12-pi5-p9-ope.json:21-30`):
  DR rel. bias 0.74%, **DR replicate sd 0.024 at n=10,000 decisions/replicate**
  (20 replicates), IPS sd 0.0384, under the shipped ε=0.1 logger with the candidate
  disagreeing with the incumbent greedy arm in 3/4 contexts (json:33).
- Estimators: IPS `crates/meridian-searx/src/ope.rs:31-48`; DR with per-(bucket,arm)
  empirical-mean model, prior mean 0.5 (`ope.rs:52-71,90-97`); paired percentile
  bootstrap for the uplift CI (`ope.rs:117-167`).

Hard constraints carried throughout: no query text (risk #11, ADR-24); anon-lane
isolation by construction (`bandit.rs:5-7`, `decision_log.rs:16-18`); `/v1/forget`
untouched (the log holds no doc references); Profile-R budgets
(`docs/plan/02-budgets.md`); rebaseline §3 killed methods stay killed.

---

## A1 — DR-OPE power analysis + organic-traffic accrual plan

### A1.1 Minimum detectable uplift

The gate statistic is Δ = DR(candidate) − incumbent realized mean, paired bootstrap
(`ope.rs:117-167`; the pairing cancels shared sampling noise, `ope.rs:137-139`).
The incumbent-mean term has sd ≤ √(0.25/n) ≈ 0.005 at n=10k and is positively
correlated with the DR term, so sd(Δ) ≤ sd(DR); we use the measured DR replicate
sd as the anchor, conservatively.

**Anchor**: sd(DR; n=10⁴) = 0.024 (measured, suite 14). Scaling sd(n) = 0.024·√(10⁴/n).

- **CI-excludes-zero threshold** (what the gate literally tests):
  1.96 × 0.024 = **0.047** → a true uplift below ~4.7pp of reward rate has <50%
  chance of passing even at n=10k.
- **MDE at 80% power**: (1.96 + 0.84) × 0.024 = **0.067 → 6.7pp at n=10k**.
- At smaller n: MDE₈₀ = 9.5pp (n=5k), **12.3pp (n=3k)**, 21.3pp (n=1k), 27.4pp (n=600).

**Variance decomposition (sanity-checks the anchor and refines it).** For a
deterministic candidate agreeing with the logger's greedy arm on fraction *a* of
rows, the per-row DR variance ≈ a·σ²ᵣ/p²_greedy + (1−a)·σ²ᵣ/(ε/K), with σ²ᵣ the
residual reward variance after the model. The reward is Bernoulli, so
σ²ᵣ = q(1−q) ≤ 0.25; taking σ²ᵣ ≈ 0.2 (q in the 0.6–0.75 band one expects when the
chosen arm contains a major engine):

| Disagreement rate d=1−a | per-row var | sd @10k | MDE₈₀ @10k | MDE₈₀ @3k |
|---|---|---|---|---|
| 0.75 (suite-14 regime) | 4.56 | 0.0213 | 6.0pp | 10.9pp |
| 0.50 | 3.12 | 0.0176 | 4.9pp | 9.0pp |
| 0.25 (realistic candidate) | 1.67 | 0.0129 | 3.6pp | 6.6pp |
| 0.10 | 0.81 | 0.0090 | 2.5pp | 4.6pp |

The d=0.75 row reproduces the measured 0.024 within 12% — the model is trustworthy.
The dominating term is the explore-arm correction (weight 1/(ε/K) = 30); IPS sd
0.0384 vs DR 0.024 (json:24,27) confirms DR's model term absorbs most of it.

**The uplift ceiling makes this bite twice.** True uplift Δ = d·g, where g is the
per-row reward gap on re-routed traffic. To pass at 80% power: d=0.25 needs
g ≥ 3.6pp/0.25 = **14.4pp arm-gap at n=10k** (26pp at n=3k). The context features
are 4 intent classes × coarse buckets (`contextual.rs:33-37`); a 14pp+ per-context
gap between curated engine subsets is possible (e.g., `reference` on lookup
intents) but is a strong effect, not a subtle one. **The gate, as designed, can
only ever certify a large, obvious win — which is the correct bar for an
appliance, and worth stating in the verdict note.**

### A1.2 The accrual arithmetic — and a structural finding

10k decisions in 60 days needs 167/day. But **the 30-day TTL caps what the gate
can ever see**: the OPE input is `read_all()` over retained rows
(`decision_log.rs:247-276`), the TTL sweep drops rows older than 30 days
(`decision_log.rs:180-191`), and the replay tally counts retained rows
(`contextual.rs:132-143`). So n_max = 30 × (decisions/day): **reaching 10k requires
≥334 organic direct-lane decisions/day sustained for the trailing 30 days.** A
single-operator appliance doing 20–100 organic searches/day yields n ∈ [600, 3000]
— MDE₈₀ between 6.6pp (best case d=0.25, n=3k) and 27pp. Risk #23
(`docs/plan/03-risk-register.md:33`, L=4) is therefore not just likely — under the
TTL it is **structural**: no amount of calendar time fixes it at organic rates.
The 60-day sunset and the 30-day TTL interact such that "extend the deadline"
is mathematically useless unless the *rate* rises.

### A1.3 Pre-registered verdict branches (recommend recording these before 2026-08-11)

1. **PASS** (n≥10k, CI_lo > 0): flip `searx.contextual_policy=true`
   (`config.rs:333,343`); amend ADR-25 status; re-run the OPE report on the next
   30-day window with the TS MC propensities as the logging policy (the ADR-25
   status note already requires this re-validation, `00-adr.md:636-639`,
   `contextual.rs:16-22`); keep ε-greedy behind the flag for one release as the
   rollback path.
2. **INCONCLUSIVE** (n≥10k, CI straddles zero — or the ADR's own clause fires:
   MDE exceeds the CI width, `00-adr.md:618-619`): execute ADR-25 verbatim —
   ε-greedy retained, log TTLs out, recorded in the exit note. Keep the dark
   module (cost <1MB, `02-budgets.md:144`); re-review **only** if A2's graded
   reward lands (a genuinely new gate, not a re-roll of the same one).
3. **NEGATIVE** (CI_hi < 0): ε-greedy retained; mark ADR-25
   REFUTED-for-this-traffic; re-proposal requires a different featurization AND
   new evidence through suite 14 — same re-proposal discipline as rebaseline §3.
4. **INSUFFICIENT_DATA** (n<10k — the near-certain branch): apply the
   extend-vs-sunset rule below.

### A1.4 A principled extend-vs-sunset rule

Extension is justified only if it can change the answer. Because n is TTL-capped
at 30·rate, the only quantity that matters is the trailing organic rate:

> **Extend once (60 days) iff the trailing-14-day direct-lane decision rate
> ≥ 334/day** (the rate that saturates the TTL window at 10k). **Otherwise
> sunset**: declare inconclusive per ADR-25, ε-greedy retained, log TTLs out,
> module stays dark. No second extension.

Do **not** generate synthetic traffic to feed the gate: rewards from replayed or
scripted queries measure the generator, not the operator (the risk-#21 pathology,
`03-risk-register.md:31`), and "organic" is load-bearing in the ADR-25 record.

If the operator wants the gate to be *winnable* at organic rates, the honest
mechanism is **sufficient-statistic retention** (P2, flagged for ADR-24
re-sign-off): retain per-(context-bucket, arm, day) aggregates
(n, Σr, Σr/p, Σ(r/p)²) beyond the row TTL. IPS/DR point estimates are linear in
rows, so aggregates reconstruct them exactly; the percentile bootstrap is replaced
by a delta-method normal CI from the retained second moments. Privacy surface:
k-floored aggregates are strictly weaker than the rows they summarize, but the
ADR-24 field list is operator-signed (`decision_log.rs:41`, `00-adr.md:583-593`)
— this is a template amendment, not a quiet change. A cumulative u64 decision
counter (so "≥10k" can at least be *measured* across TTL windows) is the minimal
version of the same amendment.

---

## A2 — Graded reward shaping

The binary reward wastes information the planner already has at the reward site
(it computes "appeared in top-10" *from* the per-result source attribution). Three
candidates, all r ∈ [0,1], all estimator-compatible — `Logged.reward` is already
f64 (`ope.rs:25`), DR for bounded real rewards is the standard Dudík et al. setting
(`ope.rs:11-15`), and the Beta-style model prior (`ope.rs:96`) is well-defined on
[0,1]:

| Candidate | Definition | Estimator compat. | Schema change | Variance reduction |
|---|---|---|---|---|
| **(a) Marginal contribution** | r = c/10, c = arm-sourced results in final top-10 (marginal vs the local-only lane — only one arm is queried per request, so "beyond other arms" operationalizes as "beyond local") | DR unchanged | 1 byte: c in reserved `row[11]` (`decision_log.rs:158`) — **no row growth, no migration** | σ²: 0.2 → ~0.02–0.05 if c concentrates on {1..4}; sd ∝ √σ² ⇒ MDE shrinks ~2× |
| **(b) Unique-domain contribution** | r = (unique domains the arm added to top-10)/10 | DR unchanged | same 1 byte | similar to (a) |
| **(c) Rank-weighted (recommended)** | r = Σ_{i∈arm hits} w_i / Σ_{i=1..10} w_i, w_i = 1/log₂(i+1), quantized to u8 (error ≤0.4%, negligible vs sd 0.013+) | DR unchanged | same 1 byte | best: distinguishes rank-1 from rank-10 contributions; expected ≥2× sd reduction |

All three force the **ADR-24 re-sign-off**: the row's field list is
operator-approved verbatim (`00-adr.md:583-587`); populating `row[11]` changes the
list even though the value is derived purely from result ranks (no query content,
no URLs — privacy delta ≈ nil, procedure delta = one ADR amendment). (b) is scored
lower because it shifts the estimand from relevance toward diversity, muddying the
single question ADR-25 asks. A latency-penalized reward was considered and dropped:
the shed ladder already owns the cost axis as hard constraints
(ADR-25's own argument, `00-adr.md:622-625`).

**Deployment shape that avoids new exploration risk**: keep the live ε-greedy on
the binary reward (no behavior change, `bandit.rs` untouched in semantics), log
*both* rewards (binary `row[10]`, graded `row[11]`), and pre-register: after the
first 500 organic rows, if var(graded)/var(binary) < 0.5, the graded reward
becomes the gate estimand (a new gate, justifying branch-2 re-review). With a 2×
sd reduction, MDE₈₀ at the realistic n=3k drops from 6.6–12.3pp to **3.3–6.2pp**
— it does not rescue the 10k bar alone, but it is the single highest-leverage
change against risk #23, and it must land **before** accrual starts to be of any
use (rows are immutable once written).

---

## A3 — Non-stationarity guard

Risk #12 (`03-risk-register.md:22`) claims "bandit demotes failing engines
automatically." With lifetime counts — (pulls, reward_sum) packed and persisted
forever (`bandit.rs:13-14,46-49,176-202`) — that claim is weak: an arm with n
historical pulls and mean μ₁ needs O(n·(μ₁−μ₂)/μ₂) new zero-reward pulls to fall
below the runner-up. At n=10k and organic rates, that is months. The TS candidate
has the same issue within a process lifetime (A, b accumulate,
`contextual.rs:76-86,145-150`) — but **not across boots**: state is rebuilt by
replaying the decision log (`contextual.rs:24-28`), and the log TTLs at 30 days —
the sliding window already exists at every restart.

Compared options:

1. **Discounted linear TS** (A ← γA + xxᵀ, N_eff = 1/(1−γ)): same O(d²) update
   cost, but introduces a new frozen constant γ, which under the repo's
   experiment-first rule (`01-wbs.md:253-254`) requires its own sweep + suite
   before it may ship.
2. **Sliding window via periodic log re-replay (PICK)**: every 256 inserts (the
   existing sweep cadence, `decision_log.rs:171-174`), rebuild the posterior with
   the existing boot path `from_decisions`. Cost: worst-case ~30k rows × DIM²=676
   mul-adds ≈ 20M flops ≈ <10ms on one A76 core, amortized to noise. **Zero new
   constants** (the window *is* the already-signed-off 30-day TTL), zero schema
   change, deterministic, persistence story unchanged.
3. **Restart-on-changepoint reusing the ADR-28 two-state Viterbi** over per-arm
   daily reward means: decode cost is trivially small (the whole suite-17 sweep
   ran <0.1s, `02-budgets.md:147`), but (s, γ) were frozen for *count* series via
   suite 17 — reward-rate series would need re-derivation through a new suite; it
   adds per-arm daily-series state and a meridian-analytics→meridian-searx
   coupling; and a false changepoint hard-resets learning. Machinery in search of
   a consumer — the same ground BOCPD was rejected on (rebaseline §3:48).

**Decision: option 2**, plus a two-line exponential-forgetting cap for the live
ε-greedy (when pulls ≥ 2000, halve pulls and reward_sum at the reward site) so the
demotion time-constant for risk #12 becomes explicit and bounded instead of
growing with history. Both are Pi-free, constant-free (the halving threshold is a
cap, not a tuned constant — any value ≥ a few hundred works), and neither touches
the propensity contract: choice-time propensities remain exact and are logged
as-is.

---

## J1 — Knob-level Pareto-frontier synthesis (from data already on disk)

Knobs with **measured curves** today:

| Knob (default) | Measured points | Source |
|---|---|---|
| `answer_passage_cap` (8, `config.rs:306`) | cap 16 → p50 3196/3224ms; cap 8 → 2502ms; winning-passage positions [0,0,1,1,1,1,2,7,11]: 8/9 within first 8 (n=9, honesty-flagged) | `bench/2026-06-13-pi5-answer-cap-study.md:10-25` |
| `expansion_search` / ef (R: 64, F: 128) | @1M: ef 64→recall 0.94, **128→0.98 (p50 0.78ms/p99 1.73ms)**, 192→0.99, 256→1.00; @100k: 64→0.98 (0.45/0.95ms), 128→1.00 (p99 1.45ms) | `bench/2026-06-13-pi5-p10-ann-1m.md:11-14,26-30`; `02-budgets.md:83-85` |
| `deep_fetch_max` / VoI budget (2, `config.rs:303`) | replay: fetch-all 10 fetches → nDCG 0.7434; VoI 6.95 → 0.7411 (−30.5% fetches); rank-greedy → 0.7058; suite 15b: deeper savings (to 41%) always quality-negative | `bench/2026-06-12-pi5-p9-voi.json`; rebaseline §3:44 |
| rerank depth/batch | **single point only**: top-20 @ batch-4 → deep p50 204ms | `docs/plan/phase-exits/p3.md:7`; knob named in risk #8 (`03-risk-register.md:18`) |

Proposal: one operator-facing frontier table (in `02-budgets.md` or the operator
manual) normalizing each knob to (quality, latency) pairs and marking dominated
settings — e.g., ef=64 at 1M is **dominated** (−4pp recall to save 0.95ms p99
against a 40ms gate). Gaps to fill with bench-only sweeps (no production code):
rerank depth {10,20,40} × batch {4,8}; answer cap 12 (the position telemetry
suggests the knee is at ~8, but n=9); `deep_fetch_deadline_ms` (1200,
`config.rs:304`, never swept). This is documentation + bench work; it also gives
J2 and any future shed-ladder tuning their cost curves.

## J2 — Deadline/admission control for mixed query classes

Current concurrency contract: tokio 4 IO workers, one rayon pool ×4 @ nice 5,
per-query concurrency 8 (anon separate budget 2) (`02-budgets.md:88-90`;
`concurrency_limit: 8`, `config.rs:61,79`). The shed ladder is resource-triggered
and class-blind (`shed.rs:6-12,46-74`). Classes and budgets: fast ≤25ms working
target, deep ≤2.5s, answer ≤3.0s (`02-budgets.md:74-77,148`). Failure mode: 5+
concurrent deep/answer requests hold permits for seconds and head-of-line-block
fast queries at admission — a realistic single-operator-plus-guests burst even
though >10 QPS is out of scope (`02-budgets.md:93-94`).

**Policy (two atomic counters, no per-user state):** admission state
(c_fast, c_heavy). Admit fast iff c_total < 8. Admit heavy (deep/answer) iff
c_heavy < 4 AND c_total < 8; else serve the fast-path result with
`degraded: ["admission_shed"]` (the existing degraded idiom). H=4 is **anchored,
not tuned**: a 5th concurrent CE rerank only queues on the 4-thread rayon pool —
admitting it adds latency without throughput. Compared and rejected: EDF
(per-request deadline state for marginal benefit at ≤10 QPS), utilization-derived
dynamic limits (a model where a constant suffices). The shed ladder is untouched
— it remains the resource backstop; this is the workload-mix gate in front of it.

Evaluation: **new suite 19 `admission`** in the suite-10 margin/no-collapse idiom
(rebaseline S4:33): tuning mix (6 fast + 2 heavy closed-loop) must show fast p99
≤ 2× fast-solo p99 (margin), and a held-out heavier mix (4+4) must show no
collapse (≤4×); heavy completion rate non-degrading vs the plain cap. Precondition
(cheap, do first): class-split stage-latency gauges in `/metrics`; if measured
mixes never inflate fast p99, **do not ship** — record as designed-but-deferred.

## J3 — Suite power analysis (the gate idiom, quantified — feeds A1)

For the existing gate types:

- **Proportion gates** (suite-18 style): MDE₈₀ ≈ 2.8·√(2p̄(1−p̄)/n). At p̄=0.65,
  n=1000: **6.0pp** — suite 18's observed +10.7pp (rebaseline S5:34) was
  adequately powered. To certify a 3pp effect: n ≈ 3,970.
- **Paired nDCG gates** (suite-15's ±0.01 bar): with paired per-query sd ≈ 0.1,
  MDE₈₀ = 2.8·0.1/√n ⇒ the ±0.01 bar needs **n ≥ ~780 paired queries**; 1000-query
  suites are right-sized, 200-query suites can only honestly claim a ±0.02 bar.
- **Latency-median arms at n=16** (the probe idiom): replicate medians 3196/3224ms
  imply run-level sd ≲ 30ms ⇒ deltas ≳ ~100ms are detectable; the answer-cap study's
  694ms delta was ~20σ. Gates targeting <50ms deltas at n=16 are underpowered.
- **The OPE gate** is the proportion case inflated by the IPS correction factor
  (A1's table: ×1.15 to ×30 per-row variance depending on disagreement rate) —
  the bridge that makes A1's MDE the same arithmetic as every other gate.

Process change (cheap, P1): every bench gate pre-registers its MDE in one line of
the bench doc; an observed margin below the gate's MDE is recorded
**"underpowered"**, never "pass". This is the same honesty discipline as suite
13c/15's tuning/hold-out idiom, applied to sample size.

---

## Rejections (engaged, then rejected)

- **Neural routers / MCTS / bandits-with-knapsacks**: rejected a-priori in ADR-25
  (`00-adr.md:622-625`) — no training data, no GPU, unexplainable; NP-hard; shed
  ladder already hard-gates the action set. Nothing new weakens that record; the
  data regime is *worse* than ADR-25 assumed (0 organic rows, A1.2's TTL cap), and
  the newly-documented Hailo-8L (rebaseline §4:53-64) changes nothing: routing is
  data-bound, not compute-bound, and the DFC toolchain is x86-only anyway.
  **Stays killed.**
- **M/G/1 queueing optimization**: arrivals are single-operator bursty (not
  Poisson), service times are a heavy bimodal mixture (5ms fast vs 2500ms answer),
  and the system is closed-loop behind hard caps (concurrency 8, 1 req/2s
  per-domain, `02-budgets.md:88-97`) — every M/G/1 assumption fails, and fitting
  its parameters needs the traffic that risk #23 says doesn't exist. J2's static
  reservation captures the practical benefit with zero model risk. **Reject.**
- **Simulation-based capacity planning**: the repo's protocol is
  measure-on-device (risk #8's bench-first rule, `03-risk-register.md:18`); both
  profiles are now measured at their scales (100k and 1M, `02-budgets.md:117-126`,
  ann-1m). A simulator would be calibrated against the same measurements it
  replaces, then rot. **Reject.**
- **RL for scheduling**: per-user state is forbidden, no scheduling reward exists
  in telemetry, the action space is two counters, and an unauditable scheduler
  contradicts the appliance's explainability stance (the ADR-25 rejection rationale
  transfers verbatim). **Reject.**

---

## Candidate table (mandated row schema)

| Proposal | Meridian problem | Math formulation | Expected gain | Complexity (time/space) | Pi-5 cost | Privacy impact | Required data | Evaluation method | Implementation location | Priority | Acceptance / kill |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **A1 Verdict protocol + extend-vs-sunset rule** | ADR-25 verdict (~2026-08-11) lands on an unpowered gate; risk #23 structural under the 30d TTL (n_max=30·rate) | MDE₈₀ = 2.8·0.024·√(10⁴/n); 6.7pp @10k, 12.3pp @3k; extend iff trailing-14d rate ≥334/day | An honest, pre-registered verdict; no limbo | O(1) / one doc + optional u64 counter | nil | counter = ADR-24 text amendment (no per-row info) | existing log + suite-14 anchor | the existing `/v1/decision-log/ope` report (`00-adr.md:638-640`) | `docs/plan/00-adr.md` ADR-25 amendment; optionally `decision_log.rs` counter | **P0** | Accept: branches recorded before 2026-08-11. Kill: n/a (process) |
| **A2 Rank-weighted graded reward (log-both)** | Binary reward wastes rank information; MDE too high at organic n | r = Σ w_i/Σ₁₀ w_i, w_i=1/log₂(i+1), u8-quantized; DR unchanged on r∈[0,1] | ~2× sd reduction ⇒ MDE₈₀ @3k: 12.3→~6pp | O(1)/row; 0 extra bytes (reserved `row[11]`, `decision_log.rs:158`) | ~ns at reward site | derived from ranks only, BUT field-list change ⇒ **ADR-24 re-sign-off** | existing telemetry + 1 derived byte | pre-registered: var ratio <0.5 on first 500 organic rows; suite-14 re-run with graded synthetic truth | `meridian-searx/src/{bandit,decision_log}.rs` reward site; `ope.rs` untouched | **P1** (must precede accrual) | Accept: var ratio <0.5 AND suite-14 bias gate holds. Kill: ratio ≥0.5 ⇒ binary stands |
| **A3 Sliding-window posterior via log re-replay + ε-greedy count-halving** | Engine drift (risk #12) vs lifetime counts; "auto-demotes" claim weak | re-run `from_decisions` every 256 inserts (window=TTL); halve (pulls, Σr) at pulls≥2000 | drift response bounded by 30d window / ~1k pulls instead of unbounded history | O(n_rows·d²) ≈ 20M flops per 256 inserts / 0 extra state | <10ms amortized, 1 core | none (no new data) | existing log | new micro-suite: simulated engine-break replay, demotion-lag gate (margin idiom) vs lifetime-count baseline | `contextual.rs` (replay hook), `bandit.rs:176` (halving) | **P2** | Accept: demotion lag ≤ ½ baseline, zero propensity change. Kill: any propensity drift vs logged values |
| **A2-ext Sufficient-statistic retention (enables a winnable gate)** | TTL caps n at 30·rate forever | retain (n,Σr,Σr/p,Σ(r/p)²) per (bucket,arm,day); delta-method CI replaces bootstrap | gate horizon decoupled from TTL | O(#cells) ≈ KB / negligible | nil | aggregates < rows, but **new retention surface ⇒ explicit ADR-24 sign-off** | aggregate of existing telemetry | suite 14 extended: aggregate-CI vs bootstrap-CI agreement on synthetic truth | `decision_log.rs` + `ope.rs` | **P2** (only if operator wants the gate winnable) | Accept: CI agreement ±10% + sign-off. Kill: operator declines sign-off ⇒ sunset stands |
| **J1 Pareto-frontier synthesis + gap sweeps** | Measured knob curves scattered across bench files; one knob (rerank depth) has no curve | per-knob (quality, latency) frontier; dominance marking | operator-visible tradeoffs; ef=64@1M flagged dominated | doc work / bench-only sweeps | bench time only | none | all on disk (see J1 table) | the existing suites that produced each curve | `02-budgets.md` or operator manual; `meridian-eval` sweeps | **P1** (synthesis) / P2 (sweeps) | Accept: table cites a measured source per cell. Kill: n/a |
| **J2 Class-aware admission (heavy-cap 4)** | deep/answer (2.5–3s holds) head-of-line-block fast queries within the class-blind cap 8 | admit fast iff c_tot<8; heavy iff c_heavy<4 ∧ c_tot<8; H=4 anchored to rayon ×4 | bounded fast p99 under mix | O(1) / 2 atomics | nil | none (class counts only — no per-user state) | class-split latency gauges (add first) | **new suite 19 `admission`**: margin (mix fast-p99 ≤2× solo) + no-collapse (held-out mix ≤4×) + heavy completion non-degrading | `meridian-api` admission layer; `shed.rs` untouched | **P2** (gauges first; ship only on measured inflation) | Accept: suite-19 gates + zero fast-path change unmixed. Kill: gauges show no inflation ⇒ defer |
| **J3 MDE pre-registration in every bench gate** | gates can "pass" on noise below their detectable effect | MDE₈₀: proportions 2.8√(2p̄(1−p̄)/n); paired nDCG 2.8σ/√n; medians 2.8√2·1.253σ/√n | prevents noise-mining; sizes future suites (feeds A1) | one line per bench doc | nil | none | existing replicate data | self-applying (a doc rule) | `docs/plan/04-bench-plan.md` §6 idiom | **P1** | Accept: every new gate states its MDE; margin<MDE ⇒ recorded "underpowered". Kill: n/a |

## Implications for the top-5 bet shortlist

1. The **A1 TTL×accrual finding** (10k needs ≥334/day; organic reality 600–3k rows
   ⇒ MDE₈₀ 6.6–27pp) should demote any shortlist bet that *assumes* the contextual
   policy ships — the modal 2026-08-11 outcome is `insufficient_data` → sunset.
2. **A2 (graded reward)** is the only sub-week change that materially improves the
   gate's physics, and it is time-critical: rows are immutable, so it pays only if
   it lands before accrual. If routing keeps a shortlist slot, this is the slot.
3. **J3/J1** are near-free process bets with cross-track payoff (every future
   suite, every knob discussion) and no privacy or budget surface.
