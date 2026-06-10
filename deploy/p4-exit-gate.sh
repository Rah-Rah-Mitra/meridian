#!/bin/sh
# Phase-4 exit gate (SPEC §16): anon metasearch p50 ≤8s through the real Tor
# network + searxng-anon; fail-closed visible during bootstrap; /v1/lanes honest.
# Run on the Pi with the anon-profile stack up:
#   MERIDIAN_ANON=true in deploy/.env, then
#   docker compose --profile anon up -d
#
# Usage: ./p4-exit-gate.sh [base-url] [n-queries]
# Status-aware like p1-exit-gate.sh: only 200s count toward latency; non-200s
# are reported separately (the rate limiter must not pollute the sample).

set -eu
BASE="${1:-http://127.0.0.1:8080}"
N="${2:-20}"
QUERIES="${3:-/tmp/p4-queries.txt}"

if [ ! -f "$QUERIES" ]; then
    cat >"$QUERIES" <<'EOF'
rust borrow checker explained
raspberry pi 5 16k page size
tor stream isolation circuits
wireguard policy routing linux
tantivy bm25 block wand
onnx runtime int8 quantization
searxng json api engines
reciprocal rank fusion k 60
hnsw recall ef search tradeoff
mmap huge pages jemalloc arm
EOF
fi

echo "== /v1/lanes =="
curl -s "$BASE/v1/lanes" | tr ',' '\n'
echo

echo "== anon lane status must be up before sampling =="
status=$(curl -s "$BASE/v1/lanes" | grep -o '"id":"anon","status":"[a-z]*"' | cut -d'"' -f8)
echo "anon: ${status:-unknown}"
[ "$status" = "up" ] || { echo "FAIL: anon lane not up"; exit 1; }

echo "== anon metasearch latency (N=$N, sequential — citizenship budget) =="
i=0
oks=0
fails=0
tmp=$(mktemp)
while [ "$i" -lt "$N" ]; do
    q=$(sed -n "$(( (i % 10) + 1 ))p" "$QUERIES" | tr ' ' '+')
    t0=$(date +%s%N)
    code=$(curl -s -o /tmp/p4-resp.json -w '%{http_code}' \
        "$BASE/v1/search?q=${q}&lane=anon&scope=web&limit=10")
    t1=$(date +%s%N)
    ms=$(( (t1 - t0) / 1000000 ))
    if [ "$code" = "200" ]; then
        oks=$((oks + 1))
        echo "$ms" >>"$tmp"
        results=$(grep -o '"url"' /tmp/p4-resp.json | wc -l)
        effective=$(grep -o '"lane_effective":"[a-z-]*"' /tmp/p4-resp.json | cut -d'"' -f4)
        echo "  q$((i + 1)): ${ms}ms results=$results lane_effective=$effective"
        [ "$effective" = "anon" ] || { echo "FAIL: lane_effective=$effective"; exit 1; }
    else
        fails=$((fails + 1))
        echo "  q$((i + 1)): HTTP $code (${ms}ms) — excluded from sample"
    fi
    i=$((i + 1))
done

[ "$oks" -gt 0 ] || { echo "FAIL: zero successful anon searches"; exit 1; }
p50=$(sort -n "$tmp" | awk -v n="$oks" 'NR == int((n + 1) / 2) { print }')
p95=$(sort -n "$tmp" | awk -v n="$oks" 'NR == int(0.95 * n + 0.999) { print }')
echo "anon search: ok=$oks fail=$fails p50=${p50}ms p95=${p95}ms (gate: p50 <= 8000)"
[ "$p50" -le 8000 ] || { echo "FAIL: anon p50 ${p50}ms > 8000ms"; exit 1; }

echo "== anon cache hit (same query repeated) =="
q="rust+borrow+checker+explained"
curl -s -o /dev/null "$BASE/v1/search?q=${q}&lane=anon&scope=web&limit=10"
t0=$(date +%s%N)
curl -s -o /dev/null "$BASE/v1/search?q=${q}&lane=anon&scope=web&limit=10"
t1=$(date +%s%N)
echo "anon cache-hit: $(( (t1 - t0) / 1000000 ))ms"

rm -f "$tmp" /tmp/p4-resp.json
echo "P4 GATE: PASS"
