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

/** Escape a value for safe interpolation into a tooltip's innerHTML. */
export function escapeHtml(s) {
  return String(s == null ? "" : s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
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

  // Hover: nearest x across series → tooltip readout (DOM tooltip, no redraw).
  const xfmtH = opts.xFormat || fmtNum;
  installHover(canvas, (mx, my) => {
    if (my < plot.y1 - 6 || my > plot.y0 + 6) return null;
    const live = series.filter((s) => (s.points || []).some((p) => Number.isFinite(p.x) && Number.isFinite(p.y)));
    if (!live.length) return null;
    const dataX = xLo + ((mx - plot.x0) / (plot.w || 1)) * (xHi - xLo);
    let anchor = null;
    const rows = [];
    for (const s of live) {
      const pts = s.points.filter((p) => Number.isFinite(p.x) && Number.isFinite(p.y));
      let np = pts[0];
      for (const p of pts) if (Math.abs(p.x - dataX) < Math.abs(np.x - dataX)) np = p;
      rows.push(`<span style="color:${escapeHtml(s.color || "currentColor")}">■</span> ${escapeHtml(s.label || "y")} ${escapeHtml(fmtNum(np.y))}`);
      if (!anchor || Math.abs(np.x - dataX) < Math.abs(anchor.x - dataX)) anchor = np;
    }
    if (!anchor) return null;
    return {
      html: `<b>${escapeHtml(xfmtH(anchor.x))}</b>\n${rows.join("\n")}`,
      x: projX(anchor.x),
      y: projY(anchor.y),
    };
  });
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

  // Hover: the bar under the cursor → exact label + value.
  installHover(canvas, (mx) => {
    if (mx < plot.x0 || mx > plot.x1 || !bars.length) return null;
    const idx = Math.floor((mx - plot.x0) / (slot || 1));
    if (idx < 0 || idx >= bars.length) return null;
    const b = bars[idx];
    const cx = plot.x0 + slot * (idx + 0.5);
    return {
      html: `<b>${escapeHtml(b.label == null ? "" : String(b.label))}</b>\n${escapeHtml(vfmt(b.value))}`,
      x: cx,
      y: projY(b.value),
    };
  });
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

  // Hover: the cell under the cursor → its honest readout (cross-references the
  // table; significance is the defensible flag, raw count is kept visible).
  installHover(canvas, (mx, my) => {
    let best = null;
    let bestD = Infinity;
    for (const d of cells) {
      const x = px(d.lon);
      const y = py(d.lat);
      const dist = Math.hypot(mx - x, my - y);
      const r = 4 + Math.sqrt((Number(d.count) || 0) / maxCount) * 18;
      if (dist <= r + 2 && dist < bestD) {
        best = { d, x, y };
        bestD = dist;
      }
    }
    if (!best) return null;
    const d = best.d;
    const parts = [
      d.h3 != null ? `h3 ${escapeHtml(d.h3)}` : null,
      `count ${escapeHtml(fmtNum(Number(d.count) || 0))}`,
      Number.isFinite(Number(d.value)) ? `Gi* z ${escapeHtml(fmtNum(Number(d.value)))}` : null,
      Number.isFinite(Number(d.q_value)) ? `q ${escapeHtml(fmtNum(Number(d.q_value)))}` : null,
      d.significant ? "significant (q≤0.05)" : "not significant",
    ].filter(Boolean);
    return { html: parts.join("\n"), x: best.x, y: best.y };
  });
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

/** A panel is visible when its section has a layout box (its view is active). */
function isVisible(panel) {
  const section = document.getElementById(panel.id);
  return !!section && section.offsetParent !== null;
}

function mountPanel(panel) {
  // Only render now if the panel's view is the active one — the router renders
  // a view's panels when it becomes visible, so hidden panels do no network/draw.
  if (isVisible(panel)) renderPanel(panel);
  // (Re)schedule polling — the tick is gated on visibility so background views
  // stay cheap (no fetch, no canvas work) until the operator opens them.
  if (_timers.has(panel.id)) {
    clearInterval(_timers.get(panel.id));
    _timers.delete(panel.id);
  }
  if (panel.refreshMs && panel.refreshMs > 0) {
    const t = setInterval(() => {
      if (isVisible(panel)) renderPanel(panel);
    }, panel.refreshMs);
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
// Tiny reactive primitive — a ~1-dependency-free signal. Used by the search
// tuning drawer (≈18 two-way-bound controls) and the metrics live numbers; the
// existing panels keep the imperative el()+re-render model. No framework.
//   signal(initial) -> { get, set, subscribe(fn) -> unsubscribe }
// ---------------------------------------------------------------------------
export function signal(initial) {
  let v = initial;
  const subs = new Set();
  return {
    get: () => v,
    set: (nv) => {
      if (nv !== v) {
        v = nv;
        for (const f of subs) f(v);
      }
    },
    subscribe: (f) => {
      subs.add(f);
      return () => subs.delete(f);
    },
  };
}

/** Two-way bind a form control to a signal. parse: string->value (control to
 *  signal), format: value->string (signal to control). */
export function bind(input, sig, { event = "input", parse = (x) => x, format = (x) => String(x) } = {}) {
  input.value = format(sig.get());
  input.addEventListener(event, () => sig.set(parse(input.value)));
  sig.subscribe((v) => {
    const f = format(v);
    if (input.value !== f) input.value = f;
  });
  return input;
}

// ---------------------------------------------------------------------------
// installHover(canvas, hitTest) — shared chart interactivity. Ensures the canvas
// sits in a positioned .chart-wrap with one reused .chart-tip, and on mousemove
// calls hitTest(px, py) -> { html, x } | null to position/fill the tooltip and a
// 1px crosshair. Dependency-free, no per-move canvas redraw (cheap on the Pi).
// ---------------------------------------------------------------------------
function installHover(canvas, hitTest) {
  let wrap = canvas.parentElement;
  if (!wrap || !wrap.classList.contains("chart-wrap")) {
    wrap = el("div", { class: "chart-wrap" });
    canvas.replaceWith(wrap);
    wrap.appendChild(canvas);
  }
  let tip = wrap.querySelector(".chart-tip");
  if (!tip) {
    tip = el("div", { class: "chart-tip" });
    wrap.appendChild(tip);
  }
  // Replace any prior handler from an earlier render of this canvas.
  if (canvas.__hoverMove) canvas.removeEventListener("mousemove", canvas.__hoverMove);
  if (canvas.__hoverLeave) canvas.removeEventListener("mouseleave", canvas.__hoverLeave);
  const move = (e) => {
    const rect = canvas.getBoundingClientRect();
    const px = e.clientX - rect.left;
    const py = e.clientY - rect.top;
    const hit = hitTest(px, py);
    if (!hit) {
      tip.style.display = "none";
      return;
    }
    tip.innerHTML = hit.html;
    tip.style.left = `${hit.x != null ? hit.x : px}px`;
    tip.style.top = `${hit.y != null ? hit.y : py}px`;
    tip.style.display = "block";
  };
  const leave = () => {
    tip.style.display = "none";
  };
  canvas.__hoverMove = move;
  canvas.__hoverLeave = leave;
  canvas.addEventListener("mousemove", move);
  canvas.addEventListener("mouseleave", leave);
}

// ---------------------------------------------------------------------------
// /v1/config — the effective tunable defaults + clamps + deploy config + feature
// availability. Fetched once and cached; the tuning drawer + deploy panel read
// it. Bearer-optional like search (retries with the token only on 401).
// ---------------------------------------------------------------------------
let _configCache = null;
export async function getConfig({ force = false } = {}) {
  if (_configCache && !force) return _configCache;
  let res = await fetchJSON("/v1/config");
  if (res.status === 401 && _bearer) res = await fetchJSON("/v1/config", { bearer: _bearer });
  _configCache = res;
  return res;
}

// ---------------------------------------------------------------------------
// Theme: persist the (non-secret) preference in localStorage; toggle the
// data-theme attribute on <html>. The pre-paint inline script in index.html
// applies it before first paint to avoid a flash.
// ---------------------------------------------------------------------------
function currentTheme() {
  return document.documentElement.getAttribute("data-theme") === "light" ? "light" : "dark";
}
function applyTheme(t) {
  document.documentElement.setAttribute("data-theme", t);
  try {
    localStorage.setItem("meridian.theme", t);
  } catch (_) {
    /* localStorage unavailable — in-memory only for this session */
  }
}
function wireThemeToggle() {
  const btn = document.getElementById("theme-toggle");
  if (!btn) return;
  btn.addEventListener("click", () => {
    applyTheme(currentTheme() === "light" ? "dark" : "light");
    // Re-render visible panels so the canvas charts repaint with theme colors.
    for (const panel of _panels.values()) renderPanel(panel);
  });
}

// ---------------------------------------------------------------------------
// Hash router: the sidebar selects one view (#/search, #/metrics, …). Only the
// active view is shown; hidden panels skip their network+draw (renderPanel gates
// on visibility) so polling stays cheap.
// ---------------------------------------------------------------------------
const DEFAULT_VIEW = "search";
function currentView() {
  const h = (location.hash || "").replace(/^#\/?/, "");
  return h || DEFAULT_VIEW;
}
function applyRoute() {
  const view = currentView();
  let matched = false;
  for (const sec of document.querySelectorAll(".view")) {
    const on = sec.dataset.view === view;
    sec.classList.toggle("view--active", on);
    if (on) matched = true;
  }
  if (!matched) {
    // Unknown route → land on the default view, no blank screen.
    const def = document.querySelector(`.view[data-view="${DEFAULT_VIEW}"]`);
    if (def) def.classList.add("view--active");
  }
  for (const a of document.querySelectorAll(".nav-item")) {
    a.classList.toggle("active", a.dataset.view === (matched ? view : DEFAULT_VIEW));
  }
  const titleEl = document.getElementById("view-title");
  if (titleEl) titleEl.textContent = (matched ? view : DEFAULT_VIEW).replace("gate", "ope gate");
  // Render the now-visible panels (they were skipped while hidden).
  for (const panel of _panels.values()) {
    const sec = document.getElementById(panel.id);
    if (sec && sec.offsetParent !== null) renderPanel(panel);
  }
}

/** Version • CE-availability pill in the sidebar, fed by /v1/config. */
async function wireVersionPill() {
  const pill = document.getElementById("ver-pill");
  if (!pill) return;
  const { ok, data } = await getConfig();
  if (!ok || !data) {
    pill.textContent = "version unknown";
    return;
  }
  const ce = data.features && data.features.cross_encoder_present;
  const ver = data.version || "?";
  pill.textContent = `v${ver} · CE ${ce ? "live" : "dormant"}`;
  pill.classList.add(ce ? "ver-pill--live" : "ver-pill--dormant");
  pill.title = ce
    ? "deep / answer / corroboration are LIVE (cross-encoder present)"
    : "deep / answer / corroboration are DORMANT (scratch image, no ort — ADR-02)";
}

// ---------------------------------------------------------------------------
// Boot: wire the token input, import the panel modules (they self-register),
// then mount every registered panel.
// ---------------------------------------------------------------------------
function wireTokenInput() {
  const input = document.getElementById("op-token");
  if (!input) return;
  const apply = () => {
    _bearer = input.value || "";
    _configCache = null; // re-fetch /v1/config (may have been bearer-gated)
    // Re-render visible bearer-gated panels so they pick up (or lose) the token;
    // hidden ones re-read _bearer when their view is next opened.
    for (const panel of _panels.values()) {
      if (panel.requiresBearer && isVisible(panel)) renderPanel(panel);
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
  wireThemeToggle();
  // Import the panel modules for their self-registration side effects. Each
  // module calls registerPanel() at import time. Imported AFTER the registry +
  // helpers above are defined so the exports resolve.
  const modules = [
    "./panels/search.js",
    "./panels/metrics.js",
    "./panels/lanes.js",
    "./panels/geo.js",
    "./panels/trends.js",
    "./panels/ope.js",
    "./panels/deploy.js",
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
  // Hash router: show the requested view and render its (now-visible) panels.
  window.addEventListener("hashchange", applyRoute);
  applyRoute();
  // Version • CE pill (fire-and-forget; never blocks first paint).
  wireVersionPill();
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", boot);
} else {
  boot();
}
