# Hybrid-nDCG re-evaluation on the healed dense lane (2026-06-13, Pi 5)

**Verdict: the carry closes with a finding OPPOSITE to its premise.** The
pending item (since the P7 numkong restoration) expected the Phase-2
baseline (hybrid 0.42 vs BM25 0.38, +0.04) to UNDERSTATE hybrid because the
dense lane was broken when it was measured. The healed-lane re-eval says
otherwise:

| System | nDCG@10 | MRR@10 | Recall@100 |
|---|---|---|---|
| BM25 | 0.4099 | 0.3436 | 0.9300 |
| hybrid (RRF) | 0.4184 | 0.3544 | 0.9100 |
| hybrid+LTR | 0.4184 | 0.3544 | 0.9100 |

Hybrid leads BM25 by **+0.85pp** (gates hold: hybrid ≥ BM25 ✓, LTR
no-regression ✓, suite-13 ρ = 0.256 ≥ 0.25 ✓) — a smaller margin than the
Phase-2 number, on a dense lane that is PROVABLY healthy (the same-day 1M
re-baseline: recall@10 0.98 @ ef=128).

## The honest reading

1. **The two measurements are not comparable.** Different generated query
   sets, different corpus snapshots, different harness details — the
   Phase-2 "+0.04" was never a stable baseline to beat, and quoting either
   number against the other is seed-noise theater.
2. **The eval set is BM25's home turf.** Known-item queries are sampled
   body words of the target document — exact lexical matches by
   construction. The dense lane earns its keep on PARAPHRASE queries
   (suite 15b measured potion separating same-meaning different-words
   registers at 0.716 vs 0.052 cosine), which this query generator never
   produces. A modest hybrid margin on known-item queries is the expected
   shape, not a defect.
3. **What would actually move this number** is a paraphrase-style eval set
   (the suite-16 phrase-query generator is halfway there: phrase queries
   measured a 0.32–0.53 base-rate spread vs known-item's 0.51–0.53 —
   harder, more lexical-divergent). Recorded as the follow-up shape if the
   hybrid margin ever needs defending; not built now.

The README "known pending" item closes with this record.
