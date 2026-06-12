# VoI fetch & stopping — design requirements (Phase 9 / WBS 9.6–9.7, ADR-26)

Status: DESIGN SETTLED 2026-06-12 — written so implementation can start
without re-litigating scope. The pure selection core (`meridian-fetch::voi`)
ships first with its unit gates; the deep-mode wiring and suite 15 land
together; the ingest-frontier hook is explicitly deferred.

## 1. What exists today (verified against code, not memory)

- **Deep mode does NOT fetch.** `mode=deep` cross-encoder-reranks the top-20
  (title, snippet) pairs (`planner.rs`); there is no query-time page fetch
  anywhere in the search path. ADR-26's "deep-mode fetch-candidate selection"
  therefore lands as a NEW, opt-in capability, not a swap of an existing one.
- The fetch ladder (`meridian-fetch`) already provides everything a
  query-time fetch must pass through: SSRF vet → per-domain budget → robots →
  pinned GET → size-capped stream → extraction; raw bodies are never kept.
- Phase-7 sketches expose `Sketch::containment` — the novelty signal — and
  sketches are computable in-memory from extracted text (no ingest needed).
- There is no `analysis` response block yet; it is additive under ADR-20.

## 2. Scope decisions (the requirements)

1. **Opt-in surface:** `mode=deep&fetch_budget=N` (new param; default 0 =
   exactly today's behavior; hard cap `search.deep_fetch_max`, default 2).
   With N>0 the planner may fetch up to N of the top-K web candidates to
   re-score them on full text instead of snippets.
2. **Privacy line (must ship in privacy.md with the code):** query-time
   fetching creates per-query egress to RESULT domains — observable by those
   domains. Direct lane only in v0.4.0 (an anon-lane fetch ladder exists, but
   Tor fetch latency cannot fit the deep 2.5s budget; revisit with async
   compare in Phase 10). Never auto-triggered; `fetch_budget` is per-request
   and explicit. Fetched text is used in-RAM for scoring and sketching only —
   **a search never ingests, persists, or caches what it fetched** (the
   fetch cache is the ladder's own 24h extract cache, same as /v1/fetch).
3. **Selection = Pandora's box (Weitzman):** candidate i gets a reservation
   index `z_i = g_i − c_i / p_i` (the closed form of
   `c_i = p_i · (g_i − z_i)` under a two-point value model: with probability
   p_i the fetch realizes gain g_i, else 0). Open candidates in decreasing
   z_i; STOP when the best realized value so far ≥ the next candidate's z
   (the optimal rule), or the budget/deadline runs out.
4. **Value model, no generative anything (ADR-26):**
   - `novelty_i` = 1 − max sketch-containment vs already-fetched docs and
     vs the head's existing evidence clusters → spending budget on a copy of
     something already read is worthless, and the suite-15 diversity guard
     (median `independent_source_count` non-degrading) enforces it.
   - `coverage_i` = 1 − max cosine(embed(title+snippet_i), selected set) —
     potion embeds, microseconds.
   - v1 blend: `p_i = clamp(β0 + β1·novelty_i + β2·coverage_i)`,
     `g_i` = DCG mass between candidate's current rank and rank 1 (the most
     it could matter), `c_i` = expected ladder latency (robots-cached vs
     cold, per-domain budget state) normalized to the deep deadline.
   - β/α constants are FIXED by the suite-15 sweep on a tuning seed and
     judged frozen on a hold-out seed (risk-#21 protocol, same as ADR-18/21
     — never hand-picked).
5. **Response surface:** deep responses with `fetch_budget>0` carry an
   additive `analysis` block (`schema: 1`): `{ fetches_made,
   search_stopped_because: value_below_reservation | budget_exhausted |
   deadline, estimated_marginal_gain_remaining }` — the honesty contract:
   the engine SAYS why it stopped reading.
6. **Gates (SPEC §16 P9, unchanged):** suite 15 replay shows ≥25% fewer
   fetches at equal nDCG@10 (±1%) vs fetch-all-top-N, with non-degrading
   median `independent_source_count`; deep p50 ≤2.5s holds on-device with
   the default cap.
7. **Ingest frontier: DEFERRED.** Today's ingest is operator-push
   (`/v1/ingest`, external crawl scripts); there is no in-process frontier
   queue to reorder. The VoI core is written frontier-agnostic
   (`Candidate { id, p, gain, cost }`) so the hook drops in when an ingest
   queue exists — recorded as the deviation from ADR-26's "(and the ingest
   frontier)" wording rather than inventing a queue nobody operates.

## 3. Suite 15 — hermetic replay design (no network, CI-runnable)

Reuses the suite-13b generator machinery. Per query: the generator plants a
known-item answer in the FULL BODY of `d` decisive docs while keeping their
snippets ambiguous (snippet = body prefix; the decisive term is placed past
the snippet cut), plus syndicated near-copies of decisive docs (the
diversity trap: fetching two copies of the same decisive doc wastes budget
and collapses `independent_source_count`).

Replay loop: rank from snippets (the shipped path) → VoI selects fetch order
under budget b ∈ {1, 2, 4, ∞} → "fetch" = reveal the full body from the
corpus → re-score revealed docs (deterministic full-text BM25 re-query; CE
is not available in CI and is not what the gate is about) → measure nDCG@10,
fetches used, distinct evidence clusters fetched. Baselines: (a) fetch ALL
top-N, (b) rank-greedy top-b. Gate: VoI reaches baseline-(a) nDCG within ±1%
using ≥25% fewer fetches, and its fetched-cluster count ≥ baseline-(b)'s
(the novelty term must beat rank-greedy on diversity).

## 4. Implementation order

1. `meridian-fetch/src/voi.rs` — pure core (this PR): `Candidate`,
   `reservation_index`, `pandora_walk` (ordering, optimal stop, budget stop,
   `WalkResult { opened, stopped_because, est_gain_remaining }`), unit gates
   for the stopping rule including the cheap-likely-beats-expensive-big
   ordering case.
2. Suite 15 generator + replay (`meridian-eval/src/bench/voi.rs`) against
   the core, constants swept and frozen (tuning/hold-out seeds).
3. Planner wiring behind `fetch_budget` + `analysis` block + privacy.md /
   api.md / operator-manual in the same train; device re-validation of the
   deep 2.5s gate.
4. v0.4.0 exit: suite-15 gate row + the deferred-frontier deviation note.
