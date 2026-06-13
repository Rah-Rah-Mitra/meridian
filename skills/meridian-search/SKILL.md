---
name: meridian-search
description: "Query a running Meridian edge-search appliance over HTTP — web/local/both scope, egress lanes (direct/anon/region), extractive answer mode (best_passage, verbatim + cited, relevance not correctness), and source-independence evidence (N syndicated copies are not N sources). Copy-paste curl recipes for humans and tool-call guidance for agents."
version: 1.0.0
author: Meridian
license: MIT
platforms: [linux]
metadata:
  hermes:
    tags: [Search, Retrieval, Web, Evidence, Privacy]
    related_skills: [meridian-operate]
---

# Meridian search (the consumer skill)

Drive a running Meridian appliance's `GET /v1/search` to retrieve web and/or
on-device results, optionally extract a single best answer passage, and read
the honesty signals (corroboration, degraded stages) that come back with every
response. **Extractive and explainable only** — Meridian never generates prose;
it returns documents, ranked, with their signals exposed.

## When to use
- You need fresh web results, on-device index results, or both, behind one
  privacy-conscious HTTP contract.
- You want a single cited passage to start reading from (answer mode).
- You need to know how many *independent* sources actually back a result.

## When NOT to use
- Ingesting, forgetting, or inspecting the decision log → use `meridian-operate`.
- You expect a generated/summarized answer — Meridian is extractive; the
  `best_passage` block is verbatim from one page, never synthesized.

## Setup (read these from the environment — never hardcode the token)

```sh
export MERIDIAN_URL="http://127.0.0.1:8080"
export MERIDIAN_BEARER_TOKEN="…"   # value of MERIDIAN_BEARER_TOKEN in deploy/.env
```

`/v1/search` may be gated by a bearer token (when
`auth.require_bearer_for_search = true`, recommended off-localhost). Always
send the header — it is harmless when search is open and required when gated.
**Never** print, log, or commit the literal token; only use the
`$MERIDIAN_BEARER_TOKEN` placeholder.

## The request

```
GET /v1/search?q=<query>&scope=<web|local|both>&lane=<direct|anon|region:id>&limit=<1..50>
```

| Param | Default | Notes |
|---|---|---|
| `q` | required | Query text. Never logged by the appliance. |
| `scope` | `both` | `web` = metasearch only · `local` = on-device index only · `both`. |
| `lane` | `direct` | `direct` · `anon` (Tor, fail-closed) · `region:<id>`. |
| `limit` | `10` | 1..50. |
| `mode` | `fast` | `deep` adds the cross-encoder rerank; required for `answer`. |
| `fetch_budget` | `0` | `mode=deep` + `lane=direct` only: read up to N result pages on full text. |
| `answer` | `false` | Extractive answer mode; requires `fetch_budget ≥ 1` (and deep, direct). |

### Human recipe — basic web search

```sh
curl -s -G "$MERIDIAN_URL/v1/search" \
  -H "Authorization: Bearer $MERIDIAN_BEARER_TOKEN" \
  --data-urlencode "q=raspberry pi 5 power draw" \
  --data-urlencode "scope=web" \
  --data-urlencode "lane=direct" \
  --data-urlencode "limit=5"
```

### Agent tool-call guidance

Build the same call as your HTTP/tool primitive:

- Method `GET`, path `/v1/search`, header `Authorization: Bearer $MERIDIAN_BEARER_TOKEN`.
- URL-encode `q`. Set `scope`, `lane`, `limit` per the table above.
- Parse the JSON `results[]`; **the per-result text field is `snippet`, NOT
  `content`**. Map `snippet` → your tool's description/content field.
- A result is one object: `{ url, title, snippet, score, source, rank_signals, ts?, h3?, evidence }`.
  `source` ∈ `local | web | both`. `evidence` is `{ "cluster": int, "canonical": bool }`
  or `null` (see below).

## Reading the response

```json
{
  "results": [
    { "url": "https://…", "title": "…", "snippet": "…", "score": 0.83,
      "source": "web", "rank_signals": { "bm25": 12.1, "rrf": 0.03, "ltr": 0.83 },
      "evidence": { "cluster": 0, "canonical": true } }
  ],
  "timings": { "total": 14 },
  "lane_requested": "direct",
  "lane_effective": "direct",
  "degraded": ["searx_timeout"],
  "evidence": {
    "schema": 2,
    "independent_source_count": 3,
    "apparent_source_count": 10,
    "sketched_results": 7,
    "clusters": [ { "id": 0, "members": 5, "domains": 2 } ]
  }
}
```

- `lane_requested` vs `lane_effective`: confirm you got the lane you asked for.
- `degraded[]`: stages that timed out or were skipped — results are still
  served, honestly labeled. Common flags: `searx_timeout`, `searx_unavailable`,
  `geo_web_unfiltered`, `fetch_unavailable`, `answer_unavailable`, `ann_unavailable`.

## scope: local vs web vs both

- `scope=local` — query the **on-device hybrid index** only (Tantivy BM25 +
  USearch vectors). Use it for corpora you ingested, offline operation, or when
  you must not touch the network. Geo/time filters (`lat`/`lon`/`h3`/`after`/
  `before`) apply to local results only.
- `scope=web` — metasearch fan-out (SearXNG sidecar) only. A geo/time filter
  with `scope=web` is **refused (`400`)** rather than silently ignored.
- `scope=both` (default) — fuse local + web. Under a geo/time filter the web
  half is unfiltered and you get `degraded: ["geo_web_unfiltered"]`.

## Lane semantics

- `direct` (default) — normal egress.
- `anon` — Tor via embedded Arti, **fail-closed**: while Tor is bootstrapping or
  down the request gets `503` (never a silent fallback to direct). Anon/region
  lanes are **never** written to the decision log.
- `region:<id>` — operator WireGuard vantage; refuses (`503`) until egress-IP
  verification passes.

## Answer mode (extractive, ADR-29)

Switch the fetch selector to "single best passage" and read the most promising
pages on full text. Requires `mode=deep`, `lane=direct`, and `fetch_budget ≥ 1`.

```sh
curl -s -G "$MERIDIAN_URL/v1/search" \
  -H "Authorization: Bearer $MERIDIAN_BEARER_TOKEN" \
  --data-urlencode "q=what voltage does the pi 5 require" \
  --data-urlencode "scope=web" \
  --data-urlencode "lane=direct" \
  --data-urlencode "mode=deep" \
  --data-urlencode "fetch_budget=3" \
  --data-urlencode "answer=true"
```

A successful answer attaches a top-level `best_passage`:

```json
"best_passage": {
  "schema": 1,
  "text": "…the sentence-aligned extract, ≤500 chars, verbatim from the page…",
  "url": "https://the-page-it-was-read-from.example/…",
  "ce_score": 7.1
}
```

- The text is **verbatim** from the cited `url` — never generated, never
  stitched across documents.
- **`ce_score` is a RELEVANCE score, NOT a correctness probability.** A
  confidently relevant passage can still be wrong. Treat `best_passage` as "the
  best place to start reading", not "the answer". Always show the `url`.
- If no passage materializes (fetches failed, CE unavailable), the block is
  **absent** and the response carries `degraded: ["answer_unavailable"]` —
  silence never means "no answer exists".

## Evidence: corroboration, honestly (ADR-18 / schema 2)

The top-level `evidence` block (absent when evidence is disabled) clusters
results by text-derivation similarity (MinHash over ingest-time sketches):

- `independent_source_count` < `apparent_source_count` means some results are
  **copies/syndications of each other, not independent corroboration**. N
  near-duplicate copies of one syndicated story collapse into one cluster.
  Read `independent_source_count` as the real number of distinct origins.
- A cluster with `members > domains` is cross-domain syndication (the same
  story republished under several domains).
- **Honesty contract:** only results whose documents were ingested (and thus
  sketched from full text) participate. A web result that was never fetched
  carries `evidence: null` per-result and is **excluded** from the count —
  independence is never guessed from a snippet. `sketched_results` states how
  many results had a sketch.
- Per-result `evidence.canonical: true` marks the cluster member with the most
  shingles — the superset its copies derive from (the one `diversity=evidence`
  promotes). Ties break deterministically by response order.

**Agent guidance:** before asserting "multiple sources confirm X", check
`evidence.independent_source_count`, not `results.length`. If a result's
`evidence` is `null`, do not count it as corroboration.

## Errors and retries

| Status | Meaning | Action |
|---|---|---|
| `400` | invalid params (e.g. geo filter with `scope=web`) | fix the request; **not** retryable |
| `429` | rate / concurrency cap | retry with exponential backoff |
| `503` | anon lane down/bootstrapping, or auth not configured | do not silently fall back to `direct` for `anon`; surface it |
| `504` | whole-request timeout | retry once, or narrow the query / lower `fetch_budget` |

Errors are RFC 9457 problem+json: `{ "type", "title", "status", "detail" }`.
Always inspect `degraded[]` even on `200` — a `200` can still be partial.

## Privacy note (so you query honestly)

A request appends one coarse routing-log row **only** when `lane=direct` AND
`scope` includes web AND engines are unpinned (no `compare`). `anon`/`region`
lanes and `compare=vantages` are never logged; query text, URLs, and IPs are
never stored. **Do not synthesize or script traffic to pad the log** — only
organic searches should drive it (see `meridian-operate` and `docs/api.md`).
