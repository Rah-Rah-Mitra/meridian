#!/usr/bin/env python3
"""Re-ingest the bench corpus for the Phase-5 gate (schema v3 + geo tags).

Streams corpus.jsonl in batches of 100 to /v1/ingest with a synthetic URL per
title and timestamps spread over the trailing 14 days (so windowed heatmaps
have data). Paces under the public rate limit (5 rps, burst 20).

Usage: p5-ingest.py corpus.jsonl [base-url] [max-docs] [skip-docs]
Env:   MERIDIAN_BEARER_TOKEN (required)
"""

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

corpus = sys.argv[1]
base = sys.argv[2] if len(sys.argv) > 2 else "http://127.0.0.1:8080"
max_docs = int(sys.argv[3]) if len(sys.argv) > 3 else 10**9
skip_docs = int(sys.argv[4]) if len(sys.argv) > 4 else 0
token = os.environ["MERIDIAN_BEARER_TOKEN"]

now = int(time.time())
WEEK2 = 14 * 86_400

def post(batch, attempt=0):
    req = urllib.request.Request(
        f"{base}/v1/ingest",
        data=json.dumps(batch).encode(),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {token}"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        # 429 = pacing fell behind the limiter; 503 = shed; 504 = a slow batch
        # hit the whole-request ceiling (SD-card merge stall) — the write may
        # still have landed, but dedup makes the retry harmless.
        if e.code in (429, 503, 504) and attempt < 8:
            time.sleep(2 + attempt)
            return post(batch, attempt + 1)
        raise
    except urllib.error.URLError:
        if attempt < 8:
            time.sleep(2 + attempt)
            return post(batch, attempt + 1)
        raise

accepted = deduped = sent = 0
batch = []
started = time.time()
with open(corpus, encoding="utf-8") as f:
    for i, line in enumerate(f):
        if i >= max_docs:
            break
        if i < skip_docs:
            continue
        doc = json.loads(line)
        title = doc.get("title") or f"doc-{i}"
        slug = urllib.parse.quote(title.replace(" ", "_"), safe="")
        batch.append(
            {
                "text": doc["body"],
                "title": title,
                "url": f"https://simple.wikipedia.org/wiki/{slug}",
                # Deterministic spread over the trailing 14 days.
                "ts": now - (i * 7919) % WEEK2,
            }
        )
        if len(batch) == 100:
            r = post(batch)
            accepted += r["accepted"]
            deduped += r["deduped"]
            sent += len(batch)
            batch = []
            if sent % 10_000 == 0:
                rate = sent / (time.time() - started)
                print(f"{sent} sent ({rate:.0f} docs/s) accepted={accepted} deduped={deduped}", flush=True)
            time.sleep(0.21)  # ~4.8 rps steady — under the 5 rps limiter
if batch:
    r = post(batch)
    accepted += r["accepted"]
    deduped += r["deduped"]
    sent += len(batch)

print(f"DONE sent={sent} accepted={accepted} deduped={deduped} in {time.time() - started:.0f}s")
