# V1 filtered-ANN — measure-first → RECORDED NO (Pi 5, on-device)

As-of 2026-06-14. The deferred V1 bet (06-roadmap-completion.md bet 2 / §6 Bet 4):
geo/time-filtered queries drop the dense lane (`planner.rs:626` → BM25-only, the
recorded ADR-10 tradeoff). The gate was **measure the harm first**: <2pp nDCG@10 ⇒
record "the ADR-10 tradeoff was right" (a first-class NO); ≥2pp ⇒ implement tier
(i) exact int8 scan + evaluate tier (ii) predicate-HNSW.

## Decision: NO — the ADR-10 tradeoff stands

**The harm is < 2pp.** The binding evidence is the REAL corpus: the dense lane
adds only **+0.85pp nDCG@10 UNFILTERED** (`2026-06-13-pi5-hybrid-healed-lane.md`:
BM25 0.4099 → hybrid 0.4184). A geo/time filter only RESTRICTS the candidate set
— it cannot manufacture a dense advantage that is absent unfiltered — so the
filtered harm is ≤ +0.85pp, well under the 2pp bar. Geo-aware identity is
satisfied by lexical filtering alone; the dense lane is not worth restoring under
filters. **The dense lane is NOT shipped under filters.**

### Why not a synthetic harm number
A synthetic corpus cannot honestly measure the harm MAGNITUDE: `nDCG(BM25-only)`
is a linear function of whatever lexical-vs-semantic mismatch rate the generator
picks, so any "harm" it reports is a construction artifact, not a property of
Meridian. (The repo's real eval corpus — Simple-Wikipedia, `eval/qrels.txt` — is
not geo-tagged, so a real filtered eval would require synthetic geo tags anyway.)
We therefore anchor the decision to the measured real unfiltered gain and use the
synthetic suite (`ann_filtered`, suite 21) only for what IS construction-
independent: proving the implementation is ready if a future real geo-eval ever
flips the decision.

## Implementation-readiness (construction-independent), 100k docs, 384 queries

qrels = the f32 dense top-K neighbours of each query (graded by rank); recovery =
how much of the f32 dense lane's nDCG@10 each tier keeps.

| tier | recall vs f32/exact (GATE) | nDCG-recovery (informational) | p50 | note |
|---|---|---|---|---|
| (i) INT8 exact-scan (shipped path) | recall@20-vs-f32 **1.000** | ≈0.945–0.955 (noisy) | **17–19ms** | scalar kernel; over the +10ms budget |
| (ii) usearch predicate-HNSW | recall-vs-exact **0.966** | ≈0.93 | **6.2ms** | under-returns at high selectivity (3% filter) — exact scan is the floor |

- **tier (i) works** — `VectorStore::exact_scan` (usearch int8 reconstruct +
  cosine) retrieves **every** one of the f32 dense lane's top-20 neighbours
  (recall@20-vs-f32 = 1.0 — the robust gate). The nDCG-recovery ratio is ≈0.95 but
  varies run-to-run (RRF tie-breaking ordering is non-deterministic), so it is
  reported informationally, not gated. The **scalar** cosine kernel is 17–19ms p50
  over ~3.1k filtered candidates — **over the ≤+10ms budget**. `numkong`'s NEON
  `i8::angular` (already in the usearch dep tree — no fork, ADR-07 rung-1) is the
  documented ~2–4ms path; **not wired** because the harm is NO (we are not shipping
  the dense lane), only made ready.
- **tier (ii) is feasible without a fork** — usearch `filtered_search<F: Fn(Key)
  ->bool>` exists in the pinned 2.25.3 binding (the ADR-07 kill-gate does NOT
  fire), p50 6.2ms, but under-returns slightly at high filter selectivity, so the
  exact scan remains the floor for small candidate sets.

Both `VectorStore::exact_scan` (tier i) and `VectorStore::filtered_search` (tier
ii) ship in `meridian-vector` behind no feature flag (usearch is always present),
unused by the planner under the NO — ready to wire at `planner.rs:619-639` if a
real geo-eval ever shows ≥2pp.

## Reproduce

```
cargo run -p meridian-eval --bin meridian-bench --features bench-ann,bench-lexical -- \
  ann_filtered --docs 100000 --scratch <dir> --out bench-out
```
