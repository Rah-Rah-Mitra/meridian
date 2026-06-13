# C1 claim-level corroboration — suite-20 production arm (Pi 5, on-device)

As-of 2026-06-13. The binding gate for shipping `best_passage.corroboration`
(roadmap §8.2 / 06-roadmap-completion.md bet 1). The deferred bet had a KILL
criterion: *a false "k independent sources agree" badge is worse than none* —
same-cluster syndicated copies must NEVER count. So the suite-20 `answer_trust`
judge was extended from its synthetic `support_score` arm to the **real
production signal**: real claim text → the SHIPPED `meridian_index::sketch` +
`evidence::cluster_tags` clustering → the REAL ort INT8 ms-marco cross-encoder.

## Method

- **Corpus** (`gen_claims`): per query a claim, near-verbatim **syndicated copies**
  (outlet boilerplate around the verbatim claim → containment 1.0 → same cluster,
  the trap), **independent restatements** (same entities, reordered → distinct
  cluster, no shared 4-word run), and **chaff** (off-topic + a hard 30%/60%
  "adjacent" fraction sharing one entity). Tuning / hold-out / style-shift seeds
  (risk-#21 protocol); MDE pre-registered (J3). 360 queries/variant.
- **Arm A (hermetic, CI, no model):** the shipped sketch clusters the corpus;
  assert every copy stays in the claim's cluster (zero same-cluster leakage by
  construction) and restatements split off. Build-profile-independent.
- **Arm B (gnu, `--features bench-ce-real`, `models/`):** for each distinct
  non-claim cluster, `Reranker::rerank(claim, [canonical snippet])` (the real
  INT8 CE); a cluster supports iff `ce ≥ τ_corr`. τ_corr swept on tuning (max
  recall s.t. precision ≥0.9), FROZEN, judged on hold-out + style-shift.
  Precision/recall counted at the CLUSTER level — the badge must be RIGHT.
- **Support rule (documented deviation from §8.2):** absolute `ce(claim,snippet) ≥
  τ_corr`. The illustrative `ce ≥ ce_score − δ` mixed two incomparable CE pairings
  (query↔passage vs passage↔snippet); the absolute, swept-and-frozen bar is what
  the precision gate actually validates.

## Result — PASS (ships default-ON)

| metric | tuning | hold-out | style-shift | gate |
|---|---|---|---|---|
| cluster precision | 0.980 | 0.981 | 0.978 | ≥0.9 (both variants) ✓ |
| cluster recall | 1.000 | 0.996 | 1.000 | ≥0.6 (hold-out) ✓ |
| same-cluster copies counted | 0 | 0 | 0 | ZERO ✓ |
| copy false-split (sketch) | 0 | 0 | 0 | ZERO ✓ |
| indep-restatement distinct rate | 0.993 | 0.989 | 0.964 | (recall ceiling) |

τ_corr frozen = **3.0** (raw ms-marco logit; corpus-specific — operators
re-derive it from their own `answer_trust` run). MDE₈₀(precision) ≈ **0.031**, so
the 0.9 bar is meaningful (precision 0.98 ≫ 0.9 + MDE). H3 abstention re-confirmed
in the same run (selective hit 0.904 vs 0.734 baseline at 81% coverage; no >10pp
style collapse). A few adjacent-chaff clusters clear τ (chaff_counted 4–6/variant)
— the honest residual of an unsigned, support-only CE signal — but precision
holds far above the bar.

## Budget

- Corroboration adds exactly ONE CE batch per answer over the distinct
  other-clusters (mean **~4.7** clusters/query → ~5 short pairs). Measured
  on-device (the Arm-B batch IS the production op, n=1080):
  corroboration-batch p50 = **311.6 ms**, p99 = **508.2 ms**.
- **The roadmap's "~10–20 ms" estimate was WRONG** — ms-marco INT8 on the A76 is
  ≈60 ms/pair, so ~5 pairs ≈ 311 ms. Recorded honestly (measure, don't assume).
- Answer p50 budget: baseline **2502 ms** (cap 8, 02-budgets.md) + **312 ms** ≈
  **2813 ms ≤ 3.0 s** ✓ (headroom now ~187 ms, down from ~498 ms). The feature
  caps the CE deadline at **450 ms** so even the deadline-bounded worst case
  (2502+450 ≈ 2952 ms) stays under 3.0 s; on overrun the count thins (never
  inflates). p50 is the gated budget row and it holds.

## Reproduce

```
cargo run -p meridian-eval --bin meridian-bench --features bench-ce-real -- \
  answer_trust --models-dir models --out bench-out      # gnu, needs models/
cargo test -p meridian-eval --lib bench::answer_trust   # Arm A + clustering (CI)
cargo test -p meridian-query corroboration              # counting + exclusion unit
```
