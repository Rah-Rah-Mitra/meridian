# 06 — Operator Questions (with assumed defaults)

> SPEC §1.7 — up to 8 clarifying questions. Work proceeds on the stated defaults;
> answers adjust config/docs, not architecture. Four structural questions were
> already asked and answered on 2026-06-10.

## Answered (2026-06-10)

| # | Question | Answer |
|---|---|---|
| A1 | Session/dev scope | Plan docs + scaffold now; sign-off gates Phase 1+ |
| A2 | Repo visibility | Private (`Rah-Rah-Mitra/meridian`); public at v0.1.0 |
| A3 | Dev loop on the (shared) target Pi | Local `cargo check`/clippy allowed; all builds/tests/cross in CI (ADR-D1) |
| A4 | On-device scale given 7.7GB free, no NVMe | Dual profile: F = 1M docs/8GB (published, NVMe), R = 100k docs/3GB (this device) (ADR-D2) |

## Open (defaults assumed; answer any time before the phase that consumes it)

| # | Question | Default assumed | Consumed by |
|---|---|---|---|
| Q1 | **Corpus domain** for the local index & the 100-query eval set? (research/technical web? news? a specific field?) | Research/technical-web pages + arXiv/abstract-style docs (matches the operator's research-tooling usage) | Phase 2 eval set, engine trim |
| Q2 | **Expected QPS / concurrent users?** Single operator, occasional bursts? | Single-operator interactive use, ≤2 QPS sustained, ≤10 burst | Budgets §4, load tests |
| Q3 | **Internet exposure?** Will `meridiand` ever leave localhost/LAN? | Localhost only (other tenants on the Pi unaffected); no Caddy profile | API auth posture, P6 docs |
| Q4 | **WireGuard endpoints:** do any exist today for region lanes? Which regions? | None yet → RegionLane ships tested against a loopback/netns mock; real-endpoint verification deferred until one exists | Phase 4 (4.5–4.6) |
| Q5 | **.onion fetching** ever needed? | No → `allow_onion=false` permanently; `.onion` rejected on all lanes | egress config |
| Q6 | **CJK search** needed (lindera, ~15–35MB dicts, feature off by default)? Note: lindera-tantivy currently lags tantivy 0.26 | No CJK at launch | index tokenizer config |
| Q7 | **GeoLite2**: do you have/want a MaxMind account for the optional geoip feature (region-lane verification + analytics enrichment)? | No → region verification uses IP-echo only; analytics skips IP-region enrichment | Phase 4/5 features |
| Q8 | **Geocoder**: public Nominatim at ≤1 rps (attribution, fine for Profile R volume) or an operator-supplied endpoint/key? | Public Nominatim, 1 rps, aggressive redb caching | Phase 5 (5.2) |

## Answered (2026-06-11, Phases 7–9 planning)

| # | Question | Answer |
|---|---|---|
| A5 | Evidence block default-on or flag-gated? | **Default-on** with `evidence.enabled` kill-switch; compare-vantages stays strictly flag-gated (ADR-18/22) |
| A6 | Is a coarse-bucket per-decision routing log compatible with the "no query logging" promise? | **Yes, with safeguards** (no text/IPs, k-anon floor, 30d TTL, wipe path, anon never logged) — ADR-24 |
| A7 | Region-lane metasearch for vantage divergence? | **Fetch-only now**; per-region SearXNG sidecars (option b) recorded as the Phase-10 architecture behind a budget row + sign-off — ADR-22 |
| A8 | Versioning for the next phases? | One minor per phase: v0.2.0 (P7), v0.3.0 (P8), v0.4.0 (P9) |

## Open (Phases 7–9; defaults assumed)

| # | Question | Default assumed | Consumed by |
|---|---|---|---|
| Q9 | **BEIR subset** for the ranking-sanity eval (which 2–3 tasks fit Profile R's 100k-doc budget — SciFact ~5k docs and NFCorpus ~3.6k fit easily; FiQA ~57k is the stretch pick)? | SciFact + NFCorpus | Phase 8 (8.8), suite 13 eval set |
| Q10 | **Curated region-sensitive query set** for suite 12b (news/geopolitics/local-services classes; ~50 queries): operator-supplied topics or generic defaults? | Generic defaults (news + local-services templates), operator may extend | Phase 8 divergence gate |
| Q11 | **Compare-mode jitter window** (privacy/UX tradeoff: wider = less correlatable, slower) | 0–30s uniform, configurable; default ON | Phase 8 (8.5) |

## Open (Phase 10; defaults assumed)

| # | Question | Default assumed | Consumed by |
|---|---|---|---|
| Q12 | **Confidence-band risk targets** — **MOOT (suite 16, 2026-06-12)**: the default (90%) was unachievable outright AND the absolute claim collapsed under a query-style shift; bands were withdrawn pre-ship, so no target needs choosing until a stronger predictor exists | — | ADR-27 (REFUTED), suite 16 standing |
| Q13 | **Answer-mode passage cap** (length × count drive the second CE batch's cost) | ≤500 chars/passage, ≤32 passages/request | Phase 10 (10.6), ADR-29 |
| Q14 | **amd64 image flavor** — **default taken 2026-06-13**: gnu/distroless variant shipped (multi-arch manifest at v0.5.0); drop it if upstream lands a `__GLIBC__` guard | gnu/distroless | Phase 10 (10.8), done |
