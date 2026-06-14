//! Planner-level §12.5 invariant coverage (invariants 1 and 5): a requested
//! anon/region lane that cannot be satisfied fails the SEARCH — visibly, with
//! zero network traffic — and `lane_effective` is always the lane that actually
//! carried traffic. Transport-layer coverage lives in
//! `meridian-egress/tests/invariants.rs`.

use meridian_common::config::{IndexConfig, LanesConfig, SearchConfig, VectorConfig};
use meridian_common::ids::RegionId;
use meridian_common::shed::ShedState;
use meridian_egress::{EgressError, Lane, LaneRegistry};
use meridian_embed::Embedder;
use meridian_index::lexical::LexicalIndex;
use meridian_query::planner::{PlanError, Planner, SearchRequest};
use meridian_query::{Scope, SearchMode};
use meridian_rerank::Reranker;
use meridian_searx::client::SearxClient;
use meridian_vector::VectorStore;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::AsyncWriteExt;

/// Counting canary HTTP server: any connection = a leak for these tests.
async fn canary() -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        loop {
            if let Ok((mut sock, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                let _ = sock
                    .write_all(b"HTTP/1.0 200 OK\r\ncontent-length: 2\r\n\r\n{}")
                    .await;
            }
        }
    });
    (addr, hits)
}

/// Tests run in parallel: nanosecond timestamps collided on fast CI runners
/// (tantivy LockBusy), so uniqueness comes from a process-wide counter.
static DIR_SEQ: AtomicUsize = AtomicUsize::new(0);

fn temp_planner(
    lanes_cfg: LanesConfig,
    searx_url: Option<String>,
    anon_url: Option<String>,
) -> Planner {
    let dir = std::env::temp_dir().join(format!(
        "meridian-lane-inv-{}-{}",
        std::process::id(),
        DIR_SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    let index_cfg = IndexConfig {
        data_dir: dir.clone(),
        writer_threads: 1,
        writer_heap_bytes: 16 * 1024 * 1024,
        merge_max_docs: 100_000,
    };
    let vector_cfg = VectorConfig::default();
    let index = Arc::new(LexicalIndex::open_or_create(&index_cfg).unwrap());
    let embedder = Arc::new(Embedder::test_stub(64));
    let vectors = Arc::new(VectorStore::open_or_create(&dir, 64, &vector_cfg).unwrap());
    let lanes = Arc::new(LaneRegistry::new(&lanes_cfg, &dir).unwrap());
    let searx = searx_url.map(|u| Arc::new(SearxClient::new(&u, 500, None).unwrap()));
    let searx_anon = anon_url.map(|u| Arc::new(SearxClient::new(&u, 500, None).unwrap()));
    Planner::new(
        index,
        embedder,
        vectors,
        searx,
        searx_anon,
        Arc::new(Reranker::unavailable()),
        None,
        None,
        None,
        None,
        lanes,
        Arc::new(ShedState::default()),
        2,
        Arc::new(meridian_common::prior::NoPrior),
        None,
        &SearchConfig::default(),
        &vector_cfg,
        &meridian_common::config::EvidenceConfig::default(),
    )
}

fn request(lane: Lane, scope: Scope) -> SearchRequest {
    SearchRequest {
        q: "meridian arc".to_owned(),
        mode: SearchMode::Fast,
        scope,
        lane,
        limit: 10,
        geo: None,
        after: None,
        before: None,
        bypass_cache: false,
        pin_engines: false,
        fetch_budget: 0,
        answer: false,
        diversity_evidence: false,
        overrides: Default::default(),
    }
}

/// Invariant 19 (Phase 9, ADR-26): the VoI fetch phase fails SAFE. With
/// fetch_budget set but no cross-encoder available (the musl image / test
/// stub), deep mode degrades with `fetch_unavailable` and performs ZERO
/// fetch egress; a non-direct lane never fetches regardless (the planner
/// guard behind the API validation).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv19_fetch_budget_fails_safe_without_reranker() {
    let (direct_addr, _hits) = canary().await;
    let planner = temp_planner(
        LanesConfig::default(),
        Some(format!("http://{direct_addr}/")),
        None,
    );
    let mut req = request(Lane::Direct, Scope::Web);
    req.mode = SearchMode::Deep;
    req.fetch_budget = 2;
    let resp = planner
        .search(req)
        .await
        .expect("deep search degrades, never fails");
    assert!(
        resp.degraded.contains(&"fetch_unavailable"),
        "must say WHY no fetching happened: {:?}",
        resp.degraded
    );
    assert!(
        resp.analysis.is_none(),
        "no analysis block without a fetch phase"
    );
}

/// Invariant 21 (Phase 10, ADR-29): answer mode fails SAFE exactly like the
/// fetch phase it rides on. With answer=true + fetch_budget but no
/// cross-encoder, the response degrades (`fetch_unavailable`), carries NO
/// best_passage block, and performs zero fetch egress; a non-direct lane
/// never reaches the phase at all (planner guard behind the API validation).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv21_answer_mode_fails_safe_without_reranker() {
    let (direct_addr, _hits) = canary().await;
    let planner = temp_planner(
        LanesConfig::default(),
        Some(format!("http://{direct_addr}/")),
        None,
    );
    let mut req = request(Lane::Direct, Scope::Web);
    req.mode = SearchMode::Deep;
    req.fetch_budget = 2;
    req.answer = true;
    let resp = planner
        .search(req)
        .await
        .expect("answer mode degrades, never fails");
    assert!(
        resp.degraded.contains(&"fetch_unavailable"),
        "must say WHY: {:?}",
        resp.degraded
    );
    assert!(
        resp.best_passage.is_none(),
        "no passage can exist without a CE"
    );
    assert!(resp.analysis.is_none(), "no analysis without a fetch phase");

    // The anon lane never reaches the fetch phase regardless of params —
    // answer mode inherits the lane firewall wholesale.
    let mut anon_req = request(Lane::Anon, Scope::Local);
    anon_req.mode = SearchMode::Deep;
    anon_req.fetch_budget = 2;
    anon_req.answer = true;
    let anon_resp = planner.search(anon_req).await.expect("local scope works");
    assert!(anon_resp.best_passage.is_none());
    assert!(anon_resp.analysis.is_none());
}

/// Invariant 18 (Phase 9, ADR-24): decision-log rows come ONLY from unpinned
/// direct-lane searches. The anon lane cannot write one even when the log is
/// enabled (the log call shares the bandit-reward site, which the anon branch
/// never reaches — SPEC §12.4), and the pinned compare halves write none
/// (pinning skips the bandit, so there is no decision to log).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv18_decision_log_direct_lane_only() {
    let (direct_addr, _direct_hits) = canary().await;
    let dir = std::env::temp_dir().join(format!(
        "meridian-lane-inv-{}-{}",
        std::process::id(),
        DIR_SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    let index_cfg = IndexConfig {
        data_dir: dir.clone(),
        writer_threads: 1,
        writer_heap_bytes: 16 * 1024 * 1024,
        merge_max_docs: 100_000,
    };
    let vector_cfg = VectorConfig::default();
    let bandit =
        Arc::new(meridian_searx::bandit::Bandit::open(&dir, 0.1).expect("bandit in temp dir"));
    let dlog = Arc::new(
        meridian_searx::decision_log::DecisionLog::new(bandit.database())
            .expect("decision log on the bandit db"),
    );
    let planner = Planner::new(
        Arc::new(LexicalIndex::open_or_create(&index_cfg).unwrap()),
        Arc::new(Embedder::test_stub(64)),
        Arc::new(VectorStore::open_or_create(&dir, 64, &vector_cfg).unwrap()),
        Some(Arc::new(
            SearxClient::new(&format!("http://{direct_addr}/"), 500, None).unwrap(),
        )),
        None,
        Arc::new(Reranker::unavailable()),
        Some(bandit),
        Some(dlog.clone()),
        None,
        None,
        Arc::new(
            LaneRegistry::new(
                &LanesConfig {
                    anon_enabled: true,
                    ..LanesConfig::default()
                },
                &dir,
            )
            .unwrap(),
        ),
        Arc::new(ShedState::default()),
        2,
        Arc::new(meridian_common::prior::NoPrior),
        None,
        &SearchConfig::default(),
        &vector_cfg,
        &meridian_common::config::EvidenceConfig::default(),
    );

    // (a) Unpinned direct web search: exactly one decision row (the write is
    // spawn_blocking'd off the request path, so poll briefly).
    planner
        .search(request(Lane::Direct, Scope::Web))
        .await
        .expect("direct web search");
    let mut rows = 0;
    for _ in 0..40 {
        rows = dlog.len().unwrap();
        if rows == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(rows, 1, "one unpinned direct search = one decision row");

    // (b) Anon search attempt (lane down ⇒ fails closed): still one row.
    let err = planner.search(request(Lane::Anon, Scope::Web)).await;
    assert!(err.is_err(), "anon must fail closed here");

    // (c) Pinned direct search (how BOTH compare halves run): still one row.
    let mut pinned = request(Lane::Direct, Scope::Web);
    pinned.pin_engines = true;
    pinned.bypass_cache = true;
    planner.search(pinned).await.expect("pinned direct search");

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        dlog.len().unwrap(),
        1,
        "anon attempts and pinned compare halves must never log decisions"
    );

    // Invariant 20 (SPEC §16 P9 exit: "decision log provably holds zero
    // query text"): the raw bytes of the redb file must not contain the
    // query string that produced the row — the hermetic form of the
    // extended privacy smoke. request() used q = "meridian arc".
    let raw = std::fs::read(dir.join("egress.redb")).expect("bandit/log db file");
    let needle = b"meridian arc";
    assert!(
        !raw.windows(needle.len()).any(|w| w == needle),
        "query text leaked into the decision-log db file"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Phase-8 invariant (ADR-22): compare-vantages fails CLOSED when the anon
/// half cannot run — never a silent direct-only answer under a compare label —
/// and the failed compare leaves the shared query cache untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv14_compare_fails_closed_when_anon_down() {
    let (direct_addr, _direct_hits) = canary().await;
    let (anon_addr, anon_hits) = canary().await;
    let lanes_cfg = LanesConfig {
        anon_enabled: true, // enabled but NOT started ⇒ injected Arti-down
        ..LanesConfig::default()
    };
    let planner = temp_planner(
        lanes_cfg,
        Some(format!("http://{direct_addr}/")),
        Some(format!("http://{anon_addr}/")),
    );

    let err = meridian_query::compare::compare_vantages(
        &planner,
        request(Lane::Direct, Scope::Web),
        0, // jitter elided: hermetic test, no upstream to decorrelate from
        0.3,
    )
    .await
    .expect_err("compare must fail closed when the anon half cannot run");
    assert!(
        matches!(err, PlanError::Lane(EgressError::NotReady(_))),
        "unexpected error: {err:?}"
    );
    assert_eq!(anon_hits.load(Ordering::SeqCst), 0, "anon backend was hit");
    let [(_, query_entries, _), _] = planner.cache_stats();
    assert_eq!(
        query_entries, 0,
        "compare halves must never enter the shared cache"
    );
}

/// Phase-8 invariant (ADR-22): `bypass_cache` requests neither read nor write
/// the shared query cache — the mechanism both compare halves ride.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv15_bypass_cache_writes_nothing() {
    let (direct_addr, _hits) = canary().await;
    let planner = temp_planner(
        LanesConfig::default(),
        Some(format!("http://{direct_addr}/")),
        None,
    );

    let mut req = request(Lane::Direct, Scope::Web);
    req.bypass_cache = true;
    planner.search(req).await.expect("direct search works");
    let [(_, after_bypass, _), _] = planner.cache_stats();
    assert_eq!(after_bypass, 0, "bypassed search must not be cached");

    planner
        .search(request(Lane::Direct, Scope::Web))
        .await
        .expect("direct search works");
    let [(_, after_normal, _), _] = planner.cache_stats();
    assert_eq!(after_normal, 1, "normal search caches (control arm)");
}

/// Phase-8 invariant (ADR-22, found live): a compare whose direct half comes
/// back EMPTY refuses before dispatching the anon half — an empty half is an
/// outage, and jsd(∅,∅)=0 would fabricate "no divergence". The anon canary
/// must see zero connections.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv17_compare_refuses_empty_direct_half() {
    let (direct_addr, direct_hits) = canary().await;
    let (anon_addr, anon_hits) = canary().await;
    let lanes_cfg = LanesConfig {
        anon_enabled: true, // enabled-not-started; must never be consulted
        ..LanesConfig::default()
    };
    let planner = temp_planner(
        lanes_cfg,
        Some(format!("http://{direct_addr}/")), // canary returns {} = no results
        Some(format!("http://{anon_addr}/")),
    );

    let err = meridian_query::compare::compare_vantages(
        &planner,
        request(Lane::Direct, Scope::Web),
        0,
        0.3,
    )
    .await
    .expect_err("empty direct half must refuse the comparison");
    assert!(
        matches!(err, PlanError::Lane(EgressError::NotReady(_))),
        "unexpected error: {err:?}"
    );
    assert!(
        direct_hits.load(Ordering::SeqCst) >= 1,
        "direct fan-out must have been attempted"
    );
    assert_eq!(
        anon_hits.load(Ordering::SeqCst),
        0,
        "no anon dispatch after a doomed direct half"
    );
}

/// Phase-8 invariant (risk #18 tripwire): the timing-decorrelation jitter is
/// ON by default — a compare dispatched with default config never sends both
/// halves back-to-back.
#[test]
fn inv16_compare_jitter_defaults_on() {
    assert!(
        SearchConfig::default().compare_jitter_ms_max > 0,
        "risk #18: default compare config must include inter-lane jitter"
    );
}

/// Invariant 1: anon requested, Arti down (lane enabled, never started) —
/// the search FAILS; neither the direct-searx canary nor the anon-searx canary
/// sees a single connection. No silent local-only serving under an anon label.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv1_anon_down_fails_search_with_zero_egress() {
    let (direct_addr, direct_hits) = canary().await;
    let (anon_addr, anon_hits) = canary().await;
    let lanes_cfg = LanesConfig {
        anon_enabled: true, // enabled but NOT started ⇒ injected Arti-down
        ..LanesConfig::default()
    };
    let planner = temp_planner(
        lanes_cfg,
        Some(format!("http://{direct_addr}/")),
        Some(format!("http://{anon_addr}/")),
    );

    for scope in [Scope::Web, Scope::Both] {
        let err = planner
            .search(request(Lane::Anon, scope))
            .await
            .expect_err("anon search must fail closed while Arti is down");
        assert!(
            matches!(err, PlanError::Lane(EgressError::NotReady(_))),
            "unexpected error: {err:?}"
        );
    }
    assert_eq!(direct_hits.load(Ordering::SeqCst), 0, "direct egress leak");
    assert_eq!(anon_hits.load(Ordering::SeqCst), 0, "anon backend was hit");
}

/// Anon requested with no anon metasearch backend configured: fail closed
/// (NOT a degraded local-only answer wearing an anon label — the P3 bug class).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anon_without_backend_fails_closed() {
    let (direct_addr, direct_hits) = canary().await;
    let lanes_cfg = LanesConfig {
        anon_enabled: true,
        ..LanesConfig::default()
    };
    let planner = temp_planner(lanes_cfg, Some(format!("http://{direct_addr}/")), None);

    let err = planner
        .search(request(Lane::Anon, Scope::Both))
        .await
        .expect_err("no anon backend ⇒ refuse");
    assert!(matches!(err, PlanError::Lane(EgressError::NotReady(_))));
    assert_eq!(direct_hits.load(Ordering::SeqCst), 0);
}

/// Region + web scope refuses: metasearch through the direct sidecar would
/// violate invariant 1, so there is no region metasearch at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn region_web_scope_fails_closed() {
    let (direct_addr, direct_hits) = canary().await;
    let mut lanes_cfg = LanesConfig {
        regions_enabled: true,
        ..LanesConfig::default()
    };
    lanes_cfg.regions.insert(
        "local".into(),
        meridian_common::config::RegionConfig {
            source_ip: "127.0.0.1".parse().unwrap(),
            verify_url: format!("http://{direct_addr}/"),
            expected_ip: None,
        },
    );
    let planner = temp_planner(lanes_cfg, Some(format!("http://{direct_addr}/")), None);

    let err = planner
        .search(request(Lane::Region(RegionId("local".into())), Scope::Web))
        .await
        .expect_err("region metasearch must refuse");
    assert!(matches!(err, PlanError::Lane(EgressError::NotReady(_))));
    assert_eq!(direct_hits.load(Ordering::SeqCst), 0);
}

/// Invariant 5: a local-only scope uses no lane at all and says so —
/// `lane_effective` is "local-only" even when an exotic lane was requested.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv5_local_scope_reports_local_only() {
    let (direct_addr, direct_hits) = canary().await;
    let lanes_cfg = LanesConfig {
        anon_enabled: true,
        regions_enabled: true,
        ..LanesConfig::default()
    };
    let planner = temp_planner(lanes_cfg, Some(format!("http://{direct_addr}/")), None);

    for lane in [
        Lane::Direct,
        Lane::Anon,
        Lane::Region(RegionId("de".into())),
    ] {
        let resp = planner
            .search(request(lane.clone(), Scope::Local))
            .await
            .expect("local scope needs no egress");
        assert_eq!(resp.lane_effective, "local-only");
        assert_eq!(
            resp.lane_requested,
            match &lane {
                Lane::Direct => "direct".to_owned(),
                Lane::Anon => "anon".to_owned(),
                Lane::Region(id) => format!("region:{}", id.0),
            }
        );
    }
    assert_eq!(direct_hits.load(Ordering::SeqCst), 0);
}

/// Invariant 5 (positive case): a direct web search reports effective=direct
/// and actually used the direct backend.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv5_direct_web_reports_direct() {
    let (direct_addr, direct_hits) = canary().await;
    let planner = temp_planner(
        LanesConfig::default(),
        Some(format!("http://{direct_addr}/")),
        None,
    );

    let resp = planner
        .search(request(Lane::Direct, Scope::Web))
        .await
        .expect("direct web search");
    assert_eq!(resp.lane_requested, "direct");
    assert_eq!(resp.lane_effective, "direct");
    assert!(
        direct_hits.load(Ordering::SeqCst) >= 1,
        "direct backend must have been queried"
    );
}
