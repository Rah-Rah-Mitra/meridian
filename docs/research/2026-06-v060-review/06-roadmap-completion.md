# v0.6.0 roadmap — completion record + the two deferred bets (C1, V1)

As-of: 2026-06-13, post-implementation. Companion to `00-review.md`. Records what the
implementation pass shipped/recorded, and the **complete, ready-to-execute design** for
the two bets deliberately deferred for a dedicated session (C1 has a kill criterion; V1
needs a heavy build + a fresh measurement).

## What shipped / was recorded (experiment-first, all on `main` unless noted)

| Bet | Outcome | Where |
|---|---|---|
| E3 seasonal baselines | **RECORDED NO** — DOW deseasonalisation worsens FPR (dispersion already absorbs the weekly variance); 3 adversarial verifiers couldn't refute | suite 19 `seasonal`, `main` |
| J3 MDE pre-registration + J1 knob Pareto | shipped (docs) | `04-bench-plan.md §8`, `main` |
| Answer-trust **judge** (C1+H3) | **PASS** (corroboration 0.98 precision / 0 leakage; abstention +17pp) | suite 20 `answer_trust`, `main` |
| **H3 abstention** | **SHIPPED** — `answer_abstain_threshold`, withhold + `answer_below_threshold`, honest api.md | `feat/answer-trust-feat` → `main` |
| B1 answer-mode embedding-prune | **RECORDED NO** — kill-shape; budget-2 stops early so no redundant-fetch headroom; closes the 15b nomination | suite 18b `answer_embed`, `main` |
| F1 forget remediation | **SHIPPED** — `forget_domain` drain loop (G2), privacy.md residual + usearch finding (G1), tombstone note (G3) | `feat/f1-forget` → `main` |
| Analytics UI | **SHIPPED + live-smoke verified** — embedded `/ui/*`, 5 panels, zero-dep, honest | `feat/analytics-ui` → `main`, served on `:8090` smoke |
| A2 graded reward | **SHIPPED** (log-both into reserved `row[11]`; switch organic-gated + ADR-24 re-sign-off) | `feat/a2-graded-reward` (pushed) |
| **K1 Hailo offload** | **RECORD-NO-GO (measured)** — no retrieval HEF (x86 DFC only) + Gen2-x1 link benchmarks ~24 FPS resnet50 (link-bound), so ≥2× bar unlikely | `feat/k1-hailo-probe` (pushed) |

Net: 3 features shipped (H3, F1, UI) + A2 (pushed), 3 recorded-NOs (E3, B1, K1) — the
experiment-first discipline earned its keep, and no method the repo killed was re-proposed.

## Deferred bet 1 — C1 claim-level corroboration (the answer-trust other half)

**Why deferred, not rushed:** §8.2 is explicit — *a false "k independent sources agree"
badge is worse than none* (the conformal lesson at claim level), and the production
support-signal differs from the suite-20 judge's synthetic `support_score`, so it needs
its own validation arm before ship. This is the one bet where haste is dangerous.

**Complete production design (worked out; ready to implement):**
1. **Track the winner.** At the `best_passage` set site (`planner.rs:~1238`) capture
   `best_idx = Some(idx)` alongside the block.
2. **Compute after the evidence stage.** The ADR-18 `annotate(&mut results, &found)` at
   `planner.rs:~1368` populates `results[i].evidence: Some(ResultEvidence{cluster, canonical})`.
   Compute corroboration *after* it, when both the winner and the result-set clusters exist:
   - `c_w = results[best_idx].evidence?.cluster`.
   - For each DISTINCT cluster `c != c_w` present in `results`, take its top (canonical, else
     highest-ranked) member's snippet.
   - **Support test (the principled signal):** reuse `self.reranker.rerank(best_passage.text,
     [those snippets], deadline)` — one CE batch, ~10–20ms (within the ~500ms answer
     headroom). A cluster supports iff its CE ≥ a **conservative** bar (e.g. CE ≥ τ_corr AND
     CE ≥ best_passage.ce_score − δ). Textual-containment support (i) is moot here: a
     containment-≥τ doc is in the SAME cluster by construction, so it is already excluded.
   - `independent_clusters = count of supporting distinct clusters`; `supporting_urls = their
     top members' urls`.
3. **Additive block** (ADR-20, own schema):
   ```
   best_passage.corroboration = {
     schema: 1, independent_clusters: u32, supporting_urls: [..],
     basis: { candidates_checked: u32, method: "ce-cross-cluster" }
   }
   ```
   `basis.candidates_checked` is the honesty payload (how many distinct other-clusters were
   *sketched/checkable*) — a thin-coverage result degrades to a low count, never a false badge.
   Same-cluster copies are excluded **structurally** (`c != c_w`), so syndication can never
   inflate the count.
4. **api.md:** additive `corroboration` block; wording = "k *independent evidence clusters*
   whose passage the cross-encoder finds states the same claim — never a copy; absent ⇒ no
   independent support found, not 'no answer'."
5. **The binding gate — extend suite 20 (`answer_trust`) with the PRODUCTION signal** before
   ship: replace the synthetic `support_score` arm with a CE-modelled support arm (plant
   independent-2nd-originals whose snippet CE-supports vs syndicated copies vs chaff), re-assert
   **precision ≥0.9 / recall ≥0.6 / ZERO same-cluster leakage** on tuning + style-shift, and a
   `τ_corr` sweep frozen on the hold-out. Ship default-ON only if precision ≥0.9 on BOTH
   variants; otherwise record the NO. Plus a unit test of the counting + same-cluster exclusion.
6. **Budget re-check:** answer p50 row holds (the extra CE batch is ≤20ms; measure n≥16).

**Accept:** suite-20 production arm precision ≥0.9 (both variants) + recall ≥0.6 + zero
leakage + answer p50 holds. **Kill:** precision <0.9 on either variant, or any same-cluster
leakage → keep the page-set-level `independent_source_count` only.

## Deferred bet 2 — V1 filtered ANN (measure the harm first)

**Why deferred:** it opens with a *measurement* (no kill-criterion risk), but it needs the
heavy `bench-ann` (usearch C++) build + a fresh filtered-query harness — a clean
fresh-session task, not an end-of-marathon one.

**Plan (per §6 Bet 4 — the gate is "measure harm first"):**
1. Extend `bench/ann.rs` + `bench/fusion.rs` with a **geo/time-filtered query set** at
   100k/1M: a corpus carrying H3 cells + ts, queries with a geo/ts predicate, qrels.
2. Measure **nDCG@10 of the missing dense lane**: hybrid-without-filtered-dense (today:
   `planner.rs:624` drops ANN under any filter → BM25-only) vs hybrid-with an exact int8 scan
   over the filtered candidate set (H3 TermSet → doc ids → NEON cosine, ≈2–4ms ≤50k cands).
3. **If the nDCG delta < 2pp → DEPRIORITISE and record "the ADR-10 tradeoff was right"** (a
   first-class result — geo-aware identity is satisfied by lexical filtering alone). **If ≥2pp**
   → implement tier (i) (the exact scan, trivial) and gate tier (ii) (predicate-aware HNSW)
   strictly on usearch exposing a filter callback **without a fork** (ADR-07 ladder discipline);
   accept ≥95% of the dense contribution recovered at ≤+10ms p50 @100k; kill on a required fork
   or a vector-budget-row bust.

## Operational note (this implementation pass)

Worked entirely in git worktrees (`meridian-{e3,b1,f1,ui,k1,c1}`) sharing one `target/` —
the user's `docs/agent-skills` main tree was never touched. A mid-pass power outage killed
the live appliance stack; it was restored from both compose files (v5 volume preserved, no
re-ingest) and verified serving before work resumed. Disk on the SD card is the binding
constraint — `cargo clean` between heavy build waves.
