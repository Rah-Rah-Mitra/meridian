# 02 — Competitive evidence ledger

Status: REVIEW — proposes, does not decide. As-of: v0.6.0 / `812d3d4` / 2026-06-13.

Per-competitor claim ledgers produced by four research passes against current
official documentation (access date 2026-06-13). Every claim carries a source
URL and a CONFIRMED / INFERRED / UNKNOWN label. The distilled capability
matrix lives in `00-review.md` §3; this file is the backing evidence.

---

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


---

# C2 — Deep-research category ledger: OpenAI Deep Research, Perplexity Sonar, (Gemini skim)

Status: DRAFT — research agent C2. Access date: 2026-06-13. All sources are
current official documentation unless flagged. Comparison anchors from
`01-rebaseline.md`: Meridian ships **extractive-only** answer mode (`best_passage`,
`ce_score` = "relevance, not correctness"), **VoI fetch stopping with explicit
stop reasons** (`search_stopped_because` + `estimated_marginal_gain_remaining`),
evidence/derivation clusters (`diversity=evidence`), JSD vantage divergence vs a
measured noise floor, and NQC+Clarity QPP confidence (explicitly uncalibrated).

Legend: each row is `capability | claim | source URL | quote/paraphrase | status`.
Status: CONFIRMED (read on the cited page this session), CONFIRMED† (primary URL
403'd to the fetcher; quote captured via search excerpt of the primary +
cross-corroboration), INFERRED (my deduction, labeled), UNKNOWN (no evidence found).

---

## 1. OpenAI Deep Research (`o3-deep-research`, `o4-mini-deep-research`, Responses API)

| capability | claim | source URL | quote/paraphrase | status |
|---|---|---|---|---|
| Orchestration | Agentic multi-step research: the model decomposes the task itself, runs iterative tool calls (search → open_page → find_in_page), optionally runs code | https://developers.openai.com/api/docs/guides/deep-research | models "conduct multi-step research"; "find, analyze, and synthesize hundreds of sources to create a comprehensive report at the level of a research analyst" | CONFIRMED |
| Orchestration (data sources) | Must attach ≥1 source: web search, file search over vector stores (max 2), remote MCP servers (search/fetch interface), connectors; code interpreter optional | https://developers.openai.com/api/docs/guides/deep-research | "include at least one data source" | CONFIRMED |
| Citation mechanics | Inline citations as annotations on the **generated** report text: each carries URL, title, and `start_index`/`end_index` character offsets into the output | https://developers.openai.com/api/docs/guides/deep-research | results include "inline citations" with structured annotation objects | CONFIRMED |
| Citation honesty | Citations are span-anchored to *generated* prose, not verbatim-extractive quotes of the source; no doc claim that cited spans are verbatim-grounded. Contrast: Meridian's `best_passage` is the source text itself | https://developers.openai.com/api/docs/guides/deep-research | (absence of any verbatim-grounding claim in the guide) | INFERRED |
| Hallucination caveats | OpenAI's own launch post: can "hallucinate facts in responses or make incorrect inferences" (at a lower rate than ChatGPT models per internal evals); "may struggle with distinguishing authoritative information from rumors" | https://openai.com/index/introducing-deep-research/ | paraphrase per launch post (2025-02-02), corroborated by https://en.wikipedia.org/wiki/ChatGPT_Deep_Research | CONFIRMED† |
| Structured outputs | **Not supported** on the deep-research models (also no function calling, no fine-tuning); output is a report + annotations | https://developers.openai.com/api/docs/models/o3-deep-research | "structured outputs … not supported" (model capability matrix) | CONFIRMED |
| Confidence/uncertainty | No confidence/uncertainty field in the API response. Launch post admits "weakness in confidence calibration, often failing to convey uncertainty accurately" | https://developers.openai.com/api/docs/guides/deep-research ; https://openai.com/index/introducing-deep-research/ | (no such field in response schema); calibration-weakness wording per launch post | CONFIRMED (absence) / CONFIRMED† (caveat) |
| Stopping criteria / transparency | Intermediate steps are exposed (`web_search_call`, `code_interpreter_call`, `mcp_tool_call`, `file_search_call` items with actions "search", "open_page", "find_in_page") — but there is **no stop-reason signal**: the run ends when the model decides or `max_tool_calls` is exhausted, and nothing equivalent to Meridian's `search_stopped_because` / `estimated_marginal_gain_remaining` is emitted | https://developers.openai.com/api/docs/guides/deep-research | output arrays show all research steps; no "why it stopped" field documented | CONFIRMED (steps) / UNKNOWN→absent (stop reason) |
| Cost | o3-deep-research: $10.00/M input, $2.50/M cached, $40.00/M output (batch $5/$20); o4-mini-deep-research: $2/M in, $8/M out (batch $1/$4); web search tool additionally $10.00/1k calls + search content tokens at model rates | https://developers.openai.com/api/docs/models/o3-deep-research ; https://developers.openai.com/api/docs/pricing | exact numbers as listed | CONFIRMED |
| Cost per task | Not published. A single run makes tens of tool calls over hundreds of K tokens → plausibly $1–$10+ per task on o3-deep-research | — | arithmetic over published rates | INFERRED |
| Latency | "can take tens of minutes to complete"; launch post: 5–30 minutes; background mode + webhooks recommended over holding the connection | https://developers.openai.com/api/docs/guides/deep-research | "tens of minutes"; background mode for long runs | CONFIRMED |
| Steerability / budget | `max_tool_calls` is "the primary tool available to you to constrain cost and latency"; plus model choice (o3 vs o4-mini), prompt-side scoping, and choice of data sources | https://developers.openai.com/api/docs/guides/deep-research | quote as given | CONFIRMED |
| Freshness | Live web search at query time; model knowledge cutoff June 1, 2024 (o3-deep-research); 200k context / 100k max output | https://developers.openai.com/api/docs/models/o3-deep-research | "Knowledge cutoff: June 1, 2024" | CONFIRMED |
| Monitoring | Webhook on background-run completion; docs recommend logging tool calls (framed as a prompt-injection/exfiltration mitigation, not a metrics surface) | https://developers.openai.com/api/docs/guides/deep-research | safety guidance: log tool calls, screen returned links, "trusted MCP servers" only | CONFIRMED |
| Privacy / retention / training | API data "is not used to train or improve OpenAI models (unless you explicitly opt in)"; default 30-day abuse-monitoring retention; ZDR available for `/v1/responses` (forces `store=false`) — **but background mode "stores response data to disk for roughly 10 minutes" and is "incompatible with Zero Data Retention (ZDR) requirements"**, and the guide recommends background mode for deep research; live Web Search is "not HIPAA eligible and is not covered by a BAA" | https://developers.openai.com/api/docs/guides/your-data ; https://developers.openai.com/api/docs/guides/deep-research | quotes as given — note the ZDR/background-mode tension is in OpenAI's own docs | CONFIRMED |
| Local deployment | **None.** Closed weights, hosted API only. No on-prem/edge option exists or is hinted at | https://developers.openai.com/api/docs/models/o3-deep-research | (no such offering anywhere in platform docs) | CONFIRMED (absence) |
| Eval claims | Humanity's Last Exam 26.6% (vs ~9% for o1/R1 at the time); GAIA "new state of the art" — both for the ChatGPT deep-research agent, **2025-02-02**, i.e. predating the API snapshots (`-2025-06-26`). Note: the o3-deep-research-2025-06-26 snapshot is marked **Deprecated** on its model page as of 2026-06-13 | https://openai.com/index/introducing-deep-research/ ; https://developers.openai.com/api/docs/models/o3-deep-research | "scores a new high at 26.6% accuracy" (HLE); snapshot "(Deprecated)" | CONFIRMED† (scores) / CONFIRMED (deprecation) |

## 2. Perplexity (Sonar family, `sonar-deep-research`)

| capability | claim | source URL | quote/paraphrase | status |
|---|---|---|---|---|
| Orchestration | `sonar-deep-research`: "conducting exhaustive searches across hundreds of sources, synthesizing expert-level insights"; autonomously searches, reads, and evaluates sources, refining its approach as it gathers information; 128K context | https://docs.perplexity.ai/docs/sonar/models/sonar-deep-research | quotes as given | CONFIRMED |
| Citation mechanics | Response carries `citations` ("URLs of sources used to generate the response") and `search_results` (objects with `title`, `url`, `date`, `last_updated`, `snippet`); inline [n] markers in generated text; **citation tokens are billed** ($2/M) | https://docs.perplexity.ai/api-reference/chat-completions-post ; https://docs.perplexity.ai/docs/sonar/models/sonar-deep-research | field names as given | CONFIRMED |
| Citation honesty | No documented claim that cited statements are verbatim-grounded in the listed URLs; only the source list + snippets are returned, not the grounding span. No hallucination caveat found in the API docs themselves | https://docs.perplexity.ai/api-reference/chat-completions-post | (absence) | UNKNOWN (caveats) / INFERRED (not verbatim-grounded) |
| Structured outputs | Supported: `response_format` with `ResponseFormatJSONSchema` (`json_schema`); docs elsewhere note JSON mode/domain filters gated to "select usage tiers" | https://docs.perplexity.ai/api-reference/chat-completions-post | field names as given | CONFIRMED |
| Confidence/uncertainty | No confidence or uncertainty field documented anywhere in the response schema | https://docs.perplexity.ai/api-reference/chat-completions-post | (absence) | UNKNOWN→absent |
| Stopping criteria / transparency | No stop-reason. Post-hoc accounting only: `usage` reports `num_search_queries`, `citation_tokens`, `reasoning_tokens` and an itemized cost breakdown — *what it spent*, never *why it stopped*. Nothing like `search_stopped_because` | https://docs.perplexity.ai/docs/sonar/models/sonar-deep-research | sample response: 21 search queries, 193,947 reasoning tokens, itemized costs | CONFIRMED (accounting) / CONFIRMED (absence of stop reason) |
| Cost | sonar-deep-research: $2/M input, $8/M output, $2/M citation tokens, $3/M reasoning tokens, $5/1k search queries. (Sonar $1/$1, Sonar Pro $3/$15, Sonar Reasoning Pro $2/$8; non-DR models pay per-request search-context fees $5–$14/1k by tier) | https://docs.perplexity.ai/getting-started/pricing ; https://docs.perplexity.ai/docs/sonar/models/sonar-deep-research | exact numbers as listed | CONFIRMED |
| Cost per task | Docs' own sample run ≈ $0.79 (193,947 reasoning tokens ×$3 + 11,395 output ×$8 + 19,028 citation ×$2 + 21 searches) → typical task O($1), roughly an order cheaper than o3-deep-research | https://docs.perplexity.ai/docs/sonar/models/sonar-deep-research | arithmetic over the documented sample response | INFERRED (from CONFIRMED sample) |
| Latency | Launch post: "completing most research tasks in under 3 minutes" (consumer surface, 2025-02); API latency scales with `reasoning_effort` | https://www.perplexity.ai/hub/blog/introducing-perplexity-deep-research | quote per launch post | CONFIRMED† |
| Steerability / budget | `reasoning_effort` minimal/low/medium/high (default medium; "high" = more citations/deeper insights, more time/tokens); `search_domain_filter` (≤20 domains, allowlist or `-`denylist, root-domain/TLD/path-boundary matching, no protocol); `search_recency_filter` (hour/day/week/month/year); `search_after_date_filter` (MM/DD/YYYY); `web_search_options.search_context_size` low/med/high | https://docs.perplexity.ai/guides/search-domain-filters ; https://docs.perplexity.ai/api-reference/chat-completions-post | "You can add a maximum of 20 domains to the `search_domain_filter` list"; allowlist/denylist "not both simultaneously" | CONFIRMED |
| Freshness | Live search over Perplexity's own index; recency/date filters are first-class request parameters | https://docs.perplexity.ai/api-reference/chat-completions-post | parameters as listed | CONFIRMED |
| Monitoring | Per-response `usage` object with itemized cost breakdown by token type and search queries; billing metadata limited to tokens/model/timestamp/key | https://docs.perplexity.ai/docs/sonar/models/sonar-deep-research ; https://docs.perplexity.ai/docs/resources/privacy-security | "Itemized cost breakdown" in usage | CONFIRMED |
| Privacy / retention / training | "strict Zero Data Retention Policy" — "We do not retain any data sent via the Sonar API"; "absolutely do not use any customer data to train our models"; only billing metadata (tokens, model, timestamp, key) retained; SOC 2 Type II. **Stronger default than OpenAI's 30-day retention** — though ZDR-by-default is a policy promise, not a customer-verifiable property | https://docs.perplexity.ai/docs/resources/privacy-security | quotes as given | CONFIRMED (claims) / INFERRED (comparison) |
| Local deployment | **None.** Hosted API only; no on-prem/edge offering documented | https://docs.perplexity.ai/docs/sonar/models/sonar-deep-research | (absence across docs) | CONFIRMED (absence) |
| Eval claims | Humanity's Last Exam 21.1%; SimpleQA 93.9% — consumer Deep Research launch, **2025-02** ("outperforming Gemini Thinking, o3-mini, o1, DeepSeek-R1"); no newer official scores found for the API model | https://www.perplexity.ai/hub/blog/introducing-perplexity-deep-research | "21.1% on Humanity's Last Exam"; "93.9% accuracy on the SimpleQA benchmark" | CONFIRMED† |

## 3. Gemini Deep Research — brief skim (consumer surface; section conclusions INFERRED)

This is the consumer Gemini app feature, not an API product; rows below quote one
official overview page and everything beyond it is **INFERRED**.

| capability | claim | source URL | quote/paraphrase | status |
|---|---|---|---|---|
| Orchestration | Four explicit stages: Planning → Searching → Reasoning → Reporting; "automatically browse up to hundreds of websites and even your Gmail, Drive and Chat"; "shows its thoughts as it reasons over information gathered iteratively"; powered by Gemini 3 | https://gemini.google/overview/deep-research/ | quotes as given | CONFIRMED |
| Steerability — **the differentiator** | The research **plan is user-editable before execution**: "You're in control of the plan: Gemini presents it to you, and you can refine it" — the only system of the three exposing the decomposition as a reviewable artifact | https://gemini.google/overview/deep-research/ | quote as given | CONFIRMED |
| Latency | "multi-page reports in minutes" | https://gemini.google/overview/deep-research/ | quote as given | CONFIRMED |
| Citations, confidence, stop reasons, retention | Not addressed on this surface; consumer privacy terms apply, not API ZDR | https://gemini.google/overview/deep-research/ | overview page is silent on all four | UNKNOWN |
| Local deployment | None — cloud consumer app (150 countries, 45+ languages, Workspace) | https://gemini.google/overview/deep-research/ | availability list; no API/on-prem mention | CONFIRMED (absence) / INFERRED |

## Sources (12)

1. https://developers.openai.com/api/docs/guides/deep-research — deep research guide (Responses API). Accessed 2026-06-13.
2. https://developers.openai.com/api/docs/models/o3-deep-research — model card (pricing, caps, capability matrix, deprecation). Accessed 2026-06-13.
3. https://developers.openai.com/api/docs/pricing — batch pricing + web search tool pricing. Accessed 2026-06-13.
4. https://developers.openai.com/api/docs/guides/your-data — retention, training-use, ZDR, background-mode caveat. Accessed 2026-06-13.
5. https://openai.com/index/introducing-deep-research/ — launch post (2025-02-02): HLE 26.6%, GAIA SOTA, 5–30 min, limitations. **Direct fetch 403'd; quotes via search excerpts of the page**, corroborated by (6).
6. https://en.wikipedia.org/wiki/ChatGPT_Deep_Research — corroboration of launch-post limitation wording.
7. https://docs.perplexity.ai/docs/sonar/models/sonar-deep-research — model page (pricing, sample usage, capabilities). Accessed 2026-06-13.
8. https://docs.perplexity.ai/api-reference/chat-completions-post — request/response schema (citations, search_results, reasoning_effort, response_format, filters). Accessed 2026-06-13.
9. https://docs.perplexity.ai/guides/search-domain-filters — domain filter semantics. Accessed 2026-06-13.
10. https://docs.perplexity.ai/getting-started/pricing — Sonar family pricing tables. Accessed 2026-06-13.
11. https://docs.perplexity.ai/docs/resources/privacy-security — ZDR policy, no-training claim, SOC 2. Accessed 2026-06-13.
12. https://gemini.google/overview/deep-research/ — consumer overview (plan editing, stages, latency). Accessed 2026-06-13. (Perplexity launch post https://www.perplexity.ai/hub/blog/introducing-perplexity-deep-research also 403'd direct; scores via search excerpts.)

## What an edge appliance can and cannot credibly take from this category

- **CAN: own stop-reason transparency as a category-adversarial row.** Neither OpenAI nor Perplexity emits *why* a research run ended — OpenAI shows the step trace, Perplexity shows the bill. Meridian's `search_stopped_because` + `estimated_marginal_gain_remaining` is a genuine differentiator; the review should phrase it as "the only system in this comparison that states its stopping rule."
- **CAN: query decomposition as a cheap, explainable planner stage.** Gemini proves decomposition is valuable enough to surface as a *user-editable plan*. A Pi-class analogue is heuristic/lexical sub-query expansion feeding the existing RRF+VoI pipeline — no generation needed for the plan to exist, only for prose. Presenting the plan (and letting the operator edit it) is UI work, not model work.
- **CAN: itemized budget accounting.** Perplexity's per-response itemized usage breakdown (`num_search_queries`, reasoning/citation tokens, cost) is directly portable: Meridian already has fetch economics (ADR-26); exposing a per-request "fetches spent / saved vs cap" ledger matches the category's emerging norm and costs nothing.
- **CAN: claim the citation-honesty high ground.** Both APIs cite *generated* prose (span-anchored or [n]-marked); neither claims verbatim grounding, and OpenAI's own launch post concedes hallucinated facts and miscalibrated confidence. Meridian's extractive-only `best_passage` ("relevance, not correctness") cannot fabricate a sentence — the comparison table should make non-fabrication a row, not a footnote.
- **CAN: domain/recency steering parity.** Perplexity's `search_domain_filter` (≤20, allow/deny, TLD/path) and recency/date filters are plain query-routing features, fully implementable on-device; worth a parity check against Meridian's filter surface.
- **CANNOT: synthesis.** "Hundreds of sources → analyst-grade multi-page report" is fundamentally generative, needs a frontier reasoning model, tens of minutes of cloud compute, and $0.5–$10+/task. No Pi-class, extractive, sub-4-second system can or should claim this; the honest framing is "evidence retrieval and passage extraction, not report writing."
- **CANNOT: benchmark on HLE/SimpleQA-style closed-book reasoning.** Those scores (26.6%/21.1% HLE, 93.9% SimpleQA, all 2025-02) measure model reasoning + recall, not retrieval quality; citing them as comparison targets would concede the wrong contest. Meridian's nDCG/alpha-nDCG suites are the right scoreboard for what it is.
- **CANNOT-but-counter: privacy is the one row where edge wins outright.** Perplexity's ZDR and OpenAI's no-training defaults are *policy promises* (and OpenAI's recommended background mode is ZDR-incompatible per its own docs); Meridian's on-device processing is a *physical property*. The review can state this without hedging — but must note Perplexity's ZDR-by-default is a stronger paper posture than OpenAI's 30-day default, so the privacy column needs three distinct values, not vendor-vs-edge binary.


---

# C3 — SearXNG baseline + academic retrieval ledger

Agent: C3 (competitive review, v0.6.0). Access date: 2026-06-13.
Scope: Part 1 frames **vanilla SearXNG** (official docs only) against what Meridian's
15 Rust crates add on top of its two embedded SearXNG sidecars (ADR-11). Part 2
evaluates LEANN, ColBERTv2/PLAID, and Seismic against the measured Meridian baseline
(`01-rebaseline.md` §4): usearch HNSW int8, 1M docs, ~506MB resident, recall@10
0.98 @ ef=128, p99 1.73ms, on a Pi 5 (4× Cortex-A76, 8GB, SD-card storage).

Labels: CONFIRMED = directly supported by the cited official source.
INFERRED = derived (incl. absence-of-feature conclusions and Pi-5 extrapolations).

---

## Part 1 — Vanilla SearXNG vs Meridian's additions

### 1.1 Claim ledger

| capability | claim | source URL | quote/paraphrase | label |
|---|---|---|---|---|
| Result fusion | SearXNG merges engine results by hashing each result, merging duplicates (keep longer content, prefer HTTPS URL), then scoring: `weight *= engines[engine].weight; weight *= len(result['positions']); score += weight / position`, summed over the positions where the result appeared; final order is descending score with category/template grouping (max 8 per group within a 20-slot window) | https://raw.githubusercontent.com/searxng/searxng/master/searx/results.py | "results = sorted(self.main_results_map.values(), key=lambda x: x.score, reverse=True)"; merge keeps longer content: "if len(other.content …) > len(origin.content …)"; score adds `weight/position` per occurrence | CONFIRMED (official source, master @ 2026-06-13) |
| Result fusion | The fusion is purely occurrence/position/engine-weight based — no rank-fusion formula like RRF, no content/lexical/semantic relevance scoring of the result text itself | https://raw.githubusercontent.com/searxng/searxng/master/searx/results.py | Score formula uses only engine weight, occurrence count and positions; no text-similarity term exists in `calculate_score` | INFERRED (from absence in the scoring code) |
| Ranking signals exposed | The only operator-tunable ranking signal per engine is `weight` ("Weighting of the results of this engine"); results carry `score` and `positions` fields | https://docs.searxng.org/dev/engines/enginelib.html | "Weighting of the results of this engine (weight)" | CONFIRMED |
| Ranking signals exposed | No learning-to-rank, no rerankers, no per-query adaptation: ranking knobs are static config (engine weight) only | https://docs.searxng.org/dev/engines/enginelib.html ; https://docs.searxng.org/admin/settings/settings_search.html | Neither settings nor engine docs expose any trained/adaptive ranking component | INFERRED (absence across the official docs surveyed) |
| Engine selection / health | Engine selection is static configuration (categories/engines lists per request); failure handling is reactive suspension: `ban_time_on_fail` default 5s, `max_ban_time_on_fail` default 120s, plus `suspended_times` of 3,600–1,296,000s for specific error classes (CAPTCHA, too-many-requests, …) | https://docs.searxng.org/admin/settings/settings_search.html | "Ban time in seconds after engine errors" (5); "Max ban time in seconds after engine errors" (120); `suspended_times` ranges "3,600 to 1,296,000 seconds depending on the error" | CONFIRMED |
| Engine selection / health | Per-engine `timeout` override exists; `retries` re-issues a failed request "On each retry, SearXNG uses a different proxy and source ip" — but there is no reward-driven or learned engine routing of any kind | https://docs.searxng.org/admin/settings/settings_outgoing.html ; https://docs.searxng.org/dev/engines/enginelib.html | retries: "Number of retry in case of an HTTP error. On each retry, SearXNG uses a different proxy and source ip"; timeout: "Specific timeout for search-engine" | CONFIRMED (mechanisms) / INFERRED (no learned routing — absence) |
| Proxied / Tor support | Outgoing requests can be routed through one or more proxies (round-robin per protocol) and through Tor: `using_tor_proxy` (default false, per-engine overridable); `source_ips` can spread requests over multiple interfaces/CIDRs | https://docs.searxng.org/admin/settings/settings_outgoing.html | "If there are more than one proxy for one protocol (http, https), requests to the engines are distributed in a round-robin fashion"; "Using tor proxy (true) or not (false) for all engines. The default is false and can be overwritten in the engines:" | CONFIRMED |
| JSON API | `GET/POST /search` with `q`, `categories`, `engines`, `language`, `pageno`, `time_range`, `format` (json/csv/rss), `safesearch`; **but the default `formats:` list is `- html` only** — JSON must be explicitly enabled, otherwise the instance returns 403 | https://docs.searxng.org/dev/search_api.html ; https://docs.searxng.org/admin/settings/settings_search.html | "Requesting an unset format will return a 403 Forbidden error. Be aware that many public instances have these formats disabled."; default YAML: `formats:\n  - html` | CONFIRMED |
| Local index | SearXNG maintains **no local document index of its own**. "Offline engines" are connectors to external tools (command-line, SQL/NoSQL DBs, search indexers such as Elasticsearch/Meilisearch/Solr) that the admin installs separately | https://docs.searxng.org/dev/engines/offline_concept.html | "An offline engine is an engine which does not need Internet connection to perform a search and does not use HTTP to communicate"; "If an offline engine depends on an external tool, SearXNG does not install it by default" | CONFIRMED |
| Analytics | Built-in analytics = anonymous **engine** metrics only: `enable_metrics` "Enabled by default. Record various anonymous metrics available at /stats, /stats/errors and /preferences"; optional password-gated OpenMetrics `/metrics` for Prometheus (disabled by default). No query analytics, no trends, no geo, no significance testing | https://docs.searxng.org/admin/settings/settings_general.html | "Enabled by default. Record various anonymous metrics available at `/stats`, `/stats/errors` and `/preferences`"; open_metrics "Disabled by default. Set to a secret password to expose an OpenMetrics API at `/metrics`" | CONFIRMED (metrics) / INFERRED (no richer analytics — absence) |
| Deletion / retention | There is nothing to delete: no accounts, no server-side profile, preferences live in client cookies; whether requests are *logged* is a deployment property of the operator, which is the stated reason to run a private instance. No retention/deletion controls are documented because no user-data store exists | https://docs.searxng.org/own-instance.html | Public-instance users "do not know whether their requests are logged, aggregated, and sent or sold to a third party"; "it does not matter if a public or private instance handles the request, because it is anonymized in both cases" | CONFIRMED (statelessness) / INFERRED (no retention machinery — absence) |
| Privacy defaults | By default SearXNG strips identifying data from upstream requests: no cookies forwarded, "generating a random browser profile for every request", no ads/tracking served, search query hidden from visited result pages (referrer protection); the instance's own IP remains visible upstream unless proxy/Tor is configured | https://docs.searxng.org/own-instance.html | "not sending cookies to external search engines and generating a random browser profile for every request"; "SearXNG can also be configured to use proxy or Tor"; "hiding from the results pages being visited" the referring page and search query | CONFIRMED |
| Privacy defaults | `image_proxy` (proxying image results through the instance) exists but is tied to memory cost and is part of the `public_instance` feature bundle, i.e. not forced on by default for local use | https://docs.searxng.org/admin/settings/settings_server.html | "Allow your instance of SearXNG of being able to proxy images. Uses memory space."; public_instance "allows to enable features specifically for public instances (not needed for local usage)" | CONFIRMED |

### 1.2 What Meridian's 15 crates add on top of the sidecars (repo-side, from `01-rebaseline.md`)

| Vanilla SearXNG behavior (above) | Meridian addition |
|---|---|
| No local index (connectors only) | Local hybrid corpus: usearch HNSW int8 ANN (1M docs, 506MB, 0.98 recall@10, p99 1.73ms) + BM25 (0.48ms p50 @100k), fused with web results |
| Occurrence/position score, no content signals | RRF(k=60) fusion (ADR-09), linear LTR cold-start scorer, domain PageRank prior (ADR-10), CE deep rerank (204ms p50 @ top-20), `diversity=evidence` canonical-cluster reorder (v0.6.0, dominates MMR on alpha-nDCG **and** nDCG) |
| Static engine config + failure bans | ε-greedy bandit over engine arms with top-10 contribution reward (R1/R2), linear-TS contextual policy dark behind DR-OPE gate (ADR-25); shed ladder |
| Engine error counters at /stats only | Geo heatmaps with Getis-Ord Gi* + BH-FDR, EB-shrunk quasi-NB trend z + Kleinberg burst decode (ADR-21/28), per-response NQC+Clarity confidence and JSD divergence vs measured noise floor (ADR-22/23) |
| Raw engine snippets only | Answer mode: Weitzman `pandora_walk` fetch selection + extractive `best_passage` (suite 18: 0.700 vs 0.593), VoI fetch ladder with `search_stopped_because` (ADR-26/29) |
| Stateless, so deletion is N/A by construction | Meridian *is* stateful (corpus, bandit state, analytics rows) and therefore ships ADR-19 deletability + retention as a designed property — a guarantee class vanilla SearXNG never has to (and cannot) make about local corpora |
| JSON `format` disabled by default (403) | Sidecar config must (and does, per ADR-11) enable `json` explicitly — worth re-checking on every SearXNG image bump since a default-reset silently 403s the planner |

---

## Part 2 — Academic / OSS retrieval for constrained hardware

Meridian measured baseline for comparison (Pi 5, Profile R): **1M docs, ~506MB resident,
recall@10 0.98 @ ef=128, p99 1.73ms; embed 42.9k docs/s (potion static embeddings,
no transformer at query time); RSS plateau ~250–261MB in a 3GB cgroup.**

### 2.1 LEANN — low-storage ANN via graph-based selective recomputation

| capability | claim | source URL | quote/paraphrase | label |
|---|---|---|---|---|
| Venue + date | arXiv:2506.08276, v1 2025-06-09, v2 2025-11-25; repo badges it "[MLSys 2026]"; Berkeley Sky Computing Lab | https://arxiv.org/abs/2506.08276 ; https://github.com/yichuan-w/LEANN | "LEANN: A Low-Storage Vector Index"; repo: "[MLSys2026]" + Berkeley Sky Computing Lab affiliation | CONFIRMED |
| Core idea / index size | Stores a pruned proximity graph and **recomputes embeddings at query time** instead of storing them: "up to 50x" smaller than conventional indices, "e.g., 5% of the original data"; repo: "Index 60 million text chunks in just 6GB instead of 201GB" (97% saving); paper Table 1: LEANN 4GB vs HNSW 188GB on RPJ-Wiki 60M chunks | https://arxiv.org/abs/2506.08276 ; https://github.com/yichuan-w/LEANN | Abstract: "uses only a fraction of the storage (e.g., 5% of the original data)" | CONFIRMED |
| Recall / latency | Target 90% recall@3; measured search latency **~2.48s per query on an RTX 4090** (NQ, 60M chunks); baselines: in-memory HNSW 0.03s, IVF-recompute 307.61s. Latency comes from re-running the embedding model (Contriever, 768-d; GTE-small gives 2.3× speedup) along the graph traversal with GPU dynamic batching | https://arxiv.org/abs/2506.08276 (v2 §5, Tables 1–2) | "LEANN achieves approximately 2.48 seconds retrieval on NQ"; hardware: "NVIDIA RTX 4090… 32GB RAM" and an AWS M1 Mac 128GB | CONFIRMED |
| Hardware envelope | CPU-only install exists (`cpu` extra; Linux/macOS/Windows), but acceptable latency depends on fast embedding recomputation — the paper's headline numbers are on a 4090; recomputation is the latency budget by design | https://github.com/yichuan-w/LEANN ; https://arxiv.org/abs/2506.08276 | "CPU-only (Linux): use the `cpu` extra"; dynamic batching "aggregates embedding computations across search hops" for the GPU | CONFIRMED (claims) / INFERRED (CPU latency penalty) |
| Insert / DELETE | Paper §6/App. B: optimized adds (O(M·efC)), batched insertion, and **soft deletes** ("marking nodes as inactive rather than removing them"); repo exposes build/search + `leann watch` file-change detection, but no hardened public delete API is documented in the README | https://arxiv.org/abs/2506.08276 ; https://github.com/yichuan-w/LEANN | Soft deletion = "marking nodes as inactive rather than removing them" | CONFIRMED (paper) / INFERRED (API maturity gap) |
| License / maturity | MIT; ~11.9k stars; v0.3.7 (2026-03); active, research-grade (pre-1.0, MLSys 2026 artifact) | https://github.com/yichuan-w/LEANN | MIT License; "11.9k stars"; v0.3.7 March 2026 | CONFIRMED |
| Vs Meridian | On Pi-5 8GB: LEANN trades storage for per-query transformer inference. Meridian's whole 1M-doc index is 506MB resident (well inside 8GB) and answers at p99 1.73ms with recall@10 0.98; LEANN is ~2.5s **on a 4090** at lower recall target (90%@3), and a Pi-5 CPU recompute of a 768-d transformer across hundreds of graph hops would be orders of magnitude past the latency floor — and past the *whole* deep-mode budget (2173ms p50). LEANN solves a storage problem Meridian does not have | (comparison vs `01-rebaseline.md` §4) | — | INFERRED |

**Verdict: REJECT.** Wrong constraint: it spends query-time compute (Meridian's scarcest
Pi-5 resource) to save storage (a resource Meridian's int8 HNSW already has under control
at 506MB/1M docs). Its soft-delete design is the one idea worth remembering if Meridian
ever needs graph-index deletes beyond ADR-19's current mechanism.

### 2.2 ColBERTv2 / PLAID — late-interaction retrieval

| capability | claim | source URL | quote/paraphrase | label |
|---|---|---|---|---|
| Venue + date | ColBERTv2: NAACL 2022 (arXiv:2112.01488, sub. 2021-12-02, rev. 2022-07-10). PLAID: arXiv:2205.09707 (sub. 2022-05-19), CIKM 2022 per the repo's publication list | https://arxiv.org/abs/2112.01488 ; https://arxiv.org/abs/2205.09707 ; https://github.com/stanford-futuredata/ColBERT | "ColBERTv2: Effective and Efficient Retrieval via Lightweight Late Interaction" (NAACL 2022); repo lists SIGIR'20…CIKM'22…EMNLP'23 | CONFIRMED |
| Index size | Residual compression cuts late-interaction index size "6–10×"; MS MARCO v1 (8.8M passages): vanilla ColBERTv2 24.6 GiB → PLAID 21.6 GiB at 2-bit/dim (1-bit used for the 138M-passage v2) — i.e. **~2.4 GiB per 1M passages**, ~5× Meridian's 506MB/1M, before the model | https://arxiv.org/abs/2112.01488 ; https://ar5iv.labs.arxiv.org/html/2205.09707 (Table 1) | "reducing the space footprint of late interaction models by 6–10×"; "Index Size (GiB): Vanilla 24.6, PLAID 21.6" | CONFIRMED |
| Recall / latency | PLAID, MS MARCO v1 dev, MRR@10 39.8 (vanilla 39.7): GPU (TITAN V) 11.5/20.2/38.4 ms at k=10/100/1000; **CPU 31.5/52.9/101.3 ms** — on a 28-core Xeon Gold 6132 (56 threads). MS MARCO v2 (138.4M): CPU 181.9ms @k=100, GPU OOM @k=1000. Speedups "up to 7× on a GPU and 45× on a CPU" over vanilla ColBERTv2 | https://ar5iv.labs.arxiv.org/html/2205.09707 (Tables 3, 6) ; https://arxiv.org/abs/2205.09707 | "latency of tens of milliseconds on a GPU and tens or just few hundreds of milliseconds on a CPU at large scale" | CONFIRMED |
| Hardware envelope | "a GPU is required for training and indexing"; CPU-only inference is supported (dedicated `conda_env_cpu.yml`); query encoding itself is a BERT-class forward pass per query | https://github.com/stanford-futuredata/ColBERT | "Note that a GPU is required for training and indexing."; "new environment file specifically for CPU-only environments" | CONFIRMED |
| Insert / DELETE | An **IndexUpdater** for adding/removing passages was merged 2023-01-29 explicitly "in beta"; no production-hardening announcement since. Default workflow remains build-once, static index | https://github.com/stanford-futuredata/ColBERT | "(1/29/23) We have merged a new index updater feature… These are in beta so please give us feedback" | CONFIRMED (beta status) / INFERRED (still not production-grade) |
| License / maturity | MIT; ~3.9k stars; strong academic pedigree but last headline feature news 2023; research codebase, not an appliance component | https://github.com/stanford-futuredata/ColBERT | MIT; "3.9k stars" | CONFIRMED |
| Vs Meridian | Per 1M docs: ~2.4 GiB index (on SD card — risk #7 endurance) vs 506MB; CPU latency 31.5ms@k=10 on a 56-thread Xeon ⇒ plausibly 150–400ms+ on 4× Cortex-A76, plus per-query BERT query encoding, vs 1.73ms p99. Quality is the draw (late interaction ≈ CE-grade), but Meridian already buys cross-attention quality where it pays: CE deep rerank 204ms p50 over top-20, gated to deep mode | (comparison vs `01-rebaseline.md` §4) | — | INFERRED |

**Verdict: REJECT.** ~5× the storage on SD, server-class-CPU latencies that don't
transfer to 4 Cortex-A76 cores, GPU-required indexing off-device, and beta-only
add/remove vs ADR-19 deletability. Meridian's CE-rerank-on-top-of-cheap-candidates
already occupies the same quality niche at a fraction of the standing cost.

### 2.3 Seismic — efficient learned-sparse retrieval (added per brief's substitution clause, alongside LEANN)

| capability | claim | source URL | quote/paraphrase | label |
|---|---|---|---|---|
| Venue + date | SIGIR 2024 (arXiv:2404.18812, sub. 2024-04-29; DOI 10.1145/3626772.3657769 = SIGIR '24 proceedings; repo lists SIGIR 2024 + CIKM 2024, ECIR 2025, ECIR 2026 follow-ups) | https://arxiv.org/abs/2404.18812 ; https://github.com/TusKANNy/seismic | "Efficient Inverted Indexes for Approximate Retrieval over Learned Sparse Representations" | CONFIRMED |
| Core idea | Inverted lists organized into "geometrically-cohesive blocks" with summary vectors; "one to two orders of magnitude faster than state-of-the-art inverted index-based solutions" at "sub-millisecond per-query latency" with high recall | https://arxiv.org/abs/2404.18812 | Abstract quotes as cited | CONFIRMED |
| Hardware envelope / index size | Rust with Python bindings, CPU-only design; MS MARCO v1 + SPLADE-v3: **7.9 GB RAM** (vs 24.0 GB for the compared alternative) ⇒ ~0.9 GB per 1M docs, in-RAM | https://github.com/TusKANNy/seismic | "written in Rust with Python bindings"; "Memory (GB): 7.9" vs 24.0 | CONFIRMED |
| Recall / latency | MRR@10 40.27 on MS MARCO at average query time **185µs** (~4× faster than competitors at matched quality) | https://github.com/TusKANNy/seismic | "MRR@10: 40.27"; "AQT (μs): 185" | CONFIRMED |
| Insert / DELETE | **Static index.** Build-then-query API only; no insert/delete/update mechanism documented in repo or paper | https://github.com/TusKANNy/seismic | README API covers building and querying; no update mechanisms described | INFERRED (absence — adversarial row: fails ADR-19-style deletability outright) |
| License / maturity | MIT; Rust 61.4%; v0.4.0 (2026-03-25), 3 releases, ~130 stars; active research line (4 papers) but small community | https://github.com/TusKANNy/seismic | MIT badge; "3 releases", v0.4.0 March 2026 | CONFIRMED |
| Vs Meridian | The only candidate that respects the CPU envelope (Rust, 185µs, ~0.9GB/1M in RAM). But: (a) it retrieves over **learned sparse** embeddings — every query needs a SPLADE-class transformer forward pass, which Meridian deliberately avoids (potion static embeddings, ADR-02 "no neural cost" intent path); (b) 0.9GB/1M RAM vs 506MB total resident pressures the 3GB cgroup; (c) no deletes; (d) MRR 40.27 is single-representation quality Meridian approximates via hybrid RRF + CE rerank | (comparison vs `01-rebaseline.md` §4) | — | INFERRED |

**Verdict: REJECT for v0.x, WATCH.** Engineering profile (Rust, CPU, µs-latency) is the
closest match to Meridian's frontier, but the SPLADE query-encoder dependency violates the
no-transformer-on-the-hot-path budget, the index is static (fails the deletability bar
ADR-19 sets), and RAM/M docs is ~1.8× Meridian's whole footprint. Re-open only if the
Hailo-8L NPU path (C4 ledger) ever makes on-device learned-sparse encoding free — and even
then it must beat hybrid BM25+ANN+CE through a suite gate, not on MS MARCO numbers.

### 2.4 Frontier verdict

Meridian's current stack (int8 HNSW + BM25 + RRF + gated CE) already sits at the right
point of the storage/latency/recall frontier *for this device*: 506MB/1M docs fits RAM
with 6× headroom, 1.73ms p99 leaves the entire latency budget for fetch/rerank/answer
phases, and 0.98 recall@10 leaves ≤2pp on the table that none of the three candidates
recovers without breaking either the latency floor (LEANN, ColBERT/PLAID), the storage
budget (ColBERT/PLAID), the no-query-transformer rule (Seismic, ColBERT, LEANN), or
deletability (Seismic, PLAID; LEANN paper-only). **No adoption recommended.** Portable
ideas worth recording: LEANN's soft-delete graph nodes; Seismic's block/summary pruning
as a possible BM25 accelerator shape if posting lists ever become the bottleneck.

---

## Sources (15)

1. https://docs.searxng.org/own-instance.html — privacy model, statelessness
2. https://docs.searxng.org/dev/search_api.html — Search API, formats, 403 behavior
3. https://docs.searxng.org/admin/settings/settings_search.html — default formats (html only), ban/suspension times
4. https://docs.searxng.org/admin/settings/settings_outgoing.html — proxies, Tor, source_ips, retries
5. https://docs.searxng.org/admin/settings/settings_general.html — enable_metrics, open_metrics
6. https://docs.searxng.org/admin/settings/settings_server.html — image_proxy, public_instance, limiter
7. https://docs.searxng.org/dev/engines/offline_concept.html — no local index; offline-engine connectors
8. https://docs.searxng.org/dev/engines/enginelib.html — engine weight, timeout, tokens
9. https://raw.githubusercontent.com/searxng/searxng/master/searx/results.py — official source: merge + score + ordering
10. https://arxiv.org/abs/2506.08276 — LEANN paper (v2 2025-11-25)
11. https://github.com/yichuan-w/LEANN — LEANN repo (MIT, v0.3.7, MLSys 2026)
12. https://arxiv.org/abs/2112.01488 — ColBERTv2 (NAACL 2022)
13. https://arxiv.org/abs/2205.09707 (+ ar5iv HTML render for Tables 1/3/6) — PLAID (CIKM 2022)
14. https://github.com/stanford-futuredata/ColBERT — ColBERT repo (MIT, IndexUpdater beta)
15. https://arxiv.org/abs/2404.18812 + https://github.com/TusKANNy/seismic — Seismic (SIGIR 2024; MIT, Rust, v0.4.0)


---

# C4 — Hailo-8L NPU feasibility for retrieval workloads (text models)

Status: RESEARCH LEDGER — evidence with per-claim confidence. Access date: 2026-06-13.
Agent: C4. Builds on `01-rebaseline.md` §4 (device facts captured live on the Pi:
HAILO8L 13 TOPS INT8, HailoRT 4.23.0, PCIe Gen2 x1 negotiated ≈400–450 MB/s,
vision-only HEFs on disk, repo has zero Hailo awareness).

Labels: **CONFIRMED** = official Hailo source (hailo.ai, github.com/hailo-ai, Hailo
staff posts on community.hailo.ai). **INFERRED** = community/third-party evidence,
or my derivation from confirmed facts.

**Headline finding (changes the prior):** Hailo Model Zoo **v2.19.0 (tagged
2026-06-01)** added **`all_minilm_l6_v2` sentence-embedding HEFs compiled for
HAILO8L** — an official, downloadable MiniLM-L6 transformer running on exactly our
device class, with published accuracy and FPS. The 2024-era community position
("language models too large for Hailo-8L") is superseded for encoder-class models.

---

## 1. Dataflow Compiler (DFC)

| Claim | Evidence | Confidence |
|---|---|---|
| The Hailo-8/8L toolchain is the **DFC v3.x line**; the v5.x line (current v5.3.0, 2026-04-05) targets Hailo-10H/15 only | Model zoo README (master): "The Hailo-8 and Hailo-8L devices are supported on the Hailo Model Zoo v2.x branch, in combination with the Hailo Dataflow Compiler v3.x branch. The master branch is intended for Hailo-10 and Hailo-15 devices only." https://github.com/hailo-ai/hailo_model_zoo ; hailort README (master): "The `master` branch supports only the Hailo-10 and Hailo-15 device families. For Hailo-8, Hailo-8R, and Hailo-8L devices, please use the `hailo8` branch." https://github.com/hailo-ai/hailort | CONFIRMED |
| **Current DFC for Hailo-8L: v3.34.0**, paired with model zoo v2.19.0 (tag commit 2026-06-01: "Update to version v2.19.0") and badged "HailoRT (optional) 4.24.0"; Python 3.10/3.11/3.12 | README.rst at tag v2.19.0 — badges: "Hailo Dataflow Compiler-3.34.0", "HailoRT (optional)-4.24.0"; release notes: "Upgraded to Dataflow Compiler v3.34.0". https://github.com/hailo-ai/hailo_model_zoo/blob/v2.19.0/README.rst | CONFIRMED |
| **Transformer encoder architectures compile** for Hailo-8L: the zoo ships MiniLM-L6 (6-layer BERT-style encoder, self-attention + LayerNorm + GELU), CLIP/TinyCLIP/SigLIP *text* encoders (up to 85.6M params), ViT classifiers, and Whisper encoder/decoder | v2.19.0 HAILO8L docs: `HAILO8L_sentence_embedding_generation.rst`, `HAILO8L_text_image_retrieval.rst`, `HAILO8L_zero_shot_classification.rst`, `HAILO8L_automatic_speech_recognition.rst` (paths verified via GitHub API at the tag) | CONFIRMED |
| **LayerNorm**: supported (ONNX and TFLite parse paths), with specifically improved quantization | Hailo staff (Omria): "LayerNorm works great in TFLite … The Tensorflow/TFLite parser treats LayerNorm as a proper 'block' even when it gets broken down into smaller operations"; "We have specifically improved how LayerNorm gets quantized". https://community.hailo.ai/t/groupnorm-layernorm-support-in-qat-process/16572 (user in that thread still hit a parse error on a decomposed-TFLite variant — support has rough edges) | CONFIRMED (caveat INFERRED) |
| **GELU**: listed by staff as "GeLU (preview)"; Mish/Hard-Swish/SiLU/PReLU/Tanh/Exp/Sqrt supported; "No automatic operator transformation is available in Hailo DFC" (unsupported ops must be manually replaced or moved to host) | Hailo staff (Omria), supported-operators thread. https://community.hailo.ai/t/supported-operators/5046 | CONFIRMED |
| **Softmax with attention mask**: supported — the MiniLM model script calls `set_input_mask_to_softmax()`, i.e. the attention mask is a first-class HEF input wired into on-chip softmax | `hailo_model_zoo/cfg/alls/generic/all_minilm_l6_v2.alls` at v2.19.0: `set_input_mask_to_softmax()` | CONFIRMED |
| **MatMul (Q·Kᵀ, attn·V)**: supported, with a dedicated quantization correction (`matmul_correction … correction_type=zp_comp_block`) | same `.alls` file: `pre_quantization_optimization(matmul_correction, layers={matmul*}, correction_type=zp_comp_block)` | CONFIRMED |
| **Embedding lookup (vocab gather) does NOT run on-chip**: the zoo's MiniLM parser starts at the `embedding` / `masked_fill` nodes — i.e. the HEF input is the *post-lookup* hidden states (16x8x384 = seq 128 × hidden 384) plus a per-head attention-mask tensor (16x8x1536 = 12 heads × 128 × 128); token/position embedding lookup, pooling and L2-normalize are host-side | `cfg/base/all_minilm.yaml` at v2.19.0: parser `nodes: [[embedding, masked_fill], [last_hidden_state]]`, `input_shape: 16x8x384, 16x8x1536`, `output_shape: 1x128x384` | CONFIRMED |
| **Sequence length is fixed at compile time** (static shapes): zoo MiniLM = **128 tokens**, CLIP text = 77, SigLIP = 64. No dynamic sequence length; a different length means a different HEF | input shapes above + CLIP/SigLIP tables (`1x77x512`, `8x8x768`) in the HAILO8L docs at v2.19.0 | CONFIRMED (shapes) / INFERRED (no-dynamic-shapes generalization, consistent with all zoo entries) |
| **Model size limit**: no published byte threshold. Staff: "The way the compiler maps layers onto the hardware isn't a simple cumulative sum of weights + activations … there isn't a fixed byte-size threshold you can calculate manually beforehand" — use the profiler iteratively. In practice the zoo compiles 85.6M-param text encoders for 8L (multi-context, low FPS) | https://community.hailo.ai/t/hailo-8-ai-accelerator-chip-how-much-sram-does-it-have/19111 (staff: Michael); SigLIP rows in HAILO8L_text_image_retrieval.rst | CONFIRMED |
| **Host requirements: x86 only** — "The DFC is only available on x86 machines because it is a compiler used to create models, but it does not actually run the models"; "On Raspberry Pi devices, we only run the pre-compiled models … we do not use the DFC compiler itself on the RPi." Distributed as a Python `.whl` via the (registration-gated) Developer Zone | Hailo staff, https://community.hailo.ai/t/how-to-install-dataflow-compiler/1276 ; gating also evident at https://hailo.ai/products/hailo-software/hailo-ai-software-suite/ ("Sign in / Sign up is required" for downloads). RAM requirement not in any public source I could capture (the gated DFC user guide carries it); community workflows run it in Docker on commodity x86 ≥16 GB | CONFIRMED (x86-only) / INFERRED (RAM) |
| **Quantization workflow: PTQ with a small calibration set is the default**; QAT exists as an advanced path. MiniLM recipe: `calibration, batch_size=8, calibset_size=64` over an MTEB-ArguAna tfrecord, plus equalization; `optimization_level=0, compression_level=0` (no 4-bit compression used for MiniLM) | `.alls` + `cfg/base/all_minilm.yaml` (`calib_set: …mteb_arguana_val.tfrecord`) at v2.19.0; QAT thread above confirms QAT path exists | CONFIRMED |

## 2. Model zoo — text/NLP entries for HAILO8L

v2.19.0 `docs/public_models/HAILO8L/` is overwhelmingly vision, but text/NLP now exists
(all CONFIRMED from the tag):

| Entry | Models | Published numbers (host: i5-9400, **PCIe Gen3 x4**, room temp) |
|---|---|---|
| **Sentence embedding** (`HAILO8L_sentence_embedding_generation.rst`) | `all_minilm_l6_v2`, `all_minilm_l6_v2_v2a` (10.6M params, 2.9 GOPS, input seq 128 × 384) — precompiled HEF + profiler report downloadable from Hailo S3 | MTEB-style Retrieval@10: 61.7 float → **61.6 on-chip** (base); 96.4 → **94.7** (v2a tool-retrieval eval). **121 FPS @ batch 1, 492 FPS @ batch 8** |
| **Text-image retrieval** (`HAILO8L_text_image_retrieval.rst`) | CLIP RN50/RN50x4 text encoders, `clip_vit_b_16/32_text_encoder` (37.8M params), TinyCLIP text encoders (3–29M), `siglip_b_16` / `siglip2_b_32_256` text encoders (85.6M params, 11 GOPS) | e.g. clip_vit_b_32_text 90.6 → 89.3, **32.1 FPS @ b1 / 99.3 @ b8**; siglip_b_16_text 96.2 → 96.0, **13.5 FPS @ b1 / 36.3 @ b8**; tinyclip_39m_text 94.0 → 94.0, 58.1/198 FPS |
| **ASR** (`HAILO8L_automatic_speech_recognition.rst`) | `whisper_base_5s_encoder` (19.85M) + `whisper_base_5s_no_kqs_decoder` (51.87M), whisper_tiny 10s pair | encoder 41 FPS, decoder 147 FPS (8L) |
| **OCR** | text detection/recognition (PaddleOCR-class) | vision-side, not retrieval-relevant |

- **No cross-encoder / reranker** entry exists in any zoo branch (searched; nothing on
  the forum either — see §5). The zoo's MiniLM is the *bi-encoder* checkpoint.
- The GenAI zoo (LLMs, `hailo_model_zoo_genai`) is **Hailo-10H only**: "Hailo-10H
  module" required; no 8/8L support. https://github.com/hailo-ai/hailo_model_zoo_genai — CONFIRMED.
- Older staff position, for the record (now superseded for encoder models): "The
  language model is too large for Hailo-8L" (re distilBERT, 66M params; the zoo now
  ships 85.6M-param SigLIP text encoders on 8L, so the practical limit is contexts/FPS,
  not a hard wall). https://community.hailo.ai/t/nlp-with-hailo8l/3177 — CONFIRMED (the
  quote) / INFERRED (that v2.19 supersedes it).

## 3. Hailo-8L hardware

| Claim | Evidence | Confidence |
|---|---|---|
| 13 TOPS; **typical power 1.5W**; industrial -40–85°C; x86/ARM hosts | Hailo-8L product page: "13 Tera-Operations Per Second (TOPS)", "typical power consumption of 1.5W". https://hailo.ai/products/ai-accelerators/hailo-8l-ai-accelerator-for-ai-light-applications/ | CONFIRMED |
| **DRAM-free**: "Does not require external memory" — all weights/activations live in on-chip distributed memory; **on-chip memory size is not published** and staff decline to give a byte threshold (profiler-driven, see §1) | product page above + SRAM thread (§1) | CONFIRMED |
| M.2 module: B+M and A+E keys; module interface advertised **"PCIe Gen-3.0, 2-lanes"** — on our Pi 5 it negotiates Gen2 x1 (≈400–450 MB/s; Gen3 x1 ≈900 MB/s is a config.txt knob) | https://hailo.ai/products/ai-accelerators/hailo-8l-m-2-ai-acceleration-module-for-ai-light-applications/ ; Pi-side from 01-rebaseline.md §4 (`lspci -vv`, captured live) | CONFIRMED |
| **INT8 native; 4-bit weight compression available** ("quantizing some larger layers to 4 bit" to fit a single context; DFC `compression_level` knob) | staff in https://community.hailo.ai/t/multi-context-flow/8017 ; `.alls` `compression_level` field | CONFIRMED |
| **Over-budget models → multi-context**: "If a network requires more resources than available, it is split by the Hailo Dataflow Compiler into multiple contexts. During runtime each context is loaded automatically by the HailoRT runtime and the network is executed context by context." Single-context "will typically run at higher FPS"; multi-context recovers throughput "by using a larger batch size" (amortizes context swaps) | Hailo staff (user1232), https://community.hailo.ai/t/multi-context-flow/8017 | CONFIRMED |
| Context switching streams weights/state **over PCIe** each pass (no on-package DRAM to park them, unlike Hailo-10H's LPDDR4) — so multi-context cost scales with *our* Gen2 x1 link, and zoo FPS numbers (measured Gen3 x4) are optimistic for the Pi | community evidence: 3-context custom YOLOv8m-pose 56 FPS vs official 2-context 65 FPS (https://community.hailo.ai/t/how-to-force-2-context-compilation-for-custom-yolov8m-pose-3-context-gives-lower-fps-than-official-2-context-hef/19167); Hailo-10H context-switch thread contrasts 8-series PCIe reload vs 10H LPDDR (https://community.hailo.ai/t/context-switching-latency-on-hailo-10h/19342) | INFERRED (mechanism widely stated on forum; no official bandwidth model published) |
| MiniLM-L6 (10.6M, 2.9 GOPS) at 121/492 FPS vs SigLIP-text (85.6M, 11 GOPS) at 13.5/36 FPS and the b1→b8 ratios (~4x for MiniLM, ~2.7x SigLIP) are consistent with these text encoders compiling **multi-context** (batch amortization is exactly the staff-described multi-context signature) | derived from §2 tables + staff description | INFERRED |

## 4. HailoRT 4.23 integration

| Claim | Evidence | Confidence |
|---|---|---|
| HailoRT = C/C++ user-space library + `pyHailoRT` Python API + CLI + GStreamer element; Linux/Windows; x86 and ARM hosts; "up to 16 Hailo AI Accelerator devices" | hailo8 branch README, https://github.com/hailo-ai/hailort/tree/hailo8 ; suite page https://hailo.ai/products/hailo-software/hailo-ai-software-suite/ ("Multi-Host architecture support – supports both x86 & ARM", "C/C++ and Python API") | CONFIRMED |
| **Async inference and multi-model/scheduler APIs exist in the 4.x C/C++ API**: shipped examples include `async_infer_basic_example`, `async_infer_advanced_example`, `raw_async_streams_*`, `multi_network_vstream_example`, `multi_process_example`, and `switch_network_groups_example` / `switch_network_groups_manually_example` (model-scheduler-driven vs manual network-group switching on one device) | https://github.com/hailo-ai/hailort/tree/hailo8/hailort/libhailort/examples/cpp (listing captured via GitHub API at branch `hailo8`) | CONFIRMED |
| GitHub's latest 8-series release is **v4.23.0 (2025-09-30)** — exactly what the Pi runs; the hailo8 branch CMake still pins firmware 4.23.0. Model zoo v2.19.0 badges **HailoRT 4.24.0**, which is not on GitHub → 4.24 exists in the Developer Zone; v2.19 HEFs are built with DFC 3.34, so **a runtime upgrade may be required to load them on the Pi** (HEF↔HailoRT version coupling; must be verified empirically with one downloaded HEF before planning anything) | release list via GitHub API (v4.23.0 2025-09-30; v5.x thereafter); v2.19.0 README badge | CONFIRMED (versions) / INFERRED (4.24-needed risk) |
| **Rust bindings: nothing official.** Two community crates: (a) `hailort-sys` 0.1.1 — "Raw FFI bindings to the HailoRT C runtime library", crates.io, created 2026-02-27, 208 downloads, repo github.com/quinnjr/hailort-sys; (b) `kadu-v/hailort-rs` — "Safe Rust bindings for Hailo8/8-L … targeting the Hailo-8L AI accelerator on Raspberry Pi 5", 1 star, pushed 2026-02-14, sync vstream API + tokio `spawn_blocking` async wrapper. Both pre-1.0, single-maintainer, essentially unused in the wild | https://crates.io/crates/hailort-sys (API-verified); https://github.com/kadu-v/hailort-rs (README captured) | CONFIRMED (existence/contents) — maturity assessment INFERRED |
| **Python sidecar is the well-trodden fallback**: `python3-hailort` is already installed on the device (01-rebaseline §4); pyHailoRT is an official first-class API | hailo8 README; device dpkg | CONFIRMED |
| Realistic integration for meridian (Rust): own thin FFI over `libhailort.so` (stable C API, MIT) or a localhost pyHailoRT sidecar; treat the community crates as reference code, not dependencies | — | INFERRED (engineering judgment) |

## 5. Prior art

- **Official, on-target**: the model zoo v2.19.0 MiniLM-L6 numbers in §2 are the only
  published BERT-class text-encoder benchmarks for HAILO8L — 121/492 FPS (b1/b8),
  seq 128, quantized retrieval metrics within 0.1–1.7pp of float. CONFIRMED.
  Note the harness: i5-9400 host, PCIe **Gen3 x4** — not a Pi 5 at Gen2 x1.
- **Official, adjacent**: Whisper encoder/decoder HEFs for 8L (§2) prove
  attention-stack compilation is production-supported on this part. CONFIRMED.
- **Community**: pre-v2.19 threads are uniformly "not yet" for NLP — staff "The
  language model is too large for Hailo-8L" (https://community.hailo.ai/t/nlp-with-hailo8l/3177);
  community-shared Whisper HEFs (https://community.hailo.ai/t/hef-files-for-whisper-base-compatible-with-hailort-4-19-0-hailo-8-amd64/19100).
  **No published cross-encoder/reranker on any Hailo part was found** (searched
  forum + web, multiple phrasings). INFERRED (absence of evidence).
- **Academic, different NPU**: arXiv 2606.11257 "Energy-Efficient On-Device RAG on a
  Mobile NPU" (Snapdragon X Elite Hexagon) — "first end-to-end RAG pipeline that runs
  all neural stages — embedding, reranking, and LLM generation — on the … NPU";
  9.1× embedding throughput, 12.3× less energy vs CPU on indexing; 4.0× lower
  end-to-end query latency. https://arxiv.org/abs/2606.11257 — INFERRED relevance
  (validates NPU-offloaded retrieval as a pattern; Hexagon ≠ Hailo dataflow, and
  their LLM stage has no Hailo-8L analog).

## 6. Honest blockers

1. **No cross-encoder exists today** — the zoo entry is the bi-encoder
   `all-MiniLM-L6-v2`. Our reranker (MiniLM-L6-class cross-encoder) is the *same
   backbone* + classification head, so the compile path is proven, but we would be
   the first public deployment: custom ONNX surgery (host-side token+position+segment
   embedding, parser start at the post-embedding node, `set_input_mask_to_softmax`),
   PTQ calibration on (query,passage) pairs, and accuracy re-validation against the
   204ms CPU baseline. Quantized-accuracy risk on a *scoring* head is real and unmeasured.
2. **Fixed sequence length**: 128 tokens in the zoo build. A cross-encoder wants
   query+passage in one window — 128 forces aggressive passage truncation (our
   answer-mode passages are ≤500 chars, ~96–125 tokens, plus query → over budget);
   a seq-256 recompile is possible but costs ~2× compute and may push the model
   into more contexts. Untested territory.
3. **Compiler is x86_64-only and registration-gated** — a separate x86 build box (or
   CI runner) enters the toolchain permanently; HEFs are version-coupled to
   HailoRT, and the current zoo HEFs reference HailoRT 4.24 vs our installed 4.23.
4. **PCIe Gen2 x1 penalty is unquantified**: all official FPS numbers are Gen3 x4. For
   single-context models the I/O per inference is small (~245 KB → ~0.6ms at 400MB/s),
   but if the model compiles multi-context, weight streaming rides the same 400MB/s
   link every frame. Must be measured, not assumed. (Gen3 x1 config.txt knob halves this risk.)
5. **Embedding throughput is the wrong fight**: 492 seq/s (b8, Gen3 x4) vs potion's
   42,900 docs/s on CPU — the NPU transformer embedder is ~2 orders of magnitude
   slower than our static-embedding indexer. It only makes sense as a *quality* play
   (true contextual embeddings) on query-side or low-rate background re-embedding,
   never for bulk indexing.
6. **Weights are baked into the HEF at compile time** on an x86 host — any scheme
   that treats the NPU as a GEMM engine over *our corpus embeddings* (brute-force
   scoring) would need an HEF recompile per index update. Operationally absurd; and
   ANN at 0.45ms p50 leaves nothing to win.
7. ~~Hard "transformers don't compile" blocker~~ — **does not exist** as of model zoo
   v2.19.0; that is the single biggest update over the 2024 community folklore.

## 7. Feasibility verdict matrix

| Workload | Verdict | Evidence |
|---|---|---|
| **Cross-encoder offload** (MiniLM-L6 CE, top-20/batch-4, beat 204ms p50 CPU) | **POSSIBLE-WITH-WORK** | Same backbone ships officially for 8L (121 b1 / 492 b8 FPS ⇒ ~165ms/~41ms compute for 20 pairs *on a Gen3 x4 host*) — §2 CONFIRMED. But: no cross-encoder prior art anywhere (§5), 128-token window vs query+passage (§6.2), custom ONNX+PTQ pipeline on a gated x86-only compiler (§6.1/6.3), Gen2 x1 penalty unmeasured (§6.4). Expected win is "frees 4 CPU cores during rerank" more than raw latency. |
| **Sentence-embedder offload** (transformer embedder) | **SUPPORTED-TODAY** (query-side / trickle) — **BLOCKED** (bulk indexing) | Official precompiled `all_minilm_l6_v2.hef` for HAILO8L, PTQ'd, ~8ms/query b1 — download-and-run modulo the HailoRT 4.23→4.24 check (§4). For indexing it is ~100–350× slower than potion's 42.9k docs/s CPU path (§6.5), so it's a quality upgrade for query embedding only — and meridian currently has no transformer-embedder consumer, so this is an enabler without a requirement. |
| **Brute-force INT8 matmul scoring** (corpus × query on NPU) | **BLOCKED** | Weights compile into the HEF on an x86 host → recompile per index update (§6.6, CONFIRMED toolchain property); on-chip memory unpublished but bounded, large matrices go multi-context and stream over our 400MB/s link (§3); and the CPU baseline being attacked is 0.45ms ANN / 0.48ms BM25 — no deficit exists. |

**Bottom line**: the NPU is real, idle, and *can* run MiniLM-class encoders as of
2026-06-01 — but the only workload with a measured deficit (CE rerank, 204ms) is
exactly the one with zero prior art and the most toolchain work. Cheapest probe:
download `all_minilm_l6_v2.hef`, run `hailortcli run` on the Pi at Gen2 x1 and
Gen3 x1, and measure b1/b8 FPS — that single experiment converts most INFERRED
rows above into device-measured facts before any compiler work is funded.
