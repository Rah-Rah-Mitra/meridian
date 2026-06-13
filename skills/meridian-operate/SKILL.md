---
name: meridian-operate
description: "Maintainer tasks against a running Meridian appliance — ingest documents, forget/delete them (tombstoned), check health, and read the ADR-24/25 decision log + its offline-policy-evaluation ship-gate. Bearer-gated. Padding the decision log with synthetic traffic is forbidden; the verdict is only valid on organic searches."
version: 1.0.0
author: Meridian
license: MIT
platforms: [linux]
metadata:
  hermes:
    tags: [Operations, Ingest, Privacy, DecisionLog, Maintenance]
    related_skills: [meridian-search]
---

# Meridian operate (the maintainer skill)

Administrative tasks against a running Meridian appliance: load documents into
the on-device index, delete them on request, check liveness, and inspect the
routing decision log and its ship-gate. **Every endpoint here except `/healthz`
is bearer-gated.**

## When to use
- Adding documents to the local index, or honoring a deletion request.
- Checking the appliance is up, or reading the decision-log / OPE ship-gate.

## When NOT to use
- Plain querying → use `meridian-search`.

## Setup

```sh
export MERIDIAN_URL="http://127.0.0.1:8080"
export MERIDIAN_BEARER_TOKEN="…"   # value of MERIDIAN_BEARER_TOKEN in deploy/.env
```

**Never** print, log, or commit the literal token; only ever the
`$MERIDIAN_BEARER_TOKEN` placeholder. The mutating endpoints answer
`503 auth not configured` if no token is set on the appliance.

## Health — `GET /healthz` (unauthenticated)

```sh
curl -s "$MERIDIAN_URL/healthz"
# {"status":"ok","uptime_secs":12345}
```

Unauthenticated and not rate-limited — safe for liveness probes.

## Ingest — `POST /v1/ingest` (bearer)

Body is a JSON array (max 100 items) of inline text and/or URLs to fetch:

```sh
curl -s -X POST "$MERIDIAN_URL/v1/ingest" \
  -H "Authorization: Bearer $MERIDIAN_BEARER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '[
    { "text": "…document body…", "title": "Notes", "url": "https://optional-canonical", "ts": 1749600000 },
    { "url": "https://fetch-me.example/page", "lane": "direct" }
  ]'
# 202 -> {"accepted":2,"deduped":0,"queued":0}
```

- Content-hash duplicates are **deduped**; documents previously erased via
  `/v1/forget` are **refused** (tombstoned) and count toward neither.
- Ingest is what creates evidence sketches and (with the gazetteer present)
  geo-tags — so re-ingesting old docs is how they gain `evidence`/`h3`.
- Under resource-pressure shedding, ingest answers `503 ingest paused`.

## Forget — `POST /v1/forget` (bearer)

Exactly one selector. Removes the doc from the lexical + vector index,
tombstones its content hash (re-ingest refused), and (default) purges caches:

```sh
curl -s -X POST "$MERIDIAN_URL/v1/forget" \
  -H "Authorization: Bearer $MERIDIAN_BEARER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{ "url": "https://example.com/page" }'
# {"removed":1,"caches_purged":true}
```

Selectors: `{ "url": … }` · `{ "domain": … }` · `{ "content_hash": "32-hex" }`.
Add `"purge_caches": false` to skip cache purge. The selector is never logged.

## Decision log — `GET /v1/decision-log` (bearer)

The ADR-24 per-decision routing log (off by default; returns `404` while
`searx.decision_log = false`). Coarse 13-byte rows — query text, URLs, and IPs
are never stored.

```sh
curl -s "$MERIDIAN_URL/v1/decision-log" \
  -H "Authorization: Bearer $MERIDIAN_BEARER_TOKEN"
# {"enabled":true,"rows":842,"approx_bytes":10946,"oldest_day":20608,
#  "newest_day":20617,"retention_days":30,"max_bytes":1048576}
```

**Which requests accrue a row:** exactly those that are `lane=direct` AND have
`scope` including web AND have engines unpinned (no `compare`) AND where the
bandit actually selected an arm. `anon`/`region` lanes are **never** logged;
`compare=vantages` pins engines and is **excluded**. (`POST /v1/decision-log/wipe`
drops every row → `{"removed":n}`.)

## OPE ship-gate — `GET /v1/decision-log/ope` (bearer)

The ADR-25 offline-policy-evaluation report. It trains the linear-Thompson
candidate on the older 80% of the log and reports its doubly-robust uplift vs
the incumbent on the held-out 20%, with a bootstrap 95% CI.

```sh
curl -s "$MERIDIAN_URL/v1/decision-log/ope" \
  -H "Authorization: Bearer $MERIDIAN_BEARER_TOKEN"
# {"n_total":842,"report":{…},"gate":{"min_decisions":10000,"verdict":"insufficient_data"}}
```

**Reading the verdict** (`gate.verdict`):

| Verdict | Meaning | Effect |
|---|---|---|
| `pass` | CI excludes zero from above on ≥10k decisions | candidate beats incumbent |
| `inconclusive` | CI straddles zero | ε-greedy stays (ADR-25 sunset rule) |
| `negative` | candidate is worse | ε-greedy stays |
| `insufficient_data` | fewer than `min_decisions` (10000) rows | keep accruing |

This endpoint is **reporting only** — it never changes routing.

**Forbidden: do not pad the log.** The verdict is only valid on **organic,
direct-lane** searches. Do not script or synthesize traffic to reach the 10k
threshold faster — synthetic rows poison the offline estimate and invalidate
the gate. Let real usage accrue it (the deployment's sunset is calendar-bound,
not traffic-forced).

## Errors

RFC 9457 problem+json. `400` invalid (not retryable), `429` rate/concurrency
(retry with backoff), `503` auth-not-configured / ingest-paused, `504`
timeout. `404` on the decision-log endpoints means the log is disabled.
