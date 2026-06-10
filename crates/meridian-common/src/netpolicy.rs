//! The ONE SSRF deny table + port allowlist (SPEC §13.1, §12.5 invariant 3).
//!
//! Lives in `meridian-common` so every enforcement point shares it: the fetch
//! ladder's `check_url`/`vet` (direct/region lanes, resolve-then-pin) and the
//! anon lane's SOCKS destination policy (where resolution happens inside Tor and
//! only the host/port form is available). Two copies of this table would be a
//! security bug waiting to diverge.

use std::net::IpAddr;

/// Standard ports only (SPEC §13.1): http, https and their common alternates.
pub const ALLOWED_PORTS: &[u16] = &[80, 443, 8080, 8443];

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

/// Host/port destination policy for proxy-style enforcement points (the anon
/// SOCKS front-end), mirroring `check_url` for the parts that exist without a
/// URL: port allowlist, `.onion` gate, literal-IP deny ranges. Hostnames that
/// are not IP literals pass here — on the anon lane they resolve inside Tor,
/// which cannot reach the operator's RFC1918 space anyway.
pub fn host_port_denied(host: &str, port: u16, allow_onion: bool) -> Option<&'static str> {
    if !ALLOWED_PORTS.contains(&port) {
        return Some("port not allowed");
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.ends_with(".onion") && !allow_onion {
        return Some(".onion is disabled");
    }
    if let Ok(ip) = bare.parse::<IpAddr>() {
        if ip_denied(ip) {
            return Some("destination address is in a denied range");
        }
    }
    None
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
    fn host_port_policy() {
        assert!(host_port_denied("example.com", 22, false).is_some());
        assert!(host_port_denied("example.com", 6379, false).is_some());
        assert!(host_port_denied("example.com", 80, false).is_none());
        assert!(host_port_denied("example.com", 443, false).is_none());
        assert!(host_port_denied("something.onion", 80, false).is_some());
        assert!(host_port_denied("something.onion", 80, true).is_none());
        assert!(host_port_denied("10.0.0.1", 80, false).is_some());
        assert!(host_port_denied("169.254.169.254", 80, false).is_some());
        assert!(host_port_denied("[::1]", 443, false).is_some());
        assert!(host_port_denied("1.1.1.1", 443, false).is_none());
    }
}
