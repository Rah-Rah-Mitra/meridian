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
      "ts": 1749600000
    }
  ],
  "timings": { "plan": 1, "lexical": 9, "fusion": 2, "total": 14 },
  "lane_requested": "direct",
  "lane_effective": "direct",
  "degraded": ["searx_timeout"]
}
```

`h3` (res-7 cell of geo-tagged local docs) and `ts` appear only when the
document has them. `degraded` lists stages that timed out or were skipped —
results are still served honestly labeled.

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
{ "res": 5, "cells": [ { "h3": "851f1d4bfffffff", "count": 42, "lat": 52.5, "lon": 13.4 } ] }
```

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
  "top_movers": [ { "root": 14, "latest": 25, "mean": 11.5, "ratio": 2.17 } ]
}
```

`series` is `(day, count)` ascending, where `day` is days since the Unix
epoch; `top_movers` ranks root codes by latest/mean ratio over the window.

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

## GET /healthz

`{ "status": "ok", "uptime_secs": n }`. Unauthenticated, not rate-limited.

## GET /metrics

Prometheus exposition. Aggregate-only by construction: label cardinality is
bounded to route/status/stage/lane/store — never per-IP, per-user, or
per-query.
