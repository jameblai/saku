//! Resolve and install a Saku Release from GitHub.

use std::fs;
use std::io::{Cursor, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use saku_harness::{Config, ReleaseChannel};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::service;

const RELEASES_URL: &str = "https://api.github.com/repos/jameblai/saku/releases?per_page=100";
const LATEST_STABLE_URL: &str = "https://api.github.com/repos/jameblai/saku/releases/latest";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct GithubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    pub assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadTarget {
    pub tag: String,
    pub asset_name: String,
    pub asset_url: String,
    pub checksums_url: String,
}

/// Map a Rust host architecture to the public Release artifact architecture.
pub fn artifact_arch(arch: &str) -> Result<&'static str, String> {
    match arch {
        "x86_64" => Ok("x86_64"),
        "aarch64" => Ok("aarch64"),
        other => Err(format!(
            "unsupported architecture `{other}` (expected x86_64 or aarch64)"
        )),
    }
}

/// Select the newest usable Release and its architecture-specific assets.
///
/// For **stable**, pass `stable_tag` from GitHub `releases/latest` so Update matches Install.
/// When `stable_tag` is `None` (unit tests), the first non-prerelease Release is used.
pub fn resolve_release(
    channel: ReleaseChannel,
    arch: &str,
    releases: &[GithubRelease],
    stable_tag: Option<&str>,
) -> Result<DownloadTarget, String> {
    let arch = artifact_arch(arch)?;
    let release = releases
        .iter()
        .find(|release| {
            if release.draft {
                return false;
            }
            match channel {
                ReleaseChannel::Stable => {
                    !release.prerelease && stable_tag.is_none_or(|tag| release.tag_name == tag)
                }
                ReleaseChannel::Nightly => {
                    release.prerelease && release.tag_name.contains("-nightly.")
                }
            }
        })
        .ok_or_else(|| format!("no {channel} Release found"))?;
    let asset_name = format!("saku-linux-{arch}.tar.gz");
    let archive_url = asset_url(release, &asset_name)?;
    let checksums_url = asset_url(release, "SHA256SUMS")?;
    Ok(DownloadTarget {
        tag: release.tag_name.clone(),
        asset_name,
        asset_url: archive_url,
        checksums_url,
    })
}

fn asset_url(release: &GithubRelease, name: &str) -> Result<String, String> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| format!("Release {} is missing asset {name}", release.tag_name))
}

/// Verify an asset against a standard SHA256SUMS body.
pub fn verify_checksum(name: &str, bytes: &[u8], sums: &str) -> Result<(), String> {
    let expected = sums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hash = fields.next()?;
        let file = fields.next()?.trim_start_matches('*');
        (file == name).then_some(hash)
    });
    let expected = expected.ok_or_else(|| format!("SHA256SUMS has no entry for {name}"))?;
    let actual = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!("checksum mismatch for {name}"))
    }
}

/// Run `saku update` using `~/.saku/config.toml`.
pub async fn update() -> Result<(), String> {
    let home = dirs::home_dir().ok_or_else(|| "could not determine home directory".to_string())?;
    let config_path = home.join(".saku/config.toml");
    let channel = load_update_channel(&config_path)?;
    update_from_channel(channel, &home.join(".local/bin/saku")).await
}

fn load_update_channel(config_path: &Path) -> Result<ReleaseChannel, String> {
    if !config_path.exists() {
        return Err(format!(
            "config not found at {}; run `saku setup` first",
            config_path.display()
        ));
    }
    Config::load_release_channel(config_path).map_err(|e| e.to_string())
}

async fn update_from_channel(channel: ReleaseChannel, destination: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("saku/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let stable_tag = match channel {
        ReleaseChannel::Stable => Some(fetch_latest_stable_tag(&client).await?),
        ReleaseChannel::Nightly => None,
    };
    let releases = client
        .get(RELEASES_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<Vec<GithubRelease>>()
        .await
        .map_err(|e| e.to_string())?;
    let target = resolve_release(
        channel,
        std::env::consts::ARCH,
        &releases,
        stable_tag.as_deref(),
    )?;
    let archive = download(&client, &target.asset_url).await?;
    let sums = String::from_utf8(download(&client, &target.checksums_url).await?)
        .map_err(|_| "SHA256SUMS was not UTF-8".to_string())?;
    verify_checksum(&target.asset_name, &archive, &sums)?;
    replace_binary_from_archive(&archive, destination)?;
    println!("Updated Saku to {} ({channel}).", target.tag);
    service::restart_service_if_active();
    Ok(())
}

async fn fetch_latest_stable_tag(client: &reqwest::Client) -> Result<String, String> {
    #[derive(Deserialize)]
    struct LatestRelease {
        tag_name: String,
    }
    let release = client
        .get(LATEST_STABLE_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<LatestRelease>()
        .await
        .map_err(|e| e.to_string())?;
    Ok(release.tag_name)
}

async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    Ok(client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?
        .to_vec())
}

fn replace_binary_from_archive(archive: &[u8], destination: &Path) -> Result<(), String> {
    let decoder = GzDecoder::new(Cursor::new(archive));
    let mut tar = tar::Archive::new(decoder);
    let mut binary = Vec::new();
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if entry.path().map_err(|e| e.to_string())? == PathBuf::from("saku") {
            entry.read_to_end(&mut binary).map_err(|e| e.to_string())?;
            break;
        }
    }
    if binary.is_empty() {
        return Err("Release archive does not contain `saku`".into());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "invalid install destination".to_string())?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(&binary).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.as_file()
        .set_permissions(fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    temp.persist(destination).map_err(|e| e.error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn release(tag: &str, prerelease: bool) -> GithubRelease {
        GithubRelease {
            tag_name: tag.into(),
            prerelease,
            draft: false,
            assets: ["x86_64", "aarch64"]
                .into_iter()
                .map(|arch| GithubAsset {
                    name: format!("saku-linux-{arch}.tar.gz"),
                    browser_download_url: format!("https://example.test/{tag}/{arch}"),
                })
                .chain(std::iter::once(GithubAsset {
                    name: "SHA256SUMS".into(),
                    browser_download_url: format!("https://example.test/{tag}/sums"),
                }))
                .collect(),
        }
    }

    fn releases_from_fixture(name: &str) -> Vec<GithubRelease> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let text = fs::read_to_string(path).expect("fixture");
        serde_json::from_str(&text).expect("deserialize fixture")
    }

    #[test]
    fn stable_resolves_latest_non_prerelease_from_fixture() {
        let releases = releases_from_fixture("stable_latest.json");
        let target =
            resolve_release(ReleaseChannel::Stable, "x86_64", &releases, Some("v0.2.0")).unwrap();
        assert_eq!(target.tag, "v0.2.0");
        assert_eq!(target.asset_name, "saku-linux-x86_64.tar.gz");
    }

    #[test]
    fn stable_resolves_pinned_tag_from_fixture() {
        let releases = releases_from_fixture("stable_pinned.json");
        let target =
            resolve_release(ReleaseChannel::Stable, "aarch64", &releases, Some("v0.1.0")).unwrap();
        assert_eq!(target.tag, "v0.1.0");
        assert_eq!(target.asset_name, "saku-linux-aarch64.tar.gz");
    }

    #[test]
    fn nightly_resolves_newest_nightly_from_fixture() {
        let releases = releases_from_fixture("nightly_list.json");
        let target = resolve_release(ReleaseChannel::Nightly, "aarch64", &releases, None).unwrap();
        assert_eq!(target.tag, "v0.2.0-nightly.20260711.3");
    }

    #[test]
    fn stable_without_tag_uses_first_non_prerelease() {
        let releases = [
            release("v0.2.0-nightly.20260711.2", true),
            release("v0.2.0", false),
        ];
        let target = resolve_release(ReleaseChannel::Stable, "x86_64", &releases, None).unwrap();
        assert_eq!(target.tag, "v0.2.0");
    }

    #[test]
    fn rejects_wrong_checksum() {
        let err = verify_checksum(
            "saku-linux-x86_64.tar.gz",
            b"tampered",
            &format!("{}  saku-linux-x86_64.tar.gz", "0".repeat(64)),
        )
        .unwrap_err();
        assert_eq!(err, "checksum mismatch for saku-linux-x86_64.tar.gz");
    }

    #[test]
    fn missing_config_explains_setup_next_step() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let err = load_update_channel(&path).unwrap_err();
        assert!(err.contains("config not found"));
        assert!(err.contains("saku setup"));
    }

    #[test]
    fn channel_only_config_loads_for_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"release_channel = "nightly""#).unwrap();
        assert_eq!(
            load_update_channel(&path).expect("load"),
            ReleaseChannel::Nightly
        );
    }
}
