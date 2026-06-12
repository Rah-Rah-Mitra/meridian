# Suite 16 `conformal` — Phase-10 entry experiment (2026-06-12, Pi 5)

**Verdict: GATE FAIL on every condition — confidence bands DO NOT ship.
ADR-27 is REFUTED for this predictor on this eval distribution.** The raw
NQC/Clarity/score signals remain exactly as ADR-23 shipped them ("not a
probability", suite-13 wording). Machine-readable run:
`2026-06-12-pi5-p10-conformal.json`. This is the MMR pattern (suite 13b)
repeating one phase later: the feature's own gate killed it before release.

## Setup

Real-pipeline calibration: 400 calibration + 200 hold-out queries
(known-item generator, disjoint seeds) + 200 held-out generator-VARIANT
queries (consecutive-phrase style — risk #21) through the planner-equivalent
local stack (hybrid RRF + LTR + the production `qpp::confidence`) on the
100k-doc corpus; per-query (blended score, nDCG@10). Threshold fit with the
(n + 1) finite-sample correction, calibration only; a-priori frontier rule
(walk α from the Q12 default, ship only if the achieved target clears the
base rate by +15pp high / +10pp med).

## What was measured

| | calibration | hold-out (same style) | variant (phrase style) |
|---|---|---|---|
| base rate P(nDCG ≥ 0.5) | 0.510 | 0.530 | **0.320** |
| high-band coverage @ λ=0.585 | 0.676 (n=37) | 0.652 (n=23) | **0.462** (n=26) |
| base rate P(nDCG ≥ 0.3) | 0.690 | 0.670 | 0.420 |
| med-or-better coverage | 0.811 | 0.826 | **0.500** |

- **The Q12 default (τ=0.5 @ 90%) is unachievable outright**: the most
  confident 5% of calibration queries reach 65% coverage, decaying toward the
  51% base rate by top-20%. ECE is excellent (0.016) — the score is honest in
  the mean — but the SELECTIVE signal is ~+12–14pp of lift, nowhere near a
  90%-coverage certificate.
- **The frontier target (65%) fails the usefulness margin on its own terms**
  (needs base + 15pp = 66%).
- **The absolute claim does not survive a query-style shift**: on phrase
  queries — same corpus, same pipeline, same metric — high-band coverage
  drops to 0.462, a 19pp miss. The predictor's RELATIVE lift persists
  (+14pp over the variant's own 0.320 base), so the score remains a useful
  ranking-comparable signal — which is precisely what it already ships as.
  What died is the band: an absolute "score ≥ λ ⇒ 65% chance of good results"
  badge that flips meaning when the query style moves.

## Why this is the right outcome

Risk #24 was registered at planning time as "conformal bands overclaim
off-distribution — a coverage badge on drifted traffic is a lie with a
confidence interval", with the held-out generator variant as its tripwire.
The tripwire fired on the FIRST off-style test, before any user ever saw a
band. The tripwire text said "method re-design, not threshold nudging": no
threshold exists that fixes a predictor whose absolute level moves 19pp under
a mild style shift.

## Carried forward

- Bands return ONLY with a materially stronger predictor (candidates: deep
  CE-score features, retrieval-overlap stability, query-length-conditioned
  models) — suite 16 stands ready as the judge, frontier rule and all.
- The suite infrastructure (the `conformal` subcommand, the `rank_query`
  harness refactor, the phrase-query variant generator) ships in this train
  and is the standing falsifier.
