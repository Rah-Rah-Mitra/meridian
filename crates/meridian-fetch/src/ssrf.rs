//! SSRF guard (SPEC §13.1) — mandatory, unit-tested, runs on ALL lanes.
//!
//! Policy: scheme allowlist {http, https}; standard ports only (80/443/8080/8443);
//! `.onion` rejected unless `allow_onion` (and even then never on direct/region);
//! every resolved A/AAAA address checked against the deny ranges BEFORE connect;
//! the connection is then pinned to a validated address (no TOCTOU re-resolution —
//! see `LaneRegistry::pinned`); each redirect hop re-validates from scratch.

use std::net::{IpAddr, SocketAddr};
use url::Url;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SsrfError {
    #[error("invalid url")]
    InvalidUrl,
    #[error("scheme not allowed")]
    SchemeDenied,
    #[error("port not allowed")]
    PortDenied,
    #[error(".onion is disabled")]
    OnionDenied,
    #[error("destination address is in a denied range")]
    AddressDenied,
    #[error("hostname did not resolve")]
    ResolutionFailed,
}

const ALLOWED_PORTS: &[u16] = &[80, 443, 8080, 8443];

/// A validated fetch target: original URL + the pinned, vetted socket address.
#[derive(Debug, Clone)]
pub struct VettedTarget {
    pub url: Url,
    pub host: String,
    pub addr: SocketAddr,
}

/// Static (pre-resolution) checks — also the only checks possible on the anon
/// lane, where resolution happens inside Tor (SPEC §13.1).
pub fn check_url(raw: &str, allow_onion: bool) -> Result<Url, SsrfError> {
    let url = Url::parse(raw).map_err(|_| SsrfError::InvalidUrl)?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err(SsrfError::SchemeDenied),
    }
    let host = url.host_str().ok_or(SsrfError::InvalidUrl)?;
    if host.ends_with(".onion") && !allow_onion {
        return Err(SsrfError::OnionDenied);
    }
    let port = url.port_or_known_default().ok_or(SsrfError::PortDenied)?;
    if !ALLOWED_PORTS.contains(&port) {
        return Err(SsrfError::PortDenied);
    }
    // Literal IPs in the URL are checked immediately (no DNS involved).
    if let Some(url::Host::Ipv4(ip)) = url.host() {
        if ip_denied(IpAddr::V4(ip)) {
            return Err(SsrfError::AddressDenied);
        }
    }
    if let Some(url::Host::Ipv6(ip)) = url.host() {
        if ip_denied(IpAddr::V6(ip)) {
            return Err(SsrfError::AddressDenied);
        }
    }
    Ok(url)
}

/// Full direct/region-lane validation: static checks, resolve ALL addresses,
/// reject if ANY falls in a denied range, pin the first vetted address.
pub async fn vet(raw: &str, allow_onion: bool) -> Result<VettedTarget, SsrfError> {
    let url = check_url(raw, allow_onion)?;
    let host = url.host_str().ok_or(SsrfError::InvalidUrl)?.to_owned();
    let port = url.port_or_known_default().ok_or(SsrfError::PortDenied)?;

    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| SsrfError::ResolutionFailed)?
        .collect();
    if addrs.is_empty() {
        return Err(SsrfError::ResolutionFailed);
    }
    // A hostname with ANY denied address is rejected outright — a rebinding or
    // split-horizon answer must not be reachable through the public answers.
    if addrs.iter().any(|a| ip_denied(a.ip())) {
        return Err(SsrfError::AddressDenied);
    }
    Ok(VettedTarget {
        addr: addrs[0],
        url,
        host,
    })
}

/// SPEC §13.1 deny ranges. Everything not globally routable is denied.
pub fn ip_denied(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()                      // 127/8
                || v4.is_private()                // RFC1918
                || v4.is_link_local()             // 169.254/16 (cloud metadata)
                || v4.is_unspecified()            // 0.0.0.0
                || o[0] == 0                      // 0.0.0.0/8
                || (o[0] == 100 && (o[1] & 0xC0) == 64) // 100.64/10 CGNAT
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24 IETF
                || (o[0] == 192 && o[1] == 0 && o[2] == 2) // 192.0.2/24 TEST-NET-1
                || (o[0] == 198 && (o[1] & 0xFE) == 18) // 198.18/15 benchmarking
                || (o[0] == 198 && o[1] == 51 && o[2] == 100) // TEST-NET-2
                || (o[0] == 203 && o[1] == 0 && o[2] == 113) // TEST-NET-3
                || v4.is_multicast()              // 224/4
                || o[0] >= 240 // 240/4 reserved + broadcast
        }
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            v6.is_loopback()                       // ::1
                || v6.is_unspecified()             // ::
                || (seg[0] & 0xffc0) == 0xfe80     // fe80::/10 link-local
                || (seg[0] & 0xfe00) == 0xfc00     // fc00::/7 ULA
                || (seg[0] & 0xff00) == 0xff00     // ff00::/8 multicast
                || (seg[0] == 0x2001 && seg[1] == 0xdb8) // 2001:db8::/32 doc
                || v6.to_ipv4_mapped().is_some_and(|v4| ip_denied(IpAddr::V4(v4)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn denied(s: &str) -> bool {
        ip_denied(s.parse().unwrap())
    }

    #[test]
    fn deny_ranges_cover_spec_13_1() {
        for ip in [
            "127.0.0.1",
            "127.8.8.8",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254", // cloud metadata
            "100.64.0.1",
            "100.127.255.254",
            "0.0.0.0",
            "0.1.2.3",
            "224.0.0.1",
            "239.255.255.255",
            "240.0.0.1",
            "255.255.255.255",
            "198.18.0.1",
            "192.0.2.1",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fdff::1",
            "ff02::1",
            "::ffff:10.0.0.1",  // v4-mapped private
            "::ffff:127.0.0.1", // v4-mapped loopback
        ] {
            assert!(denied(ip), "{ip} must be denied");
        }
        for ip in ["93.184.216.34", "1.1.1.1", "2606:4700::1111", "8.8.8.8"] {
            assert!(!denied(ip), "{ip} must be allowed");
        }
    }

    #[test]
    fn scheme_port_and_onion_rules() {
        assert_eq!(
            check_url("ftp://example.com/", false),
            Err(SsrfError::SchemeDenied)
        );
        assert_eq!(
            check_url("file:///etc/passwd", false),
            Err(SsrfError::SchemeDenied)
        );
        assert_eq!(
            check_url("gopher://example.com/", false),
            Err(SsrfError::SchemeDenied)
        );
        assert_eq!(
            check_url("http://example.com:22/", false),
            Err(SsrfError::PortDenied)
        );
        assert_eq!(
            check_url("http://example.com:6379/", false),
            Err(SsrfError::PortDenied)
        );
        assert!(check_url("https://example.com/", false).is_ok());
        assert!(check_url("http://example.com:8080/x", false).is_ok());
        assert_eq!(
            check_url("http://something.onion/", false),
            Err(SsrfError::OnionDenied)
        );
        // Literal private/loopback IPs die before any DNS.
        assert_eq!(
            check_url("http://127.0.0.1/admin", false),
            Err(SsrfError::AddressDenied)
        );
        assert_eq!(
            check_url("http://169.254.169.254/latest/meta-data/", false),
            Err(SsrfError::AddressDenied)
        );
        assert_eq!(
            check_url("http://[::1]:8080/", false),
            Err(SsrfError::AddressDenied)
        );
    }

    #[tokio::test]
    async fn vet_rejects_hostnames_resolving_to_denied_ranges() {
        // localhost resolves to loopback on any sane resolver.
        assert_eq!(
            vet("http://localhost/", false).await.unwrap_err(),
            SsrfError::AddressDenied
        );
    }
}
