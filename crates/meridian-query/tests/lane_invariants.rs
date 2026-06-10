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
        lanes,
        Arc::new(ShedState::default()),
        2,
        Arc::new(meridian_common::prior::NoPrior),
        &SearchConfig::default(),
        &vector_cfg,
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
    }
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
