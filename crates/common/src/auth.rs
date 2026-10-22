//! CLI login/session: device-key challenge auth against the central login
//! server, with a locally-cached signed session token that gates which
//! submodules this machine is allowed to run.
//!
//! Login uses a device keypair rather than a password: the CLI signs a
//! server-issued nonce to prove it holds the private half already registered for
//! this user+machine pair. The resulting session token is signed by the server
//! and cached at `~/.config/guss/token.toml` (see `paths::token_file`) so every
//! later command can check it locally, with no network round-trip per command.
//!
//! **The gate fails open until it is configured.** The login server's address and
//! public key used to be compile-time constants, with the key a throwaway whose
//! private half was discarded — so no valid token could ever exist, and every
//! gated command failed on every machine that wasn't the "main server". Both now
//! come from the `[auth]` section of `~/.config/guss/config.toml`; when that
//! section is absent, [`enforce`] allows everything. Adding the section is what
//! turns permissions on, and pointing at a different server no longer needs a new
//! release of the CLI.

use crate::config::{AuthSettings, Settings};
use crate::error::{Context, Result, anyhow};
use crate::paths;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// The `[auth]` settings, or `None` when the gate is not configured.
fn auth_settings() -> Result<Option<AuthSettings>> {
    Ok(Settings::load()?.auth)
}

/// The configured settings, or an actionable error naming the config file.
fn require_auth_settings() -> Result<AuthSettings> {
    match auth_settings()? {
        Some(settings) => Ok(settings),
        None => Err(anyhow!(
            "auth is not configured — add an [auth] section (login_host, server_key) to {}",
            crate::config::config_file()?.display()
        )),
    }
}

/// Decode a base64 ed25519 public key.
fn verifying_key(encoded: &str) -> Result<VerifyingKey> {
    let bytes = B64.decode(encoded).context("public key is not valid base64")?;
    let bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("public key has the wrong length (want 32 bytes)"))?;
    VerifyingKey::from_bytes(&bytes).map_err(|e| anyhow!("not a valid ed25519 public key: {e}"))
}

/// A signed session token cached at `-config/token.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub user: String,
    pub machine_id: String,
    pub issued_at: i64,
    pub expires_at: i64,
    /// Bare module name (`"docs"`) grants that whole crate; a qualified
    /// `"module:qualifier"` entry (`"project:release"`) restricts to just that.
    pub allowed: Vec<String>,
    /// Base64-encoded ed25519 signature over `signing_bytes()`, from the
    /// login server's key.
    pub signature: String,
}

impl Token {
    fn signing_bytes(&self) -> Vec<u8> {
        format!(
            "{}|{}|{}|{}|{}",
            self.user,
            self.machine_id,
            self.issued_at,
            self.expires_at,
            self.allowed.join(",")
        )
        .into_bytes()
    }

    fn verify_signature(&self, server_key: &VerifyingKey) -> Result<()> {
        let sig_bytes = B64.decode(&self.signature).context("cached token signature is not valid base64")?;
        let sig_bytes: [u8; 64] = sig_bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("cached token signature has the wrong length"))?;
        server_key.verify(&self.signing_bytes(), &Signature::from_bytes(&sig_bytes)).map_err(|_| {
            anyhow!("cached token failed signature verification — it may be tampered with, stale, or signed by a login server we don't yet trust")
        })
    }

    fn is_expired(&self) -> bool {
        now_unix() > self.expires_at
    }
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Load an ed25519 signing key from `path`, generating and caching one at 0600 on
/// first use. Shared by the device key here and the release-signing key in
/// `setup`, so the "generate once, keep it 0600" handling exists in one place.
pub fn signing_key_at(path: &Path) -> Result<SigningKey> {
    if path.exists() {
        let bytes = fs::read(path)?;
        let bytes: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("key at {} is corrupt (wrong length)", path.display()))?;
        return Ok(SigningKey::from_bytes(&bytes));
    }

    let key = SigningKey::generate(&mut rand_core::OsRng);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, key.to_bytes())?;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms)?;
    Ok(key)
}

/// Load this machine's device signing key, generating and caching one
/// (0600) at `~/.config/guss/device.key` on first use.
pub fn device_signing_key() -> Result<SigningKey> {
    signing_key_at(&paths::device_key_file()?)
}

/// Base64 of a signing key's public half, for pasting into config or registering
/// with the server.
pub fn public_key_base64(key: &SigningKey) -> String {
    B64.encode(key.verifying_key().to_bytes())
}

#[derive(Deserialize)]
struct ChallengeResponse {
    nonce: String,
}

#[derive(Serialize)]
struct LoginRequest {
    user: String,
    pubkey: String,
    nonce: String,
    signature: String,
}

#[derive(Deserialize)]
struct LoginResponse {
    user: String,
    machine_id: String,
    issued_at: i64,
    expires_at: i64,
    allowed: Vec<String>,
    signature: String,
}

/// Log in as `user`: fetch a challenge, sign it with this machine's device
/// key, submit it, verify and cache the resulting session token.
///
/// Requires an `[auth]` section and network access to its `login_host` — fails
/// with a clear error naming the config file, or "could not reach login server",
/// rather than a bare timeout.
pub fn login(user: &str) -> Result<Token> {
    let auth = require_auth_settings()?;
    let login_host = auth.login_host.as_str();
    let server_key = verifying_key(&auth.server_key)?;
    let signing_key = device_signing_key()?;
    let pubkey_b64 = B64.encode(signing_key.verifying_key().to_bytes());
    let client = reqwest::blocking::Client::new();

    let challenge: ChallengeResponse = client
        .get(format!("{login_host}/auth/challenge"))
        .query(&[("user", user), ("pubkey", &pubkey_b64)])
        .send()
        .with_context(|| format!("could not reach login server at {login_host}"))?
        .error_for_status()
        .with_context(|| format!("login server at {login_host} rejected the challenge request"))?
        .json()
        .context("login server sent a challenge response we couldn't parse")?;

    let nonce_bytes = B64.decode(&challenge.nonce).context("login server sent a malformed nonce")?;
    let signature = signing_key.sign(&nonce_bytes);

    let body = LoginRequest {
        user: user.to_string(),
        pubkey: pubkey_b64,
        nonce: challenge.nonce,
        signature: B64.encode(signature.to_bytes()),
    };

    let resp: LoginResponse = client
        .post(format!("{login_host}/auth/login"))
        .json(&body)
        .send()
        .with_context(|| format!("could not reach login server at {login_host}"))?
        .error_for_status()
        .with_context(|| format!("login server at {login_host} rejected the login request"))?
        .json()
        .context("login server sent a login response we couldn't parse")?;

    let token = Token {
        user: resp.user,
        machine_id: resp.machine_id,
        issued_at: resp.issued_at,
        expires_at: resp.expires_at,
        allowed: resp.allowed,
        signature: resp.signature,
    };
    token
        .verify_signature(&server_key)
        .context("login server response failed signature verification")?;

    save_token(&token)?;
    Ok(token)
}

fn save_token(token: &Token) -> Result<()> {
    let path = paths::token_file()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, toml::to_string_pretty(token)?)?;
    let mut perms = fs::metadata(&path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&path, perms)?;
    Ok(())
}

/// Whether the permission gate is switched on, i.e. whether `[auth]` is present.
pub fn is_configured() -> Result<bool> {
    Ok(auth_settings()?.is_some())
}

/// The cached session token, if present, signature-valid, and not expired.
///
/// Returns `None` when auth is unconfigured: with no server key there is nothing
/// to verify a cached token against, so any token on disk is meaningless.
pub fn cached_token() -> Result<Option<Token>> {
    let Some(auth) = auth_settings()? else {
        return Ok(None);
    };
    let server_key = verifying_key(&auth.server_key)?;
    let path = paths::token_file()?;
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path)?;
    let token: Token = toml::from_str(&contents)?;
    token.verify_signature(&server_key)?;
    if token.is_expired() {
        return Ok(None);
    }
    Ok(Some(token))
}

/// Check whether the current session is allowed to run `permission_key`
/// (e.g. `"docs"` or `"project:release"`).
///
/// **Fails open when unconfigured**: with no `[auth]` section there is no
/// authority to ask, so everything is permitted. Once configured this is an
/// implicit deny — a valid cached token is required, and its `allowed` list must
/// contain either the bare module or the exact qualified key.
///
/// There is intentionally no "main server" bypass any more. It keyed off the mere
/// existence of `db.toml`, which meant dropping a Postgres credentials file onto a
/// machine silently granted it every permission.
pub fn enforce(permission_key: &str) -> Result<()> {
    let auth = auth_settings()?;
    // Only reach for a token when there is an authority to verify it against.
    let token = match &auth {
        Some(_) => cached_token()?,
        None => None,
    };
    decide(auth.as_ref(), token.as_ref(), permission_key)
}

/// The permission decision itself, with both ambient inputs passed in so the
/// policy is testable without a config file or a live login server.
fn decide(auth: Option<&AuthSettings>, token: Option<&Token>, permission_key: &str) -> Result<()> {
    let Some(auth) = auth else {
        return Ok(());
    };
    let token = token.ok_or_else(|| {
        anyhow!(
            "not logged in (or session expired) — run `guss setup login <user>` (requires network to {})",
            auth.login_host
        )
    })?;
    let module = permission_key.split(':').next().unwrap_or(permission_key);
    let allowed = token.allowed.iter().any(|a| a == module || a == permission_key);
    if allowed {
        Ok(())
    } else {
        Err(anyhow!(
            "permission denied: '{permission_key}' is not allowed for user '{}' on this session",
            token.user
        ))
    }
}

/// Clear the cached session token, if any.
pub fn logout() -> Result<()> {
    let path = paths::token_file()?;
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> AuthSettings {
        AuthSettings {
            login_host: "http://login.example".to_string(),
            server_key: "unused-by-decide".to_string(),
        }
    }

    /// A token is only a bag of claims as far as [`decide`] is concerned; the
    /// signature is checked earlier, in `cached_token`.
    fn token(allowed: &[&str]) -> Token {
        Token {
            user: "alice".to_string(),
            machine_id: "m1".to_string(),
            issued_at: 0,
            expires_at: i64::MAX,
            allowed: allowed.iter().map(|s| s.to_string()).collect(),
            signature: String::new(),
        }
    }

    // --- the fail-open property, which is the whole point of Track 1 ---

    #[test]
    fn unconfigured_auth_permits_everything() {
        // No [auth] section: there is no authority to ask, so nothing is denied
        // and no token is required.
        for key in ["stack", "deploy:prod", "project:release", "net"] {
            assert!(decide(None, None, key).is_ok(), "denied {key}");
        }
    }

    #[test]
    fn configured_auth_without_a_token_denies() {
        let err = decide(Some(&auth()), None, "deploy").unwrap_err().to_string();
        assert!(err.contains("not logged in"), "got: {err}");
        // The error must name the configured host, not a compiled-in one.
        assert!(err.contains("http://login.example"), "got: {err}");
    }

    // --- allow-list semantics, unchanged but previously untested ---

    #[test]
    fn bare_module_grant_covers_qualified_keys() {
        let token = token(&["deploy"]);
        assert!(decide(Some(&auth()), Some(&token), "deploy").is_ok());
        assert!(decide(Some(&auth()), Some(&token), "deploy:prod").is_ok());
    }

    #[test]
    fn qualified_grant_does_not_widen_to_the_module() {
        // "project:release" must not imply "project:hotfix".
        let token = token(&["project:release"]);
        assert!(decide(Some(&auth()), Some(&token), "project:release").is_ok());
        assert!(decide(Some(&auth()), Some(&token), "project:hotfix").is_err());
    }

    #[test]
    fn missing_grant_names_the_key_and_user() {
        let token = token(&["docs"]);
        let err = decide(Some(&auth()), Some(&token), "deploy:prod").unwrap_err().to_string();
        assert!(err.contains("permission denied"), "got: {err}");
        assert!(err.contains("deploy:prod"), "got: {err}");
        assert!(err.contains("alice"), "got: {err}");
    }

    // --- key handling and signatures (also the crypto Track 4 reuses) ---

    #[test]
    fn verifying_key_round_trips_through_base64() {
        let key = SigningKey::generate(&mut rand_core::OsRng);
        let encoded = public_key_base64(&key);

        let decoded = verifying_key(&encoded).unwrap();
        assert_eq!(decoded.to_bytes(), key.verifying_key().to_bytes());
    }

    #[test]
    fn verifying_key_rejects_junk() {
        assert!(verifying_key("not base64!!!").is_err());
        // Valid base64, wrong length.
        assert!(verifying_key("AAAA").is_err());
    }

    #[test]
    fn token_signature_verifies_and_detects_tampering() {
        let server = SigningKey::generate(&mut rand_core::OsRng);
        let mut t = token(&["docs"]);
        t.signature = B64.encode(server.sign(&t.signing_bytes()).to_bytes());

        assert!(t.verify_signature(&server.verifying_key()).is_ok());

        // Escalating the allow-list must invalidate the signature.
        let mut tampered = token(&["docs", "deploy"]);
        tampered.signature = t.signature.clone();
        assert!(tampered.verify_signature(&server.verifying_key()).is_err());

        // A signature from a different key must not verify.
        let other = SigningKey::generate(&mut rand_core::OsRng);
        assert!(t.verify_signature(&other.verifying_key()).is_err());
    }

    #[test]
    fn expiry_is_checked_against_the_clock() {
        let mut t = token(&["docs"]);
        assert!(!t.is_expired());
        t.expires_at = 0;
        assert!(t.is_expired());
    }

    #[test]
    fn signing_key_at_generates_then_reloads_the_same_key() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
        let dir = std::env::temp_dir().join(format!("guss_auth_key_{nanos}"));
        let path = dir.join("nested/test.key");

        let first = signing_key_at(&path).unwrap();
        let second = signing_key_at(&path).unwrap();
        assert_eq!(first.to_bytes(), second.to_bytes());

        // Private keys must never be group/world readable.
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);

        let _ = fs::remove_dir_all(&dir);
    }
}
