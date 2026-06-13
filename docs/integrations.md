# Using Meridian from agents & humans

Meridian is one HTTP appliance with one contract ([API reference](api.md)).
This guide is the integration layer on top of it: the canonical request, the
reference agent integration, the easy mistakes, and how to handle partial
responses. The portable, copy-paste skills live in
[`skills/`](../skills/README.md) (`meridian-search`, `meridian-operate`) and
are the single source of truth for both humans and agents.

## Configuration

Both humans and agents read the same two environment variables:

```sh
export MERIDIAN_URL="http://127.0.0.1:8080"     # appliance base URL
export MERIDIAN_BEARER_TOKEN="…"                # value of MERIDIAN_BEARER_TOKEN in deploy/.env
```

Never hardcode, print, or commit the literal token — only the
`$MERIDIAN_BEARER_TOKEN` placeholder. This repo is CI-gated by a secret
scanner; committed examples must use placeholders.

## The canonical request

The request that exercises the full default path **and** accrues one coarse
ADR-24 decision-log row (so the routing policy can be evaluated, ADR-25):

```sh
curl -s -G "$MERIDIAN_URL/v1/search" \
  -H "Authorization: Bearer $MERIDIAN_BEARER_TOKEN" \
  --data-urlencode "q=your query" \
  --data-urlencode "scope=web" \
  --data-urlencode "lane=direct"
```

A request appends a decision-log row **only** when `lane=direct` AND `scope`
includes web AND engines are unpinned (no `compare=vantages`) AND the bandit
selects an arm. `anon`/`region` lanes and `compare` are never logged, and query
text / URLs / IPs are never stored. **Do not synthesize traffic to pad the
log** — the ADR-25 ship-gate is only valid on organic searches.

## Reference integration: the Hermes Meridian provider plugin

The canonical agent integration is the **Hermes Meridian provider plugin**. It:

- reads `MERIDIAN_URL` + `MERIDIAN_BEARER_TOKEN` from the environment (the same
  two variables above — no separate config);
- issues `GET /v1/search` and maps each result's **`snippet`** field to the
  tool's **description** field (the text the agent reads);
- consumes the `skills/meridian-search` skill directly (Hermes symlinks the
  skill directory into the agent's skills dir — see
  [`skills/README.md`](../skills/README.md)).

Any agent platform can follow the same pattern: one HTTP GET, bearer header,
`snippet → description`.

## Caveat: Meridian is NOT a drop-in SearXNG

Meridian listens on `:8080` and speaks JSON, so it is tempting to point an
existing SearXNG consumer at it with a bare `SEARXNG_URL=http://127.0.0.1:8080`
swap. **That fails.** Meridian is a different contract:

| | SearXNG | Meridian |
|---|---|---|
| Search path | `/search` | **`/v1/search`** |
| Auth | none | **`Authorization: Bearer …`** (when gated) |
| Per-result text field | `content` | **`snippet`** |
| Response envelope | flat results | results + `timings` + `evidence` + `degraded` + lanes |

So a raw URL swap breaks three ways at once: the path is `/v1/search` not
`/search`, a bearer header is required, and the field is `snippet` not
`content`. Use the Meridian provider plugin (or the `meridian-search` skill),
not a SearXNG shim.

## Answer mode and evidence

- **Answer mode** (`mode=deep&fetch_budget=N&answer=true`, direct lane) attaches
  a top-level `best_passage` — a verbatim, cited extract ≤500 chars with a
  `ce_score`. `ce_score` is a **relevance** score, **not** a correctness
  probability: present it as "where to start reading" with its `url`, never as
  "the answer". If no passage materializes the block is absent and you get
  `degraded: ["answer_unavailable"]`.
- **Evidence** (`evidence.independent_source_count`) is your corroboration
  signal: N near-duplicate copies of one syndicated story collapse into one
  cluster, so `independent_source_count` < `apparent_source_count` means the
  corroboration is weaker than the raw count suggests. Results never fetched
  carry `evidence: null` and are excluded from the count — do not treat them as
  independent sources.

See [`skills/meridian-search/SKILL.md`](../skills/meridian-search/SKILL.md) for
full request/response detail.

## Retry & degraded handling

Build these into any client:

| Status | Action |
|---|---|
| `400` | invalid params — fix the request; **not** retryable |
| `429` | rate / concurrency cap — retry with exponential backoff |
| `503` | anon lane down/bootstrapping, or auth not configured — surface it; do **not** silently retry `anon` as `direct` |
| `504` | whole-request timeout — retry once, or narrow the query / lower `fetch_budget` |

A `200` is not necessarily complete: always inspect `degraded[]` (`searx_timeout`,
`searx_unavailable`, `geo_web_unfiltered`, `fetch_unavailable`,
`answer_unavailable`, `ann_unavailable`) and `lane_effective` vs
`lane_requested`. Results are always served honestly labeled — a degraded
response is still usable, just partial.

## See also

- [API reference](api.md) — the field-exact HTTP contract.
- [`skills/`](../skills/README.md) — the portable consumer + operator skills.
- [Operator manual](operator-manual.md) — deploy and run the appliance.
