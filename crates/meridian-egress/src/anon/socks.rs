//! Meridian's own thin SOCKS5 front-end for the anon lane (SPEC §12.4, ADR-04).
//!
//! Serves two clients, both with circuit isolation:
//! - `searxng-anon` (its sole upstream proxy, `socks5h://` so DNS resolves in
//!   Tor) — connects without auth, and gets a **fresh isolation token per TCP
//!   connection** (stronger than sharing one group; a static proxy URL cannot
//!   vary RFC1929 credentials per query).
//! - Meridian's in-core anon fetches — a fresh RFC1929 username per logical
//!   request, so each maps to its own `IsolationToken` (SPEC §12.4 per-request
//!   stream isolation).
//!
//! Hand-rolled rather than the `arti` proxy because that listener is
//! `experimental-api`-gated (ADR-04); CONNECT-only, ~200 lines, on stable
//! `arti-client` APIs via the [`Dialer`] seam — which is also what makes the
//! §12.5 invariants testable without touching the Tor network.
//!
//! Destination policy (§12.5 invariant 3): the shared deny table runs HERE too —
//! port allowlist, literal private/reserved IPs, `.onion` gate. Tor cannot reach
//! RFC1918 space, but the policy must not depend on that.

use meridian_common::netpolicy::host_port_denied;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

/// How a proxied stream is grouped onto Tor circuits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Isolation {
    /// No credentials offered: fresh token per TCP connection.
    PerConnection,
    /// RFC1929 username: one token per distinct username (TTL-bounded).
    Keyed(String),
}

/// The transport seam: production = Arti (`connect_with_prefs` + isolation
/// token); tests = a mock recording (host, port, isolation) and serving canned
/// bytes. There is deliberately no TCP/direct implementation of this trait in
/// the crate — that absence is the anon lane's type-level fail-closed guard.
pub trait Dialer: Send + Sync + 'static {
    type Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static;

    /// False until the backend can carry traffic (Arti bootstrapped). The server
    /// refuses CONNECTs (general failure) without dialing while not ready.
    fn ready(&self) -> bool;

    fn dial(
        &self,
        host: &str,
        port: u16,
        isolation: Isolation,
    ) -> impl Future<Output = Result<Self::Stream, String>> + Send;
}

#[derive(Clone)]
pub struct SocksPolicy {
    pub allow_onion: bool,
    /// Concurrent proxied streams ≈ concurrent circuits (SPEC §12.4: default 6).
    pub max_streams: usize,
    /// Per-dial budget (circuit build + exit connect).
    pub dial_timeout: Duration,
}

/// Hard cap on one proxied stream's lifetime so a wedged peer cannot pin a
/// circuit permit forever. Generous: covers Tor latency + a slow engine page.
const STREAM_LIFETIME: Duration = Duration::from_secs(180);
/// Handshake must complete promptly — the listener is back-network/loopback.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

const REP_SUCCESS: u8 = 0x00;
const REP_FAILURE: u8 = 0x01;
const REP_DENIED: u8 = 0x02;
const REP_HOST_UNREACHABLE: u8 = 0x04;
const REP_CMD_UNSUPPORTED: u8 = 0x07;
const REP_ATYP_UNSUPPORTED: u8 = 0x08;

pub struct Socks5Server<D: Dialer> {
    dialer: Arc<D>,
    policy: SocksPolicy,
    streams: Arc<Semaphore>,
}

impl<D: Dialer> Socks5Server<D> {
    pub fn new(dialer: Arc<D>, policy: SocksPolicy) -> Self {
        let streams = Arc::new(Semaphore::new(policy.max_streams));
        Self {
            dialer,
            policy,
            streams,
        }
    }

    /// Accept loop. Runs until the listener errors fatally or the task is
    /// dropped (meridiand shutdown).
    pub async fn run(self, listener: TcpListener) {
        loop {
            match listener.accept().await {
                Ok((sock, _peer)) => {
                    let dialer = self.dialer.clone();
                    let policy = self.policy.clone();
                    let streams = self.streams.clone();
                    tokio::spawn(async move {
                        // Errors are deliberately not logged with peer/target
                        // detail — this path carries anon traffic (SPEC §13.3).
                        let _ = handle_conn(sock, dialer, policy, streams).await;
                    });
                }
                Err(e) => {
                    tracing::warn!(error = %e, "anon socks accept error");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
}

async fn handle_conn<D: Dialer>(
    mut sock: TcpStream,
    dialer: Arc<D>,
    policy: SocksPolicy,
    streams: Arc<Semaphore>,
) -> Result<(), std::io::Error> {
    let (host, port, isolation) =
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(&mut sock)).await {
            Ok(Ok(Some(t))) => t,
            Ok(Ok(None)) => return Ok(()), // refusal already written
            Ok(Err(e)) => return Err(e),
            Err(_) => return Ok(()), // handshake stalled
        };

    // Destination policy — §12.5 invariant 3 (shared deny table).
    if host_port_denied(&host, port, policy.allow_onion).is_some() {
        return reply(&mut sock, REP_DENIED).await;
    }

    // Fail-closed: backend not ready ⇒ refuse BEFORE any dial. There is no
    // other transport to try (the Dialer seam has no direct implementation).
    if !dialer.ready() {
        return reply(&mut sock, REP_FAILURE).await;
    }

    // Circuit-count citizenship (SPEC §12.4): bounded concurrent streams.
    let Ok(_permit) = streams.acquire().await else {
        return reply(&mut sock, REP_FAILURE).await;
    };

    let upstream = match tokio::time::timeout(
        policy.dial_timeout,
        dialer.dial(&host, port, isolation),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(_)) => return reply(&mut sock, REP_HOST_UNREACHABLE).await,
        Err(_) => return reply(&mut sock, REP_HOST_UNREACHABLE).await,
    };

    reply(&mut sock, REP_SUCCESS).await?;
    let mut upstream = upstream;
    let _ = tokio::time::timeout(
        STREAM_LIFETIME,
        tokio::io::copy_bidirectional(&mut sock, &mut upstream),
    )
    .await;
    Ok(())
}

/// SOCKS5 greeting + (optional) RFC1929 + CONNECT request parsing.
/// `Ok(None)` = refused with a protocol reply already sent.
async fn handshake(
    sock: &mut TcpStream,
) -> Result<Option<(String, u16, Isolation)>, std::io::Error> {
    // Greeting: VER NMETHODS METHODS...
    let mut head = [0u8; 2];
    sock.read_exact(&mut head).await?;
    if head[0] != 0x05 || head[1] == 0 {
        sock.write_all(&[0x05, 0xFF]).await?;
        return Ok(None);
    }
    let mut methods = vec![0u8; head[1] as usize];
    sock.read_exact(&mut methods).await?;

    // Prefer username/password (isolation key), else no-auth (per-connection).
    let isolation = if methods.contains(&0x02) {
        sock.write_all(&[0x05, 0x02]).await?;
        // RFC1929: VER ULEN UNAME PLEN PASSWD
        let mut ver_ulen = [0u8; 2];
        sock.read_exact(&mut ver_ulen).await?;
        if ver_ulen[0] != 0x01 {
            return Ok(None);
        }
        let mut uname = vec![0u8; ver_ulen[1] as usize];
        sock.read_exact(&mut uname).await?;
        let mut plen = [0u8; 1];
        sock.read_exact(&mut plen).await?;
        let mut passwd = vec![0u8; plen[0] as usize];
        sock.read_exact(&mut passwd).await?;
        // Any password is accepted: credentials are an isolation key, not an
        // authentication secret (the listener is loopback/back-network only).
        sock.write_all(&[0x01, 0x00]).await?;
        Isolation::Keyed(String::from_utf8_lossy(&uname).into_owned())
    } else if methods.contains(&0x00) {
        sock.write_all(&[0x05, 0x00]).await?;
        Isolation::PerConnection
    } else {
        sock.write_all(&[0x05, 0xFF]).await?;
        return Ok(None);
    };

    // Request: VER CMD RSV ATYP DST.ADDR DST.PORT
    let mut req = [0u8; 4];
    sock.read_exact(&mut req).await?;
    if req[0] != 0x05 {
        reply(sock, REP_FAILURE).await?;
        return Ok(None);
    }
    if req[1] != 0x01 {
        // CONNECT only — no BIND, no UDP ASSOCIATE.
        reply(sock, REP_CMD_UNSUPPORTED).await?;
        return Ok(None);
    }
    let host = match req[3] {
        0x01 => {
            let mut a = [0u8; 4];
            sock.read_exact(&mut a).await?;
            std::net::Ipv4Addr::from(a).to_string()
        }
        0x03 => {
            let mut len = [0u8; 1];
            sock.read_exact(&mut len).await?;
            let mut name = vec![0u8; len[0] as usize];
            sock.read_exact(&mut name).await?;
            match String::from_utf8(name) {
                Ok(s) if !s.is_empty() => s,
                _ => {
                    reply(sock, REP_ATYP_UNSUPPORTED).await?;
                    return Ok(None);
                }
            }
        }
        0x04 => {
            let mut a = [0u8; 16];
            sock.read_exact(&mut a).await?;
            std::net::Ipv6Addr::from(a).to_string()
        }
        _ => {
            reply(sock, REP_ATYP_UNSUPPORTED).await?;
            return Ok(None);
        }
    };
    let mut port = [0u8; 2];
    sock.read_exact(&mut port).await?;
    Ok(Some((host, u16::from_be_bytes(port), isolation)))
}

async fn reply(sock: &mut TcpStream, code: u8) -> Result<(), std::io::Error> {
    // BND.ADDR/BND.PORT are meaningless for our CONNECT relay: zeros.
    sock.write_all(&[0x05, code, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await
}
