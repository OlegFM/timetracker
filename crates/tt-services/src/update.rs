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

fn components(v: &str) -> Option<Vec<u64>> {
    v.strip_prefix('v')
        .unwrap_or(v)
        .split('.')
        .map(|p| p.parse().ok())
        .collect()
}

pub fn is_newer(current: &str, latest: &str) -> bool {
    let (Some(mut a), Some(mut b)) = (components(current), components(latest)) else {
        return false;
    };
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    b > a
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
    fn is_newer_cases() {
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("1.2.0", "1.10.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.1.0", "1.0.9"));
        assert!(is_newer("v1.0", "1.0.1"));
        assert!(!is_newer("1.0.0", "1.0.0-beta"));
        assert!(!is_newer("x", "1.0.0"));
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
