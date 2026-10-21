//! `latest.json` and its signature (UPD-1): what a release offers, believed only once the
//! signature verifies with a trusted key, the manifest names this app, and it has a download
//! for this platform.

use crate::keys::PublicKey;
use crate::Problem;
use base64::Engine;
use serde::Deserialize;
use std::collections::BTreeMap;

/// A file to download, as a platform's entry in `latest.json` has it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Download {
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
}

/// What a verified manifest offers this app on this platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub download: Download,
}

#[derive(Deserialize)]
struct Raw {
    app: Option<String>,
    version: String,
    #[serde(default)]
    platforms: BTreeMap<String, Download>,
}

/// The release `manifest` (the bytes as fetched) offers, if `signature` (the `.sig` file:
/// base64, maybe a newline) verifies it with one of `keys`, it names `app`, and it has an
/// entry for `platform`.
pub fn verify(
    manifest: &[u8],
    signature: &[u8],
    keys: &[PublicKey],
    app: &str,
    platform: &str,
) -> Result<Release, Problem> {
    let sig = std::str::from_utf8(signature)
        .ok()
        .and_then(|s| {
            base64::engine::general_purpose::STANDARD
                .decode(s.trim())
                .ok()
        })
        .ok_or(Problem::Signature)?;
    let verified = keys.iter().any(|key| {
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
            .verify(manifest, &sig)
            .is_ok()
    });
    if !verified {
        return Err(Problem::Signature);
    }
    let raw: Raw = serde_json::from_slice(manifest)
        .map_err(|e| Problem::Manifest(format!("latest.json isn't a release ({e})")))?;
    match raw.app.as_deref() {
        Some(a) if a == app => {}
        other => return Err(Problem::OtherApp(other.unwrap_or("no app").to_owned())),
    }
    if parse_version(&raw.version).is_none() {
        return Err(Problem::Manifest(format!(
            "version {:?} isn't X.Y.Z",
            raw.version
        )));
    }
    let download = raw
        .platforms
        .get(platform)
        .cloned()
        .ok_or_else(|| Problem::NoPlatform(platform.to_owned()))?;
    let hex = download.sha256.len() == 64
        && download
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !hex {
        return Err(Problem::Manifest(format!(
            "{platform}'s sha256 {:?} isn't one",
            download.sha256
        )));
    }
    Ok(Release {
        version: raw.version,
        download,
    })
}

/// `X.Y.Z` as numbers; `None` for anything else.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.split('.').map(|p| {
        (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse().ok())
            .flatten()
    });
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Whether `offered` is a newer version than `running`. A running version that isn't
/// `X.Y.Z` takes nothing.
pub fn newer(offered: &str, running: &str) -> bool {
    match (parse_version(offered), parse_version(running)) {
        (Some(o), Some(r)) => o > r,
        _ => false,
    }
}

/// The platform's key in `latest.json`'s `platforms`, for this build.
pub const PLATFORM: &str = if cfg!(target_os = "macos") {
    "macos-arm64"
} else if cfg!(windows) {
    "windows-x86_64"
} else {
    "linux-x86_64"
};
