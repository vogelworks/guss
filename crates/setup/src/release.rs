//! Signed release distribution from the central server.
//!
//! Deliberately **not** GitHub Releases: the artifacts are served from the
//! internal host named in `[release]`, so no hostname is compiled in. The tarball
//! is fetched over plain HTTP today, which means a checksum alone proves nothing —
//! an on-path attacker rewrites the manifest and the checksum together. Integrity
//! therefore comes from an ed25519 signature, reusing the same crypto already
//! vendored for device auth (`vhp_common::auth`).
//!
//! Both the tarball **and the manifest** are signed. Signing only the tarball
//! would leave a rollback attack: replay an older, validly-signed release. The
//! manifest carries the version, so signing it closes that, and [`update`] also
//! refuses a downgrade unless an explicit `--tag` asks for one.
//!
//! Verification order in [`update`] is load-bearing: manifest signature →
//! download → sha256 → tarball signature → *only then* unpack. Nothing is
//! extracted before it has been verified.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use clap::ValueEnum;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::Path;

use vhp_common::config::{ReleaseSettings, Settings};
use vhp_common::error::{Context, Result, anyhow};
use vhp_common::{auth, git, paths, proc, ui, version};

/// The only target published today. Static musl: the dependency tree is
/// `ring` + `rustls` with no OpenSSL and no libpq, so one binary runs on any
/// glibc version the team happens to have.
pub const TARGET: &str = "x86_64-unknown-linux-musl";

/// Which set of artifacts to read or write. Separate channels exist so a WIP
/// publish cannot update everyone's binary.
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum Channel {
    Dev,
    Prod,
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Channel::Dev => "dev",
            Channel::Prod => "prod",
        })
    }
}

/// What a channel currently offers. Serialized once and signed over those exact
/// bytes — never re-serialized for verification, since formatting differences
/// would break the signature.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub version: String,
    /// Tarball filename, resolved against the channel directory.
    pub file: String,
    /// Lowercase hex sha256 of the tarball.
    pub sha256: String,
    pub target: String,
}

impl Manifest {
    fn tarball_name(version: &str) -> String {
        format!("vhp-{version}-{TARGET}.tar.gz")
    }
}

fn require_settings() -> Result<ReleaseSettings> {
    let config_path = vhp_common::config::config_file()?;
    Settings::load()?.release.ok_or_else(|| {
        anyhow!(
            "no [release] section in {} — set base_url, ssh_host, remote_dir",
            config_path.display()
        )
    })
}

/// The key releases are verified against.
///
/// Config-only, on purpose. Shipping a compiled-in default would mean either a
/// placeholder that can never verify — the exact trap `SERVER_PUBLIC_KEY_BYTES`
/// fell into — or baking in a key that cannot then be rotated without a new
/// release. `setup keygen` prints the block to paste here.
fn verifying_key(settings: &ReleaseSettings) -> Result<VerifyingKey> {
    let config_path = vhp_common::config::config_file()?;
    let encoded = settings.public_key.as_deref().ok_or_else(|| {
        anyhow!(
            "no [release] public_key in {} — run `vhp setup keygen` on the publishing \
             machine and paste the key it prints",
            config_path.display()
        )
    })?;
    let bytes = B64
        .decode(encoded)
        .context("[release] public_key is not valid base64")?;
    let bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("[release] public_key has the wrong length (want 32 bytes)"))?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|e| anyhow!("[release] public_key is not a valid ed25519 key: {e}"))
}

/// Sign `bytes`, returning a base64 detached signature.
fn sign(key: &SigningKey, bytes: &[u8]) -> String {
    B64.encode(key.sign(bytes).to_bytes())
}

/// Verify a base64 detached signature over `bytes`. `what` names the artifact so
/// a failure says which check failed.
fn verify(key: &VerifyingKey, bytes: &[u8], signature: &str, what: &str) -> Result<()> {
    let sig = B64
        .decode(signature.trim())
        .with_context(|| format!("{what} signature is not valid base64"))?;
    let sig: [u8; 64] = sig
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("{what} signature has the wrong length"))?;
    key.verify(bytes, &Signature::from_bytes(&sig))
        .map_err(|_| anyhow!("{what} failed signature verification — refusing to continue"))
}


/// sha256 of a file, via `sha256sum` — avoids pulling a hashing crate into the
/// dependency tree for one call site.
fn sha256_file(path: &Path) -> Result<String> {
    let path_s = path.to_string_lossy().to_string();
    let out = proc::capture("sha256sum", &[&path_s])?;
    out.split_whitespace()
        .next()
        .map(|s| s.to_lowercase())
        .ok_or_else(|| anyhow!("could not parse sha256sum output for {}", path.display()))
}

// --- keygen -----------------------------------------------------------------

/// Generate (or show) this machine's release-signing key and print the config
/// block that lets other machines verify what it publishes.
pub fn keygen(force: bool, yes: bool) -> Result<()> {
    let path = paths::release_key_file()?;
    ui::heading("release signing key");

    if path.exists() && force {
        ui::warn(&format!(
            "  {} already exists — replacing it invalidates every published signature",
            path.display()
        ));
        ui::require_confirm("Overwrite the existing release key?", yes)?;
        fs::remove_file(&path)?;
    }

    let existed = path.exists();
    // Reuses auth's generate-or-load helper, which already handles 0600.
    let key = auth::signing_key_at(&path)?;
    let public = auth::public_key_base64(&key);

    if existed {
        ui::info(&format!("  using existing key at {}", path.display()));
    } else {
        ui::good(&format!("  wrote a new key to {} (0600)", path.display()));
    }
    ui::section();
    println!("Add this to ~/.config/vhp/config.toml on every machine that runs");
    println!("`vhp setup update` (and to install.sh on the server):\n");
    println!("[release]");
    println!("public_key = \"{public}\"");
    ui::section();
    Ok(())
}

// --- publish ----------------------------------------------------------------

/// Build, package, sign, and upload a release.
pub fn publish(channel: Channel, yes: bool, dry_run: bool) -> Result<()> {
    let settings = require_settings()?;
    let key_path = paths::release_key_file()?;
    if !key_path.exists() {
        return Err(anyhow!(
            "no release key at {} — run `vhp setup keygen` first",
            key_path.display()
        ));
    }
    let version = env!("CARGO_PKG_VERSION").to_string();
    let tools = paths::tools_dir()?;
    let root = paths::project_vhp()?;

    ui::heading(&format!("publish vhp {version} -> {channel}"));
    println!(
        "  target  -> {TARGET}\n  host    -> {}\n  dir     -> {}/{channel}",
        settings.ssh_host, settings.remote_dir
    );

    // Guards that CI would otherwise have enforced. Without them the artifact
    // cannot be traced back to a commit.
    if !git::is_clean(&root) {
        return Err(anyhow!(
            "{} has uncommitted changes — commit or stash before publishing",
            root.display()
        ));
    }
    check_tag(&root, &version)?;

    ui::require_confirm("Build, sign, and upload this release?", yes)?;

    proc::run_in(
        &tools,
        "cargo",
        &["build", "--release", "--locked", "--target", TARGET],
        dry_run,
    )?;

    let staging = tools.join("target/release-staging");
    let tarball_name = Manifest::tarball_name(&version);
    let tarball = staging.join(&tarball_name);
    let binary = tools
        .join("target")
        .join(TARGET)
        .join("release")
        .join("vhp");

    println!("  package {}", tarball.display());
    if !dry_run {
        fs::create_dir_all(&staging)?;
        if !binary.is_file() {
            return Err(anyhow!("no binary at {}", binary.display()));
        }
        let binary_dir = binary.parent().unwrap().to_string_lossy().to_string();
        let tarball_s = tarball.to_string_lossy().to_string();
        // -C so the archive holds a bare `vhp`, not the full target path.
        proc::run(
            "tar",
            &["czf", &tarball_s, "-C", &binary_dir, "vhp"],
            false,
        )?;
    }

    let (manifest_bytes, manifest_sig, tarball_sig) = if dry_run {
        println!("  sign    {tarball_name} + manifest.toml");
        (Vec::new(), String::new(), String::new())
    } else {
        let key = auth::signing_key_at(&key_path)?;
        let sha256 = sha256_file(&tarball)?;
        let manifest = Manifest {
            version: version.clone(),
            file: tarball_name.clone(),
            sha256,
            target: TARGET.to_string(),
        };
        // Serialize exactly once; these are the bytes that get signed, uploaded,
        // and later verified.
        let bytes = toml::to_string_pretty(&manifest)
            .map_err(|e| anyhow!("could not serialize manifest: {e}"))?
            .into_bytes();
        let tarball_bytes = fs::read(&tarball)?;
        let tarball_sig = sign(&key, &tarball_bytes);
        let manifest_sig = sign(&key, &bytes);
        (bytes, manifest_sig, tarball_sig)
    };

    // Write the four artifacts locally, then upload them together.
    let manifest_path = staging.join("manifest.toml");
    let manifest_sig_path = staging.join("manifest.toml.sig");
    let tarball_sig_path = staging.join(format!("{tarball_name}.sig"));
    if !dry_run {
        fs::write(&manifest_path, &manifest_bytes)?;
        fs::write(&manifest_sig_path, &manifest_sig)?;
        fs::write(&tarball_sig_path, &tarball_sig)?;
    }

    let remote = format!("{}:{}/{channel}/", settings.ssh_host, settings.remote_dir);
    let mkdir = format!("mkdir -p {}/{channel}", settings.remote_dir);
    proc::run("ssh", &[&settings.ssh_host, &mkdir], dry_run)?;
    for path in [
        &tarball,
        &tarball_sig_path,
        &manifest_path,
        &manifest_sig_path,
    ] {
        let path_s = path.to_string_lossy().to_string();
        proc::run("scp", &[&path_s, &remote], dry_run)?;
    }

    ui::good(&format!("published {version} to {channel}"));
    ui::info(&format!(
        "  clients fetch {}/{channel}/manifest.toml",
        settings.base_url
    ));
    Ok(())
}

/// Refuse to publish a version that HEAD is not tagged for, so a tarball always
/// corresponds to a named commit.
fn check_tag(root: &Path, version: &str) -> Result<()> {
    let tags = proc::capture_optional("git", &["-C", &root.to_string_lossy(), "tag", "--points-at", "HEAD"]);
    let wanted = format!("v{version}");
    if tags.lines().any(|t| t.trim() == wanted || t.trim() == version) {
        return Ok(());
    }
    Err(anyhow!(
        "HEAD is not tagged {wanted} (found: {}) — cut the release first, \
         e.g. `vhp project release finish {version}`",
        if tags.trim().is_empty() {
            "no tags".to_string()
        } else {
            tags.lines().collect::<Vec<_>>().join(", ")
        }
    ))
}

// --- update -----------------------------------------------------------------

/// Fetch, verify, and install a release.
pub fn update(
    channel: Channel,
    tag: Option<&str>,
    prefix: &Path,
    yes: bool,
    dry_run: bool,
) -> Result<()> {
    let settings = require_settings()?;
    let key = verifying_key(&settings)?;
    let current = env!("CARGO_PKG_VERSION");
    let base = format!("{}/{channel}", settings.base_url.trim_end_matches('/'));

    ui::heading(&format!("update vhp ({channel})"));
    println!("  running -> {current}\n  source  -> {base}");

    if dry_run {
        println!("  GET {base}/manifest.toml (+ .sig)");
        println!("  GET {base}/<tarball> (+ .sig), verify, install into {}", prefix.display());
        return Ok(());
    }

    let client = reqwest::blocking::Client::new();

    // 1. Manifest and its signature. Verify before parsing, and verify over the
    //    exact bytes received.
    let manifest_bytes = fetch(&client, &format!("{base}/manifest.toml"))?;
    let manifest_sig = String::from_utf8_lossy(&fetch(
        &client,
        &format!("{base}/manifest.toml.sig"),
    )?)
    .to_string();
    verify(&key, &manifest_bytes, &manifest_sig, "manifest")?;

    let manifest: Manifest = std::str::from_utf8(&manifest_bytes)
        .map_err(|e| anyhow!("manifest is signed but not valid UTF-8: {e}"))
        .and_then(|text| {
            toml::from_str(text).map_err(|e| anyhow!("manifest is signed but unparseable: {e}"))
        })?;

    if manifest.target != TARGET {
        return Err(anyhow!(
            "published release targets {}, this build expects {TARGET}",
            manifest.target
        ));
    }

    // 2. Version policy. An explicit --tag is the only way to move sideways or
    //    backwards, so a rolled-back manifest cannot quietly downgrade anyone.
    match tag {
        Some(wanted) if wanted.trim_start_matches('v') != manifest.version => {
            return Err(anyhow!(
                "{channel} currently offers {}, not {wanted}",
                manifest.version
            ));
        }
        Some(_) => {}
        None => {
            if !version::is_newer(&manifest.version, current) {
                ui::good(&format!(
                    "already up to date ({current}; {channel} offers {})",
                    manifest.version
                ));
                return Ok(());
            }
        }
    }

    ui::info(&format!("  available -> {}", manifest.version));
    ui::require_confirm(
        &format!("Install {} over {current}?", manifest.version),
        yes,
    )?;

    // 3. Tarball, checksum, signature — in that order, before anything is unpacked.
    let staging = std::env::temp_dir().join(format!("vhp-update-{}", manifest.version));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let tarball = staging.join(&manifest.file);

    let tarball_bytes = fetch(&client, &format!("{base}/{}", manifest.file))?;
    fs::write(&tarball, &tarball_bytes)?;

    let actual = sha256_file(&tarball)?;
    if actual != manifest.sha256.to_lowercase() {
        return Err(anyhow!(
            "sha256 mismatch: manifest says {}, download is {actual}",
            manifest.sha256
        ));
    }

    let tarball_sig =
        String::from_utf8_lossy(&fetch(&client, &format!("{base}/{}.sig", manifest.file))?)
            .to_string();
    verify(&key, &tarball_bytes, &tarball_sig, "tarball")?;
    ui::good("  verified checksum and signature");

    // 4. Only now is it safe to unpack.
    let tarball_s = tarball.to_string_lossy().to_string();
    let staging_s = staging.to_string_lossy().to_string();
    proc::run("tar", &["xzf", &tarball_s, "-C", &staging_s], false)?;

    crate::install_binary(&staging.join("vhp"), &prefix.join("vhp"), false)?;
    let _ = fs::remove_dir_all(&staging);

    ui::good(&format!("updated to {}", manifest.version));
    Ok(())
}

/// GET a URL, returning its bytes, with the URL in any error.
fn fetch(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>> {
    let resp = client
        .get(url)
        .send()
        .with_context(|| format!("could not reach {url}"))?
        .error_for_status()
        .with_context(|| format!("{url} returned an error status"))?;
    Ok(resp
        .bytes()
        .with_context(|| format!("could not read the body of {url}"))?
        .to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keypair() -> (SigningKey, VerifyingKey) {
        let key = SigningKey::generate(&mut rand_core::OsRng);
        (key.clone(), key.verifying_key())
    }

    fn manifest() -> Manifest {
        Manifest {
            version: "0.2.0".to_string(),
            file: Manifest::tarball_name("0.2.0"),
            sha256: "abc123".to_string(),
            target: TARGET.to_string(),
        }
    }

    // --- signing ---

    #[test]
    fn sign_then_verify_round_trips() {
        let (signing, verifying) = keypair();
        let bytes = b"release artifact";
        let sig = sign(&signing, bytes);
        assert!(verify(&verifying, bytes, &sig, "tarball").is_ok());
    }

    #[test]
    fn verify_rejects_tampered_bytes() {
        let (signing, verifying) = keypair();
        let sig = sign(&signing, b"original");
        let err = verify(&verifying, b"modified", &sig, "tarball")
            .unwrap_err()
            .to_string();
        assert!(err.contains("tarball"), "got: {err}");
        assert!(err.contains("signature verification"), "got: {err}");
    }

    #[test]
    fn verify_rejects_a_signature_from_another_key() {
        let (signing, _) = keypair();
        let (_, other) = keypair();
        let sig = sign(&signing, b"artifact");
        assert!(verify(&other, b"artifact", &sig, "manifest").is_err());
    }

    #[test]
    fn verify_rejects_malformed_signatures() {
        let (_, verifying) = keypair();
        assert!(verify(&verifying, b"x", "not base64!!!", "manifest").is_err());
        // Valid base64, wrong length.
        assert!(verify(&verifying, b"x", "AAAA", "manifest").is_err());
    }

    #[test]
    fn a_tampered_manifest_does_not_verify() {
        // The rollback defence: the manifest carries the version, so editing the
        // version invalidates its signature.
        let (signing, verifying) = keypair();
        let original = toml::to_string_pretty(&manifest()).unwrap().into_bytes();
        let sig = sign(&signing, &original);
        assert!(verify(&verifying, &original, &sig, "manifest").is_ok());

        let mut downgraded = manifest();
        downgraded.version = "0.1.0".to_string();
        let edited = toml::to_string_pretty(&downgraded).unwrap().into_bytes();
        assert!(verify(&verifying, &edited, &sig, "manifest").is_err());
    }

    #[test]
    fn manifest_bytes_round_trip() {
        let bytes = toml::to_string_pretty(&manifest()).unwrap().into_bytes();
        let parsed: Manifest = toml::from_str(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(parsed, manifest());
    }

    // --- version policy ---

    #[test]
    fn version_parsing_accepts_v_prefix_and_short_forms() {
        assert_eq!(version::parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(version::parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(version::parse_version("2"), Some((2, 0, 0)));
        assert_eq!(version::parse_version("2.1"), Some((2, 1, 0)));
        assert_eq!(version::parse_version("1.2.3.4"), None);
        assert_eq!(version::parse_version("1.2.3-rc1"), None);
        assert_eq!(version::parse_version("garbage"), None);
    }

    #[test]
    fn newer_compares_numerically_not_lexically() {
        assert!(version::is_newer("0.10.0", "0.9.0"), "0.10.0 must beat 0.9.0");
        assert!(version::is_newer("1.0.0", "0.99.99"));
        assert!(version::is_newer("0.1.1", "0.1.0"));
    }

    #[test]
    fn equal_or_older_is_not_newer() {
        assert!(!version::is_newer("0.1.0", "0.1.0"));
        assert!(!version::is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn unparseable_versions_are_never_newer() {
        // Refusing to guess means the user must pass --tag explicitly.
        assert!(!version::is_newer("weird", "0.1.0"));
        assert!(!version::is_newer("0.2.0", "weird"));
    }

    // --- naming ---

    #[test]
    fn tarball_name_includes_version_and_target() {
        let name = Manifest::tarball_name("0.2.0");
        assert_eq!(name, format!("vhp-0.2.0-{TARGET}.tar.gz"));
    }

    #[test]
    fn channel_renders_as_its_directory_name() {
        assert_eq!(Channel::Dev.to_string(), "dev");
        assert_eq!(Channel::Prod.to_string(), "prod");
    }
}
