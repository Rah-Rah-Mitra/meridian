//! Salted, rotating client-IP hashing for rate-limit keys ONLY (SPEC §13.4).
//!
//! `key = blake3::keyed_hash(daily_salt, ip_bytes)` truncated to 8 bytes. The
//! salt is random per process start and rotates every 24h, so keys cannot be
//! reversed to IPs or correlated across days/restarts. The raw IP exists in
//! memory only for the duration of the request; it is never stored or logged.

use std::net::IpAddr;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use zeroize::Zeroize;

const ROTATION: Duration = Duration::from_secs(24 * 60 * 60);

/// 8-byte opaque rate-limit key. Deliberately NOT Display/Serialize — it exists
/// to key an in-memory governor map and nothing else.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RateKey([u8; 8]);

struct Salt {
    key: [u8; 32],
    created: Instant,
}

impl Drop for Salt {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

/// Process-wide hasher; cheap to share behind an `Arc`.
pub struct IpHasher {
    salt: RwLock<Salt>,
}

impl IpHasher {
    pub fn new() -> Self {
        Self {
            salt: RwLock::new(Self::fresh_salt()),
        }
    }

    fn fresh_salt() -> Salt {
        let mut key = [0u8; 32];
        // getrandom failure means the OS RNG is broken — refusing to start is
        // the only safe behavior for security-bearing randomness.
        getrandom::fill(&mut key).expect("OS RNG unavailable");
        Salt {
            key,
            created: Instant::now(),
        }
    }

    /// Hash a client IP into a rate-limit key, rotating the salt if stale.
    pub fn key(&self, ip: IpAddr) -> RateKey {
        {
            let salt = self.salt.read().expect("salt lock");
            if salt.created.elapsed() < ROTATION {
                return Self::derive(&salt.key, ip);
            }
        }
        let mut salt = self.salt.write().expect("salt lock");
        if salt.created.elapsed() >= ROTATION {
            *salt = Self::fresh_salt();
        }
        Self::derive(&salt.key, ip)
    }

    fn derive(salt: &[u8; 32], ip: IpAddr) -> RateKey {
        let bytes = match ip {
            IpAddr::V4(v4) => v4.octets().to_vec(),
            IpAddr::V6(v6) => v6.octets().to_vec(),
        };
        let digest = blake3::keyed_hash(salt, &bytes);
        let mut key = [0u8; 8];
        key.copy_from_slice(&digest.as_bytes()[..8]);
        RateKey(key)
    }
}

impl Default for IpHasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_within_a_salt_distinct_across_processes() {
        let h = IpHasher::new();
        let ip: IpAddr = "203.0.113.9".parse().unwrap();
        // RateKey has no Debug (deliberately — it must never be formatted), so
        // compare with plain asserts.
        assert!(h.key(ip) == h.key(ip), "same salt must give a stable key");
        assert!(h.key(ip) != h.key("203.0.113.10".parse().unwrap()));

        // A different hasher (≈ restart) must not correlate.
        let h2 = IpHasher::new();
        assert!(
            h.key(ip) != h2.key(ip),
            "keys must not survive a salt change"
        );
    }

    #[test]
    fn key_never_embeds_the_ip_bytes() {
        let h = IpHasher::new();
        let ip: IpAddr = "10.1.2.3".parse().unwrap();
        let RateKey(key) = h.key(ip);
        assert!(!key.windows(4).any(|w| w == [10, 1, 2, 3]));
    }
}
