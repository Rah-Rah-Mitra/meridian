#!/usr/bin/env bash
# Phase-6 soak (SPEC §16): 10 rps direct + 1 rps anon + continuous ingest.
# Emits one CSV line per minute: ts, rss_mib, temp_c, p99_ms(last min, direct),
# direct_2xx, direct_err, anon_2xx, anon_503, ingest_2xx.
#
# 10 rps sustained exceeds the public 5 rps/IP limiter, so the search load is
# driven from TWO rate keys: half the requests carry X-Forwarded-For (the
# limiter hashes the first hop) — same total load on the engine, honest about
# the limiter's existence.
#
# Usage: p6-soak.sh [duration-secs] [base-url]   (runs until killed if 0)
# Env:   MERIDIAN_BEARER_TOKEN (ingest), SOAK_OUT (csv path, default /tmp/soak.csv)

set -euo pipefail
DUR="${1:-0}"
BASE="${2:-http://127.0.0.1:8080}"
OUT="${SOAK_OUT:-/tmp/soak.csv}"
: "${MERIDIAN_BEARER_TOKEN:?}"

QUERIES=(city river history music mountain country island war language king
  bridge ocean physics painter empire railway forest planet treaty harbor)

echo "ts,rss_mib,temp_c,p99_ms,direct_2xx,direct_err,anon_2xx,anon_503,ingest_2xx" > "$OUT"
WORK=$(mktemp -d)
trap 'kill 0 2>/dev/null; rm -rf "$WORK"' EXIT
START=$(date +%s)

# --- direct search: two workers × 5 rps ---------------------------------------
search_worker() { # $1 = extra curl args (rate key), $2 = stats file
  local i=0
  while :; do
    q=${QUERIES[$((RANDOM % ${#QUERIES[@]}))]}
    # shellcheck disable=SC2086
    curl -s -o /dev/null -w '%{http_code} %{time_total}\n' $1 \
      --get --data-urlencode "q=$q soak$((RANDOM))" --data-urlencode "scope=local" \
      "$BASE/v1/search" >> "$2" 2>/dev/null || echo "000 0" >> "$2"
    i=$((i+1))
    sleep 0.2
  done
}
search_worker "" "$WORK/direct" &
search_worker "-H X-Forwarded-For:198.51.100.7" "$WORK/direct" &

# --- anon search: 1 rps (metasearch via Tor when profile is up) ---------------
( while :; do
    q=${QUERIES[$((RANDOM % ${#QUERIES[@]}))]}
    curl -s -o /dev/null -m 15 -w '%{http_code}\n' \
      -H "X-Forwarded-For:198.51.100.8" \
      --get --data-urlencode "q=$q" --data-urlencode "lane=anon" --data-urlencode "scope=web" \
      "$BASE/v1/search" >> "$WORK/anon" 2>/dev/null || echo 000 >> "$WORK/anon"
    sleep 1
  done ) &

# --- continuous ingest: one small batch every 5s ------------------------------
( n=0
  while :; do
    n=$((n+1))
    BODY="[{\"text\": \"soak document $n about $(printf '%s ' "${QUERIES[@]:$((n%15)):5}") generated during the endurance run\", \"title\": \"soak-$n\", \"url\": \"https://soak.invalid/$n\", \"ts\": $(date +%s)}]"
    curl -s -o /dev/null -w '%{http_code}\n' \
      -H "Authorization: Bearer $MERIDIAN_BEARER_TOKEN" -H 'Content-Type: application/json' \
      -H "X-Forwarded-For:198.51.100.9" \
      -X POST -d "$BODY" "$BASE/v1/ingest" >> "$WORK/ingest" 2>/dev/null || echo 000 >> "$WORK/ingest"
    sleep 5
  done ) &

# --- sampler -------------------------------------------------------------------
while :; do
  sleep 60
  NOW=$(date +%s)
  RSS=$(docker stats --no-stream --format '{{.Name}} {{.MemUsage}}' 2>/dev/null \
    | awk '/meridiand/ {print $2}' | sed 's/MiB.*//;s/GiB.*/e3/' || echo "")
  TEMP=$(awk '{printf "%.1f", $1/1000}' /sys/class/thermal/thermal_zone0/temp 2>/dev/null || echo "")
  P99=$(python3 - "$WORK/direct" <<'PY' 2>/dev/null || echo ""
import sys
rows=[l.split() for l in open(sys.argv[1]) if l.strip()]
t=sorted(float(r[1])*1000 for r in rows if r[0]=="200")
print(f"{t[max(0,int(0.99*len(t))-1)]:.0f}" if t else "")
PY
)
  D2=$(grep -c '^2' "$WORK/direct" 2>/dev/null || echo 0)
  DE=$(grep -cv '^2' "$WORK/direct" 2>/dev/null || echo 0)
  A2=$(grep -c '^2' "$WORK/anon" 2>/dev/null || echo 0)
  A5=$(grep -c '^503' "$WORK/anon" 2>/dev/null || echo 0)
  I2=$(grep -c '^2' "$WORK/ingest" 2>/dev/null || echo 0)
  echo "$NOW,$RSS,$TEMP,$P99,$D2,$DE,$A2,$A5,$I2" >> "$OUT"
  : > "$WORK/direct"; : > "$WORK/anon"; : > "$WORK/ingest"
  [[ "$DUR" -gt 0 && $((NOW - START)) -ge "$DUR" ]] && break
done
echo "soak finished after $(( $(date +%s) - START ))s — $OUT"
