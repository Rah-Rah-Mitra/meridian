#!/usr/bin/env bash
# UI smoke test (SPEC §UI / candidate-ADR 05-ui-track): assert the embedded
# operator console is served correctly at /ui/* AND that every panel's backing
# /v1 endpoint returns the keys that panel needs against a seeded corpus.
#
# The UI is read-only GET-only bytes baked into the meridiand musl binary
# (crates/meridian-api/src/ui.rs). This script never writes anything: it only
# probes the static asset table and the GET analytics endpoints. It does NOT
# touch /v1/ingest, /v1/forget, or /v1/decision-log/wipe — seed the corpus with
# deploy/p5-ingest.py before running.
#
# Usage: deploy/ui-smoke.sh [base-url]
# Env:   BEARER  operator token for the guarded /v1/decision-log/ope call.
#                If unset, that one assertion is SKIPPED-with-note (not failed).
#
# Modeled on deploy/p5-exit-gate.sh / deploy/p1-exit-gate.sh.

set -euo pipefail

BASE="${1:-http://127.0.0.1:8080}"
BEARER="${BEARER:-}"
fail=0

# --- helpers ---------------------------------------------------------------

# pass/fail line printers; bump $fail on failure.
pass() { printf 'PASS: %s\n' "$*"; }
failit() { printf 'FAIL: %s\n' "$*"; fail=1; }
skip() { printf 'SKIP: %s\n' "$*"; }

# Fetch an asset's HTTP status and content-type in one request.
# Prints "<code> <content-type>" (content-type lowercased, params kept).
http_head() {
  curl -s -o /dev/null -w '%{http_code} %{content_type}\n' "$1" \
    | tr '[:upper:]' '[:lower:]'
}

# Assert: GET $path -> expected status, and (optionally) content-type contains
# the given substring. $1=label $2=url $3=want_status $4=want_ctype_substr(opt)
assert_asset() {
  local label="$1" url="$2" want_status="$3" want_ctype="${4:-}"
  local got code ctype
  got=$(http_head "$url") || got="000 "
  code="${got%% *}"
  ctype="${got#* }"
  if [[ "$code" != "$want_status" ]]; then
    failit "$label: GET $url -> $code (want $want_status)"
    return
  fi
  if [[ -n "$want_ctype" && "$ctype" != *"$want_ctype"* ]]; then
    failit "$label: GET $url content-type '$ctype' lacks '$want_ctype'"
    return
  fi
  pass "$label: $code${want_ctype:+ $want_ctype} ($url)"
}

# GET a /v1 endpoint and pipe its body through a python json predicate.
# $1=label $2=url $3=python-body. The python reads stdin and must print "1"
# (assertion holds) or "0"; any exception -> FAIL with the raised message.
assert_json() {
  local label="$1" url="$2" py="$3"
  local body verdict
  body=$(curl -s "$url" || true)
  verdict=$(printf '%s' "$body" | python3 -c "
import json, sys
try:
    d = json.load(sys.stdin)
except Exception as e:
    print('ERR json: %s' % e); sys.exit(0)
try:
$(printf '%s\n' "$py" | sed 's/^/    /')
except Exception as e:
    print('ERR %s' % e)
" 2>/dev/null) || verdict="ERR python"
  case "$verdict" in
    1) pass "$label" ;;
    0) failit "$label: predicate false (endpoint shape mismatch)" ;;
    *) failit "$label: $verdict" ;;
  esac
}

echo "== preflight: server up =="
curl -s -o /dev/null "$BASE/healthz" || { echo "FAIL: server unreachable at $BASE"; exit 1; }
pass "healthz reachable at $BASE"

# --- 1. static asset table (ui.rs lookup coverage) -------------------------

echo
echo "== UI static assets (/ui/*, GET-only embedded bytes) =="
# Shell: bare prefix resolves to index.html -> 200 text/html.
assert_asset "shell"        "$BASE/ui/"               200 "text/html"
# App entry module -> 200 text/javascript.
assert_asset "app.js"       "$BASE/ui/app.js"         200 "text/javascript"
# A panel module (parallel-authored) -> 200 (js mime).
assert_asset "panels/lanes" "$BASE/ui/panels/lanes.js" 200 "text/javascript"
# Unknown path -> 404 (negative coverage of the asset table; no fs traversal).
assert_asset "unknown 404"  "$BASE/ui/nope"           404

# --- 2. backing endpoints: each panel's keys against a seeded corpus -------

echo
echo "== panel backing endpoints (key presence on a seeded corpus) =="

# lanes panel: array of {id,status,detail}; status is the honesty field.
assert_json "lanes: array w/ status field" "$BASE/v1/lanes" '
assert isinstance(d, list) and len(d) >= 1, "not a non-empty array"
row = d[0]
assert "status" in row, "row missing status"
assert "id" in row, "row missing id"
print(1)
'

# trends panel: series + top_movers (requires [analytics] enabled).
# A 404 here means analytics is opt-out on this node — report it as a skip-note
# rather than a hard fail, since the contract makes trends conditional.
TRENDS_CODE=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/v1/trends" || echo 000)
if [[ "$TRENDS_CODE" == "404" ]]; then
  skip "trends: /v1/trends 404 — [analytics] disabled on this node (GDELT opt-in)"
else
  assert_json "trends: series + top_movers" "$BASE/v1/trends" '
assert isinstance(d.get("series"), list), "series not a list"
assert isinstance(d.get("top_movers"), list), "top_movers not a list"
print(1)
'
fi

# geo panel: {res, cells:[...]} with the Gi* fields.
assert_json "geo: cells present" "$BASE/v1/geo/heatmap?res=5&window=all" '
assert "cells" in d and isinstance(d["cells"], list), "no cells array"
# A seeded, geo-tagged corpus has cells; allow empty only by warning below.
assert len(d["cells"]) >= 1, "cells empty (gazetteer/corpus not seeded?)"
c = d["cells"][0]
for k in ("h3", "count", "significant"):
    assert k in c, "cell missing %s" % k
print(1)
'

# search panel: confidence (uncalibrated) + degraded array. scope=local so the
# assertion never depends on web reachability.
SEARCH_URL=$(printf '%s/v1/search?q=%s&scope=local&limit=5' "$BASE" "city")
assert_json "search: confidence + degraded" "$SEARCH_URL" '
assert isinstance(d.get("results"), list), "no results array"
assert "degraded" in d and isinstance(d["degraded"], list), "no degraded array"
conf = d.get("confidence")
assert isinstance(conf, dict) and "score" in conf, "no confidence.score block"
# lane fields the UI shows when they diverge:
assert "lane_requested" in d and "lane_effective" in d, "missing lane_* fields"
print(1)
'

# --- 3. guarded OPE endpoint (bearer); insufficient_data IS the honesty pass -

echo
echo "== OPE ship-gate (guarded; insufficient_data on a fresh node is expected) =="
if [[ -z "$BEARER" ]]; then
  skip "ope: BEARER unset — guarded /v1/decision-log/ope not exercised (set BEARER=... to run)"
else
  OPE_RAW=$(curl -s -H "Authorization: Bearer $BEARER" "$BASE/v1/decision-log/ope" || true)
  OPE_CODE=$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $BEARER" "$BASE/v1/decision-log/ope" || echo 000)
  if [[ "$OPE_CODE" == "404" ]]; then
    skip "ope: /v1/decision-log/ope 404 — searx.decision_log disabled (default); panel degrades to a clear note"
  else
    VERDICT=$(printf '%s' "$OPE_RAW" | python3 -c "
import json, sys
try:
    d = json.load(sys.stdin)
except Exception as e:
    print('ERR json: %s' % e); sys.exit(0)
g = d.get('gate')
if not isinstance(g, dict) or 'verdict' not in g:
    print('ERR no gate.verdict'); sys.exit(0)
print(g['verdict'])
" 2>/dev/null) || VERDICT="ERR python"
    case "$VERDICT" in
      insufficient_data)
        pass "ope: gate.verdict=insufficient_data (first-class 'not enough decisions yet' state — honesty contract)" ;;
      pass|inconclusive|negative)
        # A non-fresh node may report a real verdict; the assertion is that the
        # gate.verdict key exists and is one of the contract's strings.
        pass "ope: gate.verdict=$VERDICT (contract verdict present)" ;;
      *)
        failit "ope: $VERDICT" ;;
    esac
  fi
fi

# --- summary ---------------------------------------------------------------

echo
if [[ "$fail" -eq 0 ]]; then
  echo "UI SMOKE: ALL PASS"
else
  echo "UI SMOKE: FAILURES — see above"
fi
exit "$fail"
