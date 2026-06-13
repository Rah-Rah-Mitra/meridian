// Meridian operator console — the shared runtime + the contract every panel
// imports. Vanilla ES module, zero third-party JS, hand-rolled <canvas>.
//
// Appliance posture: this file makes GET requests only, never persists anything
// (no localStorage, no cookie), and attaches the operator bearer ONLY when a
// caller explicitly opts in (opts.bearer). It is the single source of truth for
// the DOM/chart/fetch helpers the five panel modules build on.

// ---------------------------------------------------------------------------
// Session-only bearer token. Held in a module variable for the page session
// ONLY — deliberately not in localStorage and never in a cookie. Cleared when
// the tab closes.
// ---------------------------------------------------------------------------
let _bearer = "";

/** The current operator token (may be ""). Panels usually receive it via the
 *  render() ctx, but on-demand actions can read it here. */
export function currentBearer() {
  return _bearer;
}

// ---------------------------------------------------------------------------
// fetchJSON — the one network primitive. GET-only. Never throws.
//   returns { ok, status, data, error }
//   - ok:    response.ok (2xx)
//   - status: HTTP status (0 on a network/transport error)
//   - data:  parsed JSON body when present, else null
//   - error: a short string when !ok or on transport failure, else null
// If opts.bearer is truthy, attach `Authorization: Bearer <bearer>`.
// ---------------------------------------------------------------------------
export async function fetchJSON(path, opts = {}) {
  const headers = { Accept: "application/json" };
  if (opts.bearer) {
    headers["Authorization"] = `Bearer ${opts.bearer}`;
  }
  let res;
  try {
    res = await fetch(path, {
      method: "GET",
      headers,
      // Read-only appliance: never send ambient cookies/credentials.
      credentials: "omit",
      cache: "no-store",
      referrerPolicy: "no-referrer",
      signal: opts.signal,
    });
  } catch (err) {
    // Network / transport error: surface as status 0 (the caller distinguishes
    // "endpoint down" from an HTTP status).
    return { ok: false, status: 0, data: null, error: String(err && err.message ? err.message : err) };
  }

  const status = res.status;
  let data = null;
  let error = null;
  const ctype = res.headers.get("content-type") || "";
  // Both success and RFC-9457 problem+json bodies are JSON; parse defensively.
  if (ctype.includes("json")) {
    try {
      data = await res.json();
    } catch (err) {
      error = `bad json (${String(err && err.message ? err.message : err)})`;
    }
  } else if (!res.ok) {
    try {
      const txt = await res.text();
      error = txt.slice(0, 200);
    } catch (_) {
      /* ignore */
    }
  }

  if (!res.ok && !error) {
    // Prefer the problem+json `title`/`detail` when present (api.md: RFC 9457).
    if (data && (data.title || data.detail)) {
      error = [data.title, data.detail].filter(Boolean).join(" — ");
    } else {
      error = `HTTP ${status}`;
    }
  }

  return { ok: res.ok, status, data, error };
}

// ---------------------------------------------------------------------------
// el — tiny DOM builder. el(tag, attrs, ...children) -> HTMLElement
//   attrs:   { class, id, text, html, on:{click,...}, dataset:{...}, <attr> }
//   children: strings (text nodes) or Nodes; nested arrays are flattened;
//             null/undefined/false are skipped.
// ---------------------------------------------------------------------------
export function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === "class" || k === "className") {
      node.className = v;
    } else if (k === "text") {
      node.textContent = v;
    } else if (k === "html") {
      node.innerHTML = v;
    } else if (k === "dataset") {
      for (const [dk, dv] of Object.entries(v)) node.dataset[dk] = dv;
    } else if (k === "on" && typeof v === "object") {
      for (const [ev, fn] of Object.entries(v)) node.addEventListener(ev, fn);
    } else if (k === "style" && typeof v === "object") {
      for (const [sk, sv] of Object.entries(v)) node.style.setProperty(sk, sv);
    } else {
      node.setAttribute(k, v === true ? "" : String(v));
    }
  }
  appendChildren(node, children);
  return node;
}

function appendChildren(node, children) {
  for (const child of children) {
    if (child == null || child === false) continue;
    if (Array.isArray(child)) {
      appendChildren(node, child);
    } else if (child instanceof Node) {
      node.appendChild(child);
    } else {
      node.appendChild(document.createTextNode(String(child)));
    }
  }
}

// ---------------------------------------------------------------------------
// Canvas plumbing shared by the chart helpers: HiDPI-correct sizing + a small
// plot-area frame with axes. Returns the 2d context and a px<->data projector.
// ---------------------------------------------------------------------------
function prepCanvas(canvas, pad = { l: 44, r: 12, t: 12, b: 26 }) {
  const dpr = window.devicePixelRatio || 1;
  // Fall back to sensible CSS sizes if the canvas has not been laid out yet.
  const cssW = canvas.clientWidth || canvas.width || 600;
  const cssH = canvas.clientHeight || canvas.height || 220;
  canvas.width = Math.round(cssW * dpr);
  canvas.height = Math.round(cssH * dpr);
  const ctx = canvas.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, cssW, cssH);
  const plot = {
    x0: pad.l,
    y0: cssH - pad.b,
    x1: cssW - pad.r,
    y1: pad.t,
    w: cssW - pad.l - pad.r,
    h: cssH - pad.t - pad.b,
    cssW,
    cssH,
  };
  return { ctx, plot };
}

function axisColors() {
  // Pull from CSS custom properties so the charts track the theme; fall back to
  // dark-appliance defaults if the stylesheet has not applied.
  const cs = getComputedStyle(document.documentElement);
  const get = (name, dflt) => (cs.getPropertyValue(name) || "").trim() || dflt;
  return {
    axis: get("--chart-axis", "#3a4351"),
    grid: get("--chart-grid", "#232a34"),
    text: get("--chart-text", "#8b97a7"),
    fg: get("--fg", "#d7dee8"),
  };
}

function niceExtent(values, fallback = [0, 1]) {
  const nums = values.filter((v) => Number.isFinite(v));
  if (nums.length === 0) return fallback;
  let lo = Math.min(...nums);
  let hi = Math.max(...nums);
  if (lo === hi) {
    // Avoid a zero-height range.
    const pad = Math.abs(lo) > 1 ? Math.abs(lo) * 0.1 : 1;
    lo -= pad;
    hi += pad;
  }
  return [lo, hi];
}

// ---------------------------------------------------------------------------
// lineChart(canvas, series, opts)
//   series: [{ label, color, points:[{x,y}] }]
//   opts:   { markers:[{x,color,label}], shade:[{x0,x1,color}], yLabel,
//             xLabel, xFormat(fn), legend(bool) }
// Draws axes, gridlines, one polyline per series, vertical significance
// markers, and shaded x-ranges (used for burst-state shading — a SEPARATE
// visual channel from significance markers, never replacing them).
// ---------------------------------------------------------------------------
export function lineChart(canvas, series, opts = {}) {
  const { ctx, plot } = prepCanvas(canvas);
  const c = axisColors();
  series = Array.isArray(series) ? series : [];

  const allX = [];
  const allY = [];
  for (const s of series) for (const p of s.points || []) {
    if (Number.isFinite(p.x)) allX.push(p.x);
    if (Number.isFinite(p.y)) allY.push(p.y);
  }
  for (const m of opts.markers || []) if (Number.isFinite(m.x)) allX.push(m.x);
  for (const sh of opts.shade || []) {
    if (Number.isFinite(sh.x0)) allX.push(sh.x0);
    if (Number.isFinite(sh.x1)) allX.push(sh.x1);
  }

  const [xLo, xHi] = niceExtent(allX, [0, 1]);
  let [yLo, yHi] = niceExtent(allY, [0, 1]);
  // Anchor counts/rates to zero when all-positive — honest baselines.
  if (yLo > 0 && yLo < yHi) yLo = 0;

  const projX = (x) => plot.x0 + ((x - xLo) / (xHi - xLo || 1)) * plot.w;
  const projY = (y) => plot.y0 - ((y - yLo) / (yHi - yLo || 1)) * plot.h;

  // Shaded x-ranges first (behind everything) — burst-state bands.
  for (const sh of opts.shade || []) {
    if (!Number.isFinite(sh.x0) || !Number.isFinite(sh.x1)) continue;
    const a = projX(sh.x0);
    const b = projX(sh.x1);
    ctx.fillStyle = sh.color || "rgba(214, 158, 46, 0.16)";
    ctx.fillRect(Math.min(a, b), plot.y1, Math.abs(b - a) || 2, plot.h);
  }

  // Gridlines + y ticks (4 divisions).
  ctx.strokeStyle = c.grid;
  ctx.fillStyle = c.text;
  ctx.lineWidth = 1;
  ctx.font = "11px ui-monospace, monospace";
  ctx.textAlign = "right";
  ctx.textBaseline = "middle";
  for (let i = 0; i <= 4; i++) {
    const yv = yLo + ((yHi - yLo) * i) / 4;
    const py = projY(yv);
    ctx.beginPath();
    ctx.moveTo(plot.x0, py);
    ctx.lineTo(plot.x1, py);
    ctx.stroke();
    ctx.fillText(fmtNum(yv), plot.x0 - 6, py);
  }

  // Axes.
  ctx.strokeStyle = c.axis;
  ctx.beginPath();
  ctx.moveTo(plot.x0, plot.y1);
  ctx.lineTo(plot.x0, plot.y0);
  ctx.lineTo(plot.x1, plot.y0);
  ctx.stroke();

  // x tick labels (start + end).
  ctx.textAlign = "left";
  ctx.textBaseline = "top";
  const xfmt = opts.xFormat || fmtNum;
  ctx.fillText(xfmt(xLo), plot.x0, plot.y0 + 6);
  ctx.textAlign = "right";
  ctx.fillText(xfmt(xHi), plot.x1, plot.y0 + 6);

  // Series polylines.
  for (const s of series) {
    const pts = (s.points || []).filter((p) => Number.isFinite(p.x) && Number.isFinite(p.y));
    if (pts.length === 0) continue;
    ctx.strokeStyle = s.color || c.fg;
    ctx.lineWidth = 1.75;
    ctx.beginPath();
    pts.forEach((p, i) => {
      const px = projX(p.x);
      const py = projY(p.y);
      if (i === 0) ctx.moveTo(px, py);
      else ctx.lineTo(px, py);
    });
    ctx.stroke();
    // Dots so single points are visible.
    ctx.fillStyle = s.color || c.fg;
    for (const p of pts) {
      ctx.beginPath();
      ctx.arc(projX(p.x), projY(p.y), 2, 0, Math.PI * 2);
      ctx.fill();
    }
  }

  // Vertical significance markers (drawn on top, dashed).
  for (const m of opts.markers || []) {
    if (!Number.isFinite(m.x)) continue;
    const px = projX(m.x);
    ctx.strokeStyle = m.color || "#e2557b";
    ctx.lineWidth = 1.5;
    ctx.setLineDash([4, 3]);
    ctx.beginPath();
    ctx.moveTo(px, plot.y1);
    ctx.lineTo(px, plot.y0);
    ctx.stroke();
    ctx.setLineDash([]);
    if (m.label) {
      ctx.save();
      ctx.fillStyle = m.color || "#e2557b";
      ctx.textAlign = "left";
      ctx.textBaseline = "top";
      ctx.translate(px + 3, plot.y1 + 2);
      ctx.fillText(m.label, 0, 0);
      ctx.restore();
    }
  }

  // y-axis label.
  if (opts.yLabel) {
    ctx.save();
    ctx.fillStyle = c.text;
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    ctx.translate(12, (plot.y1 + plot.y0) / 2);
    ctx.rotate(-Math.PI / 2);
    ctx.fillText(opts.yLabel, 0, 0);
    ctx.restore();
  }

  // Inline legend.
  if (opts.legend !== false && series.some((s) => s.label)) {
    let lx = plot.x0 + 6;
    const ly = plot.y1 + 2;
    ctx.textAlign = "left";
    ctx.textBaseline = "top";
    for (const s of series) {
      if (!s.label) continue;
      ctx.fillStyle = s.color || c.fg;
      ctx.fillRect(lx, ly + 3, 9, 9);
      ctx.fillStyle = c.text;
      const tw = ctx.measureText(s.label).width;
      ctx.fillText(s.label, lx + 13, ly);
      lx += 13 + tw + 14;
    }
  }
}

// ---------------------------------------------------------------------------
// barChart(canvas, bars, opts)
//   bars: [{ label, value, color }]  (e.g. latency stage_ms breakdown)
//   opts: { yLabel, valueFormat(fn), max }
// ---------------------------------------------------------------------------
export function barChart(canvas, bars, opts = {}) {
  const { ctx, plot } = prepCanvas(canvas, { l: 44, r: 12, t: 12, b: 40 });
  const c = axisColors();
  bars = Array.isArray(bars) ? bars.filter((b) => b && Number.isFinite(b.value)) : [];

  const maxV = opts.max != null ? opts.max : Math.max(1, ...bars.map((b) => b.value));
  const projY = (v) => plot.y0 - (v / (maxV || 1)) * plot.h;
  const vfmt = opts.valueFormat || fmtNum;

  // y gridlines.
  ctx.strokeStyle = c.grid;
  ctx.fillStyle = c.text;
  ctx.font = "11px ui-monospace, monospace";
  ctx.textAlign = "right";
  ctx.textBaseline = "middle";
  for (let i = 0; i <= 4; i++) {
    const yv = (maxV * i) / 4;
    const py = projY(yv);
    ctx.beginPath();
    ctx.moveTo(plot.x0, py);
    ctx.lineTo(plot.x1, py);
    ctx.stroke();
    ctx.fillText(fmtNum(yv), plot.x0 - 6, py);
  }

  ctx.strokeStyle = c.axis;
  ctx.beginPath();
  ctx.moveTo(plot.x0, plot.y1);
  ctx.lineTo(plot.x0, plot.y0);
  ctx.lineTo(plot.x1, plot.y0);
  ctx.stroke();

  const n = bars.length || 1;
  const slot = plot.w / n;
  const bw = Math.min(slot * 0.62, 56);
  bars.forEach((b, i) => {
    const cx = plot.x0 + slot * (i + 0.5);
    const top = projY(b.value);
    ctx.fillStyle = b.color || "#4f9cf2";
    ctx.fillRect(cx - bw / 2, top, bw, plot.y0 - top);
    // value above bar
    ctx.fillStyle = c.fg;
    ctx.textAlign = "center";
    ctx.textBaseline = "bottom";
    ctx.fillText(vfmt(b.value), cx, top - 2);
    // label below axis
    ctx.fillStyle = c.text;
    ctx.textBaseline = "top";
    drawWrappedLabel(ctx, b.label == null ? "" : String(b.label), cx, plot.y0 + 5, slot - 4);
  });

  if (opts.yLabel) {
    ctx.save();
    ctx.fillStyle = c.text;
    ctx.textAlign = "center";
    ctx.textBaseline = "top";
    ctx.translate(12, (plot.y1 + plot.y0) / 2);
    ctx.rotate(-Math.PI / 2);
    ctx.fillText(opts.yLabel, 0, 0);
    ctx.restore();
  }
}

function drawWrappedLabel(ctx, text, cx, y, maxW) {
  // crude: truncate with an ellipsis if too wide.
  let t = text;
  while (t.length > 1 && ctx.measureText(t).width > maxW) t = t.slice(0, -1);
  if (t !== text && t.length > 1) t = t.slice(0, -1) + "…";
  ctx.fillText(t, cx, y);
}

// ---------------------------------------------------------------------------
// hexMap(canvas, cells, opts)
//   cells: [{ lat, lon, value, significant, count }]
//   opts:  { valueLabel, diverging:[lo,mid,hi] colors, pad }
// Equirectangular projection of lat/lon to the canvas, one filled disc per
// cell (radius ~ count), fill from a diverging scale on `value` (the Gi* z),
// a heavy ring when `significant`. Cells with null/non-finite lat/lon are
// dropped. No basemap tiles (appliance posture: no egress).
// ---------------------------------------------------------------------------
export function hexMap(canvas, cells, opts = {}) {
  const { ctx, plot } = prepCanvas(canvas, { l: 8, r: 8, t: 8, b: 8 });
  const c = axisColors();
  cells = (Array.isArray(cells) ? cells : []).filter(
    (d) => d && Number.isFinite(d.lat) && Number.isFinite(d.lon)
  );

  if (cells.length === 0) {
    ctx.fillStyle = c.text;
    ctx.font = "12px ui-monospace, monospace";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText("no geo-tagged cells", plot.cssW / 2, plot.cssH / 2);
    return;
  }

  const lats = cells.map((d) => d.lat);
  const lons = cells.map((d) => d.lon);
  let [latLo, latHi] = niceExtent(lats);
  let [lonLo, lonHi] = niceExtent(lons);
  // Pad the bounds so discs near the edge are not clipped.
  const latPad = (latHi - latLo || 1) * 0.08;
  const lonPad = (lonHi - lonLo || 1) * 0.08;
  latLo -= latPad; latHi += latPad; lonLo -= lonPad; lonHi += lonPad;

  const px = (lon) => plot.x0 + ((lon - lonLo) / (lonHi - lonLo || 1)) * plot.w;
  // Latitude increases upward on screen.
  const py = (lat) => plot.y0 - ((lat - latLo) / (latHi - latLo || 1)) * plot.h;

  const maxCount = Math.max(1, ...cells.map((d) => Number(d.count) || 0));
  const vals = cells.map((d) => Number(d.value)).filter(Number.isFinite);
  const absMax = Math.max(0.001, ...vals.map((v) => Math.abs(v)));

  // Frame.
  ctx.strokeStyle = c.axis;
  ctx.strokeRect(plot.x0, plot.y1, plot.w, plot.h);

  for (const d of cells) {
    const r = 4 + Math.sqrt((Number(d.count) || 0) / maxCount) * 18;
    const x = px(d.lon);
    const y = py(d.lat);
    ctx.fillStyle = divergingColor(Number(d.value) || 0, absMax);
    ctx.beginPath();
    ctx.arc(x, y, r, 0, Math.PI * 2);
    ctx.fill();
    // Significance is the DEFENSIBLE flag (api.md): heavy ring, not color.
    if (d.significant) {
      ctx.strokeStyle = "#f4f6fa";
      ctx.lineWidth = 2.5;
      ctx.stroke();
    } else {
      ctx.strokeStyle = "rgba(255,255,255,0.18)";
      ctx.lineWidth = 1;
      ctx.stroke();
    }
  }

  // Tiny legend: significance ring meaning.
  ctx.fillStyle = c.text;
  ctx.font = "11px ui-monospace, monospace";
  ctx.textAlign = "left";
  ctx.textBaseline = "bottom";
  ctx.fillText("ring = significant hot spot (q≤0.05) · area = raw count", plot.x0 + 4, plot.y0 - 4);
}

/** Diverging blue↔grey↔red scale on a signed value, |v| clamped to absMax. */
function divergingColor(v, absMax) {
  const t = Math.max(-1, Math.min(1, v / (absMax || 1)));
  // t in [-1,1]: -1 → cool blue, 0 → neutral grey, +1 → hot red.
  const cool = [70, 110, 200];
  const mid = [120, 128, 140];
  const hot = [220, 70, 70];
  let rgb;
  if (t >= 0) rgb = lerp3(mid, hot, t);
  else rgb = lerp3(mid, cool, -t);
  return `rgb(${rgb[0]|0},${rgb[1]|0},${rgb[2]|0})`;
}
function lerp3(a, b, t) {
  return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
}

// ---------------------------------------------------------------------------
// gauge(container, { label, value, min, max, caption })
// A simple horizontal bar/gauge with an HONEST caption underneath (e.g.
// "uncalibrated, ranking-comparable on this deployment, not a probability").
// ---------------------------------------------------------------------------
export function gauge(container, { label, value, min = 0, max = 1, caption } = {}) {
  const v = Number(value);
  const span = (max - min) || 1;
  const pct = Number.isFinite(v) ? Math.max(0, Math.min(1, (v - min) / span)) : 0;
  const wrap = el(
    "div",
    { class: "gauge" },
    el(
      "div",
      { class: "gauge-head" },
      el("span", { class: "gauge-label", text: label || "" }),
      el("span", { class: "gauge-value", text: Number.isFinite(v) ? fmtNum(v) : "—" })
    ),
    el(
      "div",
      { class: "gauge-track" },
      el("div", { class: "gauge-fill", style: { width: `${(pct * 100).toFixed(1)}%` } })
    ),
    el(
      "div",
      { class: "gauge-scale" },
      el("span", { text: fmtNum(min) }),
      el("span", { text: fmtNum(max) })
    ),
    caption ? el("div", { class: "gauge-caption honesty", text: caption }) : null
  );
  if (container) {
    container.appendChild(wrap);
  }
  return wrap;
}

// ---------------------------------------------------------------------------
// chips(container, items, kind)
// Render an array of strings as labeled chips (e.g. degraded[] flags). `kind`
// is appended as a class (chip--<kind>) so styling can distinguish degraded /
// honesty / status chips.
// ---------------------------------------------------------------------------
export function chips(container, items, kind = "") {
  const list = Array.isArray(items) ? items : items == null ? [] : [items];
  const wrap = el("div", { class: "chips" });
  for (const item of list) {
    if (item == null) continue;
    const cls = "chip" + (kind ? ` chip--${kind}` : "");
    wrap.appendChild(el("span", { class: cls, text: String(item) }));
  }
  if (container) container.appendChild(wrap);
  return wrap;
}

// ---------------------------------------------------------------------------
// Panel registry + runtime.
// registerPanel({ id, title, refreshMs, requiresBearer, render })
//   - id:             the <section> id the panel renders into.
//   - title:          panel heading.
//   - refreshMs:      polling interval; null/0 = on-demand only.
//   - requiresBearer: when true the panel re-renders on token change and is
//                     gated behind a token-present check in its own render().
//   - render(container, { bearer }): called on load and on each tick. May be
//                     async; the runtime awaits it.
// ---------------------------------------------------------------------------
const _panels = new Map();
const _timers = new Map();

export function registerPanel(def) {
  if (!def || !def.id) {
    console.warn("registerPanel: missing id");
    return;
  }
  _panels.set(def.id, {
    id: def.id,
    title: def.title || def.id,
    refreshMs: def.refreshMs || 0,
    requiresBearer: !!def.requiresBearer,
    render: def.render || (() => {}),
  });
  // If the runtime has already started, render immediately (late registration).
  if (_started) {
    mountPanel(_panels.get(def.id));
  }
}

let _started = false;

function panelContainer(panel) {
  const section = document.getElementById(panel.id);
  if (!section) return null;
  // Ensure a stable header + body so re-renders only replace the body.
  let body = section.querySelector(".panel-body");
  if (!body) {
    section.appendChild(
      el(
        "div",
        { class: "panel-head" },
        el("h2", { class: "panel-title", text: panel.title }),
        panel.requiresBearer
          ? el("span", { class: "panel-gate", text: "bearer-gated" })
          : null,
        el("span", { class: "panel-status muted", "data-role": "status" })
      )
    );
    body = el("div", { class: "panel-body" });
    section.appendChild(body);
  }
  return body;
}

function setPanelStatus(panel, text) {
  const section = document.getElementById(panel.id);
  const status = section && section.querySelector('[data-role="status"]');
  if (status) status.textContent = text || "";
}

async function renderPanel(panel) {
  const body = panelContainer(panel);
  if (!body) return;
  const fresh = el("div", { class: "panel-body-inner" });
  try {
    setPanelStatus(panel, "loading…");
    await panel.render(fresh, { bearer: _bearer });
    setPanelStatus(panel, `updated ${new Date().toLocaleTimeString()}`);
  } catch (err) {
    // A panel must never crash the runtime — degrade gracefully (no blank box).
    fresh.appendChild(
      el("div", { class: "panel-error" }, `panel failed to render: ${String(err && err.message ? err.message : err)}`)
    );
    setPanelStatus(panel, "error");
  }
  body.replaceChildren(fresh);
}

function mountPanel(panel) {
  renderPanel(panel);
  // (Re)schedule polling.
  if (_timers.has(panel.id)) {
    clearInterval(_timers.get(panel.id));
    _timers.delete(panel.id);
  }
  if (panel.refreshMs && panel.refreshMs > 0) {
    const t = setInterval(() => renderPanel(panel), panel.refreshMs);
    _timers.set(panel.id, t);
  }
}

// ---------------------------------------------------------------------------
// Global honest/degraded banner area.
// ---------------------------------------------------------------------------
export function setGlobalBanner(message, kind = "info") {
  const banner = document.getElementById("global-banner");
  if (!banner) return;
  if (!message) {
    banner.hidden = true;
    banner.replaceChildren();
    return;
  }
  banner.hidden = false;
  banner.className = `global-banner banner--${kind}`;
  banner.replaceChildren(el("span", { text: message }));
}

// ---------------------------------------------------------------------------
// Number formatting shared by the charts/gauges.
// ---------------------------------------------------------------------------
function fmtNum(v) {
  if (!Number.isFinite(v)) return "—";
  const a = Math.abs(v);
  if (a !== 0 && (a < 0.01 || a >= 1e5)) return v.toExponential(1);
  if (Number.isInteger(v)) return String(v);
  if (a >= 100) return v.toFixed(0);
  if (a >= 1) return v.toFixed(2);
  return v.toFixed(3);
}
export { fmtNum };

// ---------------------------------------------------------------------------
// Boot: wire the token input, import the panel modules (they self-register),
// then mount every registered panel.
// ---------------------------------------------------------------------------
function wireTokenInput() {
  const input = document.getElementById("op-token");
  if (!input) return;
  const apply = () => {
    _bearer = input.value || "";
    // Re-render bearer-gated panels so they pick up (or lose) the token.
    for (const panel of _panels.values()) {
      if (panel.requiresBearer) renderPanel(panel);
    }
  };
  input.addEventListener("change", apply);
  // Also catch Enter without a blur.
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      apply();
    }
  });
}

async function boot() {
  wireTokenInput();
  // Import the panel modules for their self-registration side effects. Each
  // module calls registerPanel() at import time. Imported AFTER the registry +
  // helpers above are defined so the exports resolve.
  const modules = [
    "./panels/lanes.js",
    "./panels/trends.js",
    "./panels/geo.js",
    "./panels/search.js",
    "./panels/ope.js",
  ];
  for (const m of modules) {
    try {
      await import(m);
    } catch (err) {
      console.error(`failed to load panel module ${m}`, err);
      setGlobalBanner(`a panel module failed to load (${m}) — see console`, "warn");
    }
  }
  _started = true;
  for (const panel of _panels.values()) {
    mountPanel(panel);
  }
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", boot);
} else {
  boot();
}
