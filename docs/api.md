# Meridian API Reference (v0.1)

Base URL: `http://127.0.0.1:8080` (loopback by default — see the
[operator manual](operator-manual.md) before exposing further).

All errors are RFC 9457 problem+json:

```json
{ "type": "about:blank", "title": "invalid geo constraint", "status": 400, "detail": "use lat+lon(+radius_km) OR h3, not both" }
```

Every response carries a random per-request `x-request-id` header (no
correlation across requests). Requests on `/v1/*` are rate-limited per hashed
client IP (default 5 rps, burst 20 → `429`) and shed at the concurrency cap
(default 8 in flight → `429`). The whole-request ceiling (default 12 s)
returns `504`.

## Authentication

Mutating / egress-driving endpoints (`/v1/ingest`, `/v1/fetch`, `/v1/forget`)
require `Authorization: Bearer <token>`. The token is supplied via the
`MERIDIAN_BEARER_TOKEN` env var (compose: `deploy/.env`, mode 0600) or
`auth.token_file`. With `auth.require_bearer_for_search = true`, `/v1/search`
is gated too (recommended when exposed beyond localhost). If no token is
configured, gated endpoints answer `503 auth not configured`.

---

## GET /v1/search

| Param | Type | Default | Notes |
|---|---|---|---|
| `q` | string | required | Query text. Never logged. |
| `mode` | `fast` \| `deep` | `fast` | `deep` adds the cross-encoder rerank stage. |
| `scope` | `local` \| `web` \| `both` | `both` | `local` = on-device index only; `web` = metasearch only. |
| `lane` | `direct` \| `anon` \| `region:<id>` | `direct` | See lane semantics below. |
| `limit` | int | 10 | Capped at `search.max_limit` (50). |
| `lat`, `lon` | float | — | Geo filter centre. Both or neither; needs `radius_km` semantics below. |
| `radius_km` | float | 25 | With `lat`+`lon`. Clamped to 250 km. |
| `h3` | string | — | Alternative to lat/lon: one H3 cell (hex like `871f1d489ffffff`, or decimal). Mutually exclusive with `lat`/`lon`. |
| `after`, `before` | int | — | Unix-seconds document-timestamp window (inclusive). |
| `compare` | `vantages` | — | v0.2.0: run the query over direct AND anon and attach the `divergence` block. Requires `scope=web`; the `lane` param must be omitted (compare governs lanes). See the privacy note below. |

A `diversity=mmr` parameter was built for v0.3.0 and **withdrawn before
release**: its own gate (suite 13b, two generator seeds) showed token-overlap
MMR demotes canonical originals along with their near-duplicate copies —
alpha-nDCG gains came only at >1% plain-nDCG cost at every λ tried. An
evidence-cluster-aware diversifier (reusing the Phase-7 sketch clusters) is
the planned replacement.

Geo/time filters apply to **local results only** — the SearXNG fan-out cannot
be geo-filtered (recorded tradeoff, ADR-10). Under a geo/time filter the local
retrieval path is exact lexical (ANN is skipped: usearch has no filtered
search). A geo/time filter with `scope=web` is refused (`400`) rather than
silently ignored; with `scope=both` the web portion is unfiltered and the
response carries `degraded: ["geo_web_unfiltered"]`.

Lane semantics: `anon` is fail-closed — while Tor is bootstrapping or down the
request gets `503` (never a silent direct fallback); concurrent anon searches
beyond `lanes.anon.max_concurrent_searches` also get `503 anon busy`.
`region:<id>` refuses (`503`) until egress-IP verification passes.

Response:

```json
{
  "results": [
    {
      "url": "https://…", "title": "…", "snippet": "…", "score": 0.83,
      "rank_signals": { "bm25": 12.1, "ann": 0.71, "rrf": 0.032, "ltr": 0.83, "freshness": 0.62, "geo": 0.9, "domain_prior": 0.4 },
      "source": "local",
      "h3": 608533319155138559,
      "ts": 1749600000,
      "evidence": { "cluster": 0 }
    }
  ],
  "timings": { "plan": 1, "lexical": 9, "fusion": 2, "evidence_ms": 0, "total": 14 },
  "lane_requested": "direct",
  "lane_effective": "direct",
  "degraded": ["searx_timeout"],
  "evidence": {
    "schema": 1,
    "independent_source_count": 3,
    "apparent_source_count": 10,
    "sketched_results": 7,
    "clusters": [ { "id": 0, "members": 5, "domains": 2 } ]
  }
}
```

`h3` (res-7 cell of geo-tagged local docs) and `ts` appear only when the
document has them. `degraded` lists stages that timed out or were skipped —
results are still served honestly labeled.

**Divergence block** (`compare=vantages`, v0.2.0 preview of Phase 8, ADR-22):
the response is the DIRECT half's results plus:

```json
"divergence": {
  "schema": 1, "lanes_compared": ["direct", "anon"],
  "jsd": 0.41, "noise_floor_p90": 0.30, "exceeds_floor": true,
  "domains_only_in_direct": ["example-a.com"],
  "domains_only_in_anon": ["example-b.org"],
  "anon_result_count": 18, "jitter_applied_ms": 12040
}
```

Semantics: `jsd` is the Jensen-Shannon divergence (bounded [0,1]) between the
two lanes' registered-domain distributions; `exceeds_floor` compares it to the
deployment's measured same-lane noise floor (`search.compare_noise_floor_p90`)
— a per-request signal, not a population claim. Both halves bypass the query
caches and pin instance-default engines (bandit arm churn alone measures p90
JSD 0.67 — pinning makes the halves differ by vantage only). Fail-closed: an
unavailable anon lane errors the whole request (`503`), never a silent
direct-only answer. **Privacy:** compare mode intentionally sends the same
query over Tor AND directly within one window — explicitly opt-in, jittered
(`search.compare_jitter_ms_max`, default on), see privacy.md.

**Confidence block** (v0.3.0, additive, ADR-23): every search response carries
raw query-performance predictors —

```json
"confidence": { "schema": 1, "nqc": 0.84, "clarity": 1.92, "score": 0.61 }
```

`nqc` is the dispersion of the top results' fused scores against the candidate
pool (a confident head separates); `clarity` is the KL divergence of the top
results' vocabulary against the pool's (a focused result set reads
distinctively). `score` is an **uncalibrated** [0,1] blend — comparable across
queries on one deployment, NOT a probability; calibration against measured
nDCG lands with the v0.3.0 eval suite. Honest use: treat low scores as "verify
before trusting", not high scores as "true".

**Evidence block** (v0.2.0, additive — absent when `[evidence] enabled =
false`): results are clustered by text-derivation similarity (MinHash
containment over ingest-time sketches); each cluster is one apparent ORIGIN, so
`independent_source_count` < `apparent_source_count` means some results are
copies/syndications of each other, not independent corroboration. A cluster
with `members > domains` is cross-domain syndication — the case plain domain
grouping cannot see. **Honesty contract:** only results whose documents were
ingested (and therefore sketched from full text) participate; web results
never fetched carry `evidence: null` and are excluded from the count —
independence is never guessed from a snippet. `sketched_results` states the
basis. Documents ingested before v0.2.0 lack sketches until re-ingested.

## POST /v1/ingest  (bearer)

Body: JSON array (max `ingest.batch_max`, default 100) of items, each either
inline text or a URL to fetch through the ladder:

```json
[
  { "text": "…document body…", "title": "…", "url": "https://optional-canonical", "ts": 1749600000 },
  { "url": "https://fetch-me.example/page", "lane": "direct" }
]
```

Returns `202` with `{ "accepted": n, "deduped": n, "queued": 0 }`.
Content-hash duplicates are deduped; documents previously erased via
`/v1/forget` are **refused** (tombstoned) and count toward neither. During
resource-pressure shedding ingest answers `503 ingest paused`.

Ingested text is geo-tagged via the offline gazetteer when
`models/gazetteer.fst` is present (title + lead 500 chars, capitalized-phrase
match), populating `h3`/heatmaps. No network call is made at ingest time.

## GET /v1/fetch?url=…&lane=…  (bearer)

One-off fetch+extract through the fetch ladder (robots-respecting, 5 MB cap,
per-domain politeness budget). Returns `{ url, title, text, http_status }`.

## GET /v1/lanes

Lane health, honestly reported:

```json
[
  { "id": "direct", "status": "up", "detail": null },
  { "id": "anon", "status": "bootstrapping", "detail": "45%" },
  { "id": "region:sg", "status": "degraded", "detail": "egress IP mismatch" }
]
```

`status` ∈ `up | bootstrapping | degraded | down | disabled`.

## GET /v1/geo/heatmap

| Param | Default | Notes |
|---|---|---|
| `q` | — | Optional query; omitted = whole corpus. |
| `res` | 5 | H3 resolution 3..=7 (docs are indexed at res 7, rolled up to coarser cells). |
| `window` | `7d` | `24h` / `7d` / `all`. Windows narrower than `all` exclude docs without a timestamp. |

```json
{ "res": 5, "schema": 1, "cells": [
  { "h3": "851f1d4bfffffff", "count": 42, "lat": 52.5, "lon": 13.4,
    "z": 3.1, "q_value": 0.004, "significant": true }
] }
```

`z` / `q_value` / `significant` (v0.2.0, ADR-21): Getis-Ord Gi* hot-spot
statistics over each cell's H3 k-ring-1 neighborhood with Benjamini-Hochberg
FDR across all returned cells. `significant` (q ≤ 0.05) is the defensible
"this is a hot spot" flag; a big raw `count` alone is not.

## GET /v1/trends

Requires `[analytics] enabled = true` (GDELT opt-in), else `404`.

| Param | Default | Notes |
|---|---|---|
| `topic` | all | GDELT EventRootCode 1–20. |
| `h3` | all | H3 **res-5** cell (hex or decimal). |
| `window` | `7d` | Same syntax as heatmap. |

```json
{
  "series": [[20608, 17], [20609, 25]],
  "top_movers": [
    { "root": 14, "latest": 25, "mean": 11.5, "ratio": 2.17,
      "shrunk_rate": 18.2, "z": 4.7, "q_value": 0.001, "significant": true },
    { "root": 3, "latest": 4, "mean": 1.2, "ratio": 3.33,
      "shrunk_rate": 2.1, "z": 1.1, "q_value": 0.41, "significant": false,
      "label": "likely low-sample noise" }
  ]
}
```

`series` is `(day, count)` ascending, where `day` is days since the Unix
epoch. `top_movers` (v0.2.0, ADR-21) ranks root codes by `z` — an
empirical-Bayes-shrunk, overdispersion-aware standardized excess of the latest
day over the window baseline, with Benjamini-Hochberg FDR across roots.
`significant` (q ≤ 0.05) is the defensible "this moved" flag; `ratio` is the
raw latest/mean kept for explainability, and an elevated ratio WITHOUT
significance carries the `label` honesty marker. GDELT caveat: all trends
describe *media coverage*, not ground truth about the world (ADR-15).

## POST /v1/forget  (bearer)

Operator deletion path — see the [privacy guide](privacy.md#deletion) for
semantics. Exactly one selector:

```json
{ "url": "https://…" }
{ "domain": "example.com" }
{ "content_hash": "32-hex-chars" }
{ "url": "https://…", "purge_caches": false }
```

Removes matching documents from the lexical index and the vector store,
tombstones their content hashes so re-ingest is refused, and (default)
purges the query + fetch caches. Returns `{ "removed": n, "caches_purged": true }`.
The selector itself is never logged.

## GET /v1/decision-log · POST /v1/decision-log/wipe  (bearer)

Operator surface for the ADR-24 per-decision routing log (see the
[privacy guide](privacy.md#decision-log-v030-off-by-default-adr-24)). Both
return `404` while `searx.decision_log` is `false` (the default). `GET`
reports `{ enabled, rows, approx_bytes, oldest_day, newest_day,
retention_days, max_bytes }`; `wipe` drops every retained row and returns
`{ "removed": n }`.

## GET /v1/decision-log/ope  (bearer)

The ADR-25 ship-gate report: trains the linear-TS candidate on the older 80%
of the retained log and reports its doubly-robust uplift vs the incumbent's
realized reward on the held-out 20%, with a bootstrap 95% CI. The `gate`
block names the verdict — `pass` only when the CI excludes zero from above
on ≥10k decisions; `inconclusive` and `negative` mean ε-greedy stays (the
ADR-25 sunset rule). Reporting only — it never changes routing.

## GET /healthz

`{ "status": "ok", "uptime_secs": n }`. Unauthenticated, not rate-limited.

## GET /metrics

Prometheus exposition. Aggregate-only by construction: label cardinality is
bounded to route/status/stage/lane/store — never per-IP, per-user, or
per-query.
