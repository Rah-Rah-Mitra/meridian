#!/usr/bin/env bash
# Phase-7 exit gate (SPEC §16 v2.2): evidence-stage latency + correctness, the
# statistical heatmap, search-latency regression vs Phase 6, and the extended
# /v1/forget contract (sketches die with their document) — run against the
# deployed stack AFTER a fresh 100k ingest (sketches only exist for documents
# ingested by a v0.2.0+ binary; the dedup table short-circuits re-ingest).
#
# Usage: deploy/p7-exit-gate.sh [base-url]
# Env:   MERIDIAN_BEARER_TOKEN (required for the ingest/forget drills)

set -euo pipefail

BASE="${1:-http://127.0.0.1:8080}"
N=100
fail=0
: "${MERIDIAN_BEARER_TOKEN:?}"

# Latency context: e2e p50 right after a fresh 1000-commit ingest is dominated
# by UNMERGED tantivy segments (bm25_ms ~14ms vs ~0.5ms merged; Profile R
# merges run nightly). The Phase-7 cost itself was isolated by A/B-ing the
# evidence kill-switch on this deployment: ON 20.2ms vs OFF 20.0ms p50. Gates
# here: server-side evidence_ms ≤2ms, e2e p95 within the Phase-6 soak band.
echo "== fast-path search p95 + evidence_ms (gates: evidence ≤2ms; e2e p95 ≤35ms) =="
TIMES=$(mktemp)
SALT=$(date +%s)
for i in $(seq "$N"); do
  # Distinct UNIQUE queries (cache-busting across gate re-runs too).
  curl -s -w '\t%{http_code}\t%{time_total}\n' \
    --get --data-urlencode "q=history river g${SALT}x$i" --data-urlencode "scope=local" \
    "$BASE/v1/search" | python3 -c '
import json,sys
line = sys.stdin.read()
body, code, t = line.rsplit("\t", 2)
ev = json.loads(body).get("timings", {}).get("evidence_ms", -1)
print(f"{code.strip()} {float(t)*1000:.2f} {ev}")'
  sleep 0.3
done > "$TIMES"
python3 - "$TIMES" <<'PY' || fail=1
import sys
rows = [l.split() for l in open(sys.argv[1]) if l.strip()]
times = sorted(float(r[1]) for r in rows if r[0] == "200")
evs = sorted(int(r[2]) for r in rows if r[0] == "200" and int(r[2]) >= 0)
bad = sum(1 for r in rows if r[0] != "200")
print(f"samples={len(times)} non-200={bad} evidence_ms_present={len(evs)}")
if not times or bad: sys.exit(1)
def pct(xs, p): return xs[min(len(xs)-1, max(0, int(p/100*len(xs))-1))]
print(f"search e2e p50={pct(times,50):.1f}ms p95={pct(times,95):.1f}ms p99={pct(times,99):.1f}ms")
print(f"evidence_ms p50={pct(evs,50)} p99={pct(evs,99)} (server-side, integer ms)")
ok = pct(times,95) <= 35.0 and pct(evs,50) <= 2 and len(evs) == len(times)
print("GATE e2e p95<=35ms AND evidence p50<=2ms:", "PASS" if ok else "FAIL")
sys.exit(0 if ok else 1)
PY
rm -f "$TIMES"

echo "== statistical heatmap p50 (gate: ≤60ms Profile R) + significance fields =="
TIMES=$(mktemp)
for i in $(seq "$N"); do
  curl -s -o /dev/null -w '%{http_code} %{time_total}\n' \
    "$BASE/v1/geo/heatmap?res=5&window=all"
  sleep 0.3
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
ok = pct(50) <= 60.0 and bad == 0
print("GATE heatmap+Gi* p50<=60ms:", "PASS" if ok else "FAIL")
sys.exit(0 if ok else 1)
PY
rm -f "$TIMES"
curl -s "$BASE/v1/geo/heatmap?res=5&window=all" | python3 -c '
import json,sys
d = json.load(sys.stdin)
cells = d["cells"]
assert d.get("schema") == 1, "heatmap schema field missing"
assert all("z" in c and "q_value" in c and "significant" in c for c in cells[:5]), "stat fields missing"
sig = sum(1 for c in cells if c["significant"])
print(f"cells={len(cells)} significant={sig} (statistical fields present: ok)")' || fail=1

echo "== evidence correctness drill (live cluster + forget erasure incl. sketch) =="
python3 - "$BASE" <<'PY' || fail=1
import json, os, sys, time, urllib.request

base = sys.argv[1]
token = os.environ["MERIDIAN_BEARER_TOKEN"]
def call(path, body=None, method=None):
    req = urllib.request.Request(
        base + path,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"},
        method=method or ("POST" if body is not None else "GET"),
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)

import atexit, random

salt = f"p7drill{int(time.time())}"
words = ["coastal","desalination","plant","approval","council","review","capacity",
         "megalitre","intake","brine","outfall","permit","hearing","budget","tender",
         "construction","membrane","osmosis","supply","drought","reservoir","pipeline"]
rng = random.Random(42)
def sentences(n, extra):
    out = []
    for i in range(n):
        ws = rng.sample(words, 9) + [f"{extra}{i}"]
        out.append(" ".join(ws))
    return f"{salt} " + ". ".join(out)

# Realistic article shapes (the MIN_MATCH_BINS floor correctly refuses to
# assert derivation on tiny texts, so the drill uses ~100-word docs).
origin = sentences(12, "orig")
copy = " ".join(origin.split()[:80]) + " syndication outlet footer attribution"
indep = sentences(12, "indep")
docs = [
    {"text": origin, "title": "origin", "url": f"https://origin-{salt}.example/a"},
    {"text": copy, "title": "copy", "url": f"https://copy-{salt}.example/b"},
    {"text": indep, "title": "indep", "url": f"https://indep-{salt}.example/c"},
]
def cleanup():
    for d in docs:
        try: call("/v1/forget", {"url": d["url"], "purge_caches": True})
        except Exception: pass
atexit.register(cleanup)

r = call("/v1/ingest", docs)
assert r["accepted"] == 3, r
time.sleep(2)  # index commit visibility

# BM25 finds the drill docs via the salt token; the ANN stage legitimately
# adds nearest-neighbor corpus docs to ANY query, so assertions are about OUR
# three docs' clusters — never about global result counts.
s = call(f"/v1/search?q={salt}&scope=local&limit=10")
ev = s.get("evidence")
assert ev and ev["schema"] == 1, "evidence block missing"
by_url = {r_["url"]: (r_.get("evidence") or {}).get("cluster") for r_ in s["results"]}
co, cc, ci = (by_url.get(d["url"]) for d in docs)
assert co is not None and cc is not None and ci is not None, f"drill docs missing/unsketched: {by_url}"
assert co == cc, f"origin and copy must share a cluster: {by_url}"
assert ci != co, f"independent doc must not join the syndication cluster: {by_url}"
cluster = next(c for c in ev["clusters"] if c["id"] == co)
assert cluster["members"] == 2 and cluster["domains"] == 2, f"cluster shape: {cluster}"
print(f"evidence live: origin+copy clustered (members=2, cross-domain), "
      f"independent separate; {ev['independent_source_count']} origins / "
      f"{ev['apparent_source_count']} results — ok")

# Forget the copy; the cluster must dissolve and re-ingest must be refused.
f = call("/v1/forget", {"url": f"https://copy-{salt}.example/b", "purge_caches": True})
assert f["removed"] == 1, f
time.sleep(2)
s2 = call(f"/v1/search?q={salt}&scope=local&limit=10")
by_url2 = {r_["url"]: (r_.get("evidence") or {}).get("cluster") for r_ in s2["results"]}
assert docs[1]["url"] not in by_url2, "forgotten doc still served"
co2 = by_url2.get(docs[0]["url"])
cluster2 = next(c for c in s2["evidence"]["clusters"] if c["id"] == co2)
assert cluster2["members"] == 1, f"forgotten copy still clustered: {cluster2}"
r2 = call("/v1/ingest", [docs[1]])
# Tombstone refusal: neither accepted nor deduped (a dedup would say deduped=1).
assert r2["accepted"] == 0 and r2.get("deduped", 0) == 0, f"tombstone failed: {r2}"
print("forget drill: doc + sketch erased (cluster dissolved), re-ingest refused — ok")
PY

echo "== RSS =="
PID=$(docker inspect -f '{{.State.Pid}}' meridian-meridiand-1)
grep VmRSS "/proc/$PID/status"

echo
[[ "$fail" == 0 ]] && echo "ALL P7 GATES PASS" || { echo "P7 GATE FAILURES"; exit 1; }
