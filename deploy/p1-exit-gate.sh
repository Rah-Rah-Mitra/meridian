#!/usr/bin/env bash
# Phase-1 exit gate (SPEC §16): run against the deployed compose stack AFTER the
# corpus is ingested. Measures end-to-end search p50/p99, RSS, disk, and repeats
# the privacy canary asserts against the real container.
#
# Usage: deploy/p1-exit-gate.sh [base-url] [queries-file]

set -euo pipefail

BASE="${1:-http://127.0.0.1:8080}"
QUERIES="${2:-/tmp/p1-queries.txt}"
N=200

if [[ ! -f "$QUERIES" ]]; then
  echo "!! $QUERIES missing — generate with terms sampled from the corpus"
  exit 1
fi

echo "== doc count =="
curl -s "$BASE/healthz" >/dev/null || { echo "server down"; exit 1; }

echo "== search latency ($N local queries) =="
TIMES=$(mktemp)
head -n "$N" "$QUERIES" | while IFS= read -r q; do
  curl -s -o /dev/null -w '%{time_total}\n' \
    --get --data-urlencode "q=$q" --data-urlencode "scope=local" \
    "$BASE/v1/search"
done > "$TIMES"
python3 - "$TIMES" <<'PY'
import sys
times = sorted(float(l) * 1000 for l in open(sys.argv[1]) if l.strip())
if not times:
    print("no samples"); sys.exit(1)
def pct(p):
    return times[min(len(times) - 1, max(0, int(p / 100 * len(times)) - 1))]
print(f"samples={len(times)} p50={pct(50):.1f}ms p95={pct(95):.1f}ms p99={pct(99):.1f}ms")
gate = pct(50) < 50.0
print("GATE p50<50ms:", "PASS" if gate else "FAIL")
sys.exit(0 if gate else 1)
PY
rm -f "$TIMES"

echo "== privacy canaries against the live container =="
CANARY_Q="CANARYx91b4e7dQ"
CANARY_IP="198.51.100.88"
curl -s -o /dev/null -H "X-Forwarded-For: $CANARY_IP" \
  --get --data-urlencode "q=$CANARY_Q" --data-urlencode "scope=local" "$BASE/v1/search"
HDRS=$(curl -s -D - -o /dev/null --get --data-urlencode "q=headers check" "$BASE/v1/search?scope=local" || true)
sleep 1
LOGS=$(docker compose -f "$(dirname "$0")/compose.yaml" logs meridiand 2>/dev/null | tail -500)
METRICS=$(curl -s "$BASE/metrics")
fail=0
echo "$LOGS" | grep -qF "$CANARY_Q" && { echo "FAIL: canary query in container logs"; fail=1; } || echo "ok: no canary query in logs"
echo "$LOGS" | grep -qF "$CANARY_IP" && { echo "FAIL: canary IP in container logs"; fail=1; } || echo "ok: no canary IP in logs"
echo "$METRICS" | grep -qF "$CANARY_Q" && { echo "FAIL: canary query in metrics"; fail=1; } || echo "ok: no canary query in metrics"
echo "$HDRS" | grep -qi 'set-cookie' && { echo "FAIL: Set-Cookie"; fail=1; } || echo "ok: no Set-Cookie"

echo "== resources =="
docker stats --no-stream --format '{{.Name}} RSS={{.MemUsage}} CPU={{.CPUPerc}}' | grep -E 'meridian|searxng' || true
docker system df -v 2>/dev/null | grep meridian-data || true

exit $fail
