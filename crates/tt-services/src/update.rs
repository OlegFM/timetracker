//! Disabled-by-default updater: manifest check and signed-installer verification.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, ServiceError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateConfig {
    pub manifest_url: String,
    pub public_key: [u8; 32],
}

/// The release manifest. The whole file is signed (`manifest.json.sig`), so the installer
/// needs only its checksum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub url: String,
    /// Lowercase hex SHA-256 of the installer.
    pub sha256: String,
    /// Release page shown to the user.
    pub page: String,
}

fn parse_update_config(url: Option<&str>, key_hex: Option<&str>) -> Option<UpdateConfig> {
    let url = url.map(str::trim).filter(|u| u.starts_with("https://"))?;
    let public_key: [u8; 32] = hex::decode(key_hex?.trim()).ok()?.try_into().ok()?;
    Some(UpdateConfig {
        manifest_url: url.to_owned(),
        public_key,
    })
}

/// `None` unless both build-time variables are set and valid. Always `None` outside
/// Windows: the update downloads and runs the Windows installer.
pub fn update_config() -> Option<UpdateConfig> {
    if cfg!(not(windows)) {
        return None;
    }
    parse_update_config(
        option_env!("TT_UPDATE_URL"),
        option_env!("TT_UPDATE_PUBKEY"),
    )
}

/// `[v]X(.Y)*[-pre]`: numeric core (missing parts are 0 when comparing) and optional
/// non-empty dot-separated pre-release identifiers.
struct Version {
    core: Vec<u64>,
    pre: Option<Vec<String>>,
}

fn parse_version(v: &str) -> Option<Version> {
    let v = v.strip_prefix('v').unwrap_or(v);
    let (core, pre) = match v.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (v, None),
    };
    let core = core
        .split('.')
        .map(|p| p.parse().ok())
        .collect::<Option<Vec<u64>>>()?;
    let pre = match pre {
        None => None,
        Some(p) => Some(
            p.split('.')
                .map(|id| (!id.is_empty()).then(|| id.to_owned()))
                .collect::<Option<Vec<String>>>()?,
        ),
    };
    Some(Version { core, pre })
}

fn cmp_pre_id(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        (Err(_), Err(_)) => a.cmp(b),
    }
}

fn cmp_versions(a: &Version, b: &Version) -> std::cmp::Ordering {
    let n = a.core.len().max(b.core.len());
    let at = |v: &Version, i: usize| v.core.get(i).copied().unwrap_or(0);
    for i in 0..n {
        let o = at(a, i).cmp(&at(b, i));
        if o.is_ne() {
            return o;
        }
    }
    match (&a.pre, &b.pre) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(x), Some(y)) => {
            for (p, q) in x.iter().zip(y) {
                let o = cmp_pre_id(p, q);
                if o.is_ne() {
                    return o;
                }
            }
            x.len().cmp(&y.len())
        }
    }
}

/// True when `latest` is strictly newer than `current`; unparsable input is never newer.
pub fn is_newer(current: &str, latest: &str) -> bool {
    let (Some(a), Some(b)) = (parse_version(current), parse_version(latest)) else {
        return false;
    };
    cmp_versions(&b, &a).is_gt()
}

/// The version this build reports: `TT_BUILD_VERSION` (set by CI from the tag), else the crate's.
pub fn current_version() -> &'static str {
    option_env!("TT_BUILD_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// A release tag (`vX.Y.Z[-pre]`) must carry the crate version as its core.
pub fn tag_matches_version(tag: &str, cargo_version: &str) -> bool {
    let Some(rest) = tag.strip_prefix('v') else {
        return false;
    };
    parse_version(rest).is_some() && rest.split('-').next() == Some(cargo_version)
}

/// Budget for connecting and for the response headers (and the small manifest body).
const SHORT_TIMEOUT: Duration = Duration::from_secs(10);
/// Budget for reading the installer body (slow connections).
const INSTALLER_BODY_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// HTTPS only (also after redirects); `body` limits reading the response body.
fn agent(body: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .timeout_connect(Some(SHORT_TIMEOUT))
        .timeout_recv_response(Some(SHORT_TIMEOUT))
        .timeout_recv_body(Some(body))
        .build()
        .into()
}

fn manifest_agent() -> ureq::Agent {
    agent(SHORT_TIMEOUT)
}

fn installer_agent() -> ureq::Agent {
    agent(INSTALLER_BODY_TIMEOUT)
}

fn upd(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Update(e.to_string())
}

/// Upper bound for the manifest and for its signature file.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 1024;
/// Upper bound for a downloaded installer.
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;

fn read_capped(reader: impl Read, max: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(max + 1).read_to_end(&mut bytes).map_err(upd)?;
    if bytes.len() as u64 > max {
        return Err(ServiceError::Update("too large".into()));
    }
    Ok(bytes)
}

fn fetch(url: &str, max: u64) -> Result<Vec<u8>> {
    let mut resp = manifest_agent().get(url).call().map_err(upd)?;
    read_capped(resp.body_mut().as_reader(), max)
}

/// Checks the signature over the raw bytes first, then parses and validates the manifest.
pub fn verify_manifest(json: &[u8], sig_b64: &str, pubkey: &[u8; 32]) -> Result<Manifest> {
    let bad_sig = || ServiceError::Update("signature".into());
    let key = VerifyingKey::from_bytes(pubkey).map_err(|_| bad_sig())?;
    let raw = STANDARD.decode(sig_b64.trim()).map_err(|_| bad_sig())?;
    let sig = Signature::from_slice(&raw).map_err(|_| bad_sig())?;
    key.verify_strict(json, &sig).map_err(|_| bad_sig())?;

    let bad = || ServiceError::Update("manifest".into());
    let m: Manifest = serde_json::from_slice(json).map_err(|_| bad())?;
    let sha_ok = m.sha256.len() == 64
        && m.sha256
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c));
    if !(m.url.starts_with("https://") && m.page.starts_with("https://") && sha_ok) {
        return Err(bad());
    }
    Ok(m)
}

/// Base64 signature of the manifest bytes (used by the release tooling).
pub fn sign_manifest(seed: &[u8; 32], json: &[u8]) -> String {
    STANDARD.encode(SigningKey::from_bytes(seed).sign(json).to_bytes())
}

/// Hex of the public key that belongs to `seed`.
pub fn public_key_hex(seed: &[u8; 32]) -> String {
    hex::encode(SigningKey::from_bytes(seed).verifying_key().to_bytes())
}

pub fn manifest_json(m: &Manifest) -> Vec<u8> {
    serde_json::to_vec_pretty(m).expect("a manifest of strings always serializes")
}

pub fn check_for_update(cfg: &UpdateConfig, current: &str) -> Result<Option<Manifest>> {
    let json = fetch(&cfg.manifest_url, MAX_MANIFEST_BYTES)?;
    let sig = fetch(&format!("{}.sig", cfg.manifest_url), MAX_SIGNATURE_BYTES)?;
    let sig = String::from_utf8(sig).map_err(|_| ServiceError::Update("signature".into()))?;
    let m = verify_manifest(&json, &sig, &cfg.public_key)?;
    Ok(is_newer(current, &m.version).then_some(m))
}

fn check_sha256(bytes: &[u8], expected: &str) -> Result<()> {
    if hex::encode(Sha256::digest(bytes)) != expected.to_ascii_lowercase() {
        return Err(ServiceError::Update("checksum".into()));
    }
    Ok(())
}

/// Downloads and verifies the installer in memory; `dest` is only created once it is valid.
pub fn download_and_verify(m: &Manifest, dest: &Path) -> Result<()> {
    if !m.url.starts_with("https://") {
        return Err(ServiceError::Update("insecure url".into()));
    }
    let mut resp = installer_agent().get(&m.url).call().map_err(upd)?;
    if resp
        .body()
        .content_length()
        .is_some_and(|n| n > MAX_INSTALLER_BYTES)
    {
        return Err(ServiceError::Update("too large".into()));
    }
    let bytes = read_capped(resp.body_mut().as_reader(), MAX_INSTALLER_BYTES)?;
    check_sha256(&bytes, &m.sha256)?;
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = std::path::PathBuf::from(part);
    let res = std::fs::write(&part, &bytes).and_then(|_| std::fs::rename(&part, dest));
    if res.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    res.map_err(ServiceError::Io)
}

/// True when the uninstaller sits beside `exe`, i.e. it runs from an installed copy.
pub fn is_installed_copy(exe: &Path) -> bool {
    exe.parent()
        .is_some_and(|dir| dir.join("uninstall.exe").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn updates_are_off_outside_windows() {
        assert!(update_config().is_none());
    }

    #[test]
    fn update_config_none_without_env() {
        assert!(update_config().is_none());
    }

    #[test]
    fn parse_config_cases() {
        let key = "ab".repeat(32);
        let c = parse_update_config(Some("https://x/m.json"), Some(&key)).unwrap();
        assert_eq!(c.manifest_url, "https://x/m.json");
        assert_eq!(c.public_key, [0xab; 32]);
        assert!(parse_update_config(Some("http://x/m.json"), Some(&key)).is_none());
        assert!(parse_update_config(None, Some(&key)).is_none());
        assert!(parse_update_config(Some("u"), None).is_none());
        assert!(parse_update_config(Some(""), Some(&key)).is_none());
        assert!(parse_update_config(Some("u"), Some("abcd")).is_none());
        assert!(parse_update_config(Some("u"), Some(&"zz".repeat(32))).is_none());
    }

    #[test]
    fn is_newer_with_pre_releases() {
        let ordered = [
            "0.3.0",
            "0.4.0-rc.1",
            "0.4.0-rc.2",
            "0.4.0-rc.10",
            "0.4.0",
            "0.4.1",
        ];
        for w in ordered.windows(2) {
            assert!(is_newer(w[0], w[1]), "{} < {}", w[0], w[1]);
            assert!(!is_newer(w[1], w[0]), "{} > {}", w[1], w[0]);
        }
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("0.4.0-rc.1", "0.4.0-rc.1"));
        assert!(is_newer("v1.0", "1.0.1"));
        assert!(is_newer("1.2.0", "1.10.0"));
        assert!(is_newer("1.0.0-beta.2", "1.0.0-rc.1")); // strings compare as text
        assert!(is_newer("1.0.0-rc.1", "1.0.0-rc.a")); // a number part is lower than a text part
        for bad in ["x", "", "1.0.x", "1..0", "1.0.0-", "1.0.0-rc..1"] {
            assert!(!is_newer(bad, "9.9.9"), "{bad}");
            assert!(!is_newer("0.0.1", bad), "{bad}");
        }
    }

    #[test]
    fn tag_must_match_the_cargo_version() {
        assert!(tag_matches_version("v0.4.0", "0.4.0"));
        assert!(tag_matches_version("v0.4.0-rc.1", "0.4.0"));
        assert!(!tag_matches_version("v0.4.1", "0.4.0"));
        assert!(!tag_matches_version("0.4.0", "0.4.0")); // the tag needs the leading v
        assert!(!tag_matches_version("v0.4.0-", "0.4.0"));
    }

    #[test]
    fn current_version_defaults_to_the_crate_version() {
        if option_env!("TT_BUILD_VERSION").is_none() {
            assert_eq!(current_version(), env!("CARGO_PKG_VERSION"));
        }
    }

    const SEED: [u8; 32] = [7u8; 32];

    fn sample() -> Manifest {
        Manifest {
            version: "0.4.0".into(),
            url: "https://x/TimeTracker-0.4.0-setup.exe".into(),
            sha256: hex::encode(Sha256::digest(b"installer")),
            page: "https://x/releases/tag/v0.4.0".into(),
        }
    }

    fn pk() -> [u8; 32] {
        hex::decode(public_key_hex(&SEED))
            .unwrap()
            .try_into()
            .unwrap()
    }

    #[test]
    fn signed_manifest_round_trips() {
        let json = manifest_json(&sample());
        let sig = sign_manifest(&SEED, &json);
        assert_eq!(verify_manifest(&json, &sig, &pk()).unwrap(), sample());
    }

    #[test]
    fn verify_manifest_checks_the_signature_before_parsing() {
        let json = manifest_json(&sample());
        let sig = sign_manifest(&SEED, &json);
        let mut tampered = json.clone();
        let i = tampered.iter().position(|&b| b == b'4').unwrap();
        tampered[i] = b'9'; // 0.4.0 -> 0.9.0
        assert!(matches!(verify_manifest(&tampered, &sig, &pk()),
            Err(ServiceError::Update(s)) if s == "signature"));
        let other = sign_manifest(&[8u8; 32], &json);
        assert!(matches!(verify_manifest(&json, &other, &pk()),
            Err(ServiceError::Update(s)) if s == "signature"));
        // Signed but not a manifest: the signature passes, the parse fails.
        let junk = b"not json".to_vec();
        assert!(verify_manifest(&junk, &sign_manifest(&SEED, &junk), &pk()).is_err());
    }

    #[test]
    fn verify_manifest_rejects_missing_or_garbage_signature() {
        let json = manifest_json(&sample());
        for sig in ["", "!!notbase64", "AAAA", "<html>Not Found</html>"] {
            assert!(
                matches!(verify_manifest(&json, sig, &pk()),
                Err(ServiceError::Update(s)) if s == "signature"),
                "{sig:?}"
            );
        }
    }

    #[test]
    fn verify_manifest_rejects_insecure_addresses() {
        for m in [
            Manifest {
                url: "http://x/i.exe".into(),
                ..sample()
            },
            Manifest {
                page: "file:///C:/x".into(),
                ..sample()
            },
            Manifest {
                sha256: "XYZ".into(),
                ..sample()
            },
        ] {
            let json = manifest_json(&m);
            assert!(verify_manifest(&json, &sign_manifest(&SEED, &json), &pk()).is_err());
        }
    }

    #[test]
    fn download_rejects_insecure_url_without_creating_the_file() {
        let m = Manifest {
            url: "http://x/i.exe".into(),
            ..sample()
        };
        let dest = tempfile::tempdir().unwrap().path().join("u.exe");
        assert!(matches!(download_and_verify(&m, &dest),
            Err(ServiceError::Update(s)) if s == "insecure url"));
        assert!(!dest.exists());
    }

    #[test]
    fn checksum_must_match() {
        assert!(check_sha256(b"installer", &sample().sha256).is_ok());
        assert!(matches!(check_sha256(b"other", &sample().sha256),
            Err(ServiceError::Update(s)) if s == "checksum"));
    }

    #[test]
    fn installed_copy_has_the_uninstaller_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("tt-app.exe");
        assert!(!is_installed_copy(&exe));
        std::fs::write(dir.path().join("uninstall.exe"), b"").unwrap();
        assert!(is_installed_copy(&exe));
    }

    #[test]
    fn read_capped_enforces_limit() {
        assert_eq!(read_capped(&b"abc"[..], 3).unwrap(), b"abc");
        assert!(matches!(read_capped(&b"abcd"[..], 3),
            Err(ServiceError::Update(s)) if s == "too large"));
    }

    #[test]
    fn agents_are_https_only_and_the_installer_body_may_take_minutes() {
        for (agent, body) in [
            (manifest_agent(), Duration::from_secs(10)),
            (installer_agent(), Duration::from_secs(600)),
        ] {
            let cfg = agent.config();
            assert!(cfg.https_only());
            let t = cfg.timeouts();
            assert_eq!(t.global, None, "no cap over the whole download");
            assert_eq!(t.connect, Some(Duration::from_secs(10)));
            assert_eq!(t.recv_response, Some(Duration::from_secs(10)));
            assert_eq!(t.recv_body, Some(body));
        }
    }
}
