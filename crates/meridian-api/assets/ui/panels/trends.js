// Trends panel — GET /v1/trends (GDELT media-coverage trends, ADR-21/-28).
//
// Honesty contract (api.md + appliance posture):
//   - `significant` (q ≤ 0.05) is the PRIMARY "this moved" badge. `z` and
//     `q_value` are shown as supporting detail, never as the headline claim.
//   - An elevated `ratio` WITHOUT significance carries the `label`
//     ("likely low-sample noise") — rendered prominently so a big raw ratio is
//     never mistaken for a real move.
//   - `burst` (sustained multi-day elevation) is a SEPARATE flag from a
//     different detector. When a mover is bursting we shade its trailing
//     elevated run on the chart (onset_day → onset_day + days_active) via
//     lineChart opts.shade — it never replaces or overrides significance.
//   - GDELT caveat shown verbatim: trends describe media COVERAGE, not the
//     world (ADR-15).
//   - 404 = analytics disabled ([analytics] enabled = false), an opt-in;
//     degrade gracefully, never a blank box or a fabricated value.
//
// Read-only: a single GET, no token (trends is unguarded), nothing persisted.
import { registerPanel, fetchJSON, el, chips, lineChart, fmtNum } from "../app.js";

// GDELT CAMEO EventRootCode 1..20 → short human labels (ADR-21 ranks roots).
// Kept local + offline (appliance posture: no lookup egress); unknown roots
// fall back to the bare code so we never invent a name we don't have.
const ROOT_NAMES = {
  1: "Make public statement",
  2: "Appeal",
  3: "Express intent to cooperate",
  4: "Consult",
  5: "Diplomatic cooperation",
  6: "Material cooperation",
  7: "Provide aid",
  8: "Yield",
  9: "Investigate",
  10: "Demand",
  11: "Disapprove",
  12: "Reject",
  13: "Threaten",
  14: "Protest",
  15: "Exhibit force posture",
  16: "Reduce relations",
  17: "Coerce",
  18: "Assault",
  19: "Fight",
  20: "Use unconventional mass violence",
};

function rootLabel(root) {
  const n = Number(root);
  const name = ROOT_NAMES[n];
  return name ? `${n} · ${name}` : `root ${root}`;
}

// `series` is [[day, count], …] where day is days since the Unix epoch. Convert
// to a UTC date string for axis tick labels (no timezone egress; pure Date math).
function dayToLabel(day) {
  if (!Number.isFinite(day)) return "—";
  const d = new Date(day * 86400 * 1000);
  // YYYY-MM-DD, UTC — explicit so the axis is unambiguous on any operator host.
  const mm = String(d.getUTCMonth() + 1).padStart(2, "0");
  const dd = String(d.getUTCDate()).padStart(2, "0");
  return `${d.getUTCFullYear()}-${mm}-${dd}`;
}

registerPanel({
  id: "panel-trends",
  title: "trends",
  refreshMs: 60000, // conservative: trends/geo >= 60s
  requiresBearer: false,
  async render(container) {
    const { ok, status, data, error } = await fetchJSON("/v1/trends");

    if (!ok) {
      // 404 = analytics disabled (GDELT opt-in). Distinguish it clearly from a
      // transport/HTTP failure; both degrade gracefully (never a blank box).
      const msg =
        status === 404
          ? "trends unavailable — analytics disabled ([analytics] enabled = false). This is a GDELT opt-in; no trend data is collected by default."
          : `trends unavailable (${status || "network"}: ${error || "unknown"})`;
      container.appendChild(el("div", { class: "panel-empty" }, msg));
      return;
    }

    // Defensive: the contract is { series: [[day,count]…], top_movers: [...] }.
    const rawSeries = Array.isArray(data && data.series) ? data.series : [];
    const movers = Array.isArray(data && data.top_movers) ? data.top_movers : [];

    // ---- series → points for the line chart -------------------------------
    // Each entry is a [day, count] pair, ascending by day.
    const points = rawSeries
      .filter((row) => Array.isArray(row) && row.length >= 2)
      .map((row) => ({ x: Number(row[0]), y: Number(row[1]) }))
      .filter((p) => Number.isFinite(p.x) && Number.isFinite(p.y));

    // ---- burst shade bands (SEPARATE channel from significance) -----------
    // For every mover whose burst is active, shade its trailing elevated run on
    // the count chart: [onset_day, onset_day + days_active]. This is the ADR-28
    // sustained-elevation signal — it is drawn behind the line and never stands
    // in for the `significant` (q ≤ 0.05) flag.
    const shade = [];
    for (const m of movers) {
      const b = m && m.burst;
      if (!b || !b.active) continue;
      const onset = Number(b.onset_day);
      const daysActive = Number(b.days_active);
      if (!Number.isFinite(onset)) continue;
      const span = Number.isFinite(daysActive) && daysActive > 0 ? daysActive : 1;
      shade.push({
        x0: onset,
        x1: onset + span,
        color: "rgba(214, 158, 46, 0.16)", // --warn, low alpha (burst band)
      });
    }

    // ---- header line: window summary --------------------------------------
    const summaryBits = [];
    if (points.length) {
      summaryBits.push(`${points.length} day${points.length === 1 ? "" : "s"}`);
      const first = points[0].x;
      const last = points[points.length - 1].x;
      summaryBits.push(`${dayToLabel(first)} → ${dayToLabel(last)}`);
    }
    summaryBits.push(`${movers.length} top mover${movers.length === 1 ? "" : "s"}`);
    container.appendChild(el("div", { class: "muted" }, summaryBits.join(" · ")));

    // ---- the chart --------------------------------------------------------
    if (points.length) {
      const canvas = el("canvas", { class: "chart", "aria-label": "event count over time" });
      // Mount before drawing so clientWidth/clientHeight resolve for HiDPI sizing.
      container.appendChild(canvas);
      lineChart(
        canvas,
        [{ label: "events/day", color: "#4f9cf2", points }],
        {
          shade,
          yLabel: "count",
          xFormat: dayToLabel,
          legend: true,
        }
      );
      if (shade.length) {
        // Make the shading legible — it is a real signal, but a distinct one.
        container.appendChild(
          el(
            "div",
            { class: "honesty-label" },
            "shaded band = sustained burst run (ADR-28), a separate detector from significance"
          )
        );
      }
    } else {
      container.appendChild(
        el(
          "div",
          { class: "panel-empty" },
          "no daily series returned for this window (burst stats need a window ≥ 7 days)"
        )
      );
    }

    // ---- top_movers table -------------------------------------------------
    if (movers.length) {
      container.appendChild(renderMoversTable(movers));
    } else {
      container.appendChild(
        el("div", { class: "panel-empty" }, "no top movers ranked for this window")
      );
    }

    // ---- honesty footer (GDELT caveat, verbatim intent of ADR-15) ---------
    container.appendChild(
      chips(
        null,
        [
          "trends describe media COVERAGE, not ground truth (GDELT, ADR-15)",
          "significant = q ≤ 0.05 (BH-FDR across roots) — the defensible “this moved” flag",
        ],
        "honesty"
      )
    );
  },
});

// Build the movers table. `significant` is the PRIMARY badge; z/q_value are
// supporting columns; the noise label and the burst flag each render as their
// own distinct, non-overriding marker.
function renderMoversTable(movers) {
  const head = el(
    "tr",
    {},
    el("th", { text: "root" }),
    el("th", { text: "moved?" }),
    el("th", { text: "latest" }),
    el("th", { text: "mean" }),
    el("th", { text: "ratio" }),
    el("th", { text: "shrunk" }),
    el("th", { text: "z" }),
    el("th", { text: "q" }),
    el("th", { text: "flags" })
  );

  const rows = movers.map((m) => {
    const significant = !!(m && m.significant);

    // PRIMARY badge: significance. A blue "moved" badge when q ≤ 0.05, a plain
    // dash otherwise — we never upgrade a non-significant mover to "moved".
    const movedCell = significant
      ? el("span", { class: "sig-badge sig-badge--moved", title: "q ≤ 0.05 (BH-FDR)" }, "moved")
      : el("span", { class: "muted", title: "not significant at q ≤ 0.05" }, "—");

    // Flags cell collects the SEPARATE, non-overriding markers:
    //   - the low-sample noise honesty label (only present when the API set it,
    //     i.e. elevated ratio WITHOUT significance);
    //   - the burst state (sustained elevation) with its onset + duration.
    const flagNodes = [];

    if (m && typeof m.label === "string" && m.label) {
      // Render the API's honesty marker prominently and verbatim.
      flagNodes.push(el("span", { class: "noise-label", title: "honest marker: elevated ratio without significance" }, m.label));
    }

    const b = m && m.burst;
    if (b && b.active) {
      const days = Number(b.days_active);
      const onset = Number(b.onset_day);
      const parts = ["burst"];
      if (Number.isFinite(days) && days > 0) parts.push(`${days}d`);
      const burstChip = el(
        "span",
        {
          class: "chip chip--burst",
          title:
            "sustained multi-day elevation (ADR-28) — a SEPARATE flag from significance" +
            (Number.isFinite(onset) ? `; onset day ${onset} (${dayToLabel(onset)})` : ""),
        },
        parts.join(" ")
      );
      flagNodes.push(burstChip);
    }

    const flagsCell = flagNodes.length
      ? el("div", { class: "chips", style: { margin: "0" } }, flagNodes)
      : el("span", { class: "muted" }, "—");

    return el(
      "tr",
      {},
      el("td", { title: `EventRootCode ${m && m.root}` }, rootLabel(m && m.root)),
      el("td", {}, movedCell),
      el("td", { text: fmtMaybe(m && m.latest) }),
      el("td", { text: fmtMaybe(m && m.mean) }),
      el("td", { text: fmtMaybe(m && m.ratio) }),
      el("td", { title: "empirical-Bayes shrunk rate", text: fmtMaybe(m && m.shrunk_rate) }),
      el("td", { title: "overdispersion-aware standardized excess", text: fmtMaybe(m && m.z) }),
      el("td", { title: "BH-FDR adjusted", text: fmtMaybe(m && m.q_value) }),
      el("td", {}, flagsCell)
    );
  });

  return el(
    "table",
    { class: "data" },
    el("thead", {}, head),
    el("tbody", {}, rows)
  );
}

function fmtMaybe(v) {
  const n = Number(v);
  return Number.isFinite(n) ? fmtNum(n) : "—";
}
