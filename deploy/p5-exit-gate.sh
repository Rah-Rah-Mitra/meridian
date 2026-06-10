#!/usr/bin/env bash
# Phase-5 exit gate (SPEC §16): heatmap latency, geo/ts-filtered search, and the
# /v1/forget erasure contract — run against the deployed stack AFTER the corpus
# is re-ingested under index schema v3 (deploy/p5-ingest.py).
#
# Usage: deploy/p5-exit-gate.sh [base-url]
# Env:   MERIDIAN_BEARER_TOKEN (required for the forget drill)

set -euo pipefail

BASE="${1:-http://127.0.0.1:8080}"
N=100
fail=0

echo "== heatmap p50 (gate: ≤150ms; $N requests, res 5, window=all) =="
TIMES=$(mktemp)
for i in $(seq "$N"); do
  curl -s -o /dev/null -w '%{http_code} %{time_total}\n' \
    "$BASE/v1/geo/heatmap?res=5&window=all"
  sleep 0.21
done > "$TIMES"
python3 - "$TIMES" <<'PY' || fail=1
import sys
rows = [l.split() for l in open(sys.argv[1]) if l.strip()]
times = sorted(float(r[1]) * 1000 for r in rows if r[0] == "200")
bad = sum(1 for r in rows if r[0] != "200")
print(f"samples={len(times)} non-200={bad}")
if not times: sys.exit(1)
def pct(p): return times[min(len(times)-1, max(0, int(p/100*len(times))-1))]
print(f"p50={pct(50):.1f}ms p95={pct(95):.1f}ms p99={pct(99):.1f}ms")
ok = pct(50) <= 150.0 and bad == 0
print("GATE heatmap p50<=150ms:", "PASS" if ok else "FAIL")
sys.exit(0 if ok else 1)
PY
rm -f "$TIMES"

echo "== heatmap has geo-tagged cells (gazetteer worked at ingest) =="
CELLS=$(curl -s "$BASE/v1/geo/heatmap?res=5&window=all" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["cells"]))')
echo "res-5 cells: $CELLS"
[[ "$CELLS" -gt 10 ]] && echo "ok: corpus is geo-tagged" || { echo "FAIL: heatmap (near-)empty"; fail=1; }

echo "== geo-filtered search returns and respects the filter =="
# Berlin, 50km — corpus has plenty of Germany articles.
GEO=$(curl -s --get --data-urlencode "q=city" --data-urlencode "scope=local" \
  --data-urlencode "lat=52.52" --data-urlencode "lon=13.405" --data-urlencode "radius_km=50" \
  "$BASE/v1/search")
GEO_N=$(echo "$GEO" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["results"]))')
ALL_N=$(curl -s --get --data-urlencode "q=city" --data-urlencode "scope=local" "$BASE/v1/search" \
  | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["results"]))')
echo "unfiltered=$ALL_N geo-filtered=$GEO_N"
[[ "$GEO_N" -ge 1 && "$ALL_N" -ge "$GEO_N" ]] && echo "ok" || { echo "FAIL: geo filter shape"; fail=1; }

echo "== ts-window search =="
WEEK_N=$(curl -s --get --data-urlencode "q=city" --data-urlencode "scope=local" \
  --data-urlencode "after=$(( $(date +%s) - 7*86400 ))" "$BASE/v1/search" \
  | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["results"]))')
echo "trailing-7d results: $WEEK_N (corpus ts spread over 14d)"
[[ "$WEEK_N" -ge 1 ]] && echo "ok" || { echo "FAIL: ts window"; fail=1; }

echo "== /v1/forget end-to-end (gate: erased + re-ingest refused) =="
: "${MERIDIAN_BEARER_TOKEN:?need bearer for forget drill}"
AUTH=(-H "Authorization: Bearer $MERIDIAN_BEARER_TOKEN")
# Random suffix: tombstones are permanent, so each run needs a fresh canary.
NONCE="$RANDOM$RANDOM"
CANARY_URL="https://gate.invalid/p5-forget-canary-$NONCE"
CANARY_TEXT="The zylkophant grazes only in the Brandenburg lowlands near Berlin canary$NONCE."
ing() {
  curl -s "${AUTH[@]}" -H 'Content-Type: application/json' -X POST "$BASE/v1/ingest" \
    -d "[{\"text\": \"$CANARY_TEXT\", \"title\": \"Zylkophant $NONCE\", \"url\": \"$CANARY_URL\", \"ts\": $(date +%s)}]"
}
# ANN returns nearest neighbors for ANY query, so result COUNT proves nothing:
# the assertion is the canary URL's presence in the result list.
canary_hit() {
  curl -s --get --data-urlencode "q=zylkophant canary$NONCE" --data-urlencode "scope=local" "$BASE/v1/search" \
    | python3 -c "import json,sys; print(int(any('p5-forget-canary-$NONCE' in r['url'] for r in json.load(sys.stdin)['results'])))"
}
R1=$(ing); echo "ingest #1: $R1"
sleep 2
[[ "$(canary_hit)" == 1 ]] && echo "ok: canary searchable" || { echo "FAIL: canary not found pre-forget"; fail=1; }
FORGET=$(curl -s "${AUTH[@]}" -H 'Content-Type: application/json' -X POST "$BASE/v1/forget" \
  -d "{\"url\": \"$CANARY_URL\"}")
echo "forget: $FORGET"
echo "$FORGET" | grep -q '"removed":1' || { echo "FAIL: forget removed != 1"; fail=1; }
sleep 2
[[ "$(canary_hit)" == 0 ]] && echo "ok: erased from results (caches purged)" || { echo "FAIL: still searchable post-forget"; fail=1; }
R2=$(ing); echo "re-ingest: $R2"
echo "$R2" | grep -q '"accepted":0' && echo "ok: re-ingest refused (tombstone)" || { echo "FAIL: tombstone did not refuse"; fail=1; }
sleep 2
[[ "$(canary_hit)" == 0 ]] && echo "ok: still gone after re-ingest attempt" || { echo "FAIL: re-ingest resurrected doc"; fail=1; }

echo "== store sizes under §6.1 caps =="
curl -s "$BASE/metrics" | grep "^meridian_store_bytes" || echo "(sweep runs every 30min — may be empty right after boot)"

echo
[[ "$fail" -eq 0 ]] && echo "P5 GATE: ALL PASS" || echo "P5 GATE: FAILURES — see above"
exit "$fail"
