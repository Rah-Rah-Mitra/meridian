# Suite 13c — evidence-cluster diversity (2026-06-13, Pi 5)

**Verdict: GATE PASS on both seeds, dominating the un-diversified ranking on
BOTH metrics — the P8 carry ("evidence-cluster-aware diversifier replaces
withdrawn MMR") ships as `diversity=evidence`.** Judged by the same dup
harness and the same two conditions MMR failed (suite 13b).

## Results (GATED regime: BM25-matching head, the web-head proxy)

| Ranking | alpha-nDCG@10 | nDCG@10 |
|---|---|---|
| **tuning (seed 42):** BM25 base | 0.5833 | 0.6196 |
| MMR λ=0.7 (13b baseline, withdrawn) | 0.5961 | 0.5493 |
| **evidence-cluster (shipped)** | **0.7728** | **0.7612** |
| **hold-out (seed 1337):** BM25 base | 0.5531 | 0.5802 |
| MMR λ=0.7 | 0.5855 | 0.5411 |
| **evidence-cluster (shipped)** | **0.7838** | **0.7824** |

Gates (the 13b pair): alpha-nDCG improves ✓ (+0.19 / +0.23) and plain nDCG
loss ≤1% ✓ — in fact plain nDCG **improves +0.14 / +0.20**. The diversifier
beats the baseline on the relevance metric it was only required not to
damage. The diagnostic (hybrid+LTR) regime improves too (alpha 0.52 vs MMR
0.44 on tuning).

## Why it wins where MMR lost — and where v1 of THIS rule lost

- **MMR (13b post-mortem):** symmetric token similarity demotes a cluster's
  canonical original exactly like its copies; truncated copies are shorter →
  higher BM25 → the original is what gets pushed out.
- **v1 of this diversifier repeated the trap from the other side**
  ("first-RANKED cluster member keeps the slot"): the best-ranked member is
  usually a truncated copy, so v1 crowned copies and deferred grade-3
  originals — measured nDCG −13% (0.539 vs 0.620), worse than MMR. Recorded,
  not hidden.
- **The shipped rule** uses what the engine KNOWS: the cluster's CANONICAL
  member — the most-shingled document, i.e. the superset its copies derive
  from (a 75% truncation cannot out-shingle its original). At each cluster's
  first appearance, the canonical takes the slot; copies defer; unsketched
  results never move. Promoting originals over their higher-BM25 copies is
  why BOTH metrics improve: diversity and relevance stop being a tradeoff
  when the demoted thing is genuinely redundant AND genuinely worse.

## Harness honesty fixes (same train)

- The first run scored **all zeros and printed PASS** — the default
  `--data eval/dup-data` pointed at a non-existent dir and the harness gated
  an empty index. `run-dup` now refuses 0-doc indexes outright, and the
  default points at the real `eval/dup-data5`.
- The harness's exit gate now judges the evidence diversifier (the live
  question); MMR verdicts stay printed as the measured 13b baseline.

## Shipped surface

`diversity=evidence` request param (400 on anything else, with the MMR
history in the message) · per-result `evidence.canonical` annotation +
evidence block `schema: 2` · pure local reordering (no privacy surface, no
new config — `evidence.enabled = false` makes it a no-op) · `rank_signals`
stay raw, so a promoted canonical visibly carries a lower score than the
copy below it: that is the explainable truth of what the diversifier did.
