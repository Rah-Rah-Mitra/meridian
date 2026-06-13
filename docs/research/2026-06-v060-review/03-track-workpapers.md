# 03 — Track workpapers (A–K)

Status: REVIEW — proposes, does not decide. As-of: v0.6.0 / `812d3d4` / 2026-06-13.

Full per-track analyses: answered-by-repo-evidence / open questions / candidates
(with math, Pi-5 costs, judge suites, acceptance AND kill criteria) / rejections.
Every repo claim cites file:line at the pinned commit. The distilled opportunity
matrix and top-five bets live in `00-review.md` §5–§6.

- Tracks A (routing) + J (eval/OR): T1 below
- Tracks B (parallel finding) + D (information theory): T2
- Tracks C (evidence graphs) + H (uncertainty): T3
- Tracks E (geo) + F (streaming/deletion) + G (spectral/RMT) + I (privacy): T4
- Track K (Hailo-8L NPU): T5

---

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


---

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


---

# T3 — Tracks C (evidence & source graphs) and H (uncertainty & trustworthy ranking)

Status: DRAFT workpaper. Baseline: `01-rebaseline.md` (corrected, v0.6.0 /
2026-06-13). Hard constraints honored throughout: no query logging (ADR-24
scope, `00-adr.md:581-597`), anon isolation (planner reward site is
direct-branch-only by construction, `planner.rs:1380-1389`), forget
provability for every new structure (ADR-19, `00-adr.md:467-483`), Profile-R
budgets (`02-budgets.md` §3, §7), and the killed list (`01-rebaseline.md` §3)
stays killed.

## 0. Two load-bearing facts established first

**F1 — extraction does NOT retain hyperlinks (gates C3).** The extraction
layer keeps exactly two fields: `Extracted { title, text }`
(`crates/meridian-fetch/src/extract.rs:5-9`); `extract_html` returns
`article.text_content` from dom_smoothie and nothing else
(`extract.rs:17-28`), and the module doc records the hard rule: "the raw HTML
DISCARDED by the caller (SPEC §6.1 hard rule — content hash only)"
(`extract.rs:1-3`). No anchor, href, or outlink survives anywhere in the
ingest path (`grep -ri "href\|outlink"` over `crates/` finds nothing
retained). C3 therefore requires a schema change, costed in §C3.

**F2 — an ONNX runtime DOES exist in-tree (sharpens the NLI rejection).**
The prompt's premise "no NLI-capable runtime exists" is not literally true:
`ort` ships as the optional `ort-backend` feature of `meridian-rerank`
(`crates/meridian-rerank/Cargo.toml:13-21`, gnu image only per ADR-02) and
runs the INT8 CE today. What does not exist is any NLI *model*, tokenizer
config, or eval corpus (`grep -rin nli` over crates+docs: zero hits). The
signed-graph rejection in §R1 therefore rests on model/cost/eval grounds,
not runtime absence — stated honestly so it cannot be "refuted" by pointing
at `ort`.

---

## Track C — evidence & source graphs

### C1. Corroboration scoring for answer mode

| Field | Content |
|---|---|
| **Proposal** | After `best_passage` selection, score whether documents from *distinct* ADR-18 evidence clusters independently support the winning passage; emit an additive `corroboration` block (ADR-20 pattern, own `schema`). |
| **Meridian problem** | `best_passage` cites ONE page (`docs/api.md:307-331`); api.md itself warns "a confidently relevant passage can still be wrong, and the engine cannot tell" (`api.md:323-326`, risk #26). The existing `independent_source_count` (`evidence.rs:37`, `api.md:138-149`) is **page-set-level**: it says how many apparent origins are in the result list for the *query*. It says nothing about whether any second origin supports the *claim in the winning passage*. Claim-level vs page-set-level is the gap. |
| **Math formulation** | Let `w` be the winning passage, `C(w)` the ADR-18 cluster of its source page. Candidates: (a) other docs fetched this request — sketches already computed in-RAM at `planner.rs:1180-1182` (`fetched_sketches`), per-doc top-passage CE already retained in `doc_ce` (`planner.rs:1231`); (b) ingested result-set docs — sketches already loaded by `reader.get_many` for the evidence block (`planner.rs:1349-1356`). Two corroboration tests, cheapest first: **(i) textual**: compute `Sketch::compute(w)` (one 500-char passage, µs) and MinHash-containment `c(w, d) ≥ τ` vs each candidate doc sketch (ADR-18 containment estimator with `MIN_MATCH_BINS=5`, `00-adr.md:459-465`) — near-verbatim support; **(ii) relevance**: for fetched docs, top-passage CE `s_d` within `δ` of `w`'s `ce_score` (scores already in hand — zero new CE). Then `independent_clusters = |{ cluster(d) : test(i)∨(ii), cluster(d) ≠ C(w) }|` — **same-cluster copies never count** (syndication is exactly what ADR-18 exists to discount; counting a copy as corroboration would re-import the suite-9 baseline failure, F1 0.054, `evidence.rs:6-8`). Emit `corroboration: { schema, independent_clusters, supporting_urls, basis }` where `basis` states how many candidates were checkable (the `sketched_results` honesty idiom, `evidence.rs:39-41`). |
| **Expected gain** | A user-facing, claim-level trust signal on the highest-stakes block the engine ships; suite-18-style planted truth should show corroborated answers carry a measurably higher hit-rate than uncorroborated ones (that conditional gap IS the deliverable). |
| **Complexity** | Low-medium. ~150 LoC: in-RAM `cluster_tags` over fetched+ingested sketches (the pure function at `evidence.rs:71` already takes any `HashMap<u64, Sketch>`), one passage sketch, containment loop, block plumbing. Note the gap it incidentally closes: `fetched_sketches` are currently used only for VoI novelty (`planner.rs:1092`) and never join the response evidence block. |
| **Pi-5 cost** | Variant (i)+(ii): **≪1ms** added (µs-scale sketch + ≤50 containments; CE scores reused). Optional variant (iii) — CE-score `w` against other clusters' top passages for paraphrase support — adds ~10ms/pair (CE ≈ 204ms @ top-20/batch-4, `02-budgets.md:119`) × ≤`deep_fetch_max` pairs (default 2, `config.rs:303`) ≈ 10-20ms. Against the answer row: p50 ≤3.0s, measured 2502ms @ cap 8 (`02-budgets.md:148`) → ~500ms headroom; even variant (iii) consumes <5% of it. |
| **Privacy impact** | None new: pure local computation over text already in RAM/index; no extra egress (answer mode rides the fetch_budget ladder byte-for-byte, ADR-29 `00-adr.md:797-798`); block contains URLs already in the response. |
| **Required data** | Nothing new at serve time. Eval: suite-18 corpus extension (below). |
| **Evaluation** | **Suite-18 harness extension** (`meridian-eval/src/bench/answer.rs`): plant, alongside each decisive original, (a) genuinely independent second originals carrying the same answer in different words (true corroboration), (b) syndicated copies repeating it (the trap — must NOT count), (c) uncorroborated singletons. Gates: corroboration-label precision ≥0.9 / recall ≥0.6 on the hold-out generator variant (risk-#21 discipline), AND hit-rate(corroborated) − hit-rate(uncorroborated) > 0 with a planted-truth margin. Latency: answer p50 row must hold (re-measure, n≥16 per the probes idiom). |
| **Implementation location** | `crates/meridian-query/src/planner.rs` (answer branch, ~line 1184-1270), `crates/meridian-query/src/evidence.rs` (reuse `cluster_tags`), `meridian-fetch/src/passage.rs` unchanged, `docs/api.md` best_passage section. |
| **Priority** | **HIGH** — it strengthens the engine's most distinctive and most risk-flagged block at near-zero marginal cost, and the judge already exists. |
| **Acceptance / kill** | Accept: both gates above + budget row holds. **Kill:** if planted-truth precision <0.9 on either generator variant (a false "2 independent sources agree" badge is worse than no badge — the conformal lesson at the claim level), or if same-cluster leakage is detected at all (count a copy once → withdraw, fix, re-run). |

### C2. HITS vs weighted PageRank vs raw frequency on the GDELT co-occurrence graph

| Field | Content |
|---|---|
| **Proposal** | A CI-only operator study: is petgraph PageRank even the right centrality on this graph, vs weighted degree (raw co-reporting frequency) and eigenvector/HITS — plus an explicit popularity-bias audit of all three. |
| **Meridian problem** | `domain_prior` = PageRank(d=0.85, 30 iters) over an **undirected** co-occurrence graph (`graph.rs:7,19,31`; UnGraph) built from ≤30-domain cliques per (slice, event-root) bucket (`gdelt.rs:18-19`), weight-1 edges dropped (`graph.rs:11,21`). R7 stands: this is not a hyperlink graph (`01-rebaseline.md:22`). |
| **Math formulation** | On a connected undirected graph, the PageRank stationary vector is a teleport-smoothed weighted degree: π ≈ (1−d)/N + d·(deg_w(v)/Σ_u deg_w(u)) — exactly degree centrality at d=1, and at d=0.85 dominated by it. HITS on an undirected graph degenerates: hubs = authorities = the principal eigenvector of A (eigenvector centrality). So the three "alternatives" are *a priori* near-collinear here; the study's real output is (a) the measured Spearman ρ between the three rankings on a real GDELT day (prediction: ρ > 0.9 — if confirmed, replace the 30-iteration power method with an O(E) degree pass and record why), and (b) the **popularity-bias audit**: Lorenz/Gini of prior mass over domains; a planted-minority synthetic (small regional clique attached to a global hub component) measuring the rank of regional domains under each operator and under two mitigations — log-damped prior `log(1+deg)/log(1+max)` and within-component max-normalization. Co-occurrence cliques structurally guarantee wire services and global outlets co-occur with everything, so ANY raw centrality drowns minority/local sources; the review must say this out loud: if `domain_prior` ever gets nonzero weight, it is a popularity prior, not a quality prior. |
| **Expected gain** | None user-visible today — **the feature has LTR weight 0** (`meridian-rank/src/lib.rs:105` `w_domain`, default zero; RRF-identity cold start, `lib.rs:52-64`; ADR-10 P5 amendment per `01-rebaseline.md:21`). The gain is future-GBDT food: when training data exists, the feature that enters the model should be the audited, bias-mitigated variant, with the operator choice justified by measurement rather than by "PageRank sounds right". |
| **Complexity** | Low (~1 day): the alternatives are one-liners next to `compute_priors` (`graph.rs:17-37`); the synthetic generator is the suite-9/10 idiom. |
| **Pi-5 cost** | Nightly batch only (`graph.rs:1-5`), never query-time; degree pass is strictly cheaper than 30 power iterations on ≤200k edges. Zero serving cost. |
| **Privacy impact** | None — GDELT-derived aggregates, forget-orthogonal by construction (ADR-19 carve-out, `00-adr.md:472-475`). |
| **Required data** | One archived GDELT day (already pulled nightly) + synthetic planted-minority graphs. |
| **Evaluation** | New CI micro-suite (suite-19 candidate, `meridian-eval`): ρ matrix + Gini + planted-minority rank table, recorded like the suite-10 constant-fixing runs. No ship gate — the consumer doesn't exist. |
| **Implementation location** | `crates/meridian-analytics/src/graph.rs`, `meridian-eval` bench subcommand. |
| **Priority** | **LOW** (do the audit before GBDT training lands, not before). Priority rises to MEDIUM the day a trained LTR gives `w_domain ≠ 0`. |
| **Acceptance / kill** | Accept (as a recorded study): ρ matrix + bias audit committed to `docs/plan/bench/`. Kill criterion for the *feature*: if no mitigation keeps planted regional domains above a floor rank while preserving hub ordering, `domain_prior` should stay weight-0 permanently and the ADR should say so. |

### C3. Ingest-time citation/outlink graph

| Field | Content |
|---|---|
| **Proposal** | Retain outlinks at extraction time and build a bounded, ingest-only, directed citation graph: per-doc outlink rows → domain-level citation edges → citation-chain detection ("many apparent citations, one original source"). No query-time egress, no crawl. |
| **Meridian problem** | R7 is still true (`01-rebaseline.md:22`): the only source graph is co-occurrence (C2), which cannot see *who cites whom*. ADR-18 sees textual derivation (copies); a hyperlink graph sees **attributed** derivation — ten differently-worded articles all linking to one primary source are independent texts (ADR-18 says 10 clusters) but one citation origin. The two signals compose: claim-level independence for C1 should eventually require distinct clusters AND no common citation root. |
| **Math formulation (schema change, since F1 says links are not retained)** | (1) Extraction: dom_smoothie's `Article` exposes the readable subtree (`content`); extend `Extracted` with `outlinks: Vec<u64>` — registered-domain hashes (same `url_key`-style hashing as `lexical.rs`) of `<a href>` targets *inside the readable subtree only* (chrome links die with the chrome), deduped, capped at 64/doc. (2) Storage: new `links_v1` table in `dedup.redb`, written **in the same transaction** as `sketch_v1` (the ADR-18 pattern, `00-adr.md:425-428`) — one row per doc, ≤ 8B×64 = 512B/doc gross (vs sketch 123B/doc measured at P7 exit, `02-budgets.md:123`; this is 4× the sketch — record the ceiling honestly). (3) Graph: nightly fold of surviving rows into domain-level directed edges (src-domain → dst-domain, weight = #docs), bounded by the same 200k-edge cap idiom as the co-occurrence graph. (4) Chain detection: for a result set, `citation_root(d)` = the dst-domain receiving links from ≥m distinct clusters; flag `clusters_citing_common_root`. |
| **Deletion (ADR-19 — the design requirement, not an afterthought)** | Per-doc `links_v1` row: **option (a)** — joins the atomic forget transaction, delete-by-key alongside index/vector/dedup/sketch (`00-adr.md:469-472`); one row per doc keeps the forget transaction's row count flat. Domain-level aggregate: **option (b)** — provably rebuildable from surviving rows on the nightly schedule; a forgotten doc's edges are gone after the next recompute, and the hermetic forget test (risk #17 tripwire) extends to assert the row is gone immediately and the aggregate after one rebuild. Edges die with their doc — by construction, not by sweep. |
| **Expected gain** | New capability, not a metric bump: citation-chain provenance in the evidence block; a future directed-graph prior (C2's audit applies); a second independence axis for C1. |
| **Complexity** | Medium: extraction change + re-ingest requirement (existing corpus lacks links until re-ingested — the exact pre-v0.2.0-sketch precedent, `api.md:149`), new table, nightly fold, forget-test extension. |
| **Pi-5 cost** | Ingest-time only: DOM is already parsed (extraction is "~ms for typical pages", `extract.rs:15-16`), so link harvesting is ~free CPU; +≤512B/doc disk (SD-endurance datum, risk #7 — at 100k docs ≤51MB, acceptable Profile-R); nightly fold is O(rows). Zero fast-path latency. |
| **Privacy impact** | Outlinks are document content, not user data; no new egress class (links are *recorded*, never *followed* — following them would be crawl, rejected in §R2). Forget story above. |
| **Required data** | Re-ingested corpus. Eval: suite-9 `synfarm` generator extension planting citation structures (1 origin + N citers with varied anchor placement + chrome-link distractors). |
| **Evaluation** | Extended suite 9 (CI): chain-detection F1 >0.8 with false-root <5% on both generator variants (the ADR-18 gate shape, `04-bench-plan.md:76`); hermetic forget test green incl. `links_v1`; ingest throughput non-regression (≥50 docs/s gate). |
| **Implementation location** | `meridian-fetch/src/extract.rs`, `meridian-index` (new module beside `sketch.rs`), ingest path in `meridian-query/src/ingest.rs`, nightly job beside `meridian-analytics/src/graph.rs`. |
| **Priority** | **MEDIUM** — the only Track-C item that adds a genuinely new signal class, but it pays off in proportion to corpus citation density, which is unknown until measured; do the extraction+storage change early (it is the unrecoverable part — links discarded today are gone), defer the analytics until density is measured. |
| **Acceptance / kill** | Accept: suite-9-ext gates + forget proof + throughput hold. **Kill:** if measured intra-corpus citation density on a real operator corpus is <0.05 edges/doc after re-ingest, record the NO (the 15b idiom: signal real, no profitable consumer) and keep only the raw rows (cheap optionality). |

---

## Track H — uncertainty & trustworthy ranking

All three candidates are judged by the **reused suite-16 harness** —
frontier rule, finite-sample fit, and crucially the held-out query-STYLE
variant generator that killed conformal (`2026-06-12-pi5-p10-conformal.md:37-43`;
"the suite infrastructure … is the standing falsifier", lines 55-62) — plus
the suite-13 ρ idiom (gate ρ ≥ 0.25; measured today: blended 0.256, NQC
0.234, Clarity 0.198, `2026-06-12-pi5-p8-gates.md:48-49`).

### H1. Bootstrap rank stability

| Field | Content |
|---|---|
| **Proposal** | Resample the fusion inputs B times, measure top-k churn, emit `rank_stability ∈ [0,1]` in the confidence block (schema bump, additive per ADR-20). |
| **Meridian problem** | NQC/Clarity describe the *score curve* and *vocabulary* of one fused ranking (`qpp.rs:5-13`); neither asks "would this top-10 survive a small perturbation of its inputs?" — the most direct operational meaning of retrieval uncertainty, and a *structurally different* predictor family than the one that failed suite 16. |
| **Math formulation** | Fusion is `rrf_fuse(lists, k=60)` over per-engine ranked lists (`rrf.rs:5,12`; lists = BM25, ANN, per-engine searx). For b = 1..B: draw Poisson(1) (or multinomial) bootstrap weights `w_b,i` per input list and fuse with contributions `w_b,i/(60+rank)`; optionally add rank-preserving score jitter from `rank_signals` (per-result raw `bm25`/`ann`/`searx_rank` are already carried, `planner.rs` RankSignals). `rank_stability = (1/B) Σ_b RBO_{p=0.9}(top10(π_0), top10(π_b))` (RBO preferred over Kendall τ: head-weighted, handles non-conjoint lists, which bootstrap resampling produces). |
| **Expected gain** | Beat **NQC alone** (ρ 0.234) as a standalone; the realistic win is as the strongest *new* feature inside H2 — stability is plausibly the predictor whose absolute level is least style-sensitive (it measures the ranking's own variance, not the query's vocabulary), which is exactly the property suite 16 punishes the lack of. |
| **Complexity** | Low: `rrf_fuse` is pure (`rrf.rs:12-23`); B replays + RBO ≈ 80 LoC in `meridian-rank`. |
| **Pi-5 cost** | Fusion measured 0.144ms (`02-budgets.md:84-85`) → B=50 ≈ **7.2ms**. Against budgets: fast local stages sum <5ms with a ≤25ms working target (`02-budgets.md:74`) — +7ms fits the 25ms target but ~2.4× the current stage sum, so ship **B=20 (~2.9ms) on the fast path** (stability estimates converge fast at k=10) and B=50 on deep (where 2173ms p50, `02-budgets.md:126`, makes 7ms invisible). RBO cost is negligible (k=10). |
| **Privacy impact** | None: deterministic-seeded local resampling of in-RAM lists; nothing logged (no-query-logging untouched). |
| **Required data** | None new; suite-13/16 eval sets. |
| **Evaluation** | Suite 13: standalone Spearman ρ vs per-query nDCG@10, gate ≥ 0.25 AND > NQC's 0.234. Suite 16 harness: relative-lift frontier on calibration/hold-out/style-variant — the signal must not collapse where conformal's did. Latency: stage timer + fast-path p50 row re-measured. |
| **Implementation location** | `meridian-rank/src/qpp.rs` (or sibling `stability.rs`), planner wiring beside `planner.rs:1331-1341`, `docs/api.md` confidence block. |
| **Priority** | **MEDIUM** (HIGH as an H2 feature). |
| **Acceptance / kill** | Accept: both gates + budget rows. **Kill:** ρ < NQC alone on the hold-out, or style-variant relative lift collapses, or fast-path p50 regresses past the working target — any one suffices; B-tuning to rescue a failed ρ is threshold-nudging, prohibited by the risk-#24 precedent. |

### H2. QPP ensemble (NQC + Clarity + lane-agreement + score-gap [+ stability])

| Field | Content |
|---|---|
| **Proposal** | Replace the fixed 0.5/0.5 squash blend (`qpp.rs:49-57`) with a small linear ensemble over: NQC, Clarity, score-gap `(s₁−s₂)/s₁` over fused scores, **lane-agreement** = 1 − JSD(local-results domain/score distribution ‖ web-results distribution) computed *within one response* — reusing the divergence machinery (`compare.rs:171` `jsd`, bounded [0,1]) with zero extra egress — and optionally H1's stability. Weights fit by least squares on the suite-13 eval set, frozen in-repo like every other constant. |
| **Meridian problem** | The shipped blend is explicitly "NOT calibration — just bounded blending" (`qpp.rs:49-50`); its ρ 0.256 barely clears the 0.25 gate (and the healed-lane re-run measured the margin shrinking, `2026-06-13-pi5-hybrid-healed-lane.md:16`). Two cheap, orthogonal signals are sitting unused: cross-lane agreement (two retrieval systems agreeing is classic QPP fusion evidence) and head separation. |
| **Math formulation** | `score = σ(β₀ + Σ βᵢ fᵢ)` with features z-normalized on the eval set; fit OLS/logistic against per-query nDCG@10; report per-feature ablation ρ. Lane-agreement only exists when `scope=both` produced web results — the feature is `Option`-al and the ensemble degrades to the available subset (absent ≠ zero; the evidence-block honesty idiom). |
| **Expected gain** | Spearman ρ uplift over NQC (0.234) and Clarity (0.198) individually — gate vs the *blended* 0.256 with a tuning margin (target ≥ 0.30 tuning) and no-collapse on hold-out + style variant. |
| **Complexity** | Low: features exist or are O(k); fitting is offline in `meridian-eval`; serving is a dot product. |
| **Pi-5 cost** | ≪1ms — within the standing QPP ≤1ms budget (`02-budgets.md:138-139` region; ADR-23 `00-adr.md:575-577`). JSD over ≤50 results' domains is µs. |
| **Privacy impact** | None: same response-local data the planner already holds; no new lanes, no compare requirement, nothing logged. |
| **Required data** | Suite-13 eval set (exists) + the suite-16 variant generator (exists). The known risk: a 4-6 parameter fit on a few hundred queries can overfit — the style-shift variant is the registered tripwire for exactly that. |
| **Evaluation** | Suite 13 (ρ gates above, ECE reported); suite-16 harness for style-shift no-collapse of the *relative* lift. Explicitly NOT re-proposing bands: the output stays "uncalibrated, ranking-comparable" wording (`api.md:133-136`). If the ensemble someday produces a "materially stronger predictor", the conformal door reopens only through the standing suite-16 judge (`00-adr.md:733-735`) — that is the documented path, not part of this proposal's acceptance. |
| **Implementation location** | `meridian-rank/src/qpp.rs`, fit harness in `meridian-eval`, planner wiring `planner.rs:1331-1341`. |
| **Priority** | **MEDIUM-HIGH** — cheapest ρ uplift available; do after/with H1 so stability enters the ablation. |
| **Acceptance / kill** | Accept: ρ ≥ 0.30 tuning AND ≥ 0.25 hold-out AND > each single feature, AND style-variant relative lift within 20% of the in-style lift. **Kill:** any collapse on the style variant (re-fit on variant data is prohibited — that's nudging), or the fit's improvement comes entirely from one feature (then ship that feature alone, simpler). |

### H3. Answer abstention calibration (selective prediction, no conformal claim)

| Field | Content |
|---|---|
| **Proposal** | A score-threshold abstention for answer mode: when the winning passage's `ce_score < τ`, withhold `best_passage` and emit `degraded: ["answer_below_threshold"]` (distinct from the mechanical `answer_unavailable`, `planner.rs:1267-1269`, `api.md:329-331`). τ chosen by selective-risk analysis on the suite-18 corpus; an operator knob, default ON at the recorded operating point. |
| **Meridian problem** | Today the engine shows the best passage it found *no matter how bad* — `answer_unavailable` fires only on mechanical failure. Risk #26's failure mode ("a confident wrong passage presented as the answer", `00-adr.md:790-795`) is currently mitigated by wording alone. Suite 18 measured hit-rate 0.700 (`04-bench-plan.md:132-135`): 30% of shipped passages are misses — some fraction of which sit at low `ce_score` and are cheaply refusable. |
| **Math formulation** | From suite-18 replay, per-query pairs `(ce_score, hit ∈ {0,1})`. Coverage `φ(τ) = P(ce ≥ τ)`; selective hit-rate `h(τ) = P(hit | ce ≥ τ)`. Publish the full coverage-vs-hit-rate curve (the tradeoff IS the deliverable); choose `τ* = max{τ-grid: h(τ) ≥ h_target on tuning}` for, e.g., `h_target = 0.85`, then verify on hold-out AND the style-shift variant. **Honest framing, written into api.md:** this is selective prediction *without* distribution-free guarantees — those are killed (ADR-27 REFUTED, `00-adr.md:722-735`; 19pp absolute-coverage collapse under style shift, `2026-06-12-pi5-p10-conformal.md:37-43`). The doc wording is "tuned on the repo eval set; the threshold filters low-relevance passages, it does not certify shown ones." The asymmetry vs bands matters and should be stated: a band's failure mode is a *false certificate* on shown results; abstention's failure mode under shift is mostly *mis-set coverage* (refusing too much or too little) — a much cheaper failure, but only if no coverage number is ever advertised. |
| **Expected gain** | Selective hit-rate uplift at recorded coverage cost, e.g. (to be measured) h 0.70→0.85 at φ ≈ 0.7-0.8; plus the honesty win: silence becomes a signal ("nothing good enough"), explicitly never "no answer exists". |
| **Complexity** | Trivial serving (one comparison, the ADR-27 "2-comparison lookup" cost, `00-adr.md:717-718`); the work is the eval extension + docs. |
| **Pi-5 cost** | ~0ms; if anything it *saves* the answer-mode tail nothing (scores are already computed) — the 3.0s row (`02-budgets.md:148`) is untouched. |
| **Privacy impact** | None; no logging of refusals beyond the existing degraded marker. |
| **Required data** | Suite-18 per-query scores (the harness already produces them) + a style-shift variant of the suite-18 generator (new but mechanical — the suite-16 phrase-style trick applied to the answer corpus). |
| **Evaluation** | Extended suite 18: full risk-coverage curve on tuning/hold-out/style-variant; gate = `h(τ*) ≥ h_target` on hold-out AND `h(τ*)` on the style variant within 10pp of hold-out (no-collapse); ECE of nothing — no probability is emitted. |
| **Implementation location** | `planner.rs` answer branch (~line 1237), `meridian-eval/src/bench/answer.rs`, `meridian-common/src/config.rs` (`answer_min_ce`), `docs/api.md`. |
| **Priority** | **HIGH** — the cheapest trustworthy-ranking item in either track, with data already on disk and a registered risk it directly mitigates. |
| **Acceptance / kill** | Accept: gates above; api.md wording reviewed against the ADR-27 postmortem. **Kill:** if `ce_score` carries no usable selective signal on the answer corpus (risk-coverage curve ~flat — possible: suite 16 showed the *retrieval* score's selective signal is weak; the *passage CE* score is a different, stronger-prior signal, but that is exactly what the experiment decides), record the NO and keep mechanical-only abstention. If the style variant moves `h(τ*)` >10pp, ship the knob default-OFF with the curve published, never a default that silently means different things across query styles. |

---

## R. Engaged rejections

**R1 — Signed-graph contradiction detection (cluster A asserts X, cluster B
asserts ¬X).** Engage: this is the natural next step after C1 — corroboration's
sibling — and the signed-graph formalism (balance theory over
support/contradict edges) is well-posed. Reject: edges require NLI. Per §F2,
the `ort` runtime exists in-tree, but no NLI model does; adding one is a new
neural model dependency (tens of MB, version-coupled tokenizer) against the
ADR-02 Phase-3 "no neural cost" idiom; pairwise passage NLI across k clusters
is O(k²) CE-class inferences at ~10ms/pair (CE 204ms @ 20 pairs,
`02-budgets.md:119`) — hundreds of ms to seconds on the answer path's ~500ms
headroom; there is no contradiction-labeled eval corpus, and the failure mode
(falsely announcing "sources contradict each other") is the conformal lesson
at maximum stakes. C1 deliberately ships the *unsigned* half (support only),
which needs no NLI. Revisit only with an offline-compiled model AND a planted
contradiction suite through a C1-extended judge — and note the Hailo-8L
cannot rescue the cost: its compiler is x86_64-only and no retrieval-relevant
HEF exists (`01-rebaseline.md:58-64`).

**R2 — Full web-graph PageRank.** No crawl exists or may exist: egress is
metasearch + the bounded fetch ladder, per-domain 1 req/2s globally
(`02-budgets.md:97`), SSRF-guarded (`meridian-fetch/src/ssrf.rs`), with the
hermetic egress-invariant count a standing exit gate
(`04-bench-plan.md:172-177`). A web-scale link graph through that aperture is
arithmetic nonsense, and widening the aperture is a privacy-architecture
change no ranking gain justifies. C3 is the lawful version: links over
*ingested* docs only.

**R3 — Min-cut/spectral splitting of evidence clusters.** The problem it
solves (over-merged clusters) is measured absent: suite-9 false-merge 0.0 on
both variants, F1 1.0/0.89→0.918 hold-out after the `MIN_MATCH_BINS=5`
amendment (`00-adr.md:447-465`); union-find over a τ-thresholded containment
graph is already conservative by design (`evidence.rs:6-14`). Spectral
machinery would add eigendecompositions to a fast path bounded at ≤2ms
(suite-11 gate) to fix zero observed failures. Re-opens only if the false-merge
tripwire ever fires on organic data.

**R4 — Conformal re-proposal.** Killed stays killed: ADR-27 REFUTED with the
19pp style-shift collapse (`2026-06-12-pi5-p10-conformal.md`); the brief's
own rule is re-proposal requires *new evidence through the same judge*. H1/H2
may eventually constitute that evidence; until they measurably do, no band
ships, and nothing in this workpaper's acceptance criteria depends on one.

**R5 — Deep/Bayesian uncertainty (ensembles, MC-dropout, posteriors).**
Needs neural forward passes the budgets don't have (no GPU; CPU CE already
dominates deep mode) and answers a question nobody consumes — H1's bootstrap
is the frequentist analog at 7ms, over the *actual* production fusion rather
than a surrogate model. Same a-priori shape as the BOCPD rejection
(`00-adr.md:750-755`): machinery without a consumer.

**R6 — Ranker ensembles (multiple LTR/CE models voted/averaged).** The deep
path is CE-bound (204ms p50 @ top-20; deep p50 2173ms, `02-budgets.md:126`);
N models ≈ N× the dominant stage on a 4-core Pi sharing one rayon pool
(`02-budgets.md:88-90`). Disagreement-as-uncertainty is the only novel output,
and H1 extracts that from input resampling at 0.144ms/replay instead of
~200ms/replay.

---

## Priority ledger (track-internal)

1. **H3** answer abstention — HIGH (data exists, ~0 cost, registered risk #26).
2. **C1** corroboration — HIGH (claim-level trust on the flagship block; ≪1ms in the cheap variant; suite-18 judge ready).
3. **H2** QPP ensemble — MEDIUM-HIGH (cheapest ρ uplift; style-variant tripwire pre-registered).
4. **H1** rank stability — MEDIUM standalone, HIGH as H2's feature (ship together).
5. **C3** citation graph — MEDIUM (do the irreversible link-retention part early; analytics after density is measured).
6. **C2** centrality study — LOW until `w_domain ≠ 0` is on the table; the popularity-bias audit is a precondition for ever setting it.


---

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


---

# T5 — Research Track K: Hailo-8L NPU offload

Status: DRAFT workpaper — proposes, does not decide. As-of: v0.6.0 / 2026-06-13.
Scope rule honored: read-only repo + read-only device queries (`hailortcli
fw-control identify`, `dpkg -l`, `lspci -vv`, `ls /usr/share/hailo-models`,
`getconf PAGESIZE`, `vcgencmd get_throttled`). No installs, no model loads, no
benchmarks were run.

Baseline of record: `docs/research/2026-06-v060-review/01-rebaseline.md` §4
(lines 52–71). The repo has **zero** Hailo/NPU awareness (`grep -ri hailo`
over crates/docs excluding this review tree: no hits — re-verified this
session; consistent with 01-rebaseline.md:63).

## 0. Device ground truth (re-verified live, 2026-06-13)

| Fact | Value |
|---|---|
| Device | HAILO-8L AI ACC M.2 B+M KEY MODULE EXT TMP, arch `HAILO8L`, fw 4.23.0 (`hailortcli fw-control identify`) — 13 TOPS INT8 class (01-rebaseline.md:58) |
| Runtime stack | `hailort` 4.23.0, `hailort-pcie-driver` 4.23.0, `hailo-tappas-core` 5.1.0, `python3-hailort` 4.23.0-1, metapackage `hailo-all` 5.1.1 (`dpkg -l`) |
| Device node | `/dev/hailo0` present, mode **crw-rw-rw-** (world-writable — ops note in §5) |
| PCIe | `0001:04:00.0`, kernel driver `hailo`; link **negotiated Gen2 (5GT/s) x1 ≈ 400–450 MB/s effective**; Gen3 ≈ 900 MB/s is a config.txt knob (01-rebaseline.md:59; unprivileged `lspci` hides LnkSta — the rebaseline capture is the record) |
| Kernel | 6.18.33+rpt-rpi-2712, **16K pages** (`getconf PAGESIZE` = 16384) — the Hailo driver demonstrably runs on this kernel: module bound, `/dev/hailo0` live, fw identify succeeds. ADR-01's 16K concern (00-adr.md:17) is already answered for the driver by existence proof |
| Models on disk | Vision CNNs only (yolo/resnet/scrfd HEFs in `/usr/share/hailo-models/`) — nothing retrieval-relevant is compiled |
| Thermal | `throttled=0x0` at capture; module is the extended-temperature SKU |

CPU baselines to beat (Profile R, measured): CE deep-rerank stage p50 **204ms**
@ top-20/batch-4 (phase-exits/p3.md:7); deep p50 **2173ms**, answer p50
**3224ms @ cap 16 → 2502ms @ cap 8** (bench/2026-06-13-pi5-p10-device.md:22–23;
bench/2026-06-13-pi5-answer-cap-study.md); embed **42.9k docs/s** batch-32
(meridian-embed/src/lib.rs:2–4); ANN **0.45ms p50 @100k / p99 1.73ms @1M
ef=128**; BM25 0.48ms; fusion 0.144ms; RSS plateau ~250–261MB in a 3GB cgroup
(02-budgets.md:70–90, 119, 126).

## 1. Workload characterization — what could move, what cannot

**CE rerank (the prime candidate).** `ms-marco-MiniLM-L-6-v2` INT8 ONNX, 23.2MB
(00-adr.md:285–287), 256-token pairs at fixed padding, batch 4
(meridian-rerank/src/lib.rs:129–130, 151), top-20 candidates under a 1.5s stage
deadline (planner.rs:953–980; lib.rs:98–103). Measured stage p50 204ms — and
this cost is corpus-size-independent (p3.md:51–53). Answer mode adds a second
CE workload: per-fetch sentence-aligned passages, `answer_passage_cap` now 8
(config.rs:288, 306; planner.rs:1186–1196), historically ≤32 per ADR-29
(00-adr.md:786–788). The cap study attributes **~700ms of the answer p50 to CE
realization alone** ("the per-fetch batch is the dominant answer-mode latency
term", answer-cap-study.md:15–16). This is a dense, static-shape, INT8 GEMM
pipeline — exactly the shape an NPU wants.

**Query embedding (not a candidate as-is).** potion-base-8M via model2vec-rs is
a *static* embedding — table lookups + mean pooling, no transformer
(meridian-embed/src/lib.rs:1–4; ADR-06, 00-adr.md:205–216). At 42.9k docs/s it
is "never the ingest bottleneck" by design. There is nothing to offload. A
*transformer* embedder would be a **new workload** (a quality upgrade the NPU
might make affordable — K2), not an offload of an existing cost.

**ANN (structurally not offloadable).** usearch HNSW, cosine over int8
(meridian-vector/src/lib.rs:41–49, 91–99; ADR-07). HNSW search is
data-dependent graph traversal: each hop reads a neighbor list, computes a
handful of distances, and *branches* on the results to pick the next node. It
is irregular, memory-latency-bound pointer chasing with a dynamic control path.
The Hailo programming model executes **statically compiled dataflow graphs**
(HEFs produced offline by the Dataflow Compiler); there is no representation
for data-dependent traversal, and no general-purpose cores to emulate one.
This is an architectural mismatch, not a tuning problem — stated up front so
"use the NPU to speed up search" is evaluated honestly (the steel-manned
version is K4, brute-force scoring, which dies on arithmetic).

## 2. FLOPs/latency model for CE offload

**GEMM FLOPs per pair.** MiniLM-L6: 6 layers, hidden H=384, FFN 4H=1536, seq
L=256 (lib.rs:129). Per layer, counting multiply-accumulates (MACs):

- QKV projections: 3 · L·H·H = 3 · 256 · 384 · 384 = **113.2M MACs**
- Attention output projection: L·H·H = **37.7M MACs**
- QKᵀ scores: L·L·H = 256·256·384 = **25.2M MACs**
- scores·V: **25.2M MACs**
- FFN (two GEMMs): 2 · L·H·4H = 2 · 256 · 384 · 1536 = **302.0M MACs**

Per-layer ≈ 503M MACs → ×6 layers ≈ **3.02 GMACs ≈ 6.04 GOPs per pair**
(embedding lookup + classifier head < 0.2M, negligible).

**What 13 TOPS implies.** Marketing TOPS counts MAC = 2 ops. At a *realistic*
20–40% utilization for a transformer encoder on a dataflow NPU tuned for CNNs
(attention's batched small GEMMs and softmax utilize the array poorly):
effective 2.6–5.2 TOPS → per pair 6.04 GOPs / (2.6–5.2 T) = **1.2–2.3ms
compute**; batch-20 = **23–47ms**.

**PCIe transfer per pair (negligible — shown).** In: 256 token ids (vocab
30522 fits uint16) + 256 mask + 256 type bytes ≈ **≤1.5KB**; out: 1–2 logits ≈
8B. At 400 MB/s (Gen2 x1): 1.5KB / 400MB/s ≈ **3.8µs/pair**, batch-20 ≈ 75µs —
0.04% of the 204ms stage. I/O is not the constraint *for activations*.

**The constraint that can bite: context fit.** The 23.2MB INT8 weights likely
exceed Hailo-8L single-context on-chip memory, in which case DFC splits the
graph into contexts whose weights stream from host per inference pass (the fw
identify string "extended context switch buffer" exists precisely for this).
Worst case, one full weight re-stream per batch pass: 23.2MB / 400MB/s ≈
**58ms (Gen2 x1)** or / 900MB/s ≈ **26ms (Gen3)** added per batch.

**Stage estimate (top-20).** CPU-side tokenization stays (1–4ms for 20 pairs,
HF tokenizers) + transfer 0.1ms + HailoRT scheduling ~1–5ms + compute 23–47ms
+ 0–58ms context streaming:

| Scenario | Stage estimate | Speedup vs 204ms |
|---|---|---|
| Single-context HEF | ~30–55ms | **~4–7×** |
| Multi-context, Gen2 x1 | ~85–110ms | **~2×** |
| Multi-context, Gen3 x1 | ~55–80ms | **~2.5–3.7×** |

**Headline: 2–7× on the CE stage, with 3–6× the defensible planning range.**
Equivalently, at the *same* 204ms budget: rerank depth top-40 to top-80 (×2–4).
Loud assumptions: (a) **DFC can compile this BERT-class encoder at all** —
gated on the C4 toolchain ledger (01-rebaseline.md:62 cites `02-competitive.md`
C4, which is **not yet in the tree** at draft time — treat compilability as
UNVERIFIED); (b) **INT8-on-Hailo score parity with INT8-on-ort** is an
empirical question (different quantization pipelines) — the 100-query eval +
suite-13 harness and suite 18 are the judges, not an assumption; (c)
utilization 20–40% is an engineering estimate, not a measurement — suite 4
(`rerank`, 04-bench-plan.md:23) is the instrument that turns it into one.
Note: suite 4 never recorded ms/pair at batch 1/4/8 (tract couldn't load the
model at Phase 0, bench/2026-06-10-pi5-report.md:19; P3 recorded the stage
total instead) — the derived CPU figure is 204ms/20 ≈ **10.2 ms/pair** at
batch 4, and the answer path's small per-fetch batches measure worse (~700ms /
~16 pairs ≈ **~44 ms/pair** realized, answer-cap-study.md:8–16).

**Where the win actually lands.** Deep p50 is 2173ms and network-dominated:
cutting 204→~50ms moves deep p50 by ~7%. The budget-row prize is the **answer
row**, where CE is the dominant controllable term (§3 K3), and **depth/quality
at constant latency** for K1.

## 3. Candidates

### K1 — CE rerank offload (HailoReranker)

| Field | Value |
|---|---|
| Proposal | Compile `ms-marco-MiniLM-L-6-v2` INT8 to HEF (offline, x86_64 DFC); implement `HailoReranker` behind the existing `Reranker` seam — the planner already holds `reranker: Arc<Reranker>` (planner.rs:251) and the crate already models unavailability (`Reranker::unavailable()`, `available()`, lib.rs:71–93). Fail-open to the CPU ort path (or to LTR order, the existing ladder) when `/dev/hailo0` is absent, busy, or the module is hot |
| Meridian problem | Deep rerank depth is pinned at top-20/batch-4 by A76 cost (ADR-09, 00-adr.md:281–283); CE is the largest quality jump in the stack (lib.rs:1–2) but its reach is latency-rationed |
| Math formulation | §2 model: 6.04 GOPs/pair; stage = tok + PCIe + ctx-stream + compute; speedup 2–7× or depth ×2–4 at constant budget |
| Expected gain | Rerank stage 204ms → ~30–110ms, OR top-40–80 rerank at today's budget; deep p50 −5–8%; frees ~0.8 core-seconds/query of CPU (K5) |
| Complexity | High first time: DFC pipeline + HEF artifact + FFI + scheduler integration (§4). Code-side small: one trait impl + config |
| Pi-5 cost | RAM: HailoRT lib + buffers est. 50–150MB *replacing/alongside* the 250MB ort-sessions row (02-budgets.md:58) — needs its own budget row; weights live on-device/streamed, not in RSS. Disk: HEF ≈ 25–30MB in the image. Power +1.5–2.5W active |
| Privacy impact | Query text crosses PCIe as token ids; stateless inference; see §5 invariants (lane-blindness, no residue) |
| Required data | None new — suite-4 pairs, the 100-query eval, suite-18 replay corpus all exist |
| Evaluation | Suite 4 (rerank ms/pair batch 1/4/8, CPU vs NPU); 100-query eval + suite-13 harness for score parity (nDCG@10 delta); suite 6 thermal; suite 2 untouched |
| Implementation location | `crates/meridian-rerank` (new `hailo-backend` feature, gnu image — ADR-02 precedent); `deploy/` device mapping; `train/`-adjacent DFC scripts |
| Priority | **P1** — the only candidate with a measured CPU cost worth beating |
| Acceptance | NPU ms/pair ≤ 0.5× CPU at batch 4 AND eval nDCG@10 within noise of ort CE AND fail-open proven (yank `/dev/hailo0` mid-run → degrades, never errors). Kill: DFC cannot compile the encoder (C4); or multi-context streaming puts stage p50 > 150ms at Gen2 *and* the operator declines the Gen3 knob; or parity fails the eval |

### K2 — Transformer embedding upgrade (NPU-affordable, NOT an offload)

| Field | Value |
|---|---|
| Proposal | Replace potion static embeddings with a small transformer encoder (MiniLM-class, seq 128) for *document + query* embedding, batched on the NPU at ingest |
| Meridian problem | Static embeddings cap dense-lane quality; hybrid beats BM25 by 0.04 nDCG (02-budgets.md:118) — ceiling unknown |
| Math formulation | Seq-128 encoder ≈ 1.43 GMACs/doc (half-seq variant of §2: QKV 56.6M + proj 18.9M + attn 25.2M + FFN 151M per layer ×6) → 0.55–1.1ms/doc at 2.6–5.2 effective TOPS → **~900–1800 docs/s** NPU-bound. Ingest gate is ≥50 docs/s (02-budgets.md:79): 18–36× margin. Re-embed 1M docs ≈ **9–18min** NPU time + CPU tokenization |
| Expected gain | Unknown until measured — that is the point of the gate. Plausible hybrid nDCG movement; zero gain is a live outcome (potion already beat the bar that mattered) |
| Complexity | High: **schema break** — store is 256-d int8 (02-budgets.md:30; dims probed from the embedder, meridian-embed/src/lib.rs:38, vector/src/lib.rs:42); MiniLM-class outputs 384-d → full index rebuild; disk/RAM rows re-derive (506MB measured @1M 256-d → ~760MB @384-d; the 600MB disk row busts) |
| Pi-5 cost | Index rebuild I/O (SD endurance, risk #7); +50% vector RAM/disk; NPU contention with K1 at ingest time (scheduler arbitrates) |
| Privacy impact | Document text crosses PCIe at ingest; queries at search time — same §5 invariants |
| Required data | The operator corpus + existing eval set; no new labels |
| Evaluation | Suite 1 (embed throughput, NPU arm); suite 2 (recall\@10 ≥0.95 on the new vectors — the gate that caught rung 2, 00-adr.md:253–268); 100-query eval hybrid ≥ BM25 + a positive margin over potion-hybrid; suite 13 QPP unchanged |
| Implementation location | `crates/meridian-embed` (backend enum), `meridian-vector` (dims), ingest pipeline, budgets rows |
| Priority | **P2** — real quality hypothesis, but pays a schema migration for an unproven gain; sequence after K1 proves the toolchain |
| Acceptance | Hold-out nDCG@10 ≥ potion-hybrid + 0.02 AND suite-2 recall ≥0.95 AND ingest ≥50 docs/s sustained. Kill: quality delta < 0.02; or rebuild RAM/disk busts Profile R rows; or DFC fails the encoder (shared kill with K1) |

### K3 — Answer-mode passage scorer on NPU

| Field | Value |
|---|---|
| Proposal | Route the per-fetch passage-CE batch (planner.rs:1186–1196) through the same HEF/session as K1 |
| Meridian problem | CE realization is **the** dominant answer-mode latency term — the cap study bought the 3.0s row back only by halving passage depth 16→8 (answer-cap-study.md; 02-budgets.md:148), and position telemetry showed the cost: index-11 winners realize shallower passages |
| Math formulation | ~16 pairs/query @ measured ~44ms/pair realized → ~700ms CPU; NPU ~2–5ms/pair incl. overhead → ~30–80ms. Answer p50 2502ms → est. **~1.9–2.1s** at cap 8, or **cap 16 restored under the 3.0s row** (3196 − ~650 ≈ 2.5s) |
| Expected gain | Either ~0.5s answer p50 cut or 2× passage depth at the same row — un-paying the cap study's measured quality price |
| Complexity | Low *marginal* if K1 lands: same model, same backend, second call site; HailoRT model scheduler multiplexes K1+K3 on one configured network |
| Pi-5 cost | None beyond K1 |
| Privacy impact | Fetched page text crosses PCIe — same class as query text; RAM-only discipline (ADR-29, 00-adr.md:780–782) must extend to NPU buffers (§5) |
| Required data | Suite-18 replay corpus (exists) |
| Evaluation | Suite 18 (hit-rate ≥ baseline +10pp must still hold, 04-bench-plan.md:118); the answer budget-row re-measure (n=16 interpolated medians, the device protocol); cap-16-on-NPU arm vs cap-8-on-CPU |
| Implementation location | planner.rs answer path; config (`answer_passage_cap` default re-sweep) |
| Priority | **P2, conditional on K1** — standalone it cannot justify the toolchain; with K1 it is nearly free and has the largest budget-row payoff |
| Acceptance | Answer p50 ≤2.6s at cap 16 on NPU AND suite-18 hit-rate holds. Kill: K1 killed; or scheduler contention pushes deep p50 over 2.5s |

### K4 — NPU brute-force INT8 scoring for FILTERED dense search — expected REJECT

| Field | Value |
|---|---|
| Proposal | The honest "use the NPU to speed up ANN" test: exact dot-product scan of a filtered candidate subset (10k–100k × 256-d int8) on the NPU, addressing R9 (ANN is skipped under geo/time filters, 01-rebaseline.md:24) |
| Meridian problem | Real: filtered queries lose the dense lane entirely (ADR-10; api.md) |
| Math formulation | **CPU baseline:** 100k × 256-d int8 = 25.6MB streamed; A76 sdot ≈ 76.8 GMAC/s/core theoretical, so compute (25.6 GMACs… 100k·256 = 25.6M MACs) is trivial — the scan is memory-bound: 25.6MB / 8–16 GB/s ≈ **1.6–3.2ms** (4 cores, plus top-k heap → call it 2–4ms; 10k subset ≈ 0.3ms). **NPU:** vectors are per-query data, not weights — they must cross PCIe: 25.6MB / 400MB/s ≈ **64ms transfer alone** (Gen3: 28ms), 16–30× worse than the *entire* CPU scan before any compute |
| The programming-model question, answered plainly | Pre-staging the matrix on-device means making it HEF *weights* — HEFs are compiled offline and immutable; re-"compiling" per ingest batch is absurd, and HailoRT exposes **no generic matmul-as-a-service API**: everything runs through compiled network graphs. Hailo cannot economically stream non-NN bulk data per query. **It does not support this workload.** |
| Expected gain | Negative. **Verdict: REJECT with numbers** |
| Complexity / Pi-5 cost / Privacy / Data | Moot |
| Evaluation | None warranted; the arithmetic is the evaluation |
| Implementation location | The surviving alternative is **index-side filtered ANN on CPU** (usearch filtered search predicates, or the 2–4ms exact scan above for ≤100k subsets — which *is* affordable on CPU and is a separate, NPU-free candidate for the R9 gap, referred to the main review) |
| Priority | **REJECTED** |
| Acceptance/Kill | Killed a-priori by transfer arithmetic; reopen only if a future runtime exposes resident generic GEMM with on-device data updates |

### K5 — Thermal/CPU headroom and joules-per-result

| Field | Value |
|---|---|
| Proposal | Account K1/K3's side-effect: CE moves from 4 A76 cores to a 1.5–2.5W accelerator |
| Meridian problem | Not a current failure — thermal passes with margin (75.7°C max, no throttle, 02-budgets.md:85; soak ≤61°C). This is headroom accounting, not a fix |
| Math formulation | CPU CE: ~0.2s × ~5W incremental ≈ **~1.0J**/deep query (answer: ~0.7s CE ≈ 3.5J). NPU: ~0.05s × 2W ≈ **0.1J** + host margin. Frees ~0.8 core-seconds/deep-query for BM25/fusion/tokenization and co-tenants |
| Expected gain | ~10× CE-stage energy cut (estimate); lower thermal duty under QPS load; better p99 under concurrency (rerank no longer competes with the rayon pool) |
| Complexity | None beyond K1 instrumentation |
| Pi-5 cost | +1.5–2.5W NPU active power (net likely negative total board power during CE) |
| Privacy impact | None new — but see §5 timing invariant: power/utilization is an observable |
| Required data | `vcgencmd pmic_read_adc` rails + suite-6 loop |
| Evaluation | **Suite 6 (thermal)** extended with PMIC power sampling: 10-min deep-mode loop, CPU-CE arm vs NPU-CE arm; report °C, throttle flags, J/query |
| Implementation location | `meridian-bench` suite 6 (bench-device feature) |
| Priority | **P3** — measure as part of K1's exit, never standalone |
| Acceptance | NPU arm strictly ≤ CPU arm on temp AND J/query. Kill: n/a (it is a measurement); if NPU arm is *hotter* (shared M.2 thermal envelope, EXT TMP module sits near the SD/board), record it as a K1 cost |

## 4. Toolchain & ops reality

1. **DFC is x86_64-only, offline.** The Dataflow Compiler cannot run on this Pi
   (01-rebaseline.md:62). **No x86_64 build host was verified in this
   environment** — CI builds amd64 images (02-budgets.md:150), so a GitHub
   runner *could* host the DFC docker image, but the DFC is a large licensed
   SDK download; treat "a working DFC pipeline exists" as a hard, unverified
   dependency and the first gate of K1. The C4 ledger (02-competitive.md, not
   yet in tree) owns the "can DFC compile BERT-class encoders" question.
2. **HEF ↔ HailoRT version coupling.** Device runs HailoRT 4.23.0 + driver
   4.23.0 (verified). HEFs are compiled for a runtime version family; a casual
   `apt upgrade` of `hailo-all` can orphan the HEF. The repo idiom exists:
   digest-pinning (searxng) — pin the hailort debs and record the pair
   (HEF build-id, runtime version) in the model manifest (models are already
   manifest-pinned, 02-budgets.md:27).
3. **Rust integration.** Options: (a) **libhailort C API via FFI** behind a
   `hailo-backend` cargo feature — the natural landing, exactly ADR-02's
   precedent of a gnu-image-only feature (`ort-backend`, lib.rs:4–8;
   00-adr.md:80–87): the musl/scratch image stays neural-free and degrades,
   the gnu deep image gains the feature. libhailort is a shared lib → fine on
   distroless-cc/gnu, impossible in `FROM scratch` musl — which is already
   true of deep mode wholesale. (b) **python3-hailort sidecar**: violates the
   single-binary idiom, adds an IPC hop and ~100MB+ Python RSS, a second
   failure domain, and a privacy surface (query text over a local socket).
   Score: FFI strongly preferred; sidecar only as a throwaway feasibility
   probe. Container needs `/dev/hailo0` device-mapped — compatible with
   `cap_drop ALL`/`read_only` hardening via cgroup device allow; an explicit
   compose delta on the deep profile only.
4. **16K-page kernel.** The driver demonstrably runs on 6.18.33+rpt-rpi-2712
   at 16K pages (§0 existence proof). No ADR-01-class blocker.
5. **Device contention.** The Pi is shared (rpicam hailo postprocess is
   installed; other tenants exist). HailoRT's model scheduler multiplexes
   configured networks (K1+K3 share one model anyway); but a co-tenant vision
   pipeline can hold the device. Fail-open (K1 row) is therefore not optional
   polish — it is the design.
6. **PCIe Gen3 knob.** `dtparam=pciex1_gen=3` roughly doubles link bandwidth
   (matters iff the HEF is multi-context, §2). **Operator decision** — Q-item
   for 06-operator-questions; this review documents, does not decide.

## 5. Candidate-ADR sketch — "heterogeneous compute policy"

**Decision (proposed).** The NPU is an *accelerator, never a dependency*:

1. **Identical-semantics fallback.** Every NPU path has a CPU path with the
   same output contract; planner behavior is the existing ladder
   (`rerank_shed` → `rerank_unavailable` → partial `rerank_timeout`,
   planner.rs:956–1009) extended with backend provenance. If NPU and CPU
   scores are *not* bit-identical (they will not be — different quantization),
   the served semantics are "a CE score from the pinned model family", and the
   parity gate (K1 acceptance) bounds the divergence; a `rank_signals`-level
   backend tag keeps it explainable without a new degraded marker (degraded
   markers stay reserved for *capability* loss, matching precedent).
2. **No user-data residue.** HEF inference is stateless — weights are
   immutable, activations are transient device-DDR/SRAM scratch overwritten
   per inference; no on-device persistence exists for tensors. Verify, do not
   assume: confirm HailoRT service/driver logging emits metadata only (never
   tensor contents), and that no trace/profiling mode is compiled into the
   production image. Fetched-text passages (RAM-only by ADR-29) extend their
   discipline to host-side DMA buffers: freed and not recycled across queries
   without overwrite.
3. **Lane blindness (timing side-channel) — proposed invariant inv22.** Anon
   and direct lanes must be indistinguishable by NPU usage: same backend
   selection, same batch shapes (fixed 256-token padding already guarantees
   shape uniformity, lib.rs:173–176), no lane-conditional scheduling. Threat:
   `/dev/hailo0` is **world-writable (crw-rw-rw-, verified)** — any co-tenant
   process can observe device busyness and correlate NPU activity with anon
   egress timing. Mitigation is operator-side (udev group restriction) plus
   the invariant; the privacy smoke test gains a clause: NPU utilization
   pattern for an anon deep query ≡ direct deep query.
4. **Budget honesty.** NPU runtime RAM gets its own row next to "ort sessions
   250MB" (02-budgets.md:58); the HEF gets a manifest entry; suite 6 gains the
   power arm (K5).

## 6. Rejections (a-priori, with the bar each would have to clear)

- **LLM/decoder models on Hailo-8L.** Wrong architecture class: autoregressive
  decode needs KV-cache and dynamic shapes; Hailo-8L executes static dataflow
  graphs, and its on-chip memory cannot hold even a 100M-param decoder plus
  cache — official positioning routes GenAI to Hailo-10H-class parts (C4
  ledger pointer, 01-rebaseline.md:62). Independently dead: the review's
  no-generative-model constraint and ADR-29's "extractive only" honesty stance
  (00-adr.md:790–795).
- **Neural intent classifier.** The incumbent is a 103-line lexical heuristic
  at µs scale (R3, 01-rebaseline.md:18; ADR-02's "no neural cost",
  00-adr.md:76). NPU round-trip overhead alone (~ms of scheduling + PCIe) is
  ~1000× the incumbent's budget, before any quality argument — and the 4-class
  problem has no training data (the ADR-25 refrain). P3 at absolute best;
  effectively rejected.
- **NPU HNSW traversal.** §1's architectural mismatch: irregular,
  memory-latency-bound pointer chasing with data-dependent control flow has no
  static-dataflow compilation, and the CPU already answers in 0.45ms p50 —
  there is nothing to win even if it compiled. (The filtered-search gap R9 is
  real but is K4's question, and K4 dies on transfer arithmetic; the surviving
  alternative is index-side filtered ANN on CPU.)

## 7. Sequencing & open dependencies

K1 first (it proves DFC + FFI + parity + fail-open); K3 rides K1; K5 is K1's
power/thermal exit measurement; K2 only after K1 establishes the toolchain and
only with the schema-migration cost priced; K4 rejected. Hard blocker for the
whole track: the **DFC question (C4)** — no x86_64 DFC host is verified, the
C4 ledger is not yet in the tree, and BERT-class compilability on the 8L
(single- vs multi-context fit of a 23.2MB encoder) decides whether K1's
speedup is ~4–7× or ~2×. If C4 returns "cannot compile", the entire track
reduces to a documented no-go with this arithmetic on the record.
