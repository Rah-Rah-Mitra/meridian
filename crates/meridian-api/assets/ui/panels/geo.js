// Geo heatmap panel — GET /v1/geo/heatmap (Getis-Ord Gi* hot spots, ADR-21).
//
// Honesty contract (api.md + appliance posture):
//   - Cells are colored by the Gi* z-score (`z`) on a diverging scale, but the
//     DEFENSIBLE "this is a hot spot" claim is the `significant` flag (q ≤ 0.05,
//     BH-FDR across all returned cells), rendered as a heavy ring on the map and
//     as the PRIMARY badge in the table. A big raw `count` alone is NOT a hot
//     spot — api.md is explicit about this — so the raw count is always kept
//     visible (disc area on the map + a column in the table) for explainability.
//   - Cells with null/non-finite lat/lon are dropped honestly (a coarser H3 cell
//     may roll up docs whose centroid we can't place); we count and disclose how
//     many were dropped rather than silently rendering fewer points.
//   - When the endpoint is down / 404s, the panel degrades to a clear message,
//     never a blank box or a fabricated value.
//
// Read-only: GETs only, no token (heatmap is unguarded), nothing persisted. The
// res/window controls only change GET query params; they never write anything.
import { registerPanel, fetchJSON, el, chips, hexMap, fmtNum } from "../app.js";

// Allowed knobs straight from the api.md contract. Kept local + offline; we
// never send a value outside the documented domain.
const RES_OPTIONS = [3, 4, 5, 6, 7]; // H3 resolution 3..=7 (docs indexed at 7)
const WINDOW_OPTIONS = ["24h", "7d", "all"]; // narrower-than-all excludes undated docs

// Panel-local control state. Conservative defaults match the API defaults so the
// first paint mirrors `GET /v1/geo/heatmap` with no params.
let _res = 5;
let _window = "7d";

registerPanel({
  id: "panel-geo",
  title: "geo heatmap",
  refreshMs: 60000, // conservative: trends/geo >= 60s
  requiresBearer: false,
  // The runtime hands us a fresh container on load and every tick. The whole
  // panel body lives in geoRender() so the res/window controls can re-invoke
  // the exact same code path without touching the runtime registry/timer.
  render(container) {
    return geoRender(container);
  },
});

// Build the res/window control bar. Changing either re-renders the panel into
// the same container the runtime handed us: each control mutates panel-local
// state then re-issues the GET. No persistence, no writes — read-only params.
function renderControls(container) {
  const onChange = () => {
    // Clear and re-run the render body into the SAME container the runtime owns
    // (panel-body-inner), so the panel's refresh timer and status keep working.
    container.replaceChildren();
    geoRender(container).catch((err) => {
      container.appendChild(
        el("div", { class: "panel-error" }, `panel failed to render: ${String(err && err.message ? err.message : err)}`)
      );
    });
  };

  const resSelect = el(
    "select",
    {
      "aria-label": "H3 resolution",
      title: "H3 resolution 3..7 — docs are indexed at res 7 and rolled up to coarser cells",
      on: { change: (e) => { _res = Number(e.target.value) || 5; onChange(); } },
    },
    RES_OPTIONS.map((r) =>
      el("option", { value: String(r), selected: r === _res ? true : null }, `res ${r}`)
    )
  );

  const windowSelect = el(
    "select",
    {
      "aria-label": "time window",
      title: "24h / 7d / all — windows narrower than 'all' exclude docs without a timestamp",
      on: { change: (e) => { _window = WINDOW_OPTIONS.includes(e.target.value) ? e.target.value : "7d"; onChange(); } },
    },
    WINDOW_OPTIONS.map((w) =>
      el("option", { value: w, selected: w === _window ? true : null }, w)
    )
  );

  return el(
    "div",
    { class: "search-form", style: { "margin-bottom": "0.5rem" } },
    el("label", { class: "muted", style: { "align-self": "center" } }, "resolution"),
    resSelect,
    el("label", { class: "muted", style: { "align-self": "center" } }, "window"),
    windowSelect
  );
}

// The render body, factored so the control bar can re-invoke it without touching
// the runtime registry. Mirrors the registerPanel({ render }) above exactly.
async function geoRender(container) {
  container.appendChild(renderControls(container));

  const params = new URLSearchParams();
  params.set("res", String(_res));
  params.set("window", _window);
  const { ok, status, data, error } = await fetchJSON(`/v1/geo/heatmap?${params.toString()}`);

  if (!ok) {
    container.appendChild(
      el(
        "div",
        { class: "panel-empty" },
        `heatmap unavailable (${status || "network"}: ${error || "unknown"})`
      )
    );
    return;
  }

  const allCells = Array.isArray(data && data.cells) ? data.cells : [];
  const effRes = data && Number.isFinite(Number(data.res)) ? Number(data.res) : _res;
  const placeable = allCells.filter(
    (d) => d && Number.isFinite(Number(d.lat)) && Number.isFinite(Number(d.lon))
  );
  const dropped = allCells.length - placeable.length;
  const sigCount = allCells.filter((d) => d && d.significant).length;

  const bits = [
    `${allCells.length} cell${allCells.length === 1 ? "" : "s"}`,
    `res ${effRes}`,
    `window ${_window}`,
    `${sigCount} significant hot spot${sigCount === 1 ? "" : "s"}`,
  ];
  if (dropped > 0) bits.push(`${dropped} without plottable lat/lon (dropped from map)`);
  container.appendChild(el("div", { class: "muted" }, bits.join(" · ")));

  if (allCells.length === 0) {
    container.appendChild(
      el(
        "div",
        { class: "panel-empty" },
        "no geo-tagged cells for this window — ingest with the offline gazetteer present (models/gazetteer.fst) to populate heatmaps"
      )
    );
  } else {
    const canvas = el("canvas", {
      class: "chart map",
      "aria-label": "geo heatmap — disc area is raw count, color is Gi* z, heavy ring marks a significant hot spot",
    });
    container.appendChild(canvas);
    const mapCells = placeable.map((d) => ({
      lat: Number(d.lat),
      lon: Number(d.lon),
      value: Number(d.z),
      significant: !!d.significant,
      count: Number(d.count) || 0,
    }));
    hexMap(canvas, mapCells, { valueLabel: "Gi* z" });
    container.appendChild(
      el(
        "div",
        { class: "honesty-label" },
        "color = Getis-Ord Gi* z (cool ↔ hot); the heavy ring = significant hot spot (q ≤ 0.05, BH-FDR) — the only defensible hot-spot claim. Disc area = raw count, kept visible for explainability."
      )
    );
  }

  if (allCells.length) container.appendChild(renderCellsTable(allCells));

  container.appendChild(
    chips(
      null,
      [
        "significant = q ≤ 0.05 (Getis-Ord Gi*, BH-FDR across returned cells) — the defensible hot-spot flag; a big raw count alone is not",
        "geo tags come from the offline gazetteer at ingest — no network call (ADR-10); heatmap covers local docs only",
      ],
      "honesty"
    )
  );
}

// Per-cell table: the raw count stays a first-class column (api.md: a big count
// alone is not a hot spot), z/q are supporting detail, and `significant` is the
// PRIMARY badge — never upgraded from a large count or a large |z|. Cells the map
// could not place are marked so the table and the map agree.
function renderCellsTable(cells) {
  const head = el(
    "tr",
    {},
    el("th", { text: "h3" }),
    el("th", { text: "hot spot?" }),
    el("th", { text: "count" }),
    el("th", { text: "Gi* z" }),
    el("th", { text: "q" }),
    el("th", { text: "lat" }),
    el("th", { text: "lon" })
  );

  // Sort by raw count descending so the busiest cells lead — explainable and
  // independent of the (separate) significance judgement.
  const sorted = cells
    .slice()
    .sort((a, b) => (Number(b && b.count) || 0) - (Number(a && a.count) || 0));

  const rows = sorted.map((d) => {
    const significant = !!(d && d.significant);
    const placeable =
      d && Number.isFinite(Number(d.lat)) && Number.isFinite(Number(d.lon));

    const hotCell = significant
      ? el(
          "span",
          { class: "sig-badge", title: "Getis-Ord Gi* q ≤ 0.05 (BH-FDR across returned cells)" },
          "hot spot"
        )
      : el("span", { class: "muted", title: "not a significant hot spot at q ≤ 0.05" }, "—");

    return el(
      "tr",
      {},
      el(
        "td",
        { title: `H3 cell ${d && d.h3}` + (placeable ? "" : " — no plottable lat/lon (dropped from map)") },
        (d && d.h3 != null ? String(d.h3) : "—") + (placeable ? "" : " ⚐")
      ),
      el("td", {}, hotCell),
      el("td", { title: "raw document count in this cell (kept visible — a big count alone is not a hot spot)" }, fmtMaybe(d && d.count)),
      el("td", { title: "Getis-Ord Gi* z-score over the H3 k-ring-1 neighborhood" }, fmtMaybe(d && d.z)),
      el("td", { title: "Benjamini-Hochberg FDR adjusted p-value" }, fmtMaybe(d && d.q_value)),
      el("td", { text: fmtMaybe(d && d.lat) }),
      el("td", { text: fmtMaybe(d && d.lon) })
    );
  });

  return el("table", { class: "data" }, el("thead", {}, head), el("tbody", {}, rows));
}

function fmtMaybe(v) {
  const n = Number(v);
  return Number.isFinite(n) ? fmtNum(n) : "—";
}
