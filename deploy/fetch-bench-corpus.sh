#!/usr/bin/env bash
# Fetch a permissive Wikipedia slice for meridian-bench's lexical suite
# (SPEC §15.3; WBS 0.7 — implemented as dev tooling rather than in-crate so the
# bench binary needs no HTTP client or XML parser).
#
# Source: Simple English Wikipedia pages-articles dump (CC BY-SA). The enwiki
# "abstract" dumps were discontinued upstream (404 as of 2026-06-10), and full
# enwiki is far too large for the device — simplewiki gives ~250k pages with
# realistic article-length bodies in a ~300MB download. Wikitext is stripped
# crudely; good enough for BM25 latency benching (the Phase-2 quality eval uses
# the operator's corpus, not this).
#
# Usage: deploy/fetch-bench-corpus.sh [out.jsonl] [target_docs] [scratch-dir]

set -euo pipefail

OUT="${1:-bench-scratch/corpus.jsonl}"
TARGET="${2:-100000}"
SCRATCH="${3:-bench-scratch}"
DUMP_URL="https://dumps.wikimedia.org/simplewiki/latest/simplewiki-latest-pages-articles.xml.bz2"
DUMP="$SCRATCH/simplewiki-pages-articles.xml.bz2"

mkdir -p "$SCRATCH" "$(dirname "$OUT")"

if [[ ! -f "$DUMP" ]]; then
  echo ">> downloading simplewiki pages-articles (~300MB)"
  curl -fSL --retry 3 -o "$DUMP.tmp" "$DUMP_URL"
  mv "$DUMP.tmp" "$DUMP"
fi

echo ">> parsing to JSONL (target $TARGET docs)"
python3 - "$DUMP" "$OUT" "$TARGET" <<'PY'
import bz2, json, re, sys, xml.sax

dump, out, target = sys.argv[1], sys.argv[2], int(sys.argv[3])

RE_TEMPLATE = re.compile(r"\{\{[^{}]*\}\}")
RE_TABLE = re.compile(r"\{\|.*?\|\}", re.S)
RE_REF = re.compile(r"<ref[^>]*?/>|<ref[^>]*?>.*?</ref>", re.S | re.I)
RE_COMMENT = re.compile(r"<!--.*?-->", re.S)
RE_TAG = re.compile(r"<[^>]+>")
RE_FILE = re.compile(r"\[\[(?:File|Image|Category):[^\[\]]*\]\]", re.I)
RE_LINK2 = re.compile(r"\[\[[^\[\]|]*\|([^\[\]]*)\]\]")
RE_LINK1 = re.compile(r"\[\[([^\[\]]*)\]\]")
RE_EXTLINK = re.compile(r"\[https?://\S*\s?([^\]]*)\]")
RE_WS = re.compile(r"\s+")

def strip_wikitext(t: str) -> str:
    t = RE_COMMENT.sub(" ", t)
    t = RE_REF.sub(" ", t)
    t = RE_TABLE.sub(" ", t)
    for _ in range(6):  # nested templates, bounded
        t2 = RE_TEMPLATE.sub(" ", t)
        if t2 == t:
            break
        t = t2
    t = RE_FILE.sub(" ", t)
    t = RE_LINK2.sub(r"\1", t)
    t = RE_LINK1.sub(r"\1", t)
    t = RE_EXTLINK.sub(r"\1", t)
    t = RE_TAG.sub(" ", t)
    t = t.replace("'''", "").replace("''", "")
    t = RE_WS.sub(" ", t)
    return t.strip()

class Done(Exception):
    pass

class H(xml.sax.ContentHandler):
    def __init__(self, sink):
        self.sink, self.n = sink, 0
        self.field, self.buf = None, []
        self.title, self.ns, self.text, self.redirect = "", "0", "", False
    def startElement(self, name, attrs):
        if name == "page":
            self.title, self.ns, self.text, self.redirect = "", "0", "", False
        elif name == "redirect":
            self.redirect = True
        elif name in ("title", "ns", "text"):
            self.field, self.buf = name, []
    def characters(self, data):
        if self.field:
            self.buf.append(data)
    def endElement(self, name):
        if name == "title":
            self.title = "".join(self.buf).strip()
        elif name == "ns":
            self.ns = "".join(self.buf).strip()
        elif name == "text":
            self.text = "".join(self.buf)
        elif name == "page":
            if self.ns == "0" and not self.redirect and self.title:
                body = strip_wikitext(self.text)[:5000]
                if len(body) >= 120:
                    self.sink.write(json.dumps(
                        {"title": self.title, "body": body},
                        ensure_ascii=False) + "\n")
                    self.n += 1
                    if self.n % 20000 == 0:
                        print(f"   {self.n} docs", file=sys.stderr)
                    if self.n >= target:
                        raise Done
        if name in ("title", "ns", "text"):
            self.field = None

with open(out, "w", encoding="utf-8") as sink:
    h = H(sink)
    try:
        with bz2.open(dump, "rb") as f:
            xml.sax.parse(f, h)
    except Done:
        pass
print(h.n)
PY

count=$(wc -l < "$OUT")
echo ">> corpus ready: $OUT ($count docs)"
[[ $count -ge $((TARGET / 2)) ]] || { echo "!! got far fewer docs than requested ($count)"; exit 1; }
