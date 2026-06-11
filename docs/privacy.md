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
- A **randomized delay** (default up to 30s, `search.compare_jitter_ms_max`)
  separates the two dispatches. This narrows trivial timing correlation; it
  cannot defeat an adversary observing both vantages (threat model §8).
- Neither half is cached, no per-query cross-lane record is persisted, and the
  anon half touches no shared routing state — the comparison exists only in
  the response you receive.

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
