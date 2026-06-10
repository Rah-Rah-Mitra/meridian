//! SPEC §12.5 cross-lane invariant tests — the gate Phase 4 ships through.
//!
//! Hermetic: no Tor, no network beyond loopback. The SOCKS front-end is
//! exercised end-to-end through a real reqwest `socks5h` client (the exact
//! transport `AnonLane::client_for_request` builds) against a `MockDialer`
//! standing in for Arti behind the `Dialer` seam.
//!
//! Invariant 4 (lane-global per-domain budget) lives in `meridian-fetch`
//! (`budget.rs`: one keyed limiter at one chokepoint, lane-blind by
//! construction). Invariants 1/5 also have planner-level coverage in
//! `meridian-query/tests/lane_invariants.rs`.

use meridian_egress::anon::socks::{Dialer, Isolation, Socks5Server, SocksPolicy};
use meridian_egress::{EgressError, Lane, LaneRegistry};
use std::net::SocketAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Arti stand-in: records every dial, serves a canned HTTP/1.0 response.
#[derive(Default)]
struct MockDialer {
    ready: bool,
    dials: Mutex<Vec<(String, u16, Isolation)>>,
    dial_count: AtomicUsize,
}

impl MockDialer {
    fn up() -> Self {
        Self {
            ready: true,
            ..Self::default()
        }
    }

    fn down() -> Self {
        Self::default()
    }
}

impl Dialer for MockDialer {
    type Stream = tokio::io::DuplexStream;

    fn ready(&self) -> bool {
        self.ready
    }

    async fn dial(
        &self,
        host: &str,
        port: u16,
        isolation: Isolation,
    ) -> Result<Self::Stream, String> {
        self.dial_count.fetch_add(1, Ordering::SeqCst);
        self.dials
            .lock()
            .unwrap()
            .push((host.to_owned(), port, isolation));
        let (client_end, mut server_end) = tokio::io::duplex(16 * 1024);
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            let _ = server_end.read(&mut buf).await;
            let _ = server_end
                .write_all(b"HTTP/1.0 200 OK\r\ncontent-length: 2\r\n\r\nok")
                .await;
        });
        Ok(client_end)
    }
}

async fn spawn_server(dialer: std::sync::Arc<MockDialer>, allow_onion: bool) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Socks5Server::new(
        dialer,
        SocksPolicy {
            allow_onion,
            max_streams: 6,
            dial_timeout: Duration::from_secs(2),
        },
    );
    tokio::spawn(server.run(listener));
    addr
}

/// reqwest client wired exactly like `AnonLane::client_for_request`.
fn socks_client(addr: SocketAddr, user: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("socks5h://{user}:x@{addr}")).unwrap())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

/// Raw SOCKS5 CONNECT (no-auth) returning the server's reply code.
async fn raw_socks_connect(addr: SocketAddr, host: &str, port: u16) -> u8 {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(&[0x05, 0x01, 0x00]).await.unwrap(); // greeting: no-auth
    let mut resp = [0u8; 2];
    s.read_exact(&mut resp).await.unwrap();
    assert_eq!(resp, [0x05, 0x00], "server must accept no-auth");
    let mut req = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut reply = [0u8; 10];
    s.read_exact(&mut reply).await.unwrap();
    reply[1]
}

/// Invariant 2: DNS for anon resolves via the proxy protocol (socks5h ⇒ the
/// hostname crosses the SOCKS boundary verbatim; the host resolver is never
/// consulted — `.test` is unresolvable, so any local lookup would error out).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv2_hostname_crosses_socks_boundary_unresolved() {
    let dialer = std::sync::Arc::new(MockDialer::up());
    let addr = spawn_server(dialer.clone(), false).await;

    let body = socks_client(addr, "iso-a")
        .get("http://invariant-canary.test/x")
        .send()
        .await
        .expect("request through mock tor")
        .text()
        .await
        .unwrap();
    assert_eq!(body, "ok");

    let dials = dialer.dials.lock().unwrap();
    assert_eq!(dials.len(), 1);
    let (host, port, iso) = &dials[0];
    assert_eq!(host, "invariant-canary.test", "domain ATYP, not an IP");
    assert_eq!(*port, 80);
    assert_eq!(*iso, Isolation::Keyed("iso-a".into()));
}

/// SPEC §12.4 per-request isolation: distinct RFC1929 usernames arrive as
/// distinct isolation keys; the same username maps to the same key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rfc1929_usernames_become_isolation_keys() {
    let dialer = std::sync::Arc::new(MockDialer::up());
    let addr = spawn_server(dialer.clone(), false).await;

    for user in ["iso-1", "iso-2", "iso-1"] {
        socks_client(addr, user)
            .get("http://invariant-canary.test/")
            .send()
            .await
            .unwrap();
    }
    let dials = dialer.dials.lock().unwrap();
    let keys: Vec<_> = dials.iter().map(|(_, _, iso)| iso.clone()).collect();
    assert_eq!(
        keys,
        vec![
            Isolation::Keyed("iso-1".into()),
            Isolation::Keyed("iso-2".into()),
            Isolation::Keyed("iso-1".into()),
        ]
    );
}

/// Invariant 3: the shared SSRF deny table runs on the anon lane's SOCKS
/// front-end — private literals, non-allowlisted ports and `.onion` (while
/// disabled) are refused with REP 0x02 and never reach the dialer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv3_destination_policy_enforced_in_socks() {
    let dialer = std::sync::Arc::new(MockDialer::up());
    let addr = spawn_server(dialer.clone(), false).await;

    assert_eq!(raw_socks_connect(addr, "10.0.0.1", 80).await, 0x02);
    assert_eq!(raw_socks_connect(addr, "169.254.169.254", 80).await, 0x02);
    assert_eq!(raw_socks_connect(addr, "example.com", 6379).await, 0x02);
    assert_eq!(raw_socks_connect(addr, "example.com", 22).await, 0x02);
    assert_eq!(raw_socks_connect(addr, "hidden.onion", 80).await, 0x02);
    assert_eq!(
        dialer.dial_count.load(Ordering::SeqCst),
        0,
        "policy-refused destinations must never reach the dialer"
    );

    // Allowed destination still flows.
    assert_eq!(raw_socks_connect(addr, "example.com", 443).await, 0x00);
    assert_eq!(dialer.dial_count.load(Ordering::SeqCst), 1);
}

/// `.onion` is permitted only behind the explicit allow_onion flag (SPEC §12.4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn onion_gate_follows_allow_onion() {
    let dialer = std::sync::Arc::new(MockDialer::up());
    let addr = spawn_server(dialer.clone(), true).await;
    assert_eq!(raw_socks_connect(addr, "hidden.onion", 80).await, 0x00);
}

/// Invariant 1 (transport layer): with the backend down (Arti unbootstrapped),
/// the CONNECT is refused BEFORE any dial — and there is no other transport in
/// the path to leak onto. The request errors; nothing was dialed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv1_backend_down_refuses_without_dialing() {
    let dialer = std::sync::Arc::new(MockDialer::down());
    let addr = spawn_server(dialer.clone(), false).await;

    let result = socks_client(addr, "iso-x")
        .get("http://leak-canary.test/")
        .send()
        .await;
    assert!(result.is_err(), "anon request must fail, not fall back");
    assert_eq!(
        dialer.dial_count.load(Ordering::SeqCst),
        0,
        "zero dial attempts while down"
    );
}

/// Invariant 1 (registry layer): every unsatisfiable lane request errs —
/// disabled, not-started, unknown region, unverified region. No code path
/// returns a direct-lane client for a non-direct request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv1_registry_fail_closed_everywhere() {
    let dir = std::env::temp_dir().join(format!("meridian-inv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // Lanes disabled (shipped default): LaneDisabled.
    let cfg = meridian_common::config::LanesConfig::default();
    let registry = LaneRegistry::new(&cfg, &dir).unwrap();
    assert!(matches!(
        registry.resolve(&Lane::Anon),
        Err(EgressError::LaneDisabled("anon"))
    ));
    assert!(matches!(
        registry.resolve(&Lane::Region(meridian_common::ids::RegionId("de".into()))),
        Err(EgressError::LaneDisabled("region"))
    ));

    // Anon enabled but the lane was never started: NotReady, never direct.
    let cfg = meridian_common::config::LanesConfig {
        anon_enabled: true,
        ..Default::default()
    };
    let registry = LaneRegistry::new(&cfg, &dir).unwrap();
    assert!(matches!(
        registry.resolve(&Lane::Anon),
        Err(EgressError::NotReady(_))
    ));

    // Regions enabled: unknown id refused; configured-but-unverified refused.
    let mut cfg = meridian_common::config::LanesConfig {
        regions_enabled: true,
        ..Default::default()
    };
    cfg.regions.insert(
        "local".into(),
        meridian_common::config::RegionConfig {
            source_ip: "127.0.0.1".parse().unwrap(),
            verify_url: "https://checkip.amazonaws.com/".into(),
            expected_ip: None,
        },
    );
    let registry = LaneRegistry::new(&cfg, &dir).unwrap();
    assert!(matches!(
        registry.resolve(&Lane::Region(meridian_common::ids::RegionId("nope".into()))),
        Err(EgressError::UnknownRegion(_))
    ));
    assert!(
        matches!(
            registry.resolve(&Lane::Region(meridian_common::ids::RegionId(
                "local".into()
            ))),
            Err(EgressError::NotReady(_))
        ),
        "unverified region lane must refuse traffic"
    );

    // Direct still resolves — the lanes are independent.
    assert!(registry.resolve(&Lane::Direct).is_ok());
}

/// Invariant 5 (statuses surface): /v1/lanes sees honest states — anon Down
/// before start, configured regions individually listed as unverified.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lane_statuses_are_honest() {
    use meridian_egress::LaneStatus;
    let dir = std::env::temp_dir().join(format!("meridian-inv-st-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut cfg = meridian_common::config::LanesConfig {
        anon_enabled: true,
        regions_enabled: true,
        ..Default::default()
    };
    cfg.regions.insert(
        "local".into(),
        meridian_common::config::RegionConfig {
            source_ip: "127.0.0.1".parse().unwrap(),
            verify_url: "https://checkip.amazonaws.com/".into(),
            expected_ip: None,
        },
    );
    let registry = LaneRegistry::new(&cfg, &dir).unwrap();
    let statuses: std::collections::HashMap<_, _> = registry.statuses().into_iter().collect();
    assert_eq!(statuses["direct"], LaneStatus::Up);
    assert_eq!(
        statuses["anon"],
        LaneStatus::Down,
        "enabled but not started"
    );
    assert_eq!(
        statuses["region:local"],
        LaneStatus::Bootstrapping(0),
        "configured region awaits verification"
    );
}
