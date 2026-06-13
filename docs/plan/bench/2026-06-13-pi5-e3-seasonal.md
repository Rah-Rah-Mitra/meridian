# Suite 19 `seasonal` — E3 day-of-week baselines: RECORDED NO

As-of: v0.6.0 branch `feat/e3-seasonal` · Pi 5 (kernel 6.18.33, 16K page, 4 cores) · 2026-06-13
Suite: `crates/meridian-eval/src/bench/seasonal.rs` (suite 19) · debug build (FPR/TPR is
build-profile-independent; `04-bench-plan.md` §6) · `meridian-bench seasonal`.

## The question (roadmap §9.1, bet E3)

Does a multiplicative day-of-week pre-adjustment of the mover day-series reduce the
false-positive rate that GDELT's weekly cycle (weekend dip) induces in the shipped
latest-day quasi-NB z (`meridian_analytics::stats::mover_stats`), without regressing real
ramp/spike detection or disturbing non-seasonal series? Candidate:
`stats::mover_stats_seasonal` → `seasonal::deseasonalize` (per-weekday index `(n_d·r_d+λ)/(n_d+λ)`
shrunk toward 1, `y′_t = y_t / f_{dow(t)}`, identity below 21 days).

## Method (the judge)

A 20-series BH family over a 28-day window (one CAMEO-root-code report — the suite-17
shape), with a **common** multiplicative weekly cycle applied to every series (the §9.1
"correlated across all roots, so BH-FDR cannot absorb it" mechanism). Per risk #21: the
shrinkage λ is swept on a Poisson **tuning** variant (weekend dip 0.6) and judged frozen on
an NB **hold-out** with a *different* weekly shape (Monday surge 1.3 + weekend 0.7). The
binding arm is the **fixed-peak phase** (the report's latest day always lands on a peak
weekday — the worst realistic systematic bias, "the weekly report run every Monday"); the
**averaged phase** (continuous ad-hoc querying) is reported alongside. Shipped `mover_stats`
vs `mover_stats_seasonal` at matched TPR; non-seasonal parity on flat-DOW series; the MDE₈₀
is pre-registered (J3).

## Result — RECORD-NO (verdict = `record_no`, gate PASS under amend-or-record)

| metric | tuning (WeekendDip) | hold-out (MondaySurge) |
|---|---|---|
| fixed-peak null FPR, **unadjusted** (shipped z) | 0.0393 | 0.0821 |
| fixed-peak null FPR, **adjusted** | 0.0429 | 0.0893 |
| **FPR reduction (unadj/adj)** | **0.92×** | **0.92×** |
| ramp+spike TPR non-regression | yes (≤0.02) | yes |
| chosen λ (frozen on tuning) | 16 | — |

- **Averaged-phase** reduction at λ=16: **0.88×** (unadjusted 0.027).
- **Non-seasonal parity FAILS**: on flat-DOW series the adjusted detector *raises* FPR —
  Poisson 0.0179 → 0.0304, NB 0.0250 → 0.0357 — i.e. it injects estimation noise where there
  is no cycle to remove.
- **Power**: MDE₈₀ ≈ 0.0325 (n=560 null obs, p̄=0.039); the observed absolute FPR delta is
  0.0036 — far below MDE *and* in the **wrong direction** (adjusted is worse). This is not an
  underpowered near-miss; it is a decisive directional NO (reduction ≤ 1.05× and parity fails).

## Why (the mechanism — confirms §9.1 aggravator #2, inverts its conclusion)

§9.1 predicted "the pooled quasi-NB dispersion partly eats weekly variance as overdispersion,
deflating power." That is exactly what happens — **but the consequence is protective, not
harmful.** The weekly swing inflates each series' within-window variance, so the estimated
inverse-dispersion `1/r̂` rises and the z **denominator** widens; the shipped detector
therefore already holds the seasonal-null FPR near nominal (0.039–0.082). De-seasonalising
*removes* that variance, shrinking the denominator and pushing z back up — while also paying
an estimation-noise cost (7 weekday indices from ~4 observations each), visible in the parity
failure. The two effects make the adjustment net-negative. E3's specific fix is **falsified by
its own judge**.

## Decision

- **Do NOT wire `mover_stats_seasonal` into `trends.rs`.** The live mover path is unchanged.
- `seasonal::deseasonalize` / `mover_stats_seasonal` are retained **only** as this standing
  judge's candidate (like the conformal subcommand for ADR-27) — GDELT-only, no persisted
  state, no forget surface, anon untouched.
- **Re-entry condition**: a correction that beats the dispersion-absorbed baseline through this
  same suite-19 judge (e.g. a dispersion-aware DOW model that does not discard the protective
  overdispersion, or a regime with a measured weekly amplitude strong enough that the bias
  survives the dispersion). Not a re-fit on the variant data (the risk-#24 precedent).

This is a first-class recorded NO in the lineage of suites 13b (MMR), 15b (embedding-coverage),
and 16 (conformal bands): a proposed method engaged, measured against a pre-registered gate,
and closed on its own evidence.
