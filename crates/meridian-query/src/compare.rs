//! Compare-vantages orchestrator (Phase 8, ADR-22 / SPEC §16 P8).
//!
//! Runs the SAME query over the direct and anon lanes and reports how the
//! result-domain distributions diverge — the capability no SaaS search API
//! exposes. Both halves run the UNCHANGED single-lane pipeline, fail-closed
//! per §12.1: an unavailable anon lane errors the whole comparison (a silent
//! direct-only answer under a compare label would be a lie).
//!
//! Zero shared state by construction (ADR-22): both halves bypass the query
//! caches (a stale cached half would compare different points in time, and
//! the combined response must never be cached); the anon half can never
//! reward the bandit (`chosen_arm` is only ever set on the direct branch);
//! anon results never warm shared structures (§12.4, unchanged code path).
//!
//! Timing decorrelation (risk #18): a randomized delay before the anon-side
//! dispatch keeps the two upstream arrivals from being trivially linkable.
//! Compare mode is still intentionally observable from two vantages — an
//! explicit per-request choice, documented in privacy.md; never a default.

use crate::planner::{PlanError, Planner, SearchRequest, SearchResponse};
use meridian_egress::Lane;
use std::collections::HashMap;

/// Response-level `divergence` block (additive, ADR-20).
#[derive(Debug, Clone, serde::Serialize)]
pub struct DivergenceBlock {
    pub schema: u8,
    pub lanes_compared: [&'static str; 2],
    /// Jensen-Shannon divergence (log base 2, bounded [0,1]) between the two
    /// lanes' registered-domain distributions.
    pub jsd: f64,
    /// The same-lane noise floor (p90) this deployment measured in the
    /// suite-12 probe; configured, not invented.
    pub noise_floor_p90: f64,
    /// `jsd > noise_floor_p90` — the per-request signal. The defensible
    /// population-level claim comes from suite 12b's bootstrap across a query
    /// set, not from one request.
    pub exceeds_floor: bool,
    /// Registered domains present in exactly one lane's results.
    pub domains_only_in_direct: Vec<String>,
    pub domains_only_in_anon: Vec<String>,
    /// Anon-half result count (the direct half's results ARE the response).
    pub anon_result_count: usize,
    /// Decorrelation delay actually applied before the anon dispatch.
    pub jitter_applied_ms: u64,
}

/// Run the comparison. `req.lane` is ignored — compare GOVERNS lanes.
/// Returns the DIRECT half's response with the `divergence` block attached
/// (additive shape, ADR-20); the anon half is summarized inside the block.
pub async fn compare_vantages(
    planner: &Planner,
    mut req: SearchRequest,
    jitter_max_ms: u64,
    noise_floor_p90: f64,
) -> Result<SearchResponse, PlanError> {
    // Both halves bypass caches: a cached half would compare different points
    // in time, and the §12.4-clean story is "compare touches no shared state".
    // Both halves pin instance-default engines: the suite-12 probe measured
    // bandit arm churn at p90 JSD 0.67 WITHIN the direct lane — pinning makes
    // the halves differ by vantage only (and no bandit state is read or
    // rewarded anywhere in a compare).
    req.bypass_cache = true;
    req.pin_engines = true;

    let mut direct_req = req.clone();
    direct_req.lane = Lane::Direct;
    let mut direct = planner.search(direct_req).await?;
    // An empty half is an OUTAGE, not a vantage: jsd(∅,∅)=0 would read as
    // "no divergence" — fabricated agreement. Refuse before spending a Tor
    // circuit on a doomed comparison. (Found live: the searxng sidecar was
    // down and 48 gate compares silently reported zero divergence.)
    if direct.results.is_empty() {
        return Err(PlanError::Lane(meridian_egress::EgressError::NotReady(
            "compare: direct half returned no web results (engines unreachable \
             or deadline too tight) — comparison refused, not fabricated"
                .into(),
        )));
    }

    // Timing decorrelation BEFORE the anon dispatch (risk #18). SystemTime
    // nanos are not crypto-grade randomness; the adversary model here is
    // passive correlation of two arrivals, and the first arrival's absolute
    // time is itself unknown to that observer.
    let jitter_applied_ms = if jitter_max_ms > 0 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::from(d.subsec_nanos()))
            .unwrap_or(0);
        let ms = nanos % (jitter_max_ms + 1);
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        ms
    } else {
        0
    };

    let mut anon_req = req;
    anon_req.lane = Lane::Anon;
    // Fail-closed: any anon-half error (Arti down, no backend, admission
    // budget) fails the WHOLE compare — never a silent direct-only answer.
    let anon = planner.search(anon_req).await?;
    if anon.results.is_empty() {
        return Err(PlanError::Lane(meridian_egress::EgressError::NotReady(
            "compare: anon half returned no web results — comparison refused, \
             not fabricated"
                .into(),
        )));
    }

    let dist_direct = domain_distribution(&direct);
    let dist_anon = domain_distribution(&anon);
    let jsd = jsd(&dist_direct, &dist_anon);

    let only = |a: &HashMap<String, f64>, b: &HashMap<String, f64>| -> Vec<String> {
        let mut v: Vec<String> = a.keys().filter(|k| !b.contains_key(*k)).cloned().collect();
        v.sort();
        v
    };

    direct.divergence = Some(DivergenceBlock {
        schema: 1,
        lanes_compared: ["direct", "anon"],
        jsd,
        noise_floor_p90,
        exceeds_floor: jsd > noise_floor_p90,
        domains_only_in_direct: only(&dist_direct, &dist_anon),
        domains_only_in_anon: only(&dist_anon, &dist_direct),
        anon_result_count: anon.results.len(),
        jitter_applied_ms,
    });
    Ok(direct)
}

/// Registered-domain counts over a response's results — the same
/// approximation the suite-12 noise floor was measured with (last two host
/// labels; three under well-known second-level registries).
fn domain_distribution(resp: &SearchResponse) -> HashMap<String, f64> {
    let mut dist: HashMap<String, f64> = HashMap::new();
    for r in &resp.results {
        if let Some(d) = registered_domain(&r.url) {
            *dist.entry(d).or_insert(0.0) += 1.0;
        }
    }
    dist
}

fn registered_domain(url: &str) -> Option<String> {
    let host = url::Url::parse(url)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)?;
    if host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() < 2 {
        return Some(host);
    }
    const SECOND_LEVEL: &[&str] = &["co", "com", "net", "org", "ac", "gov", "edu"];
    let take = if labels.len() >= 3 && SECOND_LEVEL.contains(&labels[labels.len() - 2]) {
        3
    } else {
        2
    };
    Some(labels[labels.len() - take..].join("."))
}

/// Jensen-Shannon divergence between two count distributions; log base 2 so
/// the result is bounded [0, 1]. NaN-free: empty distributions diverge fully
/// from non-empty ones and not at all from each other.
fn jsd(p: &HashMap<String, f64>, q: &HashMap<String, f64>) -> f64 {
    let (sp, sq): (f64, f64) = (p.values().sum(), q.values().sum());
    match (sp > 0.0, sq > 0.0) {
        (false, false) => return 0.0,
        (true, false) | (false, true) => return 1.0,
        _ => {}
    }
    let mut keys: Vec<&String> = p.keys().chain(q.keys()).collect();
    keys.sort();
    keys.dedup();
    let mut acc = 0.0;
    for k in keys {
        let pi = p.get(k).copied().unwrap_or(0.0) / sp;
        let qi = q.get(k).copied().unwrap_or(0.0) / sq;
        let mi = 0.5 * (pi + qi);
        if pi > 0.0 {
            acc += 0.5 * pi * (pi / mi).log2();
        }
        if qi > 0.0 {
            acc += 0.5 * qi * (qi / mi).log2();
        }
    }
    acc.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsd_bounds_and_degenerate_cases() {
        let mut a = HashMap::new();
        a.insert("x.com".to_owned(), 5.0);
        assert_eq!(jsd(&a, &a.clone()), 0.0);
        let mut b = HashMap::new();
        b.insert("y.org".to_owned(), 5.0);
        assert!((jsd(&a, &b) - 1.0).abs() < 1e-9, "disjoint = 1");
        let empty = HashMap::new();
        assert_eq!(jsd(&empty, &empty), 0.0);
        assert_eq!(jsd(&a, &empty), 1.0, "empty vs non-empty = full divergence");
    }

    #[test]
    fn registered_domain_matches_probe_methodology() {
        assert_eq!(
            registered_domain("https://news.example.com/a"),
            Some("example.com".into())
        );
        assert_eq!(
            registered_domain("http://www.bbc.co.uk/x"),
            Some("bbc.co.uk".into())
        );
        assert_eq!(registered_domain("https://127.0.0.1/x"), None);
    }
}
