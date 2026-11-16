//! Disabled-by-default updater: manifest check and signed-installer verification.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{Result, ServiceError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateConfig {
    pub manifest_url: String,
    pub public_key: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub url: String,
    /// Lowercase hex SHA-256 of the installer.
    pub sha256: String,
    /// Base64 (standard) ed25519 signature over the installer bytes.
    pub signature: String,
    pub notes: Option<String>,
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

pub fn check_for_update(cfg: &UpdateConfig, current: &str) -> Result<Option<Manifest>> {
    let m: Manifest = manifest_agent()
        .get(&cfg.manifest_url)
        .call()
        .map_err(upd)?
        .body_mut()
        .read_json()
        .map_err(upd)?;
    Ok(is_newer(current, &m.version).then_some(m))
}

pub fn verify_bytes(pubkey: &[u8; 32], bytes: &[u8], sig_b64: &str) -> Result<()> {
    let bad = || ServiceError::Update("signature".into());
    let key = VerifyingKey::from_bytes(pubkey).map_err(|_| bad())?;
    let raw = STANDARD.decode(sig_b64.trim()).map_err(|_| bad())?;
    let sig = Signature::from_slice(&raw).map_err(|_| bad())?;
    key.verify_strict(bytes, &sig).map_err(|_| bad())
}

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

/// Checks the installer bytes against the manifest checksum, then its signature.
pub fn verify_download(bytes: &[u8], m: &Manifest, pubkey: &[u8; 32]) -> Result<()> {
    if hex::encode(Sha256::digest(bytes)) != m.sha256.to_ascii_lowercase() {
        return Err(ServiceError::Update("checksum".into()));
    }
    verify_bytes(pubkey, bytes, &m.signature)
}

fn download(m: &Manifest, dest: &Path, pubkey: &[u8; 32]) -> Result<()> {
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
    verify_download(&bytes, m, pubkey)?;
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = std::path::PathBuf::from(part);
    let res = std::fs::write(&part, &bytes).and_then(|_| std::fs::rename(&part, dest));
    if res.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    res.map_err(ServiceError::Io)
}

/// Downloads and verifies the installer in memory; `dest` is only created once it is valid.
pub fn download_and_verify(cfg: &UpdateConfig, m: &Manifest, dest: &Path) -> Result<()> {
    download(m, dest, &cfg.public_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

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

    #[test]
    fn verify_rejects_tampered_bytes() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let pk = sk.verifying_key().to_bytes();
        let data = b"installer bytes".to_vec();
        let sig = STANDARD.encode(sk.sign(&data).to_bytes());
        verify_bytes(&pk, &data, &sig).unwrap();
        let mut bad = data.clone();
        bad[0] ^= 1;
        assert!(matches!(
            verify_bytes(&pk, &bad, &sig),
            Err(ServiceError::Update(s)) if s == "signature"
        ));
        assert!(verify_bytes(&pk, &data, "!!notbase64").is_err());
    }

    fn signed_manifest(data: &[u8]) -> (Manifest, [u8; 32]) {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let m = Manifest {
            version: "2.0.0".into(),
            url: "https://x/i.exe".into(),
            sha256: hex::encode(Sha256::digest(data)),
            signature: STANDARD.encode(sk.sign(data).to_bytes()),
            notes: None,
        };
        (m, sk.verifying_key().to_bytes())
    }

    #[test]
    fn verify_download_cases() {
        let data = b"installer";
        let (m, pk) = signed_manifest(data);
        verify_download(data, &m, &pk).unwrap();
        let bad_sha = Manifest {
            sha256: "0".repeat(64),
            ..m.clone()
        };
        assert!(matches!(verify_download(data, &bad_sha, &pk),
            Err(ServiceError::Update(s)) if s == "checksum"));
        let other = SigningKey::from_bytes(&[8u8; 32]);
        let bad_sig = Manifest {
            signature: STANDARD.encode(other.sign(data).to_bytes()),
            ..m
        };
        assert!(matches!(verify_download(data, &bad_sig, &pk),
            Err(ServiceError::Update(s)) if s == "signature"));
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

    #[test]
    fn rejects_insecure_urls() {
        let key = "ab".repeat(32);
        assert!(parse_update_config(Some("http://x/m.json"), Some(&key)).is_none());
        let (m, pk) = signed_manifest(b"d");
        let m = Manifest {
            url: "http://x/i.exe".into(),
            ..m
        };
        let cfg = UpdateConfig {
            manifest_url: "https://x/m.json".into(),
            public_key: pk,
        };
        let dest = std::env::temp_dir().join("tt-insecure-test.exe");
        assert!(matches!(download_and_verify(&cfg, &m, &dest),
            Err(ServiceError::Update(s)) if s == "insecure url"));
        assert!(!dest.exists());
    }
}
