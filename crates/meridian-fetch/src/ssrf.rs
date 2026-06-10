//! SSRF guard (SPEC §13.1) — mandatory, unit-tested, runs on ALL lanes.
//!
//! Policy: scheme allowlist {http, https}; standard ports only (80/443/8080/8443);
//! `.onion` rejected unless `allow_onion` (and even then never on direct/region);
//! every resolved A/AAAA address checked against the deny ranges BEFORE connect;
//! the connection is then pinned to a validated address (no TOCTOU re-resolution —
//! see `LaneRegistry::pinned`); each redirect hop re-validates from scratch.

use std::net::{IpAddr, SocketAddr};
use url::Url;

// The ONE deny table, shared with the anon SOCKS destination policy
// (meridian-egress) — §12.5 invariant 3. Re-exported to keep this module the
// fetch-side façade.
pub use meridian_common::netpolicy::{ALLOWED_PORTS, ip_denied};

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

#[cfg(test)]
mod tests {
    use super::*;

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
