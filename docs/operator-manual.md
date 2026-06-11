# Meridian Operator Manual (v0.1)

Companion documents: [API reference](api.md) · [privacy & retention](privacy.md)
· threat model (`docs/plan/05-threat-model.md`) · the binding spec
(`docs/SPEC.md`).

## 1. Requirements

- A 64-bit ARM (Raspberry Pi 5 class, 4 GB+ RAM) or x86-64 Linux host with
  Docker + Compose. Both 4 K and 16 K page-size kernels work (the appliance
  is validated on the Pi 5's 16 K kernel).
- Disk: ~8 GB for the full profile (1 M docs); ~3 GB for a small-card profile
  (100 k docs). Budgets: `docs/plan/02-budgets.md`.
- No build toolchain needed — images are cross-compiled in CI and shipped as
  `FROM scratch` containers (~75 MB).

## 2. Install

```sh
git clone https://github.com/Rah-Rah-Mitra/meridian && cd meridian/deploy
sudo ./pi-setup.sh             # idempotent host prep: zram, sysctls, journald caps
echo "MERIDIAN_BEARER_TOKEN=$(openssl rand -hex 32)" > .env && chmod 0600 .env
docker compose up -d           # meridiand + searxng (direct lane only)
curl -s localhost:8080/healthz
```

The API listens on `127.0.0.1:8080` **only**. Exposing it further is an
explicit choice: put TLS in front, set `auth.require_bearer_for_search =
true`, and read `docs/plan/05-threat-model.md` first.

### Geo tagging (optional, recommended)

Release images ship `models/gazetteer.fst` (GeoNames `cities15000`, CC-BY 4.0)
baked in; ingest geo-tags documents offline — no network call. To rebuild it
yourself: `deploy/fetch-gazetteer.sh`. Without the file, geo tagging is
silently off (search still works; heatmaps stay empty).

## 3. Profiles

| Profile | Command | Adds |
|---|---|---|
| default | `docker compose up -d` | direct lane + SearXNG |
| anon | `MERIDIAN_ANON=true` in `.env`, then `docker compose --profile anon up -d` | Tor egress via embedded Arti + a second, Tor-only SearXNG (`searxng-anon`) on an internal network whose sole internet route is meridiand's SOCKS listener — fail-closed by topology |
| regions | `docker compose -f compose.yaml -f compose.regions.yaml up -d` | host networking + WireGuard source-IP lanes; see `deploy/wg-lane-templates/` for the `ip rule` recipes (operator-applied, never automatic) |

Expect the anon lane to report `bootstrapping` for ~30–60 s after start;
requests during that window get honest 503s (fail-closed, never a silent
direct fallback). Check `curl localhost:8080/v1/lanes`.

Region lanes refuse traffic until the lane's observed egress IP matches
`expected_ip` (re-verified every 30 min). A `degraded` region lane means the
routing-leak tripwire fired — fix the WireGuard route, don't disable the
check.

## 4. Configuration reference

Config = defaults ← `meridian.toml` (path via `MERIDIAN_CONFIG`) ←
`MERIDIAN_*` env vars (nested keys split on `__`, e.g.
`MERIDIAN_SERVER__PORT=9090`). The bearer token is **not** config — it never
transits the config layer.

| Key | Default | Notes |
|---|---|---|
| `server.bind` / `server.port` | `127.0.0.1` / `8080` | Loopback by default. |
| `server.concurrency_limit` | 8 | In-flight cap; beyond it requests shed with 429. |
| `server.request_timeout_ms` | 12000 | Last-resort ceiling; must exceed the slowest lane budget (anon = 8 s). |
| `server.body_limit_bytes` | 1048576 | 1 MB request cap. |
| `server.rate_limit_per_sec` / `_burst` | 5 / 20 | Per hashed client IP, `/v1/*` only. |
| `auth.token_env` | `MERIDIAN_BEARER_TOKEN` | Env var holding the token. |
| `auth.token_file` | — | 0600 file; takes precedence over env. |
| `auth.require_bearer_for_search` | false | Set true when exposed beyond localhost. |
| `privacy.debug_query_logging` | false | TRACE-only; startup warning when on. |
| `lanes.anon_enabled` | false | Tor lane. |
| `lanes.regions_enabled` | false | WireGuard lanes. |
| `lanes.allow_onion` | false | `.onion` refused on every lane while false. |
| `lanes.direct.contact_url` | example.invalid | **Set a real contact URL** — it rides the UA on third-party fetches. |
| `lanes.direct.hedge_after_ms` | 300 | Metasearch hedge (direct lane only; 0 = off). |
| `lanes.anon.socks_listen` | `127.0.0.1:9150` | In-process SOCKS5; compose widens to the internal network, never the host. |
| `lanes.anon.max_circuits` | 6 | Tor circuit cap. |
| `lanes.anon.max_concurrent_searches` | 2 | Excess anon searches get 503. |
| `lanes.anon.state_dir` | `<data_dir>/arti` | Arti state+cache. |
| `lanes.regions.<id>.source_ip` | — | WireGuard interface source IP. |
| `lanes.regions.<id>.verify_url` | checkip.amazonaws.com | IP-echo endpoint fetched through the lane. |
| `lanes.regions.<id>.expected_ip` | — | Exact IP, or prefix ending in `.`/`:`; unset = only verify egress works. |
| `index.data_dir` | `data` | The single mutable volume. Everything lives here. |
| `index.writer_threads` / `writer_heap_bytes` | 2 / 256 MB | Tantivy writer. |
| `index.merge_max_docs` | 500000 | Merge-policy doc cap (~256 MB segments). |
| `search.default_limit` / `max_limit` | 10 / 50 | |
| `search.bm25_top_k` | 1000 | Lexical candidate depth. |
| `search.searx_deadline_ms` | 800 | Direct metasearch deadline. |
| `search.anon_searx_deadline_ms` | 8000 | Anon metasearch deadline (Tor adds seconds). |
| `search.max_per_domain` | 3 | Domain diversity cap. |
| `searx.url` | `http://searxng:8080` | Direct sidecar (back network). |
| `searx.anon_url` | — | Tor-proxied sidecar; unset = anon search fails closed. |
| `fetch.max_body_bytes` | 5 MB | Streamed cap. |
| `fetch.max_redirects` | 3 | |
| `fetch.per_domain_interval_ms` / `_burst` | 2000 / 2 | Politeness budget, global across lanes. |
| `fetch.robots_ttl_secs` | 86400 | |
| `ingest.batch_max` | 100 | |
| `ingest.snippet_max_chars` | 240 | Snippet built at ingest; queries never read bodies. |
| `models.dir` | `models` | Baked into the image at `/models`. |
| `models.gazetteer_file` | `<dir>/gazetteer.fst` | Loaded if present. |
| `vector.connectivity` / `expansion_add` / `expansion_search` | 16 / 128 / 64 | HNSW knobs. |
| `vector.top_k` | 200 | ANN candidate depth. |
| `vector.persist_every_docs` | 25000 | Full-file persist cadence (coarse on SD cards). |
| `analytics.enabled` | false | GDELT pull is **opt-in** (it's a third-party feed). |
| `analytics.gdelt_base` | data.gdeltproject.org | Plain HTTP by upstream necessity; manifest MD5 verifies integrity (ADR-15). |
| `analytics.pull_interval_secs` | 900 | GDELT publishes every 15 min. |
| `analytics.retention_days` | 90 | Counter TTL, compacted daily. |
| `analytics.max_edges` | 200000 | PageRank substrate bound. |
| `evidence.enabled` | true | Source-independence clustering in `/v1/search` (v0.2.0, ADR-18). Pure local computation, no network/privacy surface — this is the kill-switch. Sketches are written at ingest and erased by `/v1/forget` in the same transaction; docs ingested before v0.2.0 have none until re-ingested. |

## 5. Operations

**Watch:** `/v1/lanes` for lane health; `/metrics` for
`meridian_requests_total`, `meridian_request_ms`,
`meridian_store_bytes{store}` (30-min sweep), process RSS, and shed counters.

**Shedding ladder:** under RSS / thermal / disk pressure the node sheds in
order: deep-rerank off → ANN off → ingest paused (503) → metasearch-only.
Each step is visible in `degraded` and in metrics. It recovers automatically
when pressure clears.

**Index schema:** the on-disk index carries a schema version (currently 3).
A version-mismatched volume is **refused at startup** — wipe `/data/index`
and re-ingest rather than risking silent corruption. (Schema changed in
v0.1.0 when geo/time fields landed.)

**Logs** are capped (json-file 10 MB × 3 per container) and contain no query
text or IPs; `docker compose logs meridiand` is safe to share when filing
issues.

**Upgrades:** `docker compose pull && docker compose up -d`. SearXNG images
are digest-pinned; bump the digest deliberately. Check the release notes for
index-schema bumps before upgrading.

## 6. Backup & restore

Everything mutable lives in the single `meridian-data` volume. Cold-ish
backup (sub-second pause, safe ordering — redb and tantivy files are
crash-consistent, but stopping writes gives a clean point):

```sh
cd deploy
docker compose stop meridiand
docker run --rm -v meridian_meridian-data:/data -v "$PWD":/backup alpine \
  tar -C /data -cf - . | zstd -o backup-$(date +%F).tar.zst
docker compose start meridiand
```

Restore onto a fresh volume:

```sh
docker compose down
docker volume rm meridian_meridian-data
docker volume create meridian_meridian-data
zstd -dc backup-YYYY-MM-DD.tar.zst | docker run --rm -i \
  -v meridian_meridian-data:/data alpine tar -C /data -xf -
docker compose up -d
```

Verify: `/healthz`, then a known query against `scope=local`. This drill is
exercised at every release (`docs/plan/phase-exits/p6.md`).

## 7. Troubleshooting

| Symptom | Meaning | Action |
|---|---|---|
| `503` on `lane=anon`, `/v1/lanes` says `bootstrapping` | Tor still bootstrapping (cold start ~30–60 s) | Wait; fail-closed is by design. |
| `503` on `lane=anon`, lanes say `up` | `searx.anon_url` unset, or anon admission cap hit | Set the anon sidecar URL / retry. |
| Region lane `degraded: egress IP mismatch` | Policy routing leak — traffic isn't leaving via the WG tunnel | Fix `ip rule`/WG config; the lane refuses traffic until verified. |
| Startup abort: index schema version mismatch | Volume from an older release | Back up, wipe `/data/index`, re-ingest. |
| `429` | Rate limit (5 rps/IP) or concurrency cap (8) | Back off; raise limits only behind real auth. |
| `503 ingest paused` | Shedding under resource pressure | Check temp/RSS/disk; ingest resumes automatically. |
| Heatmap empty | No gazetteer at ingest time, or docs lack timestamps for the window | Install gazetteer, re-ingest; or `window=all`. |
| `/v1/trends` 404 | Analytics disabled | `[analytics] enabled = true` (opt-in). |

## 8. Uninstall / erase

`docker compose down && docker volume rm meridian_meridian-data` removes
every byte of state. For selective erasure see
[privacy.md → Deletion](privacy.md#deletion).
