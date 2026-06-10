//! Secret material handling (SPEC §13.5): everything sensitive lives in
//! `secrecy::SecretString` (zeroized on drop, `Debug` prints `[REDACTED]`).
//! Secrets load from env or a 0600 file; they are never serialized, never in
//! errors, never in `/metrics`.

use meridian_common::config::AuthConfig;
pub use secrecy::{ExposeSecret, SecretString};
use std::os::unix::fs::PermissionsExt;

/// Load the bearer token per the auth config: file (must be 0600) wins over env.
/// `Ok(None)` means no token configured — the API then refuses mutating
/// endpoints outright rather than running open.
pub fn load_bearer_token(cfg: &AuthConfig) -> Result<Option<SecretString>, String> {
    if let Some(path) = &cfg.token_file {
        let meta =
            std::fs::metadata(path).map_err(|e| format!("token file {}: {e}", path.display()))?;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(format!(
                "token file {} is mode {mode:o}; refusing anything but owner-only (0600)",
                path.display()
            ));
        }
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("token file {}: {e}", path.display()))?;
        let token = raw.trim().to_owned();
        if token.is_empty() {
            return Err(format!("token file {} is empty", path.display()));
        }
        return Ok(Some(SecretString::from(token)));
    }
    match std::env::var(&cfg.token_env) {
        Ok(v) if !v.trim().is_empty() => Ok(Some(SecretString::from(v.trim().to_owned()))),
        _ => Ok(None),
    }
}

/// Constant-time bearer comparison (avoids timing-oracle token recovery).
pub fn token_matches(expected: &SecretString, presented: &str) -> bool {
    let expected = expected.expose_secret().as_bytes();
    let presented = presented.as_bytes();
    if expected.len() != presented.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in expected.iter().zip(presented) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_readable_token_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("meridian-secret-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("token");
        std::fs::write(&path, "s3cret\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let cfg = AuthConfig {
            token_file: Some(path.clone()),
            ..Default::default()
        };
        assert!(
            load_bearer_token(&cfg).is_err(),
            "0644 token file must be refused"
        );

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let token = load_bearer_token(&cfg).unwrap().unwrap();
        assert!(token_matches(&token, "s3cret"));
        assert!(!token_matches(&token, "s3cret2"));
        assert!(!token_matches(&token, "s3crea"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn secret_debug_never_prints_value() {
        let token = SecretString::from("super-secret-token".to_owned());
        assert!(!format!("{token:?}").contains("super-secret"));
    }
}
