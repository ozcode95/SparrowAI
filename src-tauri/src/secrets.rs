//! OS-keychain backed secret storage with a plaintext-file fallback.
//!
//! Why both? The `keyring` crate talks to the platform credential store
//! (Windows Credential Manager, macOS Keychain, Linux Secret Service). On a
//! well-configured desktop that's exactly what we want — the OS handles
//! locking, biometrics, and per-user isolation. On headless Linux, in some
//! containers, or in CI the daemon simply isn't running, and `keyring`
//! returns a `PlatformFailure` / `NoStorageAccess`. Rather than refuse to
//! work, we transparently fall back to a 0600-mode file under the app's
//! local directory so the existing behaviour is preserved.
//!
//! Whenever the fallback is touched we emit a `warn!` log so an operator
//! who *expected* keychain-only behaviour can spot the regression.
//!
//! ```text
//! set(key, val):  keychain ── on err ──▶ <root>/secrets/<key>
//! get(key):       keychain ── if None ──▶ <root>/secrets/<key>
//! delete(key):    keychain + remove file (both best-effort)
//! ```
//!
//! Keys are namespaced with [`SERVICE`] so multiple zero installs on the
//! same machine don't collide with each other or with other apps.

use crate::paths;
use anyhow::{Context, Result};
use std::path::PathBuf;

/// Keychain "service" identifier. Combined with the username (the secret key
/// we pass to `keyring::Entry`) into a single platform-level credential.
const SERVICE: &str = "zero";

/// Look up a secret by key. Returns `Ok(None)` when neither the keychain nor
/// the fallback file has anything for that key. Bubbles up only on real
/// I/O / decoding errors — a missing entry is not an error.
pub fn get(key: &str) -> Result<Option<String>> {
    validate_key(key)?;
    match keyring_entry(key).and_then(|e| match e.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }) {
        Ok(Some(v)) => Ok(Some(v)),
        Ok(None) => fallback_read(key),
        Err(e) => {
            tracing::warn!(
                "keychain unavailable for `{key}` ({e:#}); falling back to plaintext file"
            );
            fallback_read(key)
        }
    }
}

/// Store `value` under `key`. Tries the keychain first; on failure writes to
/// the fallback file so the secret is still usable. Returns the storage
/// backend that ended up holding the value so callers can surface a "this is
/// in a less secure location" warning in the UI if they want.
pub fn set(key: &str, value: &str) -> Result<Backend> {
    validate_key(key)?;
    match keyring_entry(key).and_then(|e| e.set_password(value).map_err(Into::into)) {
        Ok(()) => {
            // If we previously wrote a fallback file, scrub it now that the
            // keychain is happy again — otherwise an attacker reading the
            // file would still get a valid (if stale) token.
            let _ = fallback_delete(key);
            Ok(Backend::Keychain)
        }
        Err(e) => {
            tracing::warn!(
                "keychain unavailable for `{key}` ({e:#}); falling back to plaintext file"
            );
            fallback_write(key, value)?;
            Ok(Backend::FallbackFile)
        }
    }
}

/// Best-effort delete. Returns Ok even if the entry was never present.
pub fn delete(key: &str) -> Result<()> {
    validate_key(key)?;
    if let Ok(entry) = keyring_entry(key) {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => tracing::warn!("keychain delete `{key}` failed: {e:#}"),
        }
    }
    let _ = fallback_delete(key);
    Ok(())
}

/// Where a secret physically ended up. Useful for "stored in keychain" vs.
/// "stored in plaintext fallback" UI hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Keychain,
    FallbackFile,
}

fn keyring_entry(key: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, key)
        .with_context(|| format!("open keyring entry {SERVICE}/{key}"))
}

/// Reject keys that could escape the secrets subdir on the fallback path or
/// that would look surprising in keychain UIs. Conservative: ascii letters,
/// digits, and `_`/`-` only — enough for every secret we currently store.
fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() {
        anyhow::bail!("empty secret key");
    }
    if !key
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        anyhow::bail!("invalid secret key: {key:?}");
    }
    Ok(())
}

fn fallback_path(key: &str) -> Result<PathBuf> {
    // Belt-and-braces: `validate_key` already runs at every entry point, but
    // we re-check here so a future internal caller can't bypass it.
    validate_key(key)?;
    let dir = paths::root()?.join("secrets");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(key))
}

fn fallback_read(key: &str) -> Result<Option<String>> {
    let p = fallback_path(key)?;
    if !p.exists() {
        // Also probe the very first fallback location we used (a flat
        // `<root>/hf_token`) for backward compatibility with users who
        // saved their token before this module existed. Only `hf_token`
        // ever lived there.
        if key == "hf_token" {
            if let Ok(s) = std::fs::read_to_string(paths::root()?.join("hf_token")) {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Ok(Some(trimmed.to_string()));
                }
            }
        }
        return Ok(None);
    }
    let bytes = std::fs::read_to_string(&p)?;
    let trimmed = bytes.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

fn fallback_write(key: &str, value: &str) -> Result<()> {
    let p = fallback_path(key)?;
    std::fs::write(&p, value)?;
    // Best-effort: make the file owner-only on POSIX. Windows ACLs already
    // restrict the user's AppData directory by default.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn fallback_delete(key: &str) -> Result<()> {
    let p = fallback_path(key)?;
    if p.exists() {
        let _ = std::fs::remove_file(&p);
    }
    // Legacy location, only for hf_token.
    if key == "hf_token" {
        let legacy = paths::root()?.join("hf_token");
        if legacy.exists() {
            let _ = std::fs::remove_file(legacy);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Use a key that's namespaced with the test name so concurrent test runs
    // (or other apps) don't collide on a real machine.
    fn k() -> String {
        format!("zero_test_{}", uuid::Uuid::new_v4().simple())
    }

    #[test]
    fn round_trip_via_some_backend() {
        let key = k();
        let backend = set(&key, "hello").expect("set ok");
        let got = get(&key).expect("get ok");
        assert_eq!(got.as_deref(), Some("hello"));
        delete(&key).unwrap();
        assert_eq!(get(&key).unwrap(), None);
        // Either real keychain or fallback — both are valid outcomes.
        assert!(matches!(backend, Backend::Keychain | Backend::FallbackFile));
    }

    #[test]
    fn unknown_key_returns_none() {
        assert_eq!(get(&k()).unwrap(), None);
    }

    #[test]
    fn invalid_key_is_rejected() {
        // `.` would let an attacker target arbitrary files in the secrets
        // dir; explicitly check we refuse.
        assert!(set("../etc/passwd", "x").is_err());
        assert!(set("with/slash", "x").is_err());
    }
}
