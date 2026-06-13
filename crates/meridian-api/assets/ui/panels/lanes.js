// Lanes panel — GET /v1/lanes (unauthenticated, GET-only).
//
// This is the degrade-gracefully ANCHOR of the console: every other panel
// borrows its failure idiom from here. The endpoint reports lane health
// honestly (api.md §GET /v1/lanes):
//
//   [ { "id": "direct", "status": "up",            "detail": null },
//     { "id": "anon",   "status": "bootstrapping", "detail": "45%" },
//     { "id": "region:sg", "status": "degraded",   "detail": "egress IP mismatch" } ]
//
//   status ∈ up | bootstrapping | degraded | down | disabled
//
// We render one row per lane: a colored status dot (CSS .status-<status>), the
// lane id, a status chip, and the operator-facing `detail` string verbatim when
// present. When the endpoint itself is unreachable or errors we show a clear
// message — NEVER a blank box and NEVER a fabricated lane.
import { registerPanel, fetchJSON, el, chips } from "../app.js";

// The five honest states from api.md, in operational severity order. Any status
// the API sends that is NOT in this set is still rendered verbatim (honest about
// the unknown) rather than dropped or coerced.
const KNOWN_STATUS = new Set([
  "up",
  "bootstrapping",
  "degraded",
  "down",
  "disabled",
]);

// Short, non-alarming gloss per state — explains what the operator is looking at
// without overstating. `detail` from the API (when present) is shown verbatim
// alongside and always wins for specifics.
const STATUS_GLOSS = {
  up: "serving",
  bootstrapping: "coming up",
  degraded: "serving, impaired",
  down: "not serving",
  disabled: "off (not configured)",
};

registerPanel({
  id: "panel-lanes",
  title: "lane health",
  refreshMs: 10000, // conservative appliance polling: lanes ≥ 10s
  requiresBearer: false, // /v1/lanes is unauthenticated
  async render(container) {
    const { ok, status, data, error } = await fetchJSON("/v1/lanes");

    // --- Endpoint down / transport error: degrade gracefully. ----------------
    // status 0 = network/transport failure (meridiand unreachable); any non-2xx
    // is surfaced with the problem+json title/detail fetchJSON already distilled.
    if (!ok) {
      const where = status === 0 ? "meridiand unreachable" : `HTTP ${status}`;
      container.appendChild(
        el(
          "div",
          { class: "panel-empty" },
          `lane health unavailable — ${where}${error ? `: ${error}` : ""}. ` +
            "The appliance may be starting, stopped, or unreachable from here."
        )
      );
      return;
    }

    // The contract is a top-level JSON array. Anything else is treated as
    // "nothing to show" rather than guessed at.
    const lanes = Array.isArray(data) ? data : [];
    if (lanes.length === 0) {
      container.appendChild(
        el(
          "div",
          { class: "panel-empty" },
          "no lanes reported (the deployment exposes none, or all are disabled)"
        )
      );
      return;
    }

    // --- One row per lane. ---------------------------------------------------
    const list = el("dl", { class: "kv" });
    for (const lane of lanes) {
      const id = lane && lane.id != null ? String(lane.id) : "(unnamed lane)";
      const laneStatus =
        lane && lane.status != null ? String(lane.status) : "unknown";
      const known = KNOWN_STATUS.has(laneStatus);
      const detail =
        lane && lane.detail != null && String(lane.detail).trim() !== ""
          ? String(lane.detail)
          : null;

      // dt: colored dot (CSS .status-<status>; unknown statuses fall through to
      // a neutral dot) + the lane id.
      const dot = el("span", {
        class: `status-dot status-${known ? laneStatus : "disabled"}`,
        title: known ? laneStatus : `unrecognized status: ${laneStatus}`,
      });
      const dt = el("dt", {}, dot, id);

      // dd: a status chip + gloss + verbatim operator detail.
      const dd = el("dd", {});
      dd.appendChild(
        el("span", { class: "chip chip--status", text: laneStatus })
      );
      const gloss = STATUS_GLOSS[laneStatus];
      if (gloss) {
        dd.appendChild(el("span", { class: "honesty", text: ` ${gloss}` }));
      } else if (!known) {
        // Honest about an out-of-contract status rather than hiding it.
        dd.appendChild(
          el("span", {
            class: "honesty",
            text: " status not in the documented set — shown verbatim",
          })
        );
      }
      if (detail) {
        // Operator-facing detail (e.g. "45%", "egress IP mismatch"): verbatim.
        dd.appendChild(el("span", { text: " — " }));
        dd.appendChild(el("span", { class: "lane-detail", text: detail }));
      }

      list.appendChild(dt);
      list.appendChild(dd);
    }
    container.appendChild(list);

    // --- Footnote: surface any non-up lanes as a quick-scan summary. ---------
    // Pure read-out of the same data — no judgement, just visibility so an
    // operator does not have to scan every row to see something is off.
    const notUp = lanes
      .filter((l) => l && l.status && String(l.status) !== "up")
      .map((l) => `${l.id}: ${l.status}`);
    if (notUp.length > 0) {
      chips(container, notUp, "degraded");
    }
  },
});
