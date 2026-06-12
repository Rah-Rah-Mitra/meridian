# Meridian Privacy Policy & Data-Retention / Deletion Guide

This document states what a Meridian node records, for how long, and how to
erase it. It is written for operators; the guarantees marked **hard** are
enforced in code and covered by CI tests (`deploy/privacy-smoke.sh`, the
egress invariant suite) — not aspirations.

## What is never recorded (hard guarantees)

- **Query text never appears in logs or metrics.** The per-request access
  event carries route, status code, and latency only. A tracing redaction
  layer backstops every log line; `/metrics` label cardinality is bounded to
  route/status/stage/lane/store. (The `privacy.debug_query_logging` flag
  exists for local debugging at TRACE level only; it is off in every shipped
  config and prints a startup warning when enabled.)
- **Client IPs are never persisted.** The only use of a client IP is deriving
  a rate-limit key via a keyed hash whose secret is random per boot; the raw
  IP is dropped immediately after hashing and the hash lives only in the
  in-memory rate limiter.
- **No cookies, no CORS, no sessions.** `x-request-id` is random per request
  and correlates nothing across requests.
- **No third-party telemetry.** The node initiates outbound connections only
  to: the SearXNG sidecars (which fan out to search engines), URLs the
  operator explicitly ingests/fetches, the Tor network (anon profile), the
  operator-configured region IP-echo endpoint (regions profile), and GDELT
  (only when `[analytics]` is explicitly enabled). Nothing else — verified by
  egress capture at the v0.1.0 release gate.
- **Secrets stay out of logs, config, and the data volume.** The bearer token
  enters via env var or a 0600 file, is held as a `secrecy::SecretString`,
  compared in constant time, and is never serialized or logged.

## Anonymous lane

With `lane=anon`, traffic leaves only through Tor (embedded Arti):

- **Fail-closed** (hard): if Tor is bootstrapping, down, or misconfigured the
  request gets a 503 — there is no code path that falls back to the direct
  lane. The `searxng-anon` sidecar sits on an internal Docker network whose
  only internet route is Meridian's own SOCKS listener, so even sidecar bugs
  cannot leak.
- **No DNS leak** (hard): hostnames resolve inside Tor (SOCKS5h domain
  addressing, enforced by tests).
- **Isolation:** Meridian's own anon fetches use a fresh Tor isolation token
  per logical request (robots + redirects share one circuit family).
  `searxng-anon`'s upstream engine connections are isolated per connection —
  not per query — because a static proxy URL cannot vary credentials per
  request (honest limitation, threat model §2.3).
- **No shared state:** anon search results live in a separate ephemeral cache
  (32 MB, 5-minute TTL) and never touch the shared query cache.

## Compare-vantages mode (v0.2.0, explicit opt-in)

`compare=vantages` deliberately sends the SAME query over the direct lane AND
over Tor within one window so the result distributions can be compared. Said
plainly: **for that one request you are trading the anon lane's unlinkability
for divergence evidence.** An upstream engine (or an observer of both
vantages) that sees an identical rare query arrive twice in a short window can
link the Tor request to your direct IP.

What Meridian does about it — and what it cannot do:

- Compare mode is **never a default and never auto-triggered**; it runs only
  when the request says `compare=vantages`.
- A **randomized delay** (`search.compare_jitter_ms_max`, default 30s window)
  separates the two dispatches — clamped to what fits the synchronous request
  ceiling (`server.request_timeout_ms` minus the lane deadlines), which under
  DEFAULT config leaves well under a second. Said plainly: **meaningful timing
  decorrelation requires raising the request ceiling** (or waiting for the
  async compare, a Phase-10 candidate). The jitter narrows trivial timing
  correlation; it cannot defeat an adversary observing both vantages
  (threat model §8).
- Neither half is cached, no per-query cross-lane record is persisted, and the
  anon half touches no shared routing state — the comparison exists only in
  the response you receive.

## Deep-mode fetching (v0.4.0, **off unless asked per request**, ADR-26)

`mode=deep&fetch_budget=N` lets ONE request fetch up to N result pages
(capped by `search.deep_fetch_max`, default 2) so they can be re-scored on
their full text instead of their search snippets. Said plainly: **the node
makes HTTP requests to the result domains it selects, attributable to your
direct IP, triggered by your query.** What bounds it:

- Never a default, never auto-triggered, never on the anon or region lanes
  (requests asking for it there are refused with `400`), and never combined
  with `compare=vantages`.
- Every fetch goes through the standard ladder: SSRF vet, per-domain budget,
  robots.txt, size caps. The selection itself optimizes information per
  fetch (ADR-26), so typically fewer pages are fetched than the budget
  allows — the `analysis` block on the response says how many and why it
  stopped.
- Fetched text is used in RAM for scoring only. **A search never ingests or
  persists what it fetched**; the only retention is the fetch ladder's own
  24-hour extract cache (same as `/v1/fetch`, RAM only, see the table).

## Decision log (v0.3.0, **off by default**, ADR-24)

`searx.decision_log = true` enables a per-decision routing log that exists for
one purpose: evaluating a better engine-routing policy offline before any
live traffic shifts (ADR-25). It is **disabled in every shipped config**
through v0.3.x; enabling it is an explicit operator action.

What one row is — 13 fixed bytes, nothing else:

- coarse context buckets: intent class (4 values), query-length bucket (4),
  language id, time-of-day bucket (3-hour), and whether a geo FILTER was
  present (the requested geography, never the user's location);
- the chosen engine arm, the ε-greedy propensity it was chosen with, and a
  1-bit reward (did web results reach the top-10).

**No query text, no URLs, no IPs, no timestamps finer than day + 3-hour
bucket.** The row width is pinned by a test so nothing string-shaped can
quietly grow into it. Safeguards, all enforced in code:

- **k-anonymity floor:** until a context combination has been seen 5 times
  that day, its rows are written with the context fields blanked — rare,
  potentially identifying combinations never land readable.
- **30-day TTL** swept continuously and at startup; **20 MB hard cap** (oldest
  days dropped first); erasure via `POST /v1/decision-log/wipe` (bearer-gated;
  `GET /v1/decision-log` reports row count, size, and retained day range).
  Disabling the flag stops new rows and the TTL erases the rest within 30
  days.
- **Anon-lane decisions are never logged.** The log call sits on the same
  code path that rewards the bandit, which the anon lane cannot reach
  (SPEC §12.4 firewall) — enforced by the hermetic lane-invariant suite.

## Retention table

| Store | Contents | Where | Lifetime / cap |
|---|---|---|---|
| Lexical index + vector store | Ingested documents (operator's corpus) | `/data` volume | Until deleted via `/v1/forget` |
| Dedup store | 16-byte content hashes + url-key→hash map | `/data` | Until forgotten (then tombstoned) |
| Tombstones | Content hashes of forgotten docs | `/data` | Permanent by design (blocks re-ingest) |
| Query cache (direct) | Fused result lists | RAM only | 256 MB weighted, 15 m idle / 2 h max |
| Query cache (anon) | Anon result lists | RAM only | 32 MB, 5 m |
| Fetch cache | Extracted pages | RAM only | 96 MB, 24 h |
| robots.txt cache | Robots bodies per domain | RAM only | 24 h (`fetch.robots_ttl_secs`) |
| Analytics counters | (day, H3 res-5 cell, GDELT root code) → count; domain co-occurrence edges | `/data/analytics.redb` | 90 days (`analytics.retention_days`), compacted daily; edges capped at `analytics.max_edges` |
| Decision log (`searx.decision_log`, off by default) | 13-byte coarse routing rows: intent/length/language/time-of-day buckets + arm/propensity/reward — no text, no URLs, no IPs | `/data` (egress.redb) | 30 days, ≤20 MB cap, k-anonymity floor, wipe path |
| Arti state | Tor directory documents, guard state | `/data/arti` | Managed by Arti; contains no user/query data |
| Container logs | Route/status/latency events only | json-file | 10 MB × 3 files per container |

Heatmaps are computed from the operator's own indexed documents; analytics
counters are derived from the public GDELT feed — neither contains user
queries. Store sizes are observable at `/metrics` as
`meridian_store_bytes{store=…}` (swept every 30 minutes).

## Deletion

`POST /v1/forget` (bearer-gated) is the erasure path. Exactly one selector:

| Selector | Effect |
|---|---|
| `url` | Forgets that document. |
| `domain` | Enumerates and forgets every currently indexed document of the domain. |
| `content_hash` | Forgets by content (32-hex blake3-prefix as reported at ingest); tombstones the hash even if no live doc matches. |

For every matched document, in one operation: removed from the lexical index,
removed from the vector store, its content hash tombstoned. Tombstoned
content is **refused on re-ingest** (reported in ingest stats as `refused`) —
deletion survives a crawler re-discovering the page. By default the query and
fetch caches are purged in the same call (`purge_caches: false` opts out —
only safe if the document never appeared in results). Cached anon entries
also expire within 5 minutes regardless.

The audit log line for a forget records the number of documents removed and
whether caches were purged — never the selector.

What `/v1/forget` does **not** do: un-ring bells outside the node (engines
queried via metasearch retain their own copies) and it does not edit
analytics counters (they derive from GDELT, not from your documents).

## Swap / memory at rest

Secrets are not `mlock`ed: the deployment target swaps to compressed RAM
(zram) by default, and the threat model (§3.4) judges the residual risk —
root-level access to swap — equivalent to reading the 0600 token file
directly. Operators using disk swap who care about this class of attacker
should use encrypted swap; see `docs/plan/05-threat-model.md`.
