# C1 — Commercial search-API claim ledger: Exa, Parallel, Tavily, Brave

Agent: C1 (competitive review, commercial cloud search APIs). Access date: **2026-06-13**.
Method: official docs/pricing/policy pages only, fetched live; every claim carries a
source URL and a status: **CONFIRMED** (read in the official source on access date),
**INFERRED** (deduced or seen only via secondhand listing of an official page), or
**UNKNOWN** (no evidence found — not guessed). Comparisons keyed to
`../01-rebaseline.md` (v0.6.0 / `812d3d4`).

## Sources (14 primary, all official, accessed 2026-06-13)

| # | URL |
|---|---|
| E1 | https://exa.ai/docs/reference/search |
| E2 | https://exa.ai/pricing |
| E3 | https://exa.ai/docs/websets/overview |
| E4 | https://exa.ai/docs/reference/exa-mcp |
| E5 | https://raw.githubusercontent.com/exa-labs/openapi-spec/refs/heads/master/exa-openapi-spec.yaml (official Exa org) |
| P1 | https://docs.parallel.ai/home |
| P2 | https://docs.parallel.ai/resources/pricing |
| P3 | https://docs.parallel.ai/task-api/guides/access-research-basis |
| T1 | https://docs.tavily.com/documentation/api-reference/endpoint/search |
| T2 | https://docs.tavily.com/documentation/api-credits |
| T3 | https://docs.tavily.com/documentation/mcp |
| B1 | https://brave.com/search/api/ |
| B2 | https://api-dashboard.search.brave.com/app/documentation/web-search/query |
| B3 | https://api-dashboard.search.brave.com/documentation/resources/privacy-notice |

Supplementary official pages also read on 2026-06-13 (cited inline where load-bearing):
`exa.ai/docs/reference/getting-started`, `exa.ai/docs/reference/research/overview`,
`docs.parallel.ai/search/search-quickstart`, `parallel.ai/about`, `docs.tavily.com/faq/faq`.

---

## 1. Exa (docs.exa.ai → exa.ai/docs)

| capability | claim | source | quote/paraphrase | status |
|---|---|---|---|---|
| Index ownership | Own embeddings-first engine "made for AIs" | exa.ai/docs/reference/getting-started | search uses "embeddings-based search and other intelligent methods"; Exa is "a search engine made for AIs" | CONFIRMED |
| Index ownership (crawl scope) | Own crawl/index breadth not quantified in pages read | — | no page-count or crawler claim captured | UNKNOWN |
| Natural-language objectives | NL queries native; `auto` "intelligently selects the best search mode"; types: `neural, fast, auto, deep, deep-reasoning, instant` | E1, E5 | type enum confirmed in OpenAPI spec | CONFIRMED |
| Query rewriting/decomposition | `additionalQueries` "query variations for deep-search variants"; research models "plan by decomposing your instructions into targeted sub-tasks" | E1; exa.ai/docs/reference/research/overview | both quotes verbatim | CONFIRMED |
| Multi-hop research | `deep` "performs in-depth research with synthesis"; `deep-reasoning` "adds more reasoning"; async Research API (being folded into `/search` `deep-reasoning`, deprecation 2026-05-01 noted) | E1; research/overview | "asynchronous, multi-step research tasks that search the web, gather sources, synthesize findings" | CONFIRMED |
| Similar-page discovery | `/findSimilar`: "Find similar links to the link provided. Optionally get contents."; `excludeSourceDomain` flag | E5 | endpoint present in current official OpenAPI spec | CONFIRMED |
| Focused excerpts | `highlights` = "text snippets the LLM identifies as most relevant from each page" (optionally query-guided); `text` verbosity `compact/standard/full` + max chars; `summary` with JSON schema | E1 | verbatim | CONFIRMED |
| Structured outputs + citations | `outputSchema` synthesis on deep search; "Deep Search… multi-step workflows and structured outputs with citations"; Research API returns "validated, parsed JSON… including citations" | E1, E2, research/overview | verbatim | CONFIRMED |
| Confidence/uncertainty | `grounding` block with confidence levels `low/medium/high`; no calibration claim anywhere | E1 | confidence levels listed; semantics undocumented | CONFIRMED (existence only) |
| Freshness/live-fetch | `maxAgeHours`: positive = cache TTL, `0` = "fetches fresh content", `-1` = "always uses cache"; supersedes deprecated `livecrawl` (`always/fallback/never/preferred`) | E1 | caller-controlled cache-vs-livecrawl knob | CONFIRMED |
| Monitoring/change detection | Monitors product "$15/1k requests" "tracking web events at specified cadences"; Websets "schedule recurring searches to keep your webset updated automatically" | E2, E3 | verbatim | CONFIRMED |
| Entity-list "find all" | Websets: e.g. "agtech companies in the US that raised Series A"; "each result is checked against rules you define"; enrichments "extract specific data points (text, numbers, dates, booleans) for every result" | E3 | verification + enrichment phases explicit | CONFIRMED |
| Source-quality estimation | No engine-side source-reliability scoring documented | — | — | UNKNOWN |
| Agent APIs + MCP | Hosted MCP `https://mcp.exa.ai/mcp`; tools `web_search_exa`, `web_fetch_exa`, optional `web_search_advanced_exa` ("full control over category filters, domain restrictions, date ranges…"); local npm option | E4 | verbatim | CONFIRMED |
| Explainability | `score` 0–1 = "similarity between the query/url and the result"; `highlightScores` (cosine); "overall ranking methodology is not detailed" | E5, E1 | one opaque similarity score; no signal breakdown | CONFIRMED (score only) |
| Geo filtering / comparison | `userLocation` = "two-letter ISO country code" (bias, not strict filter); no geo *comparison* feature | E1, E5 | comparison absent from docs read | CONFIRMED filter / UNKNOWN comparison |
| Time filtering | `startPublishedDate`/`endPublishedDate` + `startCrawlDate`/`endCrawlDate` | E1 | published-date AND crawl-date windows | CONFIRMED |
| Privacy posture | Enterprise plans offer "Zero Data Retention"; default API-log retention period not published in pages read; no self-host offering | E2 | ZDR is an enterprise contract option | CONFIRMED (ZDR option) / UNKNOWN (default retention) |
| Pricing | Free 20k req/mo; Search "$7/1k requests"; Deep "$12–15/1k"; Contents "$1/1k pages per content type"; Monitors "$15/1k"; agent effort modes $0.025–$1.00/request | E2 | verbatim | CONFIRMED |
| Latency claims | Search "configurable latency (180ms to 1s)"; `instant` "lowest latency"; `outputSchema` "adds about 2 seconds of synthesis latency" | E2, E1 | verbatim | CONFIRMED |

## 2. Parallel (docs.parallel.ai / parallel.ai)

| capability | claim | source | quote/paraphrase | status |
|---|---|---|---|---|
| Index ownership | No explicit own-index/crawler claim found; about page is vision language ("a new Programmatic Web specifically for AIs") | P1; parallel.ai/about | docs home: no statement on index ownership | UNKNOWN (checked, absent) |
| Natural-language objectives | Search API "takes a natural language objective… replacing multiple keyword searches with a single call" | P1; docs.parallel.ai/search/search-quickstart | core product framing | CONFIRMED |
| Query rewriting/decomposition | Objective + "2-3 diverse keyword queries" in one call; "one round-trip" that is "faster than multi-hop research" | P1 | engine handles expansion server-side | CONFIRMED |
| Multi-hop research | Task API = "multi-hop research with citations"; processors `lite→ultra8x` (~2 to ~20-25 output fields); webhooks recommended for long runs | P1 | verbatim | CONFIRMED |
| Similar-page discovery | Not offered in any documented API | — | — | UNKNOWN |
| Focused excerpts | Search returns "LLM-optimized excerpts (pre-compressed, citation-aware)"; Extract API: ≤20 URLs + `target_content` objective → "clean markdown", handles "JavaScript-rendered pages and PDFs" | P1 | verbatim | CONFIRMED |
| Structured outputs + citations | Task enrichment takes JSON schemas, returns fields with "per-field citations"; `result.output.basis` = "per-field citations + reasoning" | P1, P3 | verbatim | CONFIRMED |
| Confidence/uncertainty | Basis confidence per output field, levels High/Medium/Low with evidence rubric ("Strong evidence from multiple authoritative sources…"); "All processors include a confidence rating for each output field"; calibration claimed in official blog title "Introducing Basis with Calibrated Confidences" | P3; parallel.ai/blog/introducing-basis-with-calibrated-confidences | docs page read does not itself say "calibrated"; blog seen via search listing only | CONFIRMED (levels) / INFERRED (calibration claim) |
| Freshness/live-fetch | `freshness` request parameter exists (listed, not detailed in pages read); Search positioned as "real-time web search" | docs.parallel.ai/search/search-quickstart, P1 | parameter named; semantics not captured | CONFIRMED (exists) / UNKNOWN (semantics) |
| Monitoring/change detection | Monitor API: NL query + frequency "1h–30d", events via webhook or polling; `lite`/`base` processors | P1 | "continuous web tracking with scheduled change detection" | CONFIRMED |
| Entity-list "find all" | FindAll API (beta): objective + match conditions → verified candidates; "evaluates every result against your match conditions"; processors preview/base/core/pro | P1 | verbatim | CONFIRMED |
| Source-quality estimation | Only indirectly: confidence rubric distinguishes "authoritative" vs "less reliable sources" per field | P3 | embedded in confidence definitions, not a standalone score | CONFIRMED (rubric wording only) |
| Agent APIs + MCP | MCP for Claude Code, Codex, Cursor, VS Code — "Free. No account, no API key"; webhooks for long tasks; Entity Search synchronous "no polling" | P1 | verbatim | CONFIRMED |
| Explainability | Basis = field + citations (URLs + excerpts) + reasoning + confidence; beta per-array-element basis (`parallel-beta: field-basis-2025-11-25`, dot notation `key_executives.0`) | P3 | answer-level provenance, not ranking-signal exposure | CONFIRMED |
| Geo filtering / comparison | No geo parameter captured in pages read; no comparison feature | — | `source_policy` governs domains, not geography | UNKNOWN |
| Time filtering | Only the undetailed `freshness` parameter; no date-range filter captured | docs.parallel.ai/search/search-quickstart | — | UNKNOWN (beyond `freshness` name) |
| Privacy posture | SOC 2 badge + Trust Center (trust.parallel.ai); query-log retention / ZDR not published in pages read; no self-host | parallel.ai/about | — | CONFIRMED (SOC 2) / UNKNOWN (retention, ZDR) |
| Pricing | Search "$5 per 1,000 requests (default 10 results)", "$0.001" per extra; Extract "$1 per 1,000 URLs"; Task lite $5 → base $10 → core $25 → pro $100 → ultra $300 per 1k; FindAll "$0.10 fixed" (preview) up to "$10.00 + $1.00 per match" (pro); Monitor $3–$10/1k executions | P2 | verbatim | CONFIRMED |
| Latency claims | Search: "one round-trip"; Task: lite "10-60s" → ultra "5-25min" (pricing table) / "up to ~2hr" for ultra tiers (home); "-fast" variants same cost, lower latency; BrowseComp claim: "48% accuracy vs GPT-4's 1% browsing" | P2, P1, docs.parallel.ai/search/search-quickstart | verbatim | CONFIRMED |

## 3. Tavily (docs.tavily.com)

| capability | claim | source | quote/paraphrase | status |
|---|---|---|---|---|
| Index ownership | Not stated; FAQ silent on own-index vs aggregation | docs.tavily.com/faq/faq | "The FAQ does not specify whether Tavily owns its own search index or aggregates" | UNKNOWN (checked, absent) |
| Natural-language objectives | `query` is an NL question (doc example "who is Leo Messi?"); `auto_parameters` configures the call "based on query intent" | T1 | NL in, but no objective/decomposition contract | CONFIRMED (basic) |
| Query rewriting/decomposition | Closest is `auto_parameters` (auto-tunes params, 2 credits) — parameter selection, not query rewriting; no decomposition documented | T1 | — | UNKNOWN (rewriting); CONFIRMED (`auto_parameters`) |
| Multi-hop research | Research tasks exist, priced dynamically: "4-110 credits (model=mini) or 15-250 credits (model=pro)" per request | T2 | priced per task; mechanics not detailed in pages read | CONFIRMED (exists) |
| Similar-page discovery | Not offered | — | — | UNKNOWN |
| Focused excerpts | `chunks_per_source` 1–3 "short content snippets (maximum 500 characters each)" per source (advanced depth); `include_raw_content` as markdown/text; Extract endpoint | T1 | verbatim | CONFIRMED |
| Structured outputs + citations | `include_answer`: "`basic` or `true` returns a quick answer. `advanced` returns a more detailed answer." — no JSON-schema output, no per-claim citations documented for it | T1 | answer string, not structured/cited object | CONFIRMED (answer) / UNKNOWN (citations/schema) |
| Confidence/uncertainty | None; only per-result relevance `score` | T1 | no uncertainty field documented | UNKNOWN (absent) |
| Freshness/live-fetch | `topic: news` "for real-time updates"; no cache-vs-livecrawl control documented | T1 | — | CONFIRMED (topic) / UNKNOWN (live-fetch policy) |
| Monitoring/change detection | Not offered in documented APIs | — | — | UNKNOWN |
| Entity-list "find all" | Not offered | — | — | UNKNOWN |
| Source-quality estimation | Not documented | — | — | UNKNOWN |
| Agent APIs + MCP | Hosted MCP `https://mcp.tavily.com/mcp/` (OAuth supported) exposing `tavily-search` + `tavily-extract`; local npx/git install option | T3 | verbatim | CONFIRMED |
| Explainability | `score` (float): "The relevance score of the search result" — semantics undocumented; `response_time` returned | T1 | one opaque score | CONFIRMED (score only) |
| Geo filtering / comparison | `country` "boosts results from specified countries (general topic only)" — a boost, not a filter; no comparison feature | T1 | verbatim | CONFIRMED (boost) / UNKNOWN (comparison) |
| Time filtering | `time_range` (`day/week/month/year` or `d/w/m/y`); `start_date`/`end_date` (YYYY-MM-DD) | T1 | verbatim | CONFIRMED |
| Privacy posture | FAQ: "SOC 2 certified, zero data retention, and built to handle high-volume workloads"; details deferred to privacy policy; no self-host | docs.tavily.com/faq/faq | blanket ZDR claim, no mechanism published in pages read | CONFIRMED (claim as stated) |
| Pricing | "1,000 free API Credits every month"; search 1 credit (basic) / 2 (advanced); extract 1 credit per 5 URLs (2 advanced); map 1 credit/10 pages (2 with NL instructions); crawl = map + extract; PAYG "$0.008 per credit"; plans $30/mo (4k) → $500/mo (100k), $0.0075→$0.005/credit | T2 | verbatim | CONFIRMED |
| Latency claims | Qualitative depth ladder: `ultra-fast` "minimizes latency", `fast`, `basic`, `advanced` "highest relevance, increased latency"; no millisecond numbers published | T1 | verbatim | CONFIRMED (qualitative only) |

## 4. Brave Search API (brave.com/search/api + api-dashboard docs)

| capability | claim | source | quote/paraphrase | status |
|---|---|---|---|---|
| Index ownership | Independent index: "not a scraper that simply uses bots to query Google or Bing"; own crawler + Web Discovery Project; "over 30 billion pages, kept fresh by over 100 million page updates every day" | B1 | strongest own-index claim of the four | CONFIRMED |
| Natural-language objectives | Classic `q=` keyword API with operators (`site:`, `filetype:`, quotes, minus); no objective contract | B2 | traditional search semantics | CONFIRMED (absent by design) |
| Query rewriting/decomposition | Not documented | — | — | UNKNOWN |
| Multi-hop research | Not offered; Answers/LLM Context are single-shot grounding, not multi-hop | B1 | — | UNKNOWN (absent) |
| Similar-page discovery | Not offered | — | — | UNKNOWN |
| Focused excerpts | `extra_snippets` "provides up to 5 additional excerpts per search result"; dedicated **LLM Context** endpoint for AI grounding | B2, B1 | verbatim | CONFIRMED |
| Structured outputs + citations | Answers (summarizer) endpoint exists, priced per token; citation format/JSON schema not captured in pages read | B1 | — | CONFIRMED (endpoint) / UNKNOWN (citation detail) |
| Confidence/uncertainty | None documented | — | — | UNKNOWN |
| Freshness/live-fetch | Index-side freshness: "over 100 million page updates every day"; no caller live-fetch control | B1 | freshness is the index's job, not a request knob | CONFIRMED |
| Monitoring/change detection | Not offered | — | — | UNKNOWN |
| Entity-list "find all" | Not offered | — | — | UNKNOWN |
| Source-quality estimation | Not engine-exposed; Goggles let the *operator* impose quality policy | B1 | user-defined, not engine-estimated | UNKNOWN (engine-side) |
| Agent APIs + MCP | LLM Context + Answers endpoints marketed for AI grounding; official MCP server not seen in pages read (known to exist via third-party listings) | B1 | — | CONFIRMED (grounding endpoints) / INFERRED (MCP) |
| Explainability | No scores/signals exposed; **Goggles** = "apply custom re-ranking on top of search results", "discard domains or re-rank results" — operator-side control rather than engine transparency | B2, B1 | unique operator-recourse mechanism | CONFIRMED (Goggles) / UNKNOWN (scores) |
| Geo filtering / comparison | `country` — "target results from specific countries using 2-character country codes"; plus `search_lang`/`ui_lang`; no comparison feature | B2 | true country targeting on web results | CONFIRMED filter / UNKNOWN comparison |
| Time filtering | `freshness`: `pd` (24h), `pw` (7d), `pm` (31d), `py` (1y), or custom date ranges | B2 | verbatim | CONFIRMED |
| Privacy posture | "Brave does not collect any identifiers that can link a search query to an individual or their devices"; API query records "retained for a maximum of 90 days" (billing/troubleshooting); "Zero Data Retention (Enterprise clients), subject to Brave's legal obligations"; SOC 2 Type II; positions itself as "a conduit"; no self-host | B3, B1 | most explicit retention numbers of the four | CONFIRMED |
| Pricing | Search "$5 per 1,000 requests" (+$5 monthly free credits); Answers "$4 per 1,000 requests + $5 per million tokens"; enterprise custom; Search tier "50 queries per second" | B1 | verbatim | CONFIRMED |
| Latency claims | "Market-leading low latencies" — no published numbers | B1 | qualitative only | CONFIRMED (qualitative) |

---

## Sharpest contrasts vs Meridian (per `../01-rebaseline.md`)

- **Uncertainty honesty is Meridian's moat, calibration is theirs to claim.** Parallel ships per-field "calibrated" Low/Med/High confidence (P3) and Exa a `grounding` level — neither publishes calibration evidence or score semantics. Meridian ships NQC+Clarity *explicitly labeled uncalibrated* plus JSD-vs-measured-noise-floor (rebaseline S6) and has a public kill record for conformal bands (suite 16, §3) — no vendor documents falsified methods.
- **Fetch economics exposed vs sold.** Vendors monetize depth as opaque tiers (Exa $7→$15/1k; Parallel lite $5→ultra $300/1k). Meridian exposes the decision itself: VoI ladder with `search_stopped_because` and `estimated_marginal_gain_remaining` (S7). Nobody else shows the stopping rationale.
- **NL objectives + decomposition is Meridian's widest confirmed gap.** Parallel's objective→"2-3 diverse keyword queries" one-round-trip and Exa's `additionalQueries`/deep decomposition are table stakes there; Meridian's intent is a 103-line four-class lexical heuristic (R3) and fusion is fixed RRF k=60 (R5).
- **Entity-list find-all + monitoring are whole product categories Meridian lacks.** Exa Websets (criteria verification + enrichment + scheduled refresh) and Parallel FindAll/Monitor have no Meridian analog; Meridian trends are aggregate movers, not per-objective subscriptions (S1 is orthogonal).
- **Web-scope geo/time filtering: every vendor does what Meridian refuses.** Brave `country`+`freshness`, Tavily `country`(boost)+`time_range`, Exa `userLocation`+date windows apply to *web* results; Meridian returns `400`/`degraded` for geo on `scope=web` (R8) and drops ANN under filters (R9). Conversely, none of them offer geographic *comparison* — Meridian's compare lanes + Gi\*/EB-z geo analytics (S2) have zero commercial counterpart in these docs.
- **Privacy: architectural vs contractual.** Brave is the strongest cloud story (no query-individual linkage, ≤90-day logs, enterprise ZDR — B3); Tavily/Exa assert ZDR as claim/option. Meridian's anonymous lane never updates shared state *by construction* (R4) and runs fully on-prem — a posture no vendor offers at any price; none offer self-hosting.
- **Explainability: one similarity float (Exa/Tavily) or nothing (Brave/Parallel-search) vs Meridian's per-result honesty labels, degraded flags, divergence blocks, and evidence-cluster diversity (S3, S6).** Brave Goggles is the only vendor mechanism giving operators reranking recourse — a UX idea worth studying for Meridian's operator policy story.
- **Structured multi-hop answers with per-field citations (Exa research/deep `outputSchema`, Parallel Task basis with per-element citations) dwarf Meridian's single ≤500-char extractive `best_passage` (S5)** — the rebaseline's `ce_score` "relevance, not correctness" honesty is better epistemics, but the output contract is a generation behind.
