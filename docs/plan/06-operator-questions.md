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
