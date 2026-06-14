// Deployment & config panel — a read-only view of the effective (D) deploy/index
// config (each field with the MERIDIAN_<STRUCT>__<FIELD> env var to change it),
// plus feature availability: whether the cross-encoder is present, so the console
// shows deep/answer/corroboration as LIVE vs DORMANT honestly. Sourced from
// GET /v1/config. The console stays read-only — nothing here mutates the server.

import { registerPanel, getConfig, el } from "../app.js";

function featureRow(label, on, onText, offText, neutral) {
  const cls = neutral ? "chip--info" : on ? "chip--ok" : "chip--degraded";
  return el(
    "div",
    { class: "cfg-row" },
    el("span", { class: "cfg-key", text: label }),
    el("span", { class: "chip " + cls, text: on ? onText || "on" : offText || "off" })
  );
}

function cfgSection(title, obj) {
  const wrap = el("div", {}, el("div", { class: "section-title", text: title }));
  const env = obj.env_prefix;
  const note = obj.note;
  for (const [k, v] of Object.entries(obj)) {
    if (k === "note" || k === "env_prefix") continue;
    const row = el(
      "div",
      { class: "cfg-row" },
      el("span", { class: "cfg-key", text: k }),
      el("span", { class: "cfg-val", text: String(v) })
    );
    if (env) row.appendChild(el("span", { class: "cfg-env", text: `${env}${k.toUpperCase()}` }));
    wrap.appendChild(row);
  }
  if (note) wrap.appendChild(el("div", { class: "cfg-warn", text: note }));
  return wrap;
}

async function render(container) {
  const { ok, status, data, error } = await getConfig({ force: true });
  if (!ok || !data) {
    if (status === 401) {
      container.appendChild(
        el("div", { class: "panel-empty" }, "config is bearer-gated on this deployment — enter the operator token above.")
      );
    } else {
      container.appendChild(el("div", { class: "panel-empty" }, `config unavailable (${error || status})`));
    }
    return;
  }

  const f = data.features || {};

  // ---- image / version + CE availability ----
  container.appendChild(el("div", { class: "section-title", text: "image" }));
  container.appendChild(
    el(
      "div",
      { class: "cfg-row" },
      el("span", { class: "cfg-key", text: "version" }),
      el("span", { class: "cfg-val", text: data.version || "?" })
    )
  );
  const ce = !!f.cross_encoder_present;
  container.appendChild(
    el(
      "div",
      { class: "cfg-row" },
      el("span", { class: "cfg-key", text: "cross-encoder (ort)" }),
      el("span", { class: "chip " + (ce ? "chip--ok" : "chip--degraded"), text: ce ? "present" : "absent" }),
      el("div", {
        class: ce ? "cfg-why" : "cfg-warn",
        text: ce
          ? "deep mode, answer mode and C1 corroboration are LIVE."
          : "deep / answer / corroboration are DORMANT — the scratch appliance image carries no cross-encoder (ADR-02). Deploy the deep/gnu image to activate them.",
      })
    )
  );

  // ---- feature availability ----
  container.appendChild(el("div", { class: "section-title", text: "features" }));
  container.appendChild(featureRow("deep mode", f.deep_available, "live", "dormant"));
  container.appendChild(featureRow("answer mode", f.answer_available, "live", "dormant"));
  container.appendChild(featureRow("corroboration (C1)", f.corroboration_available, "live", "dormant"));
  container.appendChild(featureRow("evidence clustering", f.evidence_enabled, "on", "off"));
  container.appendChild(featureRow("analytics / GDELT", f.analytics_enabled, "on", "off"));
  container.appendChild(featureRow("trends endpoint", f.trends_available, "available", "404 (analytics off)"));
  container.appendChild(featureRow("decision log (ADR-24)", f.decision_log_enabled, "accruing", "off"));
  container.appendChild(featureRow("anon lane", f.anon_lane_enabled, "enabled", "off"));
  container.appendChild(featureRow("region lanes", f.regions_lane_enabled, "enabled", "off (no WG endpoints)"));
  // contextual_policy is the safety-gated one — explain WHY it's dark.
  container.appendChild(
    el(
      "div",
      { class: "cfg-row" },
      el("span", { class: "cfg-key", text: "contextual policy (ADR-25)" }),
      el("span", { class: "chip " + (f.contextual_policy ? "chip--ok" : "chip--info"), text: f.contextual_policy ? "ON" : "dark" }),
      el("div", {
        class: "cfg-why",
        text: f.contextual_policy
          ? "experimental LinTS routing is ON."
          : "left dark BY DESIGN: the ADR-25 ship-gate (doubly-robust uplift CI excluding zero on ≥10k logged decisions) is unmet — enabling it would route on an unvalidated policy. See the OPE gate panel for the live verdict.",
      })
    )
  );

  // ---- read-only deploy config (D), each with its env var ----
  const d = data.deploy || {};
  for (const key of ["index", "vector", "server", "analytics", "lanes", "searx"]) {
    if (d[key]) container.appendChild(cfgSection(key, d[key]));
  }

  container.appendChild(
    el("div", {
      class: "honesty",
      text: "read-only: these are loaded at startup (figment). Change them via the env vars shown, then restart the appliance — the console never mutates the server (ADR-02).",
    })
  );
}

registerPanel({
  id: "panel-deploy",
  title: "deployment & config",
  refreshMs: 0,
  requiresBearer: false,
  render,
});
