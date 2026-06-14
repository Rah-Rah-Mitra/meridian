// Search panel — GET /v1/search (api.md §GET /v1/search).
//
// On-demand: a query input + controls + a search button; refreshMs null. The
// static shell is unauthenticated and search is usually open — so this panel
// fires the request WITHOUT a bearer first and attaches the operator token
// ONLY if the guarded deployment answers 401 (auth.require_bearer_for_search).
// It never persists the query (the server never logs it; neither do we) and
// only ever issues GET requests.
//
// HONESTY POSTURE (the UI must never claim more certainty than the API):
//   - confidence.score is "uncalibrated, ranking-comparable, NOT a probability".
//     Low = "verify before trusting", never high = "true".
//   - evidence: independent_source_count < apparent_source_count means copies /
//     syndication, not independent corroboration. We show both side-by-side with
//     the cluster explainer and the sketched-results basis (snippets never count).
//   - best_passage.ce_score is "relevance, not correctness" — a confidently
//     relevant passage can still be wrong. The text is EXTRACTIVE: verbatim from
//     the fetched page, never generated, never stitched across documents.
//   - EVERY degraded[] flag is rendered as a chip.
//   - lane_requested vs lane_effective is shown as a badge when they differ.
//   - divergence (compare=vantages) carries a per-request, NOT population, caveat.
//
// SearchResponse shape (verbatim from api.md):
//   { results:[{ url,title,snippet,score, rank_signals{}, source, h3?, ts?,
//                evidence?:{cluster,canonical} }],
//     timings:{ plan,lexical,fusion,evidence_ms,total },
//     lane_requested, lane_effective, degraded?:[…],
//     confidence?:{ schema,nqc,clarity,score },       // additive, ADR-23
//     evidence?:{ schema,independent_source_count,apparent_source_count,
//                 sketched_results, clusters:[{id,members,domains}] },  // additive
//     divergence?:{ schema,lanes_compared,jsd,noise_floor_p90,exceeds_floor,
//                   domains_only_in_direct,domains_only_in_anon,
//                   anon_result_count,jitter_applied_ms },  // compare=vantages
//     best_passage?:{ schema,text,url,ce_score },      // answer=true + a fetch
//     analysis?:{ … } }                                // fetch_budget>0
// Additive corroboration / abstained fields (may NOT exist yet) are read only
// when present and ignored gracefully when absent — never fabricated.
import { registerPanel, fetchJSON, el, gauge, chips, barChart, getConfig, escapeHtml } from "../app.js";

// The MANDATORY confidence-score label. Used verbatim — do not paraphrase.
const CONFIDENCE_LABEL = "uncalibrated, ranking-comparable, NOT a probability";

function fmtScore(v, digits = 3) {
  return Number.isFinite(Number(v)) ? Number(v).toFixed(digits) : "—";
}

// A human host for a URL, falling back to the raw string when it will not parse
// (the URL is still shown verbatim elsewhere — this is only a label).
function hostOf(url) {
  try {
    return new URL(url).host;
  } catch (_) {
    return String(url || "");
  }
}

// ---------------------------------------------------------------------------
// One result row: title (linked), host, score, the rank_signals breakdown, and
// the per-result evidence annotation (cluster + canonical/derived-copy).
// ---------------------------------------------------------------------------
function resultRow(r, i) {
  const url = r && r.url ? String(r.url) : "";
  const title = r && r.title ? String(r.title) : url || "(untitled result)";
  const snippet = r && r.snippet ? String(r.snippet) : "";
  const score = Number(r && r.score);
  const source = r && r.source != null ? String(r.source) : null;

  const head = el(
    "div",
    {
      class: "result-head",
      style: { display: "flex", "align-items": "baseline", gap: "0.5rem", "flex-wrap": "wrap" },
    },
    el("span", { class: "result-rank muted", style: { "font-family": "var(--mono)" }, text: `#${i + 1}` }),
    url
      ? el("a", {
          class: "result-title",
          href: url,
          target: "_blank",
          rel: "noopener noreferrer",
          text: title,
          style: { color: "var(--accent)", "text-decoration": "none", "font-weight": "600" },
        })
      : el("span", { class: "result-title", text: title, style: { "font-weight": "600" } }),
    el("span", {
      class: "result-score muted",
      style: { "margin-left": "auto", "font-family": "var(--mono)", "font-size": "0.78rem" },
      text: `score ${fmtScore(score, 3)}`,
    })
  );

  const children = [head];
  if (url) {
    children.push(
      el("div", {
        class: "result-host muted",
        style: { "font-family": "var(--mono)", "font-size": "0.72rem", "word-break": "break-all" },
        text: url,
      })
    );
  }
  if (snippet) {
    children.push(el("div", { class: "result-snippet", style: { "font-size": "0.84rem", margin: "0.2rem 0" }, text: snippet }));
  }

  // source + per-result evidence annotation (cluster / canonical). evidence is
  // null for web results that were never fetched (independence is never guessed
  // from a snippet) — say so explicitly rather than implying corroboration.
  const tags = [];
  if (source) tags.push(`source: ${source}`);
  const ev = r && typeof r.evidence === "object" ? r.evidence : null;
  if (r && Object.prototype.hasOwnProperty.call(r, "evidence")) {
    if (ev && Number.isFinite(Number(ev.cluster))) {
      tags.push(`cluster ${Number(ev.cluster)}`);
      // canonical = the superset its copies derive from (the one diversity=evidence promotes).
      tags.push(ev.canonical ? "canonical original" : "derived copy");
    } else if (ev === null) {
      tags.push("not sketched (excluded from independence)");
    }
  }
  if (tags.length) {
    const chipWrap = el("div", { class: "chips" });
    for (const t of tags) {
      chipWrap.appendChild(el("span", { class: "chip chip--status", text: t }));
    }
    children.push(chipWrap);
  }

  // rank_signals breakdown — explainability for the fused score.
  const rs = r && typeof r.rank_signals === "object" && r.rank_signals ? r.rank_signals : null;
  if (rs) {
    const parts = Object.entries(rs)
      .filter(([, v]) => Number.isFinite(Number(v)))
      .map(([k, v]) => `${k} ${fmtScore(v, 2)}`);
    if (parts.length) {
      children.push(
        el("div", {
          class: "result-signals muted",
          style: { "font-family": "var(--mono)", "font-size": "0.72rem", "margin-top": "0.2rem" },
          text: parts.join("  ·  "),
        })
      );
    }
  }

  return el(
    "div",
    {
      class: "result",
      style: {
        padding: "0.5rem 0.1rem",
        "border-bottom": "1px solid var(--border)",
      },
    },
    children
  );
}

// ---------------------------------------------------------------------------
// Confidence block (additive, ADR-23). Renders nqc / clarity / score as gauges.
// The MANDATORY honest caption rides on the score gauge. nqc & clarity are raw
// predictors with no fixed [0,1] domain — shown as informational gauges with a
// caption that says exactly that.
// ---------------------------------------------------------------------------
function confidenceBlock(conf) {
  if (!conf || typeof conf !== "object") return null;
  const wrap = el("div", { class: "search-confidence" });
  wrap.appendChild(el("h3", { class: "subhead", text: "confidence" }));

  const score = Number(conf.score);
  // The headline gauge: the uncalibrated blend, with its mandatory label.
  gauge(wrap, {
    label: "confidence score",
    value: Number.isFinite(score) ? score : 0,
    min: 0,
    max: 1,
    caption: CONFIDENCE_LABEL,
  });

  // Supporting raw predictors. Domains are not [0,1]; bound the gauge to the
  // observed value so the bar is meaningful but the number is the truth shown.
  const nqc = Number(conf.nqc);
  const clarity = Number(conf.clarity);
  if (Number.isFinite(nqc)) {
    gauge(wrap, {
      label: "nqc (top-score dispersion)",
      value: nqc,
      min: 0,
      max: Math.max(1, nqc),
      caption: "raw query-performance predictor (a confident head separates) — not a probability",
    });
  }
  if (Number.isFinite(clarity)) {
    gauge(wrap, {
      label: "clarity (vocab KL vs pool)",
      value: clarity,
      min: 0,
      max: Math.max(1, clarity),
      caption: "raw query-performance predictor (a focused result set reads distinctively) — not a probability",
    });
  }

  wrap.appendChild(
    el("div", {
      class: "honesty",
      text:
        "Honest use: treat a LOW score as “verify before trusting”, not a high " +
        "score as “true”. Calibration against measured nDCG is not applied here.",
    })
  );
  return wrap;
}

// ---------------------------------------------------------------------------
// Evidence block (additive). independent_source_count vs apparent_source_count
// side-by-side, with the cluster explainer + the sketched-results basis. Absent
// when [evidence] enabled = false — caller skips this block entirely then.
// ---------------------------------------------------------------------------
function evidenceBlock(ev) {
  if (!ev || typeof ev !== "object") return null;
  const indep = Number(ev.independent_source_count);
  const apparent = Number(ev.apparent_source_count);
  const sketched = Number(ev.sketched_results);
  const clusters = Array.isArray(ev.clusters) ? ev.clusters : [];

  const wrap = el("div", { class: "search-evidence", style: { margin: "0.6rem 0" } });
  wrap.appendChild(el("h3", { class: "subhead", text: "evidence" }));

  // Side-by-side counts.
  const counts = el(
    "div",
    {
      class: "evidence-counts",
      style: { display: "flex", gap: "1.4rem", "flex-wrap": "wrap", margin: "0.3rem 0" },
    },
    el(
      "div",
      { class: "evidence-count" },
      el("div", {
        style: { "font-family": "var(--mono)", "font-size": "1.4rem", color: "var(--accent)" },
        text: Number.isFinite(indep) ? String(indep) : "—",
      }),
      el("div", { class: "honesty", text: "independent sources" })
    ),
    el(
      "div",
      { class: "evidence-count" },
      el("div", {
        style: { "font-family": "var(--mono)", "font-size": "1.4rem" },
        text: Number.isFinite(apparent) ? String(apparent) : "—",
      }),
      el("div", { class: "honesty", text: "apparent sources" })
    )
  );
  wrap.appendChild(counts);

  // The cluster explainer — the whole point of the block.
  if (Number.isFinite(indep) && Number.isFinite(apparent) && indep < apparent) {
    wrap.appendChild(
      el("div", {
        class: "noise-label",
        text:
          `${apparent - indep} of ${apparent} apparent sources are copies / syndications of each other — ` +
          "NOT independent corroboration. Independence is the smaller number.",
      })
    );
  } else if (Number.isFinite(indep) && Number.isFinite(apparent)) {
    wrap.appendChild(
      el("div", {
        class: "honesty",
        text: "Every apparent source is its own derivation cluster — no syndication detected among sketched results.",
      })
    );
  }

  // The honesty basis: only ingested+sketched results participate.
  if (Number.isFinite(sketched)) {
    wrap.appendChild(
      el("div", {
        class: "honesty",
        text:
          `Basis: ${sketched} result(s) were sketched from full text and participate. ` +
          "Web results never fetched carry no sketch and are excluded — independence is never guessed from a snippet.",
      })
    );
  }

  // Per-cluster detail. members > domains is cross-domain syndication — the case
  // plain domain grouping cannot see.
  const realClusters = clusters.filter((c) => c && typeof c === "object");
  if (realClusters.length) {
    const tbl = el(
      "table",
      { class: "data" },
      el(
        "thead",
        {},
        el(
          "tr",
          {},
          el("th", { text: "cluster" }),
          el("th", { text: "members" }),
          el("th", { text: "domains" }),
          el("th", { text: "" })
        )
      )
    );
    const tbody = el("tbody", {});
    for (const c of realClusters) {
      const members = Number(c.members);
      const domains = Number(c.domains);
      const crossDomain = Number.isFinite(members) && Number.isFinite(domains) && members > domains;
      tbody.appendChild(
        el(
          "tr",
          {},
          el("td", { text: c.id != null ? String(c.id) : "—" }),
          el("td", { text: Number.isFinite(members) ? String(members) : "—" }),
          el("td", { text: Number.isFinite(domains) ? String(domains) : "—" }),
          el("td", {
            class: crossDomain ? "noise-label" : "muted",
            text: crossDomain ? "cross-domain syndication" : "",
          })
        )
      );
    }
    tbl.appendChild(tbody);
    wrap.appendChild(tbl);
  }

  return wrap;
}

// ---------------------------------------------------------------------------
// best_passage block (answer=true + a fetch happened). Extractive, verbatim.
// ce_score labeled "relevance, not correctness".
// ---------------------------------------------------------------------------
function bestPassageBlock(bp) {
  if (!bp || typeof bp !== "object") return null;
  const text = bp.text != null ? String(bp.text) : "";
  const url = bp.url != null ? String(bp.url) : "";
  const ce = Number(bp.ce_score);

  const wrap = el("div", { class: "search-best-passage", style: { margin: "0.6rem 0" } });
  wrap.appendChild(el("h3", { class: "subhead", text: "best passage" }));

  // Extractive banner — this is the central honesty claim of the block.
  wrap.appendChild(
    el("div", {
      class: "honesty",
      style: { "margin-bottom": "0.3rem" },
      text:
        "Extractive only — verbatim from the fetched page, never generated, never " +
        "stitched across documents. This is “the best place to start reading”, not “the answer”.",
    })
  );

  const passage = el("blockquote", { class: "passage" }, el("div", { text: text || "(empty passage)" }));
  if (url) {
    passage.appendChild(
      el(
        "cite",
        {},
        "read from ",
        el("a", {
          href: url,
          target: "_blank",
          rel: "noopener noreferrer",
          text: hostOf(url),
          style: { color: "var(--fg-muted)" },
        })
      )
    );
  }
  wrap.appendChild(passage);

  // ce_score with its mandatory honesty label.
  if (Number.isFinite(ce)) {
    wrap.appendChild(
      el(
        "div",
        { style: { display: "flex", gap: "0.5rem", "align-items": "baseline", "flex-wrap": "wrap" } },
        el("span", { style: { "font-family": "var(--mono)" }, text: `ce_score ${fmtScore(ce, 2)}` }),
        el("span", {
          class: "honesty",
          text: "relevance, not correctness — a confidently relevant passage can still be wrong.",
        })
      )
    );
  }

  // C1 corroboration (schema 2): distinct INDEPENDENT evidence clusters (≠ the
  // passage's own) whose top passage the cross-encoder finds states the same
  // claim. Same-cluster syndicated copies are excluded BY CONSTRUCTION, so
  // syndication can never inflate the count. Absence means "no independent
  // support found", never "no answer".
  const corr = bp.corroboration && typeof bp.corroboration === "object" ? bp.corroboration : null;
  if (corr) {
    const n = Number(corr.independent_clusters);
    const urls = Array.isArray(corr.supporting_urls) ? corr.supporting_urls : [];
    const basis = corr.basis && typeof corr.basis === "object" ? corr.basis : {};
    const cwrap = el("div", { class: "search-corroboration", style: { "margin-top": "0.4rem" } });
    cwrap.appendChild(
      el(
        "div",
        { style: { display: "flex", gap: "0.5rem", "align-items": "baseline", "flex-wrap": "wrap" } },
        el("span", {
          class: n > 0 ? "chip chip--ok" : "chip chip--info",
          text: Number.isFinite(n) ? `${n} independent cluster(s) corroborate` : "corroboration checked",
        }),
        el("span", {
          class: "honesty",
          text: "distinct ADR-18 clusters (≠ the passage's own) whose top passage the cross-encoder finds states the same claim — same-cluster copies excluded, so syndication can't inflate it.",
        })
      )
    );
    for (const u of urls.slice(0, 6)) {
      cwrap.appendChild(
        el(
          "div",
          { class: "result-url" },
          el("a", { href: String(u), target: "_blank", rel: "noopener noreferrer", text: hostOf(u), style: { color: "var(--fg-muted)" } })
        )
      );
    }
    if (Number.isFinite(Number(basis.candidates_checked))) {
      cwrap.appendChild(
        el("div", {
          class: "honesty",
          text: `basis: ${Number(basis.candidates_checked)} distinct other-cluster(s) checked via ${basis.method || "cross-cluster CE"} — thin coverage degrades to a low count, never a false badge.`,
        })
      );
    }
    wrap.appendChild(cwrap);
  }
  return wrap;
}

// ---------------------------------------------------------------------------
// analysis block (fetch_budget>0). VoI fetch-phase honesty: how many pages were
// fetched, why the walk stopped, and the best unopened value left on the table.
// ---------------------------------------------------------------------------
function analysisBlock(an) {
  if (!an || typeof an !== "object") return null;
  const wrap = el("div", { class: "search-analysis", style: { margin: "0.6rem 0" } });
  wrap.appendChild(el("h3", { class: "subhead", text: "fetch analysis (VoI)" }));
  const fetches = Number(an.fetches_made);
  const stop = an.search_stopped_because != null ? String(an.search_stopped_because) : "—";
  const gain = Number(an.estimated_marginal_gain_remaining);
  const kv = el("dl", { class: "kv" });
  kv.appendChild(el("dt", { text: "fetches made" }));
  kv.appendChild(el("dd", { text: Number.isFinite(fetches) ? String(fetches) : "—" }));
  kv.appendChild(el("dt", { text: "stopped because" }));
  kv.appendChild(el("dd", { text: stop }));
  kv.appendChild(el("dt", { text: "est. marginal gain remaining" }));
  kv.appendChild(el("dd", { text: Number.isFinite(gain) ? fmtScore(gain, 3) : "—" }));
  wrap.appendChild(kv);
  wrap.appendChild(
    el("div", {
      class: "honesty",
      text: "the VoI walk stops when the next fetch's expected gain falls below its cost (ADR-26) — opt-in per-query egress, direct lane only.",
    })
  );
  return wrap;
}

// ---------------------------------------------------------------------------
// divergence block (compare=vantages). jsd vs noise_floor_p90 + exceeds_floor,
// with the per-request (NOT population) caveat. Distribution diffs by lane.
// ---------------------------------------------------------------------------
function divergenceBlock(dv) {
  if (!dv || typeof dv !== "object") return null;
  const jsd = Number(dv.jsd);
  const floor = Number(dv.noise_floor_p90);
  const exceeds = dv.exceeds_floor === true;
  const lanes = Array.isArray(dv.lanes_compared) ? dv.lanes_compared : null;

  const wrap = el("div", { class: "search-divergence", style: { margin: "0.6rem 0" } });
  wrap.appendChild(
    el(
      "h3",
      { class: "subhead", style: { display: "flex", gap: "0.5rem", "align-items": "baseline" } },
      "vantage divergence",
      lanes ? el("span", { class: "muted", style: { "font-size": "0.74rem" }, text: lanes.join(" vs ") }) : null
    )
  );

  // jsd vs the floor, with the exceeds_floor verdict as the primary signal.
  const head = el(
    "div",
    { style: { display: "flex", gap: "0.5rem", "align-items": "baseline", "flex-wrap": "wrap", margin: "0.2rem 0" } },
    el("span", { style: { "font-family": "var(--mono)" }, text: `JSD ${fmtScore(jsd, 3)}` }),
    el("span", { class: "muted", style: { "font-family": "var(--mono)" }, text: `noise floor p90 ${fmtScore(floor, 3)}` }),
    el("span", {
      class: exceeds ? "sig-badge" : "honesty",
      text: exceeds ? "exceeds floor" : "within noise floor",
    })
  );
  wrap.appendChild(head);

  // jsd is bounded [0,1] — a gauge with the floor stated in the caption.
  gauge(wrap, {
    label: "domain-distribution divergence (JSD)",
    value: Number.isFinite(jsd) ? jsd : 0,
    min: 0,
    max: 1,
    caption: `vs measured same-lane noise floor p90 = ${fmtScore(floor, 3)} on this deployment`,
  });

  // domains unique to each lane — explainability for the divergence.
  const onlyDirect = Array.isArray(dv.domains_only_in_direct) ? dv.domains_only_in_direct : [];
  const onlyAnon = Array.isArray(dv.domains_only_in_anon) ? dv.domains_only_in_anon : [];
  if (onlyDirect.length) {
    wrap.appendChild(el("div", { class: "muted", style: { "font-size": "0.78rem" }, text: "only in direct:" }));
    chips(wrap, onlyDirect.map(String), "status");
  }
  if (onlyAnon.length) {
    wrap.appendChild(el("div", { class: "muted", style: { "font-size": "0.78rem" }, text: "only in anon:" }));
    chips(wrap, onlyAnon.map(String), "status");
  }

  // The MANDATORY population-level caveat.
  wrap.appendChild(
    el("div", {
      class: "honesty",
      text:
        "exceeds_floor is a PER-REQUEST signal comparing this query's JSD to the " +
        "deployment's same-lane noise floor — NOT a population-level claim that the lanes differ in general.",
    })
  );
  return wrap;
}

// ---------------------------------------------------------------------------
// Panel.
// ---------------------------------------------------------------------------
registerPanel({
  id: "panel-search",
  title: "search",
  refreshMs: null, // on-demand only
  requiresBearer: false,
  async render(container) {
    // Small labeled-field helper.
    const field = (label, control) =>
      el("label", { class: "field" }, el("span", { text: label }), control);

    // --- Primary controls. All map to documented /v1/search params. ----------
    const input = el("input", {
      type: "text",
      class: "q",
      placeholder: "query (never logged)",
      "aria-label": "search query",
    });
    const modeSel = el(
      "select",
      { "aria-label": "mode" },
      el("option", { value: "fast", text: "fast" }),
      el("option", { value: "deep", text: "deep (rerank)" })
    );
    const scopeSel = el(
      "select",
      { "aria-label": "scope" },
      el("option", { value: "both", text: "both" }),
      el("option", { value: "local", text: "local" }),
      el("option", { value: "web", text: "web" })
    );
    const laneSel = el(
      "select",
      { "aria-label": "lane" },
      el("option", { value: "direct", text: "direct" }),
      el("option", { value: "anon", text: "anon (Tor)" }),
      el("option", { value: "region", text: "region:…" })
    );
    const regionInput = el("input", { type: "text", placeholder: "region id", style: { width: "7rem", display: "none" } });
    laneSel.addEventListener("change", () => {
      regionInput.style.display = laneSel.value === "region" ? "" : "none";
    });
    const limitInput = el("input", { type: "number", min: "1", max: "50", placeholder: "limit", style: { width: "5rem" } });
    const run = el("button", { class: "action", text: "search" });

    const row1 = el("div", { class: "search-form" }, input, run);
    const row2 = el(
      "div",
      { class: "field-row" },
      field("mode", modeSel),
      field("scope", scopeSel),
      field("lane", laneSel),
      regionInput,
      field("limit", limitInput)
    );

    // --- Answer / fetch / diversity / compare toggles ------------------------
    const answerChk = el("input", { type: "checkbox" });
    const diversityChk = el("input", { type: "checkbox" });
    const compareChk = el("input", { type: "checkbox" });
    const fetchBudgetInput = el("input", { type: "number", min: "0", max: "8", placeholder: "budget", style: { width: "5rem" } });
    const row3 = el(
      "div",
      { class: "field-row" },
      el("label", { class: "checkbox" }, answerChk, "answer (extractive passage)"),
      field("fetch_budget", fetchBudgetInput),
      el("label", { class: "checkbox" }, diversityChk, "diversity=evidence"),
      el("label", { class: "checkbox" }, compareChk, "compare=vantages")
    );

    // --- Geo + time window ---------------------------------------------------
    const latInput = el("input", { type: "number", step: "any", placeholder: "lat", style: { width: "6rem" } });
    const lonInput = el("input", { type: "number", step: "any", placeholder: "lon", style: { width: "6rem" } });
    const radiusInput = el("input", { type: "number", step: "any", placeholder: "radius km", style: { width: "6rem" } });
    const h3Input = el("input", { type: "text", placeholder: "h3 cell (alt to lat/lon)", style: { width: "11rem" } });
    const afterInput = el("input", { type: "date", "aria-label": "after" });
    const beforeInput = el("input", { type: "date", "aria-label": "before" });
    const row4 = el(
      "div",
      { class: "field-row" },
      field("lat", latInput),
      field("lon", lonInput),
      field("radius_km", radiusInput),
      field("h3", h3Input),
      field("after", afterInput),
      field("before", beforeInput)
    );
    const geoNote = el("div", {
      class: "honesty",
      text: "geo/time filters apply to LOCAL documents only (ADR-10) — web results can't be geo-filtered; scope=web + a filter is refused, scope=both serves filtered-local + unfiltered-web (degraded:[geo_web_unfiltered]).",
    });

    // --- Advanced / Tuning drawer (ADR-30), built from /v1/config ------------
    const drawer = el("details", { class: "drawer" });
    drawer.appendChild(
      el(
        "summary",
        {},
        "Advanced / Tuning",
        el("span", { class: "drawer-sub", text: " — per-request overrides, clamped, persist nothing" })
      )
    );
    const drawerBody = el("div", { class: "drawer-body" }, el("div", { class: "muted", text: "loading config…" }));
    drawer.appendChild(drawerBody);
    // tuneControls: key -> { input, def, isBool }
    const tuneControls = new Map();
    getConfig().then(({ ok, data }) => {
      drawerBody.replaceChildren();
      if (!ok || !data || !data.tunable) {
        drawerBody.appendChild(el("div", { class: "panel-empty" }, "tuning config unavailable (GET /v1/config) — defaults still apply server-side."));
        return;
      }
      const resetAll = el("button", { class: "ghost", text: "reset all" });
      drawerBody.appendChild(el("div", { class: "drawer-actions" }, resetAll));
      const grid = el("div", { class: "tune-grid" });
      const entries = Object.entries(data.tunable);
      for (const [key, meta] of entries) {
        const def = meta.default;
        const isBool = typeof def === "boolean";
        const nameEl = el("span", { class: "tune-name", text: key });
        let ctl;
        if (isBool) {
          ctl = el(
            "select",
            {},
            el("option", { value: "", text: `default (${def})` }),
            el("option", { value: "true", text: "true" }),
            el("option", { value: "false", text: "false" })
          );
        } else {
          ctl = el("input", {
            type: "number",
            step: "any",
            placeholder: String(def),
          });
          if (meta.min != null) ctl.min = String(meta.min);
          if (meta.max != null) ctl.max = String(meta.max);
        }
        const mark = () => {
          const changed = isBool ? ctl.value !== "" : ctl.value !== "" && Number(ctl.value) !== Number(def);
          nameEl.classList.toggle("changed", changed);
        };
        ctl.addEventListener("input", mark);
        ctl.addEventListener("change", mark);
        tuneControls.set(key, { input: ctl, def, isBool });
        const metaLine =
          (meta.unit ? `${meta.unit} · ` : "") +
          `default ${def}` +
          (meta.min != null && meta.max != null ? ` · range ${meta.min}–${meta.max}` : "");
        grid.appendChild(
          el(
            "div",
            { class: "tune" },
            nameEl,
            ctl,
            meta.rationale ? el("div", { class: "tune-why", text: meta.rationale }) : null,
            el("div", { class: "tune-meta", text: metaLine })
          )
        );
      }
      drawerBody.appendChild(grid);
      resetAll.addEventListener("click", () => {
        for (const { input: c } of tuneControls.values()) c.value = "";
        for (const n of grid.querySelectorAll(".tune-name")) n.classList.remove("changed");
      });
    });

    const form = el("div", {}, row1, row2, row3, row4, geoNote, drawer);
    const out = el("div", { class: "search-out" });

    async function doSearch() {
      const qRaw = input.value || "";
      if (!qRaw.trim()) {
        out.replaceChildren(el("div", { class: "panel-empty" }, "enter a query to search."));
        return;
      }
      out.replaceChildren(el("div", { class: "muted" }, "searching…"));

      const params = new URLSearchParams();
      params.set("q", qRaw);
      const mode = modeSel.value || "fast";
      const scope = scopeSel.value || "both";
      if (mode !== "fast") params.set("mode", mode);
      if (scope !== "both") params.set("scope", scope);
      // lane
      const lane = laneSel.value;
      if (lane === "anon") params.set("lane", "anon");
      else if (lane === "region" && regionInput.value.trim()) params.set("lane", `region:${regionInput.value.trim()}`);
      if (limitInput.value.trim()) params.set("limit", limitInput.value.trim());
      // answer / fetch_budget / diversity / compare
      if (fetchBudgetInput.value.trim()) params.set("fetch_budget", fetchBudgetInput.value.trim());
      if (answerChk.checked) params.set("answer", "true");
      if (diversityChk.checked) params.set("diversity", "evidence");
      if (compareChk.checked) params.set("compare", "vantages");
      // geo: h3 takes precedence; else lat/lon(+radius_km)
      const h3v = h3Input.value.trim();
      if (h3v) {
        params.set("h3", h3v);
      } else {
        if (latInput.value.trim()) params.set("lat", latInput.value.trim());
        if (lonInput.value.trim()) params.set("lon", lonInput.value.trim());
        if (radiusInput.value.trim()) params.set("radius_km", radiusInput.value.trim());
      }
      // time window: date → unix seconds (UTC midnight)
      const toUnix = (v) => {
        const t = Date.parse(`${v}T00:00:00Z`);
        return Number.isFinite(t) ? Math.floor(t / 1000) : null;
      };
      if (afterInput.value) {
        const u = toUnix(afterInput.value);
        if (u != null) params.set("after", String(u));
      }
      if (beforeInput.value) {
        const u = toUnix(beforeInput.value);
        if (u != null) params.set("before", String(u));
      }
      // tuning overrides (ADR-30): only knobs CHANGED from their default → ov_<key>.
      for (const [key, { input: c, def, isBool }] of tuneControls) {
        const raw = c.value;
        if (raw === "" || raw == null) continue;
        if (isBool || Number(raw) !== Number(def)) params.set(`ov_${key}`, raw);
      }
      const path = `/v1/search?${params.toString()}`;

      // Fire WITHOUT a bearer first (the static shell is unauthenticated and
      // search is usually open). Attach the operator token ONLY if the guarded
      // deployment answers 401 (auth.require_bearer_for_search).
      let res = await fetchJSON(path);
      if (res.status === 401) {
        // currentBearer via the runtime: the token input drives it. Read it live.
        const { currentBearer } = await import("../app.js");
        const tok = currentBearer();
        if (tok) {
          res = await fetchJSON(path, { bearer: tok });
        }
      }

      // --- Honest, distinct failure modes. -----------------------------------
      if (!res.ok) {
        let msg;
        if (res.status === 401 || res.status === 403) {
          msg = "search is guarded on this deployment — paste the operator token above, then search again.";
        } else if (res.status === 503) {
          // anon bootstrapping/busy, region unverified, auth not configured, or ingest-pressure shedding.
          msg = `search refused (503)${res.error ? `: ${res.error}` : ""} — e.g. anon lane bootstrapping/busy, region unverified, or auth not configured.`;
        } else if (res.status === 400) {
          msg = `bad request (400)${res.error ? `: ${res.error}` : ""}.`;
        } else if (res.status === 429) {
          msg = "rate-limited (429) — slow down and retry.";
        } else if (res.status === 504) {
          msg = "search timed out (504) — the appliance exceeded its request ceiling.";
        } else if (res.status === 0) {
          msg = "meridiand unreachable — the appliance may be stopped or unreachable from here.";
        } else {
          msg = `search failed — HTTP ${res.status}${res.error ? `: ${res.error}` : ""}.`;
        }
        out.replaceChildren(el("div", { class: "panel-empty" }, msg));
        return;
      }

      const d = res.data || {};
      const results = Array.isArray(d.results) ? d.results : [];
      const frag = el("div", { class: "search-result-set" });

      // --- lane_requested vs lane_effective badge (only when they differ). ---
      const laneReq = d.lane_requested != null ? String(d.lane_requested) : null;
      const laneEff = d.lane_effective != null ? String(d.lane_effective) : null;
      if (laneReq && laneEff && laneReq !== laneEff) {
        frag.appendChild(
          el(
            "div",
            { class: "lane-badge", style: { margin: "0.2rem 0" } },
            el("span", { class: "chip chip--degraded", text: `lane requested ${laneReq} → effective ${laneEff}` })
          )
        );
      } else if (laneEff) {
        frag.appendChild(
          el("div", { class: "muted", style: { "font-size": "0.76rem", margin: "0.2rem 0" }, text: `lane: ${laneEff}` })
        );
      }

      // --- EVERY degraded[] flag as a chip. ----------------------------------
      const degraded = Array.isArray(d.degraded) ? d.degraded.filter((x) => x != null).map(String) : [];
      if (degraded.length) {
        frag.appendChild(el("div", { class: "muted", style: { "font-size": "0.76rem" }, text: "degraded stages (results still served honestly labeled):" }));
        chips(frag, degraded, "degraded");
      }

      // --- Additive corroboration / abstained fields (may NOT exist yet). ----
      // Read ONLY when present; ignore gracefully when absent — never fabricate.
      // `abstained` (bool/obj): the engine declined to assert a confident answer.
      if (Object.prototype.hasOwnProperty.call(d, "abstained")) {
        const ab = d.abstained;
        const abstained = ab === true || (ab && typeof ab === "object" && ab.abstained === true);
        if (abstained) {
          const reason = ab && typeof ab === "object" && ab.reason != null ? String(ab.reason) : null;
          frag.appendChild(
            el(
              "div",
              { class: "insufficient", style: { margin: "0.3rem 0" } },
              el("span", { class: "verdict", text: "abstained" }),
              el("div", { class: "honesty", text: reason ? `the engine declined to assert a confident answer: ${reason}` : "the engine declined to assert a confident answer — verify directly." })
            )
          );
        }
      }
      // Claim-level corroboration is part of best_passage (schema 2) and rendered
      // there — not a separate top-level field.

      // --- applied_overrides (ADR-30): the CLAMPED effective tuning for THIS
      //     request — the honest "you asked 9000, you got 5000" readout. -------
      if (d.applied_overrides && typeof d.applied_overrides === "object") {
        const items = Object.entries(d.applied_overrides);
        if (items.length) {
          frag.appendChild(
            el("div", {
              class: "applied-overrides",
              html:
                "tuning in effect — " +
                items.map(([k, v]) => `<b>${escapeHtml(k)}</b>=${escapeHtml(String(v))}`).join(", "),
            })
          );
        }
      }

      // --- Result count + the result list (or a clear empty message). --------
      frag.appendChild(
        el("div", {
          class: "muted",
          style: { margin: "0.3rem 0", "font-family": "var(--mono)", "font-size": "0.8rem" },
          text: `${results.length} result(s)`,
        })
      );
      if (results.length === 0) {
        frag.appendChild(el("div", { class: "panel-empty" }, "no results for this query."));
      } else {
        const list = el("div", { class: "results" });
        results.forEach((r, i) => list.appendChild(resultRow(r, i)));
        frag.appendChild(list);
      }

      // --- best_passage (answer mode). When answer was requested but no passage
      //     materialized, degraded:["answer_unavailable"] is the honest signal
      //     (already rendered above as a chip); silence never means "no answer". -
      const bp = bestPassageBlock(d.best_passage);
      if (bp) frag.appendChild(bp);

      // --- confidence (additive). --------------------------------------------
      const cb = confidenceBlock(d.confidence);
      if (cb) frag.appendChild(cb);

      // --- evidence (additive; absent when [evidence] enabled = false). ------
      const eb = evidenceBlock(d.evidence);
      if (eb) frag.appendChild(eb);

      // --- divergence (compare=vantages). ------------------------------------
      const db = divergenceBlock(d.divergence);
      if (db) frag.appendChild(db);

      // --- analysis (fetch_budget>0): the VoI fetch-phase honesty block. ------
      const an = analysisBlock(d.analysis);
      if (an) frag.appendChild(an);

      // --- timings.stage_ms as a barChart. -----------------------------------
      const t = d.timings && typeof d.timings === "object" ? d.timings : null;
      if (t) {
        // Per-stage durations; `total` is the envelope, charted separately-labeled.
        const STAGE_ORDER = ["plan", "lexical", "fusion", "evidence_ms", "total"];
        const known = STAGE_ORDER.filter((k) => Number.isFinite(Number(t[k])));
        // Include any additional stage keys the server may add (forward-compatible).
        const extra = Object.keys(t).filter((k) => !STAGE_ORDER.includes(k) && Number.isFinite(Number(t[k])));
        const stages = [...known.filter((k) => k !== "total"), ...extra];
        if (stages.length) {
          const timingWrap = el("div", { class: "search-timings", style: { margin: "0.6rem 0" } });
          timingWrap.appendChild(
            el(
              "h3",
              { class: "subhead", style: { display: "flex", gap: "0.5rem", "align-items": "baseline" } },
              "timings",
              Number.isFinite(Number(t.total))
                ? el("span", { class: "muted", style: { "font-size": "0.74rem", "font-family": "var(--mono)" }, text: `total ${Number(t.total)} ms` })
                : null
            )
          );
          const canvas = el("canvas", { class: "chart" });
          timingWrap.appendChild(canvas);
          frag.appendChild(timingWrap);
          // Chart after the canvas is in the DOM so it has a layout width.
          requestAnimationFrame(() => {
            const bars = stages.map((k) => ({
              label: k === "evidence_ms" ? "evidence" : k,
              value: Number(t[k]),
              color: "var(--accent)",
            }));
            barChart(canvas, bars, { yLabel: "ms", valueFormat: (v) => `${Math.round(v)}` });
          });
        }
      }

      out.replaceChildren(frag);
    }

    run.addEventListener("click", doSearch);
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        doSearch();
      }
    });

    container.appendChild(form);
    container.appendChild(out);
    container.appendChild(
      el("div", {
        class: "honesty",
        style: { "margin-top": "0.4rem", "font-size": "0.72rem" },
        text: "Read-only: this panel only issues GET /v1/search. The query text is never logged by the server and never stored here.",
      })
    );
  },
});
