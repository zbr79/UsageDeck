//! The latest release as the updater sees it, parsed from GitHub's API, and the pure checks on
//! it. No I/O here, so the tests run on any platform.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;

pub const ASSET_NAME: &str = "UsageDeck-Setup.exe";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl FromStr for Version {
    type Err = UpdateError;

    /// `1.2.3` or a tag `v1.2.3`. Pre-release suffixes are rejected: /releases/latest never
    /// returns a pre-release, so one here means the data is not what we expect.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bare = s.strip_prefix('v').unwrap_or(s);
        let mut parts = bare.split('.').map(|p| {
            p.parse::<u64>()
                .ok()
                .filter(|_| p.bytes().all(|b| b.is_ascii_digit()))
        });
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some(Some(major)), Some(Some(minor)), Some(Some(patch)), None) => Ok(Self {
                major,
                minor,
                patch,
            }),
            _ => Err(UpdateError::Release(format!("{s:?} is not a version"))),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The installer asset of the latest release, as GitHub describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub asset_url: String,
    pub size: u64,
    pub sha256: [u8; 32],
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("unexpected release data: {0}")]
    Release(String),
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("the download does not match the release: {0}")]
    Integrity(String),
    #[error("could not save the installer: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

/// Parse GitHub's `GET /repos/{owner}/{repo}/releases/latest` response.
pub fn parse_release(json: &str) -> Result<Release, UpdateError> {
    let raw: GithubRelease =
        serde_json::from_str(json).map_err(|e| UpdateError::Release(e.to_string()))?;
    let version = raw.tag_name.parse()?;
    let asset = raw
        .assets
        .into_iter()
        .find(|a| a.name == ASSET_NAME)
        .ok_or_else(|| UpdateError::Release(format!("release has no {ASSET_NAME}")))?;
    let digest = asset
        .digest
        .ok_or_else(|| UpdateError::Release(format!("{ASSET_NAME} has no digest")))?;
    let sha256 = digest
        .strip_prefix("sha256:")
        .and_then(parse_sha256)
        .ok_or_else(|| UpdateError::Release(format!("{ASSET_NAME} digest {digest:?}")))?;
    Ok(Release {
        version,
        asset_url: asset.browser_download_url,
        size: asset.size,
        sha256,
    })
}

fn parse_sha256(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

pub fn is_newer(current: Version, candidate: Version) -> bool {
    candidate > current
}

/// Check that downloaded bytes are exactly the release's installer.
pub fn verify(bytes: &[u8], release: &Release) -> Result<(), UpdateError> {
    if bytes.len() as u64 != release.size {
        return Err(UpdateError::Integrity(format!(
            "{} bytes, expected {}",
            bytes.len(),
            release.size
        )));
    }
    if Sha256::digest(bytes)[..] != release.sha256[..] {
        return Err(UpdateError::Integrity("SHA-256 differs".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "4f2c3a7c0b6e1d5a9f8e7d6c5b4a39281706f5e4d3c2b1a09f8e7d6c5b4a3928";

    fn fixture(tag: &str, asset: &str) -> String {
        format!(
            r#"{{
              "tag_name": "{tag}",
              "name": "UsageDeck {tag}",
              "draft": false,
              "prerelease": false,
              "assets": [
                {{
                  "name": "UsageDeck_1.2.0_x64-setup.exe",
                  "size": 1,
                  "digest": "sha256:{HEX}",
                  "browser_download_url": "https://example.invalid/versioned.exe"
                }},
                {asset}
              ]
            }}"#
        )
    }

    fn setup_asset(digest: &str) -> String {
        format!(
            r#"{{
              "name": "UsageDeck-Setup.exe",
              "size": 4242,
              {digest}
              "browser_download_url": "https://github.com/zbr79/UsageDeck/releases/download/v1.2.0/UsageDeck-Setup.exe"
            }}"#
        )
    }

    fn v(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn parses_the_setup_asset_of_the_latest_release() {
        let json = fixture(
            "v1.2.0",
            &setup_asset(&format!(r#""digest": "sha256:{HEX}","#)),
        );
        let release = parse_release(&json).unwrap();
        assert_eq!(release.version, v(1, 2, 0));
        assert_eq!(release.size, 4242);
        assert_eq!(
            release.asset_url,
            "https://github.com/zbr79/UsageDeck/releases/download/v1.2.0/UsageDeck-Setup.exe"
        );
        assert_eq!(release.sha256[0], 0x4f);
        assert_eq!(release.sha256[31], 0x28);
    }

    #[test]
    fn rejects_a_setup_asset_without_a_digest() {
        let json = fixture("v1.2.0", &setup_asset(""));
        assert!(parse_release(&json).is_err());
        let json = fixture("v1.2.0", &setup_asset(r#""digest": null,"#));
        assert!(parse_release(&json).is_err());
    }

    #[test]
    fn rejects_a_digest_that_is_not_sha256_hex() {
        for digest in [
            format!("sha512:{HEX}"),
            HEX.to_string(),
            format!("sha256:{}", &HEX[..62]),
            format!("sha256:+{}", &HEX[1..]),
        ] {
            let json = fixture("v1.2.0", &setup_asset(&format!(r#""digest": "{digest}","#)));
            assert!(parse_release(&json).is_err(), "{digest}");
        }
    }

    #[test]
    fn rejects_a_release_without_the_setup_asset() {
        let other = r#"{ "name": "notes.txt", "size": 1, "digest": null,
            "browser_download_url": "https://example.invalid/notes.txt" }"#;
        assert!(parse_release(&fixture("v1.2.0", other)).is_err());
    }

    #[test]
    fn rejects_tags_that_are_not_versions() {
        let asset = setup_asset(&format!(r#""digest": "sha256:{HEX}","#));
        for tag in ["latest", "v1.2", "v1.2.0-beta.1", "v1.2.0.1", "v1.+2.0"] {
            assert!(parse_release(&fixture(tag, &asset)).is_err(), "{tag}");
        }
        assert!(parse_release("not json").is_err());
    }

    #[test]
    fn newer_compares_numerically_not_as_text() {
        assert!(is_newer(v(1, 1, 0), v(1, 1, 1)));
        assert!(is_newer(v(1, 1, 0), v(1, 2, 0)));
        assert!(is_newer(v(1, 9, 0), v(1, 10, 0)));
        assert!(is_newer(v(1, 1, 9), v(2, 0, 0)));
        assert!(!is_newer(v(1, 1, 0), v(1, 1, 0)));
        assert!(!is_newer(v(1, 1, 0), v(1, 0, 9)));
        assert!(!is_newer(v(1, 10, 0), v(1, 9, 0)));
    }

    #[test]
    fn verify_checks_size_and_sha256() {
        let bytes = b"installer bytes";
        let mut release = Release {
            version: v(1, 2, 0),
            asset_url: String::new(),
            size: bytes.len() as u64,
            sha256: Sha256::digest(bytes).into(),
        };
        assert!(verify(bytes, &release).is_ok());
        assert!(verify(b"installer bytez", &release).is_err());
        assert!(verify(b"installer bytes!", &release).is_err());
        release.size += 1;
        assert!(verify(bytes, &release).is_err());
    }
}
