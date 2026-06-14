# Region lanes — activation guide

Region lanes (`lane=region:<id>`) give a search its egress from a chosen
geographic vantage, so an operator can compare what the web returns from
different regions. They are **disabled by default** and ship **off on the live
appliance** because they need real per-region egress endpoints the operator must
provision. This guide is the turn-key recipe for when you have them.

> Status on the reference deployment (2026-06-14): region lanes are `disabled`
> (no WireGuard endpoints configured). `GET /v1/config` reports
> `features.regions_lane_enabled: false`; the console's Deploy panel shows the
> lane as "off (no WG endpoints)". This is honest-by-construction — Meridian
> never claims a region vantage it cannot actually route from.

## What a region lane is (and isn't)

- A region lane is an **egress source binding**: Meridian's outbound fetch /
  metasearch for that lane leaves from a region-specific endpoint (a WireGuard
  tunnel to a small VPS in that region, or an exit node). The SearXNG metasearch
  core (already in the stack) does the actual querying; the region only changes
  *where the request appears to come from*.
- Region metasearch needs a backend reachable over that egress; like the anon
  lane, a region lane **fails closed** (`503`) until its egress-IP verification
  passes — never a silent fallback to direct.
- Geo *filtering* (`lat`/`lon`/`h3`) is unrelated and applies to **local
  documents only** (ADR-10). Region lanes are about *egress vantage*, not about
  filtering local docs by location.

## Egress endpoint options

The cheapest clean architecture is **one small VPS per target region, each
running WireGuard**, and binding Meridian's region egress to that tunnel. Survey
of options (operator's choice — Meridian only needs a reachable egress per
region):

| Layer | Option | Why |
|---|---|---|
| VPN data plane | **WireGuard** (+ `wg-easy` web UI) | free/open; one tunnel per region VPS |
| Managed mesh w/ exit nodes | **NetBird** | open-source WireGuard overlay with official exit-node routing (free tier covers small fleets) |
| Self-hosted mesh | **Netmaker Community** / **Headscale + Tailscale** | full control over routing/egress/DNS |
| Per-container egress | **Gluetun** | run each regional search worker behind a different WG/OpenVPN endpoint |
| Per-process egress | **vopono** | route only selected processes through specific tunnels (good for testing many region exits on one host) |
| VPS endpoints | Oracle Always-Free / Hetzner / DigitalOcean (~$4–7/mo) | stable IP under your control — preferred over commercial VPN exits, which Google/Bing fingerprint and CAPTCHA |
| Commercial fallback exits | Mullvad (flat €5/mo, WG configs) / Proton VPN (WG, 140+ countries) / AirVPN | quick multi-country coverage where a VPS is awkward |

For benchmark-quality region comparisons prefer **VPS WireGuard endpoints** (a
stable, controlled IP). Commercial VPN exits work but are heavily fingerprinted,
which skews results with CAPTCHAs.

## Activation steps

1. **Provision an egress per region** (WireGuard VPS / NetBird exit node / …) and
   confirm its public IP from that region.
2. **Configure the region lanes** in the deployment (`MERIDIAN_LANES__*` env or
   the config file):
   - `MERIDIAN_LANES__REGIONS_ENABLED=true`
   - one `lanes.regions.<id>` entry per region (source binding / endpoint), and a
     region metasearch backend reachable over that egress.
3. **Bring the lane up** alongside the existing services; the lane stays `503`
   until egress-IP verification passes — verify with `GET /v1/lanes` (the region
   row should leave `disabled`).
4. **Query it**: `GET /v1/search?scope=web&lane=region:<id>&q=…`, or pick the
   region in the console's search panel lane selector.

Nothing here is a per-request knob — region configuration is deploy-time
(figment, loaded at startup). The console exposes region lanes read-only (the
Deploy panel) and as a first-class lane in the search selector; it never mutates
the server.
