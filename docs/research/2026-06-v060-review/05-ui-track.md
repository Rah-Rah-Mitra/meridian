# 05 — Operator UI track (the embedded analytics console)

Status: REVIEW — proposes, does not decide. As-of: v0.6.0 / `812d3d4` / 2026-06-13.

Meridian ships every analytic the operator needs over the JSON API (`/v1/lanes`,
`/v1/trends`, `/v1/geo/heatmap`, `/v1/search`, `/v1/decision-log/ope`) but has no
human surface for them — an operator reads hot-spots, trend bursts, lane health,
and the ADR-25 ship-gate by curling JSON. This track proposes a **read-only
operator console** embedded in the `meridiand` binary at `/ui/*`. It is a *view
of existing endpoints*, not a new capability: it issues GETs to handlers that
already exist, carries their honesty labels verbatim, and adds **zero** new
collection surface, **zero** third-party JavaScript, and **zero** new Cargo
dependencies.

The governing tension is honesty under a GUI's pressure to round things off: a
chart invites "this number means X" in a way a JSON field does not. The whole
design is organized so the UI can never claim more certainty than the API
already states (the confidence score stays "uncalibrated", `ce_score` stays
"relevance, not correctness", `significant`/`burst.active` stay independent
flags, `insufficient_data` stays a first-class non-error state).

## Candidate-ADR sketch — "embedded read-only operator console"

**Decision (proposed).** Ship the operator UI as static assets compiled INTO the
`meridiand` musl binary via `include_bytes!` (`crates/meridian-api/assets/ui/`),
served GET-only at `/ui/*` by a new `meridian-api::ui` module. The assets are
vanilla ES modules + plain CSS + hand-rolled `<canvas>` charts — no framework,
no bundler, no CDN, no third-party JS, no new dependency, no `tower-http` fs
feature. The shell is unauthenticated; the JS holds an operator-supplied bearer
in a page-session variable and attaches it ONLY to guarded endpoints. The
console is a *view*, never a new published surface or a new privacy boundary.

1. **Exposure inherits the existing gate, adds nothing.** The `/ui` routes merge
   at the top level OUTSIDE the `/v1/*` guardrail + body-limit layer
   (lib.rs `router()`): immutable in-binary bytes are not a rate-limit or
   concurrency-shed surface, and serving them is a memcpy, not an amplifier. The
   shell is GET-only and unauthenticated by the same logic that leaves
   `/healthz` open — but it exposes **no data on its own**; every datum arrives
   through a `/v1/*` handler that keeps its own guardrails, bearer checks, and
   `404`/`503` semantics. The console therefore inherits, and does not weaken,
   the operator-manual.md "enable auth + TLS before exposing beyond loopback"
   gate: an exposed-without-auth deployment was already mis-configured; the UI
   does not change what is reachable, only that a browser can render it. No new
   route reaches `/v1/ingest`, `/v1/forget`, `/v1/fetch`, or
   `/v1/decision-log/wipe` — the UI is structurally incapable of mutation
   (GET-only `fetchJSON`, no POST primitive exists in the asset bundle).

2. **Privacy: read-only, no new collection, no client-side persistence —
   proposed invariant.** The UI MUST NOT persist anything: no `localStorage`, no
   `sessionStorage`, no cookie, no `Set-Cookie` (the API already sets none). The
   operator bearer lives in ONE module variable for the page session and is
   gone when the tab closes; it is attached as `Authorization: Bearer <token>`
   only on requests to guarded endpoints (`/v1/decision-log/ope`, and `/v1/search`
   if and only if it `401`s). `fetchJSON` sends `credentials: "omit"` and
   `referrer: no-referrer` so no ambient cookie or referrer leaks. Query text
   typed into the search panel rides the existing "never logged" `q` contract —
   the UI introduces no log line, no telemetry beacon, and no analytics counter
   of its own. The privacy smoke test is therefore **unaffected**: there is no
   new data class to assert on, no new egress, and the access-event redaction
   (route/status/latency only) already covers `/ui/*` requests (route is a
   static path, never a query).

3. **Asset-vendoring / egress: zero third-party code, hand-rolled canvas —
   proposed invariant.** The bundle contains NO third-party JavaScript and loads
   NOTHING over the network: no uPlot, no h3-js, no chart/map library, no web
   font, no tile basemap, no CDN `<script>`/`<link>`. Charts are a ~small
   hand-drawn `<canvas>` renderer (`lineChart`/`barChart`/`hexMap`/`gauge`) — a
   tiny renderer is lighter than vendoring a library AND eliminates the entire
   supply-chain + egress class in one move (this is the *point*, not a
   limitation). Geo uses an equirectangular lat/lon projection of the heatmap
   cells the API already returns — no map tiles, no reverse-geocode, no external
   call. All assets are `include_bytes!`'d into the FROM-scratch single binary;
   the binary remains self-contained and reproducible, and no `Cargo.toml`
   line is added (`&'static [u8]` already implements `IntoResponse`).

4. **Honesty under a GUI — proposed invariant (the load-bearing one).** Every
   surface that a chart would tempt into over-claiming is pinned to the API's
   own words: (a) `confidence.score` renders with the label "uncalibrated,
   ranking-comparable on this deployment, NOT a probability"; (b)
   `best_passage` shows `ce_score` as "relevance, not a correctness
   probability" and the passage as "verbatim from the fetched page, never
   generated"; (c) every `degraded[]` flag renders as a visible chip and
   `lane_requested` vs `lane_effective` are shown whenever they differ; (d) in
   trends the `significant` flag (q≤0.05) is the primary "this moved" badge, the
   "likely low-sample noise" label renders prominently when present, and
   burst-state shading is a SEPARATE visual channel that never replaces or
   stands in for significance; (e) in geo, cells are colored by Gi* `z` but the
   ring (defensible "hot spot") is driven by `significant`, with the raw `count`
   always visible for explainability; (f) the OPE `insufficient_data` verdict is
   a first-class, non-alarming state ("not enough decisions yet"), shows
   `n_total` against the 10000 min-decisions bar, and prints the `gate.rule`
   string verbatim. A down/`404`/`503` lane or endpoint degrades to a clear
   message, never a blank box or a fabricated value.

5. **Budget / polling discipline — proposed invariant.** Polling is conservative
   and bounded by the registry: lanes ≥ 10s, trends/geo ≥ 60s, search/OPE
   on-demand only (no background poll). The runtime schedules at most one timer
   per panel, all GET, all subject to the existing `/v1/*` rate limit (5 rps /
   burst 20) — five panels at their floors stay an order of magnitude under it.
   No websocket, no SSE, no long-poll: the appliance keeps its synchronous,
   request/response posture. Asset responses carry `Cache-Control: no-cache,
   max-age=0` so an in-place binary upgrade is reflected on the next load (the
   bytes are immutable per build, but we do not want a stale console after an
   upgrade).

## Rejections (engaged, then rejected)

- **`rust-embed` / `include_dir` for asset embedding.** Both are real crates
  that would tidy the `lookup()` match into a directory walk — and both are a
  new dependency for something a one-screen `match` on `include_bytes!` already
  does. The asset set is fixed and small (8 paths); the explicit table is the
  honest amount of machinery, keeps the dependency count at zero, and keeps the
  served paths auditable at a glance (no glob can smuggle an asset in). Rejected
  on the "no new Cargo deps" bar; revisit only if the asset set grows past a
  hand-maintainable table.

- **A separate static-file container (nginx/caddy sidecar) serving the UI.**
  Breaks the single-binary appliance model: a second image to build, pin,
  patch, and digest-track; a second port and TLS surface; and a split between
  "the API" and "the console" that the operator must reason about. The whole
  value of `include_bytes!` is that the console ships, versions, and is exposed
  exactly with the binary it views. Rejected — the embedded module is strictly
  less surface than a sidecar.

- **Any CDN / vendored library / tile source.** `<script src="https://cdn…">`,
  a bundled `uPlot`/`h3-js`/charting lib, a web font, or an OSM/Mapbox tile
  layer would each (a) add a supply-chain dependency to audit and vendor, and
  (b) for tiles/fonts, create per-load egress from the operator's browser to a
  third party — exactly the disclosure class the appliance exists to avoid. The
  hand-rolled canvas renderer is smaller than any of them and has no egress.
  Rejected categorically; this is the track's defining constraint, not a
  trade-off to revisit.

- **Rendering scores as probabilities (or any calibrated-looking dressing).**
  Tempting for a GUI: a "61% confident" pill, a green/red "correct/incorrect"
  badge on the best passage, a probability bar on the OPE uplift. Every one of
  these would assert a calibration the API explicitly disclaims
  (`confidence.score` is uncalibrated; `ce_score` is relevance, not
  correctness; the OPE CI is an uplift estimate, not a P(ship)). Rejected as a
  direct violation of invariant 4 — the UI's honesty floor IS the API's, and
  the GUI must round toward less certainty, never more.

- **An async/streaming live console (websocket/SSE push of search and trends).**
  Would smooth the UX but breaks the synchronous request/response posture the
  rest of the system is designed around (the compare-jitter analysis in
  lib.rs:373–385 is one symptom of how the sync ceiling shapes everything), and
  a persistent connection is a new long-lived surface and a new resource class.
  Rejected for v0.6.0; conservative polling (invariant 5) is sufficient for an
  operator console and stays inside the existing guardrails. An async surface is
  a later candidate IF a real-time need is demonstrated, priced against the
  request-ceiling work the compare path already flagged.

---

*Cross-refs: api.md (the canonical response shapes this UI binds to: `/v1/search`
confidence + `best_passage` + `degraded` + `lane_*`, `/v1/trends`
`significant`/`label`/`burst`, `/v1/geo/heatmap` `z`/`significant`/`count`,
`/v1/decision-log/ope` `gate`/`insufficient_data`); operator-manual.md
(enable-auth-and-TLS-before-exposing gate); ADR-29 (extractive-only honesty
stance the best-passage rendering inherits). Not assigned an ADR number — latest
minted is ADR-29.*
