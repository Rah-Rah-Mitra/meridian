// OPE ship-gate panel — GET /v1/decision-log/ope (bearer-gated, on-demand).
//
// The ADR-25 ship-gate report (api.md §GET /v1/decision-log/ope). The server
// trains the linear-TS candidate on the older 80% of the retained decision log
// and reports its doubly-robust uplift vs the incumbent's realized reward on the
// held-out 20%, with a bootstrap 95% CI. Canonical response shape (verbatim from
// the meridiand handler):
//
//   { "n_total": 8421, "n_train": 6736, "n_eval": 1500,
//     "n_eval_generalized_skipped": 185,
//     "report": { "n": 1500, "incumbent_mean": 0.61, "candidate_dr": 0.64,
//                 "uplift": 0.03, "ci_lo": -0.01, "ci_hi": 0.07 },
//     "gate": { "min_decisions": 10000,
//               "rule": "95% bootstrap CI of DR uplift must exclude zero from above (ADR-25)",
//               "verdict": "insufficient_data" } }
//
//   verdict ∈ insufficient_data | pass | negative | inconclusive
//   ci_lo / ci_hi may be null (NaN ⇒ no decisions in the eval split).
//
// HONESTY POSTURE:
//   - `insufficient_data` is a FIRST-CLASS, NON-ALARMING state ("not enough
//     decisions yet"), never an error or a red flag.
//   - The min-decisions bar shows n_total vs the 10000 floor explicitly.
//   - The uplift CI is drawn AROUND ZERO — the gate is "CI excludes zero from
//     above", so zero is the reference line the operator must read against.
//   - The gate `rule` string is shown VERBATIM (no paraphrase).
//   - This endpoint only reports; it never changes routing. We say so.
import { registerPanel, fetchJSON, el, gauge } from "../app.js";

// Per-verdict headline copy. insufficient_data is deliberately calm.
const VERDICT_HEADLINE = {
  insufficient_data: "not enough decisions yet",
  pass: "PASS — candidate uplift CI excludes zero from above",
  inconclusive: "inconclusive — CI straddles zero, ε-greedy stays",
  negative: "negative — candidate underperforms, ε-greedy stays",
};

// Plain-language operator gloss under the headline.
const VERDICT_NOTE = {
  insufficient_data:
    "The gate needs ≥ 10000 retained decisions before it can return a real verdict. " +
    "This is the expected early state, not a problem — keep the deployment serving.",
  pass:
    "The 95% bootstrap CI of the doubly-robust uplift lies entirely above zero. " +
    "Per ADR-25 you MAY enable searx.contextual_policy — this report does not flip it for you.",
  inconclusive:
    "The CI includes zero, so the candidate is not demonstrably better. ε-greedy stays (ADR-25 sunset rule).",
  negative:
    "The CI lies entirely below zero — the candidate is worse on the held-out split. ε-greedy stays.",
};

const MIN_DECISIONS_FALLBACK = 10000;

function fmtSigned(v) {
  if (!Number.isFinite(v)) return "—";
  return (v >= 0 ? "+" : "") + v.toFixed(3);
}

// Draw the uplift point estimate and its 95% CI on a track centered/anchored so
// that ZERO is a visible reference line. Hand-rolled DOM (no canvas needed) —
// the honest reading is "is the whole bar to the right of zero?".
function upliftCI(report) {
  const uplift = Number(report && report.uplift);
  const lo = Number(report && report.ci_lo);
  const hi = Number(report && report.ci_hi);
  const haveCI = Number.isFinite(lo) && Number.isFinite(hi);

  const wrap = el("div", { class: "ope-uplift", style: { margin: "0.5rem 0" } });
  wrap.appendChild(
    el(
      "div",
      {
        class: "ope-uplift-head",
        style: {
          display: "flex",
          "justify-content": "space-between",
          "align-items": "baseline",
          gap: "0.75rem",
          "flex-wrap": "wrap",
        },
      },
      el("span", { class: "honesty", text: "doubly-robust uplift vs incumbent (held-out 20%)" }),
      el("span", {
        class: "ope-uplift-num",
        style: { "font-family": "var(--mono, ui-monospace, monospace)" },
        text: haveCI
          ? `${fmtSigned(uplift)}  [${fmtSigned(lo)}, ${fmtSigned(hi)}]`
          : `${fmtSigned(uplift)}  (CI unavailable — no eval rows)`,
      })
    )
  );

  if (haveCI) {
    // Symmetric domain around zero so zero sits dead center; never let a tiny
    // CI collapse the scale.
    const span = Math.max(Math.abs(lo), Math.abs(hi), Math.abs(uplift), 0.02);
    const toPct = (v) => ((v + span) / (2 * span)) * 100;
    const left = Math.max(0, Math.min(100, toPct(lo)));
    const right = Math.max(0, Math.min(100, toPct(hi)));
    const point = Math.max(0, Math.min(100, toPct(uplift)));
    const excludesZeroAbove = lo > 0;

    // Structural styles are inline so this panel renders correctly with ZERO
    // dependency on new style.css rules (semantic classes are kept so a
    // stylesheet can still override later). The track is a positioned context;
    // the three children are absolutely positioned within it.
    const barColor = excludesZeroAbove
      ? "var(--ok, #4caf7d)"
      : "var(--accent-dim, #4a6fa5)";
    const track = el(
      "div",
      {
        class: "ope-ci-track",
        style: {
          position: "relative",
          height: "1.5rem",
          margin: "0.35rem 0",
          background: "var(--bg-sunken, #11151b)",
          border: "1px solid var(--border, #2a3038)",
          "border-radius": "4px",
        },
      },
      // The zero reference line — the gate's threshold (dead center).
      el("div", {
        class: "ope-ci-zero",
        style: {
          position: "absolute",
          left: "50%",
          top: "0",
          bottom: "0",
          width: "1px",
          background: "var(--fg-muted, #8b97a7)",
          transform: "translateX(-0.5px)",
        },
      }),
      // The CI interval bar; tinted ok-green only when it clears zero from above.
      el("div", {
        class: "ope-ci-bar" + (excludesZeroAbove ? " ope-ci-bar--clears" : ""),
        style: {
          position: "absolute",
          left: `${left}%`,
          width: `${Math.max(0.5, right - left)}%`,
          top: "50%",
          height: "0.45rem",
          transform: "translateY(-50%)",
          background: barColor,
          opacity: "0.55",
          "border-radius": "3px",
        },
      }),
      // The point estimate (uplift), drawn on top of the interval.
      el("div", {
        class: "ope-ci-point",
        style: {
          position: "absolute",
          left: `${point}%`,
          top: "50%",
          width: "0.55rem",
          height: "0.55rem",
          transform: "translate(-50%, -50%)",
          background: barColor,
          border: "1.5px solid var(--fg, #d7dee8)",
          "border-radius": "50%",
        },
      })
    );
    wrap.appendChild(track);
    wrap.appendChild(
      el(
        "div",
        {
          class: "ope-ci-scale",
          style: {
            display: "flex",
            "justify-content": "space-between",
            "font-family": "var(--mono, ui-monospace, monospace)",
            "font-size": "0.7rem",
            color: "var(--fg-muted, #8b97a7)",
          },
        },
        el("span", { text: fmtSigned(-span) }),
        el("span", { class: "honesty", text: "0 (gate threshold)" }),
        el("span", { text: fmtSigned(span) })
      )
    );
  }
  return wrap;
}

registerPanel({
  id: "panel-ope",
  title: "ope ship-gate",
  refreshMs: 0, // on-demand: rendered on load + when the token changes
  requiresBearer: true,
  async render(container, { bearer }) {
    // --- No token: invite one. Not an error. --------------------------------
    if (!bearer) {
      container.appendChild(
        el(
          "div",
          { class: "panel-empty" },
          "paste operator token to view OPE — this panel calls a guarded endpoint (/v1/decision-log/ope)."
        )
      );
      return;
    }

    const { ok, status, data, error } = await fetchJSON("/v1/decision-log/ope", {
      bearer,
    });

    // --- Distinct, honest messages per failure mode. ------------------------
    if (!ok) {
      let msg;
      if (status === 404) {
        // searx.decision_log = false (the default).
        msg =
          "decision log disabled (searx.decision_log = false) — there is nothing to evaluate yet.";
      } else if (status === 401 || status === 403) {
        msg = "bearer required / wrong token — check the operator token above.";
      } else if (status === 503) {
        // api.md: gated endpoints answer 503 when no token is configured server-side.
        msg = `auth not configured on the appliance${error ? ` (${error})` : ""}.`;
      } else if (status === 0) {
        msg = "meridiand unreachable — the appliance may be stopped or unreachable from here.";
      } else {
        msg = `ope report unavailable — HTTP ${status}${error ? `: ${error}` : ""}.`;
      }
      container.appendChild(el("div", { class: "panel-empty" }, msg));
      return;
    }

    const d = data || {};
    const gate = d.gate || {};
    const report = d.report || {};
    const verdict =
      gate.verdict != null ? String(gate.verdict) : "unknown";
    const nTotal = Number.isFinite(Number(d.n_total)) ? Number(d.n_total) : 0;
    const minDecisions = Number.isFinite(Number(gate.min_decisions))
      ? Number(gate.min_decisions)
      : MIN_DECISIONS_FALLBACK;

    // --- Verdict headline. insufficient_data is rendered in the calm,
    //     first-class .insufficient box; every other verdict uses its
    //     verdict-<v> color but is NOT alarmist. ------------------------------
    if (verdict === "insufficient_data") {
      container.appendChild(
        el(
          "div",
          { class: "insufficient" },
          el("div", { class: "verdict", text: VERDICT_HEADLINE.insufficient_data }),
          el("div", {
            text: `${nTotal.toLocaleString()} of ${minDecisions.toLocaleString()} decisions retained`,
          }),
          VERDICT_NOTE.insufficient_data
            ? el("div", { class: "honesty", text: VERDICT_NOTE.insufficient_data })
            : null
        )
      );
    } else {
      container.appendChild(
        el(
          "div",
          { class: "ope-verdict" },
          el("span", {
            class: `verdict-${verdict}`,
            text: VERDICT_HEADLINE[verdict] || verdict,
          })
        )
      );
      if (VERDICT_NOTE[verdict]) {
        container.appendChild(
          el("div", { class: "honesty", text: VERDICT_NOTE[verdict] })
        );
      }
    }

    // --- n_total vs the 10000 min-decisions bar (always shown). -------------
    // The gauge caption keeps the "this is a floor, not a target" framing.
    gauge(container, {
      label: "decisions toward gate floor",
      value: nTotal,
      min: 0,
      max: minDecisions,
      caption: `${nTotal.toLocaleString()} / ${minDecisions.toLocaleString()} — the gate cannot return a real verdict below this floor`,
    });

    // --- The uplift CI around zero (only meaningful once we have eval rows). -
    container.appendChild(upliftCI(report));

    // --- Supporting numbers, verbatim from the report. ----------------------
    const incumbent = Number(report.incumbent_mean);
    const candidate = Number(report.candidate_dr);
    const nEval = Number(d.n_eval);
    const nTrain = Number(d.n_train);
    const skipped = Number(d.n_eval_generalized_skipped);
    const kv = el("dl", { class: "kv" });
    const addKV = (k, v) => {
      kv.appendChild(el("dt", { text: k }));
      kv.appendChild(el("dd", { text: v }));
    };
    addKV(
      "incumbent realized reward",
      Number.isFinite(incumbent) ? incumbent.toFixed(3) : "—"
    );
    addKV(
      "candidate DR value",
      Number.isFinite(candidate) ? candidate.toFixed(3) : "—"
    );
    addKV("train rows (older 80%)", Number.isFinite(nTrain) ? nTrain.toLocaleString() : "—");
    addKV(
      "eval rows (held-out 20%)",
      Number.isFinite(nEval) ? nEval.toLocaleString() : "—"
    );
    if (Number.isFinite(skipped) && skipped > 0) {
      // k-anonymity-generalized rows carry no replayable features — excluded
      // from the eval, counted honestly here.
      addKV("eval rows skipped (k-anon generalized)", skipped.toLocaleString());
    }
    container.appendChild(kv);

    // --- The gate rule string, VERBATIM. ------------------------------------
    if (gate.rule != null) {
      container.appendChild(
        el(
          "div",
          { class: "ope-rule" },
          el("span", { class: "honesty", text: "gate rule: " }),
          el("code", { text: String(gate.rule) })
        )
      );
    }

    // --- Standing honesty disclaimer: reporting only. -----------------------
    container.appendChild(
      el("div", {
        class: "honesty",
        text:
          "This report only evaluates — it never changes routing. Enabling " +
          "searx.contextual_policy on a pass is the operator's explicit action (ADR-25).",
      })
    );
  },
});
