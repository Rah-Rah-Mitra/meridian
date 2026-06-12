#!/usr/bin/env bash
# Privacy smoke test (SPEC §13.4 / §15) — drives real traffic with canary values
# and asserts they appear NOWHERE they shouldn't. Runs in CI (x86 debug build)
# and on-device against the deployed binary.
#
# Asserts:
#   1. canary QUERY string never appears in logs or /metrics
#   2. canary CLIENT IP (X-Forwarded-For) never appears in logs, /metrics, or /data
#   3. no Set-Cookie header on any response
#   4. /metrics carries no per-query/per-IP labels (only bounded route/status)
#   5. bearer token value never appears in logs or /metrics
#
# Usage: deploy/privacy-smoke.sh <path-to-meridiand-binary>

set -euo pipefail

BIN="${1:?usage: privacy-smoke.sh <meridiand>}"
CANARY_Q="CANARYq7f3e9a2zX"
CANARY_IP="198.51.100.77"
TOKEN="smoke-bearer-$$"
PORT=18099
DATA_DIR=$(mktemp -d)
LOG="$DATA_DIR/meridiand.log"

cleanup() {
  kill "$SERVER_PID" 2>/dev/null || true
  wait "$SERVER_PID" 2>/dev/null || true
  rm -rf "$DATA_DIR"
}
trap cleanup EXIT

# searx ENABLED against a dead URL (port 9, discard): fan-outs fail but the
# bandit still chooses+rewards, so the ADR-24 decision log writes REAL rows —
# the /data canary sweep below covers egress.redb non-vacuously.
MERIDIAN_BEARER_TOKEN="$TOKEN" \
MERIDIAN_SEARX__ENABLED=true \
MERIDIAN_SEARX__URL="http://127.0.0.1:9/" \
MERIDIAN_SEARX__DECISION_LOG=true \
MERIDIAN_INDEX__DATA_DIR="$DATA_DIR/data" \
MERIDIAN_MODELS__DIR="${MERIDIAN_MODELS__DIR:-models}" \
MERIDIAN_SERVER__PORT=$PORT \
"$BIN" > "$LOG" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 50); do
  if MERIDIAN_SERVER__PORT=$PORT "$BIN" --healthcheck 2>/dev/null; then break; fi
  sleep 0.2
done
MERIDIAN_SERVER__PORT=$PORT "$BIN" --healthcheck || { echo "FAIL: server never came up"; cat "$LOG"; exit 1; }

BASE="http://127.0.0.1:$PORT"
HDRS="$DATA_DIR/headers.txt"
: > "$HDRS"

# --- drive traffic carrying the canaries ---------------------------------
curl -s -D - -o /dev/null -H "X-Forwarded-For: $CANARY_IP" \
  "$BASE/v1/search?q=$CANARY_Q&scope=local" >> "$HDRS"
curl -s -D - -o /dev/null -H "X-Forwarded-For: $CANARY_IP" \
  "$BASE/v1/search?q=$CANARY_Q+second+try&scope=local" >> "$HDRS"
# Ingest a benign doc (auth path exercised; content is NOT a privacy subject)
curl -s -D - -o /dev/null -X POST -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -H "X-Forwarded-For: $CANARY_IP" \
  "$BASE/v1/ingest" -d '[{"text":"benign smoke document body","title":"smoke"}]' >> "$HDRS"
# Unauthorized attempt (the failure path must not log the canary either)
curl -s -D - -o /dev/null -X POST -H "X-Forwarded-For: $CANARY_IP" \
  -H 'Content-Type: application/json' "$BASE/v1/ingest" -d '[]' >> "$HDRS"
# Rejected fetch (SSRF guard path)
curl -s -D - -o /dev/null -H "Authorization: Bearer $TOKEN" \
  -H "X-Forwarded-For: $CANARY_IP" \
  "$BASE/v1/fetch?url=http://169.254.169.254/$CANARY_Q" >> "$HDRS"
# ADR-24 extended smoke: drive web-scope canary searches through the bandit
# so decision rows exist, then capture the log status (asserted below).
curl -s -o /dev/null -H "X-Forwarded-For: $CANARY_IP" \
  "$BASE/v1/search?q=$CANARY_Q+log+probe&scope=both"
curl -s -o /dev/null -H "X-Forwarded-For: $CANARY_IP" \
  "$BASE/v1/search?q=$CANARY_Q+log+probe+two&scope=both"
sleep 1
DLOG_STATUS=$(curl -s -H "Authorization: Bearer $TOKEN" "$BASE/v1/decision-log")
curl -s "$BASE/metrics" > "$DATA_DIR/metrics.txt"
curl -s "$BASE/healthz" > /dev/null

sleep 0.5
kill "$SERVER_PID"; wait "$SERVER_PID" 2>/dev/null || true

fail=0
check_absent() { # $1=needle $2=haystack-file $3=label
  if grep -qF "$1" "$2"; then
    echo "FAIL: $3 leaked into $2"
    grep -nF "$1" "$2" | head -3
    fail=1
  else
    echo "ok: $3 absent from $(basename "$2")"
  fi
}

# 1+2+5: canaries and token absent from logs
check_absent "$CANARY_Q" "$LOG" "canary query"
check_absent "$CANARY_IP" "$LOG" "canary client IP"
check_absent "$TOKEN" "$LOG" "bearer token"
# canaries absent from metrics
check_absent "$CANARY_Q" "$DATA_DIR/metrics.txt" "canary query (metrics)"
check_absent "$CANARY_IP" "$DATA_DIR/metrics.txt" "canary IP (metrics)"
check_absent "$TOKEN" "$DATA_DIR/metrics.txt" "bearer token (metrics)"
# 2: canary IP absent from everything persisted under /data
if grep -rqF "$CANARY_IP" "$DATA_DIR/data" 2>/dev/null; then
  echo "FAIL: canary IP persisted to disk"; fail=1
else
  echo "ok: canary IP absent from /data"
fi
if grep -rqF "$CANARY_Q" "$DATA_DIR/data" 2>/dev/null; then
  echo "FAIL: canary query persisted to disk"; fail=1
else
  echo "ok: canary query absent from /data"
fi
# ADR-24: the decision log must EXIST and hold rows (else the sweep above is
# vacuous about it) — and those rows are covered by the canary greps.
ROWS=$(printf '%s' "$DLOG_STATUS" | sed -n 's/.*"rows":\([0-9]*\).*/\1/p')
if [ "${ROWS:-0}" -ge 2 ]; then
  echo "ok: decision log holds $ROWS rows — canary sweep covered egress.redb"
else
  echo "FAIL: decision log empty/unreachable (status: $DLOG_STATUS)"; fail=1
fi
# 3: no Set-Cookie anywhere
if grep -qi 'set-cookie' "$HDRS"; then
  echo "FAIL: Set-Cookie emitted"; fail=1
else
  echo "ok: no Set-Cookie"
fi
# 4: metrics labels bounded (no q=, no ip=)
if grep -qE 'q="|ip="|query="' "$DATA_DIR/metrics.txt"; then
  echo "FAIL: high-cardinality user labels in metrics"; fail=1
else
  echo "ok: metrics labels bounded"
fi

if [ "$fail" -eq 0 ]; then
  echo "PRIVACY SMOKE: all assertions green"
else
  echo "PRIVACY SMOKE: FAILURES"; exit 1
fi
