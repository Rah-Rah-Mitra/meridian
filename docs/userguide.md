# Meridian User Guide

A task-oriented walkthrough of every Meridian endpoint. For the terse field-level
reference see [`api.md`](api.md); for privacy/deletion semantics see
[`privacy.md`](privacy.md). Meridian is a privacy-preserving, geo-aware analytical
search appliance — hybrid lexical+dense retrieval over an on-device index plus an
optional metasearch fan-out, with honest "what it left on the table" blocks.

## 1. Getting started

The appliance listens on **`http://127.0.0.1:8080`** (loopback only by default —
exposing it beyond localhost is an explicit operator choice gated behind "enable
auth + TLS first"). Everything below assumes that base URL.

```bash
BASE=http://127.0.0.1:8080
curl -s "$BASE/healthz"          # {"status":"ok","uptime_secs":…}
```

### Authentication
Mutating / egress-driving endpoints — `/v1/ingest`, `/v1/fetch`, `/v1/forget`,
`/v1/decision-log*` — require a bearer token:

```bash
curl -s -H "Authorization: Bearer $MERIDIAN_BEARER_TOKEN" "$BASE/v1/lanes"
```

The token is set via `MERIDIAN_BEARER_TOKEN` (compose: `deploy/.env`, mode 0600)
or `auth.token_file`. With `auth.require_bearer_for_search = true`, `/v1/search`
is gated too (recommended when exposed). If no token is configured, gated
endpoints return `503 auth not configured`. **Query text and selectors are never
logged.**

### The web UI — operator console v2
The embedded operator console is served with no extra dependencies (vanilla ES
modules + hand-rolled `<canvas>`, baked into the binary, no build step, no CDN) at:

```
http://127.0.0.1:8080/ui
```

A left **sidebar** selects one view at a time (hidden views do no network/draw,
so the console stays light); a **topbar** holds the session-only operator token
input, a **light/dark theme toggle** (the only persisted UI state — the theme
name in `localStorage`; the bearer is never stored), and a **version • CE** pill
showing the running image and whether the cross-encoder is live.

Views:

- **search** — the full control set (mode, scope, **lane** direct/anon/region,
  limit, answer, fetch_budget, diversity, compare, geo lat/lon/radius **or** h3,
  time window) plus an **Advanced / Tuning** drawer. The drawer pre-fills every
  per-request knob with this deployment's default, safe range and one-line
  rationale (from `GET /v1/config`); change a value and the URL carries it as
  `ov_<knob>` — the response shows the clamped effective value
  (`applied_overrides`). Renders every response block (results + rank-signal
  breakdown, evidence clusters, confidence gauges, divergence, best passage +
  corroboration, VoI analysis, timings) with every honesty caveat verbatim.
- **metrics** — parses `GET /metrics` client-side: uptime, request totals/rates,
  latency p50/p90/p99 from the histogram, storage/cache occupancy, load-shed
  stages, free disk, GDELT rows.
- **lanes / geo / trends / ope gate** — lane health; the Gi\* hot-spot heatmap
  (now with a `q` filter, colormap legend and per-cell hover); GDELT trends
  (topic / window / h3 selectors); and the ADR-25 OPE ship-gate report
  (`insufficient_data` is a first-class calm state, never an error).
- **deploy** — read-only effective config: image version, **which features are
  live vs dormant** (the cross-encoder gate), the contextual-policy gate
  explained, and every deploy-time setting with the `MERIDIAN_<STRUCT>__<FIELD>`
  env var to change it. The console never mutates the server (it is GET-only).

`/ui/{*path}` serves the bundled static assets; nothing is fetched from the
network. Charts have hover tooltips; `lineChart` series readouts and the geo
map cross-reference the table.

## 2. Search — `GET /v1/search`

The core endpoint. Hybrid BM25 + dense (RRF-fused, LTR-reranked) over the local
index and/or a metasearch fan-out.

| Param | Default | What it does |
|---|---|---|
| `q` | *required* | Query text (never logged). |
| `mode` | `fast` | `deep` adds the cross-encoder rerank stage. |
| `scope` | `both` | `local` = on-device index only; `web` = metasearch only; `both`. |
| `lane` | `direct` | `direct` \| `anon` (Tor) \| `region:<id>`. See §10. |
| `limit` | 10 | Capped at 50. |
| `lat`,`lon` | — | Geo-filter centre (both or neither). |
| `radius_km` | 25 | With `lat`/`lon`; clamped to 250 km. |
| `h3` | — | One H3 cell instead of lat/lon (hex or decimal). |
| `after`,`before` | — | Unix-seconds document-timestamp window (inclusive). |
| `fetch_budget` | 0 | `deep`+`direct` only: fetch up to N result pages and re-score on full text; adds the `analysis` block. **Creates egress to result domains.** |
| `answer` | false | Single-best objective + the extractive `best_passage` block (needs `fetch_budget ≥ 1`). See §3. |
| `diversity` | — | `evidence`: promote each cluster's canonical doc, defer copies. |
| `compare` | — | `vantages`: run direct AND anon, attach the `divergence` block (needs `scope=web`, no `lane`). |

```bash
curl -s "$BASE/v1/search?q=harbour+bridge&mode=deep&limit=5"
curl -s "$BASE/v1/search?q=flooding&lat=52.5&lon=13.4&radius_km=40&after=1749600000"
```

Every result carries `rank_signals` (bm25/ann/rrf/ltr/freshness/geo/…), its
`source` (`local`/`web`/`both`), and — when ingested — an `evidence` cluster tag.
`degraded` honestly lists any stage that timed out or was skipped.

**Filter caveats (ADR-10):** geo/time filters apply to **local results only**
(the metasearch fan-out cannot be geo-filtered). `scope=web` + a geo/time filter
is refused (`400`); `scope=both` leaves the web half unfiltered and sets
`degraded: ["geo_web_unfiltered"]`. Under a filter the local path is exact
lexical (the dense lane is skipped — the recorded ADR-10 tradeoff, re-confirmed
in v0.6.5: the missing dense lane costs < 2 pp nDCG@10).

### Response blocks
- **`evidence`** (additive) — results clustered by text-derivation similarity
  (MinHash containment over ingest-time sketches). `independent_source_count <
  apparent_source_count` means some results are copies/syndications, not
  independent corroboration; `members > domains` in a cluster is cross-domain
  syndication. Only ingested (sketched) docs participate; web results never
  fetched are `evidence: null` (independence is never guessed from a snippet).
- **`confidence`** — uncalibrated query-performance predictors (`nqc`, `clarity`,
  `score` ∈ [0,1]). Treat *low* as "verify before trusting", not high as "true".
- **`divergence`** (`compare=vantages`) — Jensen-Shannon divergence between the
  direct and anon lanes' domain distributions vs the deployment noise floor.
- **`analysis`** (`fetch_budget>0`) — `fetches_made`, `search_stopped_because`,
  `estimated_marginal_gain_remaining` (what the VoI stop left unfetched).

## 3. Answer mode + claim corroboration — `answer=true`

`answer=true` (with `fetch_budget ≥ 1`, `mode=deep`, `direct`) switches the fetch
selector to a single-best objective and attaches an **extractive** `best_passage`
(`schema: 2`):

```json
"best_passage": {
  "schema": 2,
  "text": "…verbatim sentence-aligned extract, ≤500 chars…",
  "url": "https://the-page-it-was-read-from.example/…",
  "ce_score": 7.1,
  "corroboration": {
    "schema": 1,
    "independent_clusters": 2,
    "supporting_urls": ["https://b.example/y", "https://c.example/z"],
    "basis": { "candidates_checked": 5, "method": "ce-cross-cluster" }
  }
}
```

- **Extractive only** — never generated, never stitched across documents; the
  text appears verbatim on the cited page. `ce_score` is a *relevance* logit, **not
  a correctness probability**.
- **`corroboration`** (v0.6.5, C1) — `independent_clusters` is the number of
  *independent evidence clusters* (distinct ADR-18 clusters, never the winner's
  own) whose top passage the cross-encoder finds states the same claim.
  **Same-cluster syndicated copies are excluded by construction** — a wire story
  reprinted across fifty outlets counts once, never as corroboration. Absence of
  the block means "no independent support found", never "no answer".
  `basis.candidates_checked` is the honesty payload (thin coverage → low count,
  never a false badge). It attests an independent source *discusses the same
  claim*, not that the claim is true. (Default-ON via `search.answer_corroborate`;
  requires the cross-encoder, i.e. the deep/ort build — dormant in the scratch
  appliance image.)
- **Selective abstention** (`search.answer_abstain_threshold`, default OFF) —
  withholds a low-`ce_score` passage with `degraded: ["answer_below_threshold"]`.

```bash
curl -s "$BASE/v1/search?q=who+built+the+Halford+reservoir&mode=deep&fetch_budget=2&answer=true"
```

## 4. Geo heatmap — `GET /v1/geo/heatmap`

| Param | Default | Notes |
|---|---|---|
| `q` | — | Optional; omitted = whole corpus. |
| `res` | 5 | H3 resolution 3..=7. |
| `window` | `7d` | `24h` / `7d` / `all`. |

Returns per-cell `count` plus Getis-Ord Gi* hot-spot stats (`z`, `q_value`,
`significant` with Benjamini-Hochberg FDR). `significant` (q ≤ 0.05) is the
defensible "this is a hot spot" flag — a big raw count alone is not.

```bash
curl -s "$BASE/v1/geo/heatmap?q=protest&res=5&window=7d"
```

## 5. Trends — `GET /v1/trends`

Requires `[analytics] enabled = true` (GDELT opt-in), else `404`.

| Param | Default | Notes |
|---|---|---|
| `topic` | all | GDELT EventRootCode 1–20. |
| `h3` | all | H3 **res-5** cell. |
| `window` | `7d` | as heatmap. |

Returns a `(day, count)` `series` and `top_movers` ranked by `z` (empirical-Bayes
shrunk, overdispersion-aware), each with `significant` (BH-FDR) **and** a `burst`
flag (sustained multi-day elevation, ADR-28) — two detectors with different blind
spots. All trends describe *media coverage*, not ground truth (ADR-15).

```bash
curl -s "$BASE/v1/trends?topic=14&window=7d"
```

## 6. Ingest — `POST /v1/ingest`  (bearer)

A JSON array (max 100) of inline-text or URL items:

```bash
curl -s -X POST -H "Authorization: Bearer $MERIDIAN_BEARER_TOKEN" \
  -H 'content-type: application/json' "$BASE/v1/ingest" -d '[
    {"text":"…document body…","title":"…","url":"https://canonical","ts":1749600000},
    {"url":"https://fetch-me.example/page","lane":"direct"}
  ]'
```

Returns `202 {accepted, deduped, queued}`. Content-hash duplicates are deduped;
`/v1/forget`-tombstoned content is refused. Text is geo-tagged via the offline
gazetteer (no network call at ingest).

## 7. One-off fetch — `GET /v1/fetch?url=…&lane=…`  (bearer)

Fetch+extract one URL through the robots-respecting ladder (5 MB cap, per-domain
politeness). Returns `{url, title, text, http_status}`.

## 8. Forget — `POST /v1/forget`  (bearer)

Operator deletion. Exactly one selector (`url` | `domain` | `content_hash`):

```bash
curl -s -X POST -H "Authorization: Bearer $MERIDIAN_BEARER_TOKEN" \
  -H 'content-type: application/json' "$BASE/v1/forget" -d '{"domain":"example.com"}'
```

Removes matching docs from the lexical index **and** the vector store **and** their
derivation sketches (atomic — ADR-19), tombstones the content hashes so re-ingest
is refused, and (default) purges the query+fetch caches. Returns
`{removed, caches_purged}`. The selector is never logged.

## 9. Lane health — `GET /v1/lanes`

```bash
curl -s "$BASE/v1/lanes"   # [{"id":"direct","status":"up","detail":null}, …]
```

`status` ∈ `up | bootstrapping | degraded | down | disabled`.

## 10. Lanes & privacy

- **`direct`** — normal egress.
- **`anon`** — Tor-proxied (embedded Arti), **fail-closed**: while Tor is
  bootstrapping or down the request returns `503` (never a silent direct
  fallback); over-concurrency returns `503 anon busy`.
- **`region:<id>`** — source-IP-bound egress; refuses (`503`) until egress-IP
  verification passes.
- **`compare=vantages`** intentionally sends the query over Tor AND directly in
  one jittered window (opt-in) — see `privacy.md`.

## 11. Operator surfaces

- **`GET /v1/decision-log`** · **`POST /v1/decision-log/wipe`** (bearer) — the
  ADR-24 routing log (off by default; `404` when `searx.decision_log=false`).
- **`GET /v1/decision-log/ope`** (bearer) — the ADR-25 off-policy ship-gate
  report (doubly-robust uplift + bootstrap CI). Reporting only; never changes
  routing.
- **`GET /healthz`** — `{status, uptime_secs}`. Unauthenticated, not rate-limited.
- **`GET /metrics`** — Prometheus metrics (latency histograms, lane states, cache
  occupancy). No query text or URLs.

## 12. Honest-failure conventions

Meridian never silently degrades: `degraded: [...]` names skipped/timed-out
stages, fail-closed lanes return `503` rather than a wrong-lane answer, and the
analytical blocks (`evidence`, `confidence`, `analysis`, `corroboration`) always
state their *basis* so a thin result reads as low-confidence rather than as a
false claim.
