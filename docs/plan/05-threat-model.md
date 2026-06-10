# 05 — Threat Model (STRIDE-lite)

> SPEC §1.6 / §13.3. Scope: API surface, fetcher, three egress lanes, plus
> Meridian's own storage/telemetry (the §13.4 "protect users from Meridian itself"
> posture). Assets: operator queries & corpus, end-user queries & IPs, secrets
> (bearer, wg source config, API keys, rate-limit salt), the Tor anonymity property,
> the device itself.

## 1. API surface (`meridiand` HTTP)

| STRIDE | Threat | Mitigation (phase) |
|---|---|---|
| S | Unauthenticated ingest/forget poisons or erases the index | Bearer auth on all mutating endpoints; localhost bind default; auth+TLS gate for exposure (P1) |
| S | Node abused as an open Tor search proxy | Anon search bearer-gated by default (P4) |
| T | Malicious batch ingest floods disk | Batch ≤100, body limit 1MB, free-disk pause gate, per-IP rate limit (P1) |
| R | Operator cannot prove what the node did/didn't store | Aggregate-only metrics + documented retention map + `/v1/forget` audit count (P5) |
| I | Query text / client IPs leak via logs, metrics, or errors | Redaction layer, `Redacted<T>`, hashed-IP rate keys, RFC7807 without echo of `q`, privacy smoke test in CI (P1) |
| I | `/metrics` cardinality leaks per-user behavior | Labels bounded to lane×route×status×latency-bucket; no exemplars (P1) |
| D | Slowloris/concurrency exhaustion on a 4-core device | tower timeouts, concurrency 8 + 50ms queue → 429, governor 5 rps/IP (P1) |
| D | Anon lane's slow circuits starve the shared pool | Separate anon concurrency budget (2) + circuit cap 6 (P4) |
| E | Strict serde bypass / parser differentials | `deny_unknown_fields`, limits, fuzz the query-param parser in P6 hardening |

## 2. Fetcher (`meridian-fetch`)

| STRIDE | Threat | Mitigation |
|---|---|---|
| S/I | **SSRF** into RFC1918/loopback/link-local/metadata, or into the compose `back` network (searxng, Arti SOCKS port!) | §13.1 guard on EVERY lane: scheme allowlist, resolve-all-then-pin (no TOCTOU), deny-listed ranges incl. 100.64/10 + multicast; redirects ≤3 re-validated; **container-network CIDRs added to the deny list** (P1) |
| S | DNS rebinding between validation and connect | Pin connection to the validated IP on direct/region; anon resolves inside Tor with URL-level policy instead (P1/P4) |
| T | Decompression bombs / infinite bodies | 5MB streamed cap, total timeout per lane, content-length + actual-bytes double check (P1) |
| T | Malicious HTML breaks the extractor | Extraction bake-off includes a malformed-input fixture set; extractor runs on rayon with a stage deadline; panics isolated per task (P1) |
| I | Fetched content treated as instructions downstream | Documented invariant: fetched content is DATA; any future LLM consumer must treat it as untrusted (now) |
| D | One hostile domain absorbs the fetch budget | Per-domain token buckets, global across lanes; frontier fairness (P1) |
| E | robots.txt bypass via lane switching | robots cache + enforcement sit BELOW lane selection (one chokepoint) (P1) |

## 3. Egress lanes

### 3.1 `direct`
Baseline web client risks only: honest UA with contact URL, rustls, no cookies, no
Referer. Hedging allowed here alone — bounded by per-domain budgets.

### 3.2 `region:<id>` (WireGuard)
| Threat | Mitigation |
|---|---|
| Routing leak: packets leave eth0 with the wg-lane label | Bring-up verification (IP-echo + optional GeoLite2 region assert) → Degraded on mismatch; periodic re-verify; ip-rule recipes are operator-reviewed, never auto-applied |
| wg key exposure via Meridian | meridiand never holds the private key — only iface name/source IP; keys live in /etc/wireguard (operator domain) |
| Lane misuse for evasion | Posture documented: region lanes observe lawful region-varied results; robots + shared rate limits enforced identically |

### 3.3 `anon` (Tor via embedded Arti)
| Threat | Mitigation |
|---|---|
| **Fail-open**: Arti down → request silently uses direct | Type-level `AnonClient` with no direct transport; planner returns degraded error; injection test asserts zero direct egress (P4 gate) |
| DNS leak | `socks5h` end-to-end; searxng-anon on an internal-only network whose sole egress is the Arti SOCKS port (topology fail-closed, already in compose.yaml) |
| Cross-request linkability | Per-request IsolationToken / RFC1929 SOCKS-username isolation; no circuit reuse across logical queries. **As built (P4):** full per-request isolation on the in-core path (fresh username per request); the searxng-anon path gets per-CONNECTION isolation (SearXNG's proxy URL is static — it cannot vary credentials per query), so httpx connection pooling can carry several queries' engine hits over one tunnel. Stronger than arti's no-auth proxy default; not query-perfect. |
| Cache/state leakage de-anonymizes behavior | Anon results only in ephemeral 32MB TTL-5m cache; never warm shared query cache, LTR priors, or bandit arms |
| Tor network abuse by us | Circuit cap (6), single attempt, no hedging/duplicate circuits, shared per-domain budgets |
| .onion as an SSRF/abuse vector | `.onion` rejected on every lane unless `allow_onion` (off; separate flag) |

### 3.4 Secret material at rest in RAM (`mlock` decision, SPEC §16 Phase 5)

Secrets (`secrecy::SecretString`) are zeroized on drop and never serialized,
logged, or surfaced in metrics. We deliberately do NOT `mlock` them: this Pi
runs zram swap (compressed RAM, no disk swap device), so secret pages cannot
reach persistent storage in the default deployment; `mlock` under memory
pressure would instead fight the shed ladder (RSS tripwires) and risks
OOM-killing the appliance to protect a token that rotates with one `.env` edit.
Operators who add DISK swap should prefer encrypted swap (or none) — recorded
in the operator manual (Phase 6).

## 4. What `anon` DOES and DOES NOT provide (SPEC §13.3, stated plainly)

**DOES:** hide the operator's source IP from upstream engines/sites for that
request; resolve DNS through Tor; isolate circuits per request so queries aren't
trivially linkable to each other at exit relays.

**DOES NOT:**
- Defeat a global passive adversary or sophisticated traffic-correlation attacks.
- Hide Tor *usage* from the operator's own ISP/network observer.
- Anonymize anyone **to Meridian itself** — mitigated separately by §13.4 no-log
  defaults (anon requests log lane+timing+status only; query text never at INFO;
  TRACE logging requires an explicit flag that warns at startup).
- Make scraping lawful where it isn't, or bypass robots/rate policies (enforced on
  anon too).
- Protect third-party users: the node is not an onion service and is not designed
  for untrusted callers; anon search requires the bearer by default.

## 5. Meridian's own storage & telemetry (the §13.4 surface)

| Threat | Mitigation |
|---|---|
| Logs accumulate user data over months | No-log defaults; journald 150M cap; access logs (if enabled) carry route/status/latency-bucket/lane only |
| Rate-limit keys reversible to IPs | blake3(ip‖salt) truncated 8B; salt rotates 24h + restart, lives in a `Secret`, never logged |
| /data volume stolen or backup leaks | Data minimization (no bodies, 240B snippets); documented backup caveat (restores un-expired data); retention TTLs on every store |
| Secrets in swap / core dumps | zram (compressed, volatile) swap; best-effort mlock of the small secrets region pending 16K-page validation (ADR-08); `panic=abort` (no unwind-time formatting of secret-bearing frames); secrets never Debug/Display |
| Forget-request incompleteness | `/v1/forget` spans tantivy + vectors + caches + tombstone; tested end-to-end incl. re-ingest refusal (P5 gate) |
| Supply chain (typosquats, license traps, malicious updates) | cargo-deny (licenses+bans+sources) and cargo-audit in CI; Cargo.lock committed; images digest-pinned; SBOM at release; gitleaks on every push |

## 6. Sidecar containment (SearXNG, AGPL + attack surface)

- UIs never published; instances live on `back`; the anon instance on an
  internal-only network. meridiand is the only caller.
- Read-only rootfs, cap_drop ALL, no-new-privileges, tmpfs /tmp, cgroup caps.
- A compromised searxng can: return poisoned results (mitigated by ranking
  diversity + domain caps + bandit demotion) and try lateral movement (mitigated by
  network isolation; meridiand's API requires bearer for mutations and the SSRF
  guard blocks the `back` CIDR).

## 7. Residual risks (accepted, documented)

1. Traffic correlation against the anon lane by a capable adversary — out of scope
   (inherent Tor limitation).
2. SD-card forensic recovery of TTL-expired data (no FDE requirement in v1) —
   operators with that threat model should enable OS-level encryption; documented.
3. A malicious upstream engine fingerprinting Meridian by response-timing — partial
   mitigation via fixed header sets per lane; accepted.
4. GDELT integrity: upstream serves no valid TLS; manifest MD5 is integrity-only,
   not authenticity. Analytics counters are non-security-critical; accepted.
