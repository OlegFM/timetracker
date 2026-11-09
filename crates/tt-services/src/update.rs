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
    let url = url.map(str::trim).filter(|u| !u.is_empty())?;
    let public_key: [u8; 32] = hex::decode(key_hex?.trim()).ok()?.try_into().ok()?;
    Some(UpdateConfig {
        manifest_url: url.to_owned(),
        public_key,
    })
}

/// `None` unless both build-time variables are set and valid.
pub fn update_config() -> Option<UpdateConfig> {
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

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into()
}

fn upd(e: impl std::fmt::Display) -> ServiceError {
    ServiceError::Update(e.to_string())
}

pub fn check_for_update(cfg: &UpdateConfig, current: &str) -> Result<Option<Manifest>> {
    let m: Manifest = agent()
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

fn download(m: &Manifest, dest: &Path, pubkey: &[u8; 32]) -> Result<()> {
    let mut resp = agent().get(&m.url).call().map_err(upd)?;
    let mut bytes = Vec::new();
    resp.body_mut()
        .as_reader()
        .read_to_end(&mut bytes)
        .map_err(upd)?;
    std::fs::write(dest, &bytes).map_err(ServiceError::Io)?;
    if hex::encode(Sha256::digest(&bytes)) != m.sha256.to_ascii_lowercase() {
        return Err(ServiceError::Update("checksum".into()));
    }
    verify_bytes(pubkey, &bytes, &m.signature)
}

/// Downloads the installer to `dest`; removes it unless checksum and signature match.
pub fn download_and_verify(cfg: &UpdateConfig, m: &Manifest, dest: &Path) -> Result<()> {
    let res = download(m, dest, &cfg.public_key);
    if res.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

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
}
