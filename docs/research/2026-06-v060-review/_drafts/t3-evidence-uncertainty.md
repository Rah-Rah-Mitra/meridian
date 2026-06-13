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
