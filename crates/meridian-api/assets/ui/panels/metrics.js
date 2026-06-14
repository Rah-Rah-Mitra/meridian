// Metrics & observability panel — parses GET /metrics (Prometheus text) entirely
// client-side and visualizes request rates/latency, lane states, cache/index/
// store occupancy, shed states, free disk, GDELT rows; /healthz for uptime.
// Public (aggregate-only metrics, bounded label cardinality — SPEC §13.4).

import { registerPanel, fetchJSON, el, barChart, fmtNum } from "../app.js";

// --- tiny Prometheus text parser: returns [{name, labels:{}, value}] ----------
function parseProm(text) {
  const out = [];
  for (const raw of String(text).split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const m = line.match(/^([a-zA-Z_:][a-zA-Z0-9_:]*)(\{[^}]*\})?\s+([-+0-9.eE]+|NaN|[-+]Inf)$/);
    if (!m) continue;
    const [, name, rawLabels, rawVal] = m;
    const labels = {};
    if (rawLabels) {
      for (const part of rawLabels.slice(1, -1).split(",")) {
        const eq = part.indexOf("=");
        if (eq < 0) continue;
        const k = part.slice(0, eq).trim();
        const v = part.slice(eq + 1).trim().replace(/^"|"$/g, "");
        if (k) labels[k] = v;
      }
    }
    let value = Number(rawVal);
    if (rawVal === "+Inf") value = Infinity;
    else if (rawVal === "-Inf") value = -Infinity;
    out.push({ name, labels, value });
  }
  return out;
}

const bytes = (n) => {
  const v = Number(n);
  if (!Number.isFinite(v)) return "—";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let x = v;
  while (x >= 1024 && i < u.length - 1) {
    x /= 1024;
    i++;
  }
  return `${x.toFixed(x >= 100 || i === 0 ? 0 : 1)} ${u[i]}`;
};

const dur = (secs) => {
  const s = Math.floor(Number(secs) || 0);
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m ${s % 60}s`;
};

// Aggregate cumulative-histogram buckets across all routes → interpolated pXX.
function histPercentile(samples, p) {
  // samples: meridian_request_ms_bucket{le=...} entries (cumulative across routes)
  const byLe = new Map();
  let total = 0;
  for (const s of samples) {
    const le = s.labels.le;
    if (le == null) continue;
    const key = le === "+Inf" ? Infinity : Number(le);
    byLe.set(key, (byLe.get(key) || 0) + s.value);
  }
  const buckets = [...byLe.entries()].sort((a, b) => a[0] - b[0]);
  if (!buckets.length) return null;
  total = buckets[buckets.length - 1][1]; // +Inf bucket is the grand total
  if (total <= 0) return null;
  const target = total * p;
  let prevLe = 0;
  let prevCum = 0;
  for (const [le, cum] of buckets) {
    if (cum >= target) {
      if (!Number.isFinite(le)) return prevLe; // in the overflow bucket — report the last finite edge
      const frac = (target - prevCum) / Math.max(1, cum - prevCum);
      return prevLe + frac * (le - prevLe);
    }
    prevLe = Number.isFinite(le) ? le : prevLe;
    prevCum = cum;
  }
  return prevLe;
}

function card(container, value, label, sub, kind) {
  container.appendChild(
    el(
      "div",
      { class: "card" + (kind ? ` card--${kind}` : "") },
      el("div", { class: "card-value", text: value }),
      el("div", { class: "card-label", text: label }),
      sub ? el("div", { class: "card-sub", text: sub }) : null
    )
  );
}

async function render(container) {
  const [metricsRes, health] = await Promise.all([
    fetchJSON("/metrics"),
    fetchJSON("/healthz"),
  ]);

  if (!metricsRes.ok && metricsRes.data == null && health.status === 0) {
    container.appendChild(el("div", { class: "panel-empty" }, "metrics endpoint unreachable"));
    return;
  }
  // /metrics is Prometheus text, not JSON — fetchJSON leaves it in `error`/text.
  // Re-fetch as text for parsing (fetchJSON only parses json bodies).
  let text = "";
  try {
    const r = await fetch("/metrics", { cache: "no-store", credentials: "omit" });
    text = await r.text();
  } catch (_) {
    container.appendChild(el("div", { class: "panel-empty" }, "metrics endpoint unreachable"));
    return;
  }
  const m = parseProm(text);
  const get1 = (name) => {
    const e = m.find((x) => x.name === name);
    return e ? e.value : null;
  };
  const all = (name) => m.filter((x) => x.name === name);

  // ---- headline cards ----
  const cards = el("div", { class: "cards" });
  const uptime = health.ok && health.data ? health.data.uptime_secs : null;
  card(cards, uptime != null ? dur(uptime) : "—", "uptime", health.ok ? "healthz ok" : "healthz down", health.ok ? null : "bad");

  const reqs = all("meridian_requests_total");
  const totalReq = reqs.reduce((s, x) => s + x.value, 0);
  const errReq = reqs.filter((x) => Number(x.labels.status) >= 500).reduce((s, x) => s + x.value, 0);
  card(cards, fmtNum(totalReq), "requests total", `${fmtNum(errReq)} 5xx`, errReq > 0 ? "warn" : null);

  const free = get1("meridian_free_disk_bytes");
  card(cards, free != null ? bytes(free) : "—", "free disk", null, free != null && free < 2 * 1024 ** 3 ? "warn" : null);

  const gdelt = get1("meridian_gdelt_rows_total");
  if (gdelt != null) card(cards, fmtNum(gdelt), "gdelt rows", "analytics feed");

  // latency percentiles from the request histogram
  const buckets = all("meridian_request_ms_bucket");
  if (buckets.length) {
    const p50 = histPercentile(buckets, 0.5);
    const p90 = histPercentile(buckets, 0.9);
    const p99 = histPercentile(buckets, 0.99);
    card(cards, p50 != null ? `${fmtNum(p50)}ms` : "—", "latency p50", `p90 ${p90 != null ? fmtNum(p90) : "—"} · p99 ${p99 != null ? fmtNum(p99) : "—"} ms`);
  }
  container.appendChild(cards);

  // ---- requests by route (bar chart) ----
  if (reqs.length) {
    // sum across statuses per route
    const byRoute = new Map();
    for (const r of reqs) {
      const route = r.labels.route || "?";
      byRoute.set(route, (byRoute.get(route) || 0) + r.value);
    }
    const bars = [...byRoute.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 8)
      .map(([route, v]) => ({ label: route.replace("/v1/", ""), value: v, color: "var(--accent)" }));
    container.appendChild(el("div", { class: "section-title", text: "requests by route" }));
    const c = el("canvas", { class: "chart" });
    container.appendChild(c);
    requestAnimationFrame(() => barChart(c, bars, { yLabel: "count", valueFormat: fmtNum }));
  }

  // ---- store / cache occupancy ----
  const stores = all("meridian_store_bytes");
  const cacheEntries = all("meridian_cache_entries");
  const cacheBytes = all("meridian_cache_weighted_bytes");
  if (stores.length || cacheBytes.length) {
    container.appendChild(el("div", { class: "section-title", text: "storage & cache" }));
    const tbl = el("table", { class: "data" });
    tbl.appendChild(
      el("thead", {}, el("tr", {}, el("th", { text: "store / cache" }), el("th", { class: "num", text: "bytes" }), el("th", { class: "num", text: "entries" })))
    );
    const tb = el("tbody");
    for (const s of stores.sort((a, b) => b.value - a.value)) {
      tb.appendChild(el("tr", {}, el("td", { text: s.labels.store || "?" }), el("td", { class: "num", text: bytes(s.value) }), el("td", { class: "num", text: "—" })));
    }
    for (const c of cacheBytes) {
      const ent = cacheEntries.find((e) => e.labels.cache === c.labels.cache);
      tb.appendChild(el("tr", {}, el("td", { text: `cache:${c.labels.cache || "?"}` }), el("td", { class: "num", text: bytes(c.value) }), el("td", { class: "num", text: ent ? fmtNum(ent.value) : "—" })));
    }
    tbl.appendChild(tb);
    container.appendChild(tbl);
  }

  // ---- shed states (load-shedding stages) ----
  const shed = all("meridian_shed_state");
  if (shed.length) {
    container.appendChild(el("div", { class: "section-title", text: "load-shed stages" }));
    const chips = el("div", { class: "chips" });
    for (const s of shed) {
      const on = s.value > 0;
      chips.appendChild(el("span", { class: "chip" + (on ? " chip--degraded" : " chip--ok"), text: `${s.labels.stage || "?"}: ${on ? "shedding" : "ok"}` }));
    }
    container.appendChild(chips);
  }

  container.appendChild(
    el("div", { class: "honesty", text: "metrics are aggregate-only with bounded label cardinality — no query text, no client IPs (SPEC §13.4)." })
  );
}

registerPanel({
  id: "panel-metrics",
  title: "metrics & observability",
  refreshMs: 12000,
  requiresBearer: false,
  render,
});
