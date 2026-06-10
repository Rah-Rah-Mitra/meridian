//! Embedded Arti backend for the anon lane (SPEC §12.4, ADR-04): bootstrap
//! lifecycle + the [`Dialer`] implementation that maps isolation requests onto
//! `TorClient::connect_with_prefs` with per-key `IsolationToken`s.

use super::socks::{Dialer, Isolation};
use arti_client::config::TorClientConfigBuilder;
use arti_client::isolation::IsolationToken;
use arti_client::{BootstrapBehavior, DataStream, StreamPrefs, TorClient};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A keyed username's token lives this long after last use. Usernames are
/// per-request nonces (in-core path), so this is purely a cleanup horizon.
const TOKEN_TTL: Duration = Duration::from_secs(15 * 60);
/// Hard bound on the username→token map (defense in depth; the listener is
/// loopback/back-network only).
const TOKEN_CAP: usize = 4096;

/// Username → `IsolationToken` mapping (RFC1929-based isolation, SPEC §12.4):
/// one token per distinct credential within the TTL window. Standalone so the
/// §12.5 isolation property is unit-testable without a Tor client.
#[derive(Default)]
pub struct TokenMap {
    inner: Mutex<HashMap<String, (IsolationToken, Instant)>>,
}

impl TokenMap {
    pub fn token_for(&self, key: &str) -> IsolationToken {
        let mut map = self.inner.lock().expect("token map");
        let now = Instant::now();
        if map.len() >= TOKEN_CAP {
            map.retain(|_, (_, last)| now.duration_since(*last) < TOKEN_TTL);
            if map.len() >= TOKEN_CAP {
                // Pathological flood: shed all groupings rather than grow.
                map.clear();
            }
        }
        let entry = map
            .entry(key.to_owned())
            .or_insert_with(|| (IsolationToken::new(), now));
        entry.1 = now;
        entry.0
    }

    pub fn len(&self) -> usize {
        self.inner.lock().expect("token map").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct ArtiDialer {
    client: std::sync::Arc<TorClient<tor_rtcompat::PreferredRuntime>>,
    tokens: TokenMap,
}

impl ArtiDialer {
    pub fn new(client: std::sync::Arc<TorClient<tor_rtcompat::PreferredRuntime>>) -> Self {
        Self {
            client,
            tokens: TokenMap::default(),
        }
    }
}

impl Dialer for ArtiDialer {
    type Stream = DataStream;

    fn ready(&self) -> bool {
        self.client.bootstrap_status().ready_for_traffic()
    }

    async fn dial(
        &self,
        host: &str,
        port: u16,
        isolation: Isolation,
    ) -> Result<Self::Stream, String> {
        let token = match &isolation {
            Isolation::PerConnection => IsolationToken::new(),
            Isolation::Keyed(user) => self.tokens.token_for(user),
        };
        let mut prefs = StreamPrefs::new();
        prefs.set_isolation(token);
        self.client
            .connect_with_prefs((host, port), &prefs)
            .await
            // Error text reduced to a kind: Arti errors can embed the target.
            .map_err(|_| "tor connect failed".to_owned())
    }
}

/// Build an unbootstrapped `TorClient` rooted under `state_root` (the single
/// mutable volume, SPEC §9.7). Must run inside the Tokio runtime.
pub fn build_client(
    state_root: &Path,
) -> Result<std::sync::Arc<TorClient<tor_rtcompat::PreferredRuntime>>, String> {
    // arti-client's rustls path requires the app to install a crypto provider
    // exactly once; ring is the one compiled in (workspace policy).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let state_dir = state_root.join("state");
    let cache_dir = state_root.join("cache");
    std::fs::create_dir_all(&state_dir).map_err(|e| format!("arti state dir: {e}"))?;
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("arti cache dir: {e}"))?;

    let config = TorClientConfigBuilder::from_directories(state_dir, cache_dir)
        .build()
        .map_err(|e| format!("arti config: {e}"))?;

    // OnDemand: a use before bootstrap completes waits (bounded by the caller's
    // deadline) rather than failing spuriously; the lane still gates requests on
    // `ready_for_traffic`, so this only smooths the transition edge.
    TorClient::builder()
        .config(config)
        .bootstrap_behavior(BootstrapBehavior::OnDemand)
        .create_unbootstrapped()
        .map_err(|e| format!("arti client: {e}"))
}

#[cfg(test)]
mod tests {
    use super::TokenMap;

    /// §12.5 isolation property at the mapping layer: one token per distinct
    /// username, stable across reuse of the same username.
    #[test]
    fn distinct_usernames_get_distinct_tokens() {
        let map = TokenMap::default();
        let a1 = map.token_for("iso-1");
        let b = map.token_for("iso-2");
        let a2 = map.token_for("iso-1");
        assert_eq!(map.len(), 2, "two distinct keys → two entries");
        assert_eq!(
            format!("{a1:?}"),
            format!("{a2:?}"),
            "same key → same token"
        );
        assert_ne!(
            format!("{a1:?}"),
            format!("{b:?}"),
            "distinct keys → distinct tokens"
        );
    }
}
