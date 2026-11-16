//! Release tooling for CI: tag check, public key and signed update manifest.
//! The signing seed comes only from the `TT_UPDATE_SIGNING_KEY` environment variable.

use std::path::Path;
use std::process::ExitCode;

use sha2::{Digest, Sha256};
use tt_services::update::{
    Manifest, manifest_json, parse_seed, public_key_hex, sign_manifest, tag_matches_version,
};

const USAGE: &str = "usage: tt-release check-version <tag> | pubkey | \
manifest <version> <installer> <url> <page> <out-dir>";

fn seed() -> Result<[u8; 32], String> {
    std::env::var("TT_UPDATE_SIGNING_KEY")
        .ok()
        .and_then(|v| parse_seed(&v))
        .ok_or_else(|| "TT_UPDATE_SIGNING_KEY: expected 64 hex digits".to_owned())
}

fn write_manifest(
    version: &str,
    installer: &str,
    url: &str,
    page: &str,
    out_dir: &str,
) -> Result<(), String> {
    let seed = seed()?;
    let bytes = std::fs::read(installer).map_err(|e| format!("{installer}: {e}"))?;
    let manifest = Manifest {
        version: version.to_owned(),
        url: url.to_owned(),
        sha256: hex::encode(Sha256::digest(&bytes)),
        page: page.to_owned(),
    };
    let json = manifest_json(&manifest);
    let sig = sign_manifest(&seed, &json);
    let dir = Path::new(out_dir);
    let write = |name: &str, data: &[u8]| {
        let path = dir.join(name);
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))
    };
    write("manifest.json", &json)?;
    write("manifest.json.sig", sig.as_bytes())
}

fn run(args: &[String]) -> Result<(), String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["check-version", tag] => {
            let version = env!("CARGO_PKG_VERSION");
            if !tag_matches_version(tag, version) {
                return Err(format!("tag {tag} does not match Cargo version {version}"));
            }
            println!("{}", tag.strip_prefix('v').unwrap_or(tag));
            Ok(())
        }
        ["pubkey"] => {
            println!("{}", public_key_hex(&seed()?));
            Ok(())
        }
        ["manifest", version, installer, url, page, out_dir] => {
            write_manifest(version, installer, url, page, out_dir)
        }
        _ => Err(USAGE.to_owned()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("tt-release: {msg}");
            ExitCode::FAILURE
        }
    }
}
