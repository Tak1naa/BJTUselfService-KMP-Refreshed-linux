//! Pure GitHub release metadata parsing and version/asset selection rules.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const GITHUB_RELEASES_API: &str =
    "https://api.github.com/repos/Tak1naa/BJTUselfService-KMP-Refreshed-linux/releases?per_page=10";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag: String,
    pub version: String,
    pub published_at: Option<String>,
    pub html_url: String,
    pub assets: Vec<ReleaseAsset>,
}

/// Parses `0.1.12`, `v0.1.12`, or `cli-v0.1.12` into a strict `(major, minor, patch)` tuple.
pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim();
    let ver = trimmed
        .strip_prefix("cli-v")
        .or_else(|| trimmed.strip_prefix('v'))
        .unwrap_or(trimmed);
    let mut parts = ver.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn is_newer(current: &str, candidate: &str) -> bool {
    match (parse_semver(current), parse_semver(candidate)) {
        (Some(cur), Some(cand)) => cand > cur,
        _ => false,
    }
}

/// Selects the highest stable `cli-vX.Y.Z` release from a GitHub Releases API JSON array.
pub fn latest_cli_release(releases_json: &str) -> Result<Option<ReleaseInfo>> {
    let list: Vec<Value> =
        serde_json::from_str(releases_json).map_err(|_| Error::Malformed("github releases"))?;
    let mut best: Option<((u64, u64, u64), ReleaseInfo)> = None;
    for entry in list {
        if entry["draft"].as_bool() == Some(true) || entry["prerelease"].as_bool() == Some(true) {
            continue;
        }
        let Some(tag) = entry["tag_name"].as_str() else {
            continue;
        };
        if !tag.starts_with("cli-v") {
            continue;
        }
        let Some(semver) = parse_semver(tag) else {
            continue;
        };
        let version = tag.strip_prefix("cli-v").unwrap_or(tag).to_owned();
        let html_url = entry["html_url"]
            .as_str()
            .filter(|s| s.starts_with("https://"))
            .unwrap_or("https://github.com/Tak1naa/BJTUselfService-KMP-Refreshed-linux/releases")
            .to_owned();
        let published_at = entry["published_at"].as_str().map(str::to_owned);
        let assets = entry["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| {
                Some(ReleaseAsset {
                    name: a["name"].as_str()?.to_owned(),
                    url: a["browser_download_url"].as_str()?.to_owned(),
                    size: a["size"].as_u64().unwrap_or(0),
                })
            })
            .collect();
        let info = ReleaseInfo {
            tag: tag.to_owned(),
            version,
            published_at,
            html_url,
            assets,
        };
        if best.as_ref().is_none_or(|(b, _)| semver > *b) {
            best = Some((semver, info));
        }
    }
    Ok(best.map(|(_, info)| info))
}

/// Looks up the expected lowercase hex SHA-256 digest for `filename` in a `SHA256SUMS.txt` body.
pub fn find_sha256(sums_text: &str, filename: &str) -> Option<String> {
    for line in sums_text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((digest, rest)) = line.split_once(|c: char| c.is_ascii_whitespace()) else {
            continue;
        };
        let digest = digest.trim();
        let name = rest.trim_start().trim_start_matches('*');
        if digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()) && name == filename {
            return Some(digest.to_ascii_lowercase());
        }
    }
    None
}

/// Chooses the matching release asset for `(os, arch)` and optional package preference (`deb` | `rpm`).
pub fn select_asset<'a>(
    release: &'a ReleaseInfo,
    os: &str,
    arch: &str,
    prefer_pkg: Option<&str>,
) -> Option<&'a ReleaseAsset> {
    let ver = &release.version;
    if let Some(pkg) = prefer_pkg {
        let suffix = match (pkg, arch) {
            ("deb", "x86_64") => "_amd64.deb",
            ("rpm", "x86_64") => ".x86_64.rpm",
            _ => return None,
        };
        if let Some(found) = release.assets.iter().find(|a| a.name.ends_with(suffix)) {
            return Some(found);
        }
    }
    let candidates: &[&str] = match (os, arch) {
        ("windows", "x86_64") => &["windows-x64.zip"],
        ("macos", "aarch64") => &["macos-arm64.tar.gz", "macos-universal.tar.gz"],
        ("macos", "x86_64") => &["macos-x64.tar.gz", "macos-universal.tar.gz"],
        ("linux", "x86_64") => &["linux-x86_64.tar.gz"],
        _ => &[],
    };
    for suffix in candidates {
        let expected = format!("campus-bjtu-{ver}-{suffix}");
        if let Some(found) = release.assets.iter().find(|a| a.name == expected) {
            return Some(found);
        }
    }
    None
}

pub fn checksums_asset(release: &ReleaseInfo) -> Option<&ReleaseAsset> {
    release.assets.iter().find(|a| a.name == "SHA256SUMS.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_latest_stable_cli_release_and_matches_assets() {
        let raw = r#"[
          {"tag_name":"kmp-v2.0.0","draft":false,"prerelease":false,"assets":[]},
          {"tag_name":"cli-v0.2.0","draft":false,"prerelease":true,"assets":[]},
          {"tag_name":"cli-v0.1.12","draft":false,"prerelease":false,"html_url":"https://github.com/x/y/releases/tag/cli-v0.1.12","assets":[]},
          {"tag_name":"cli-v0.1.13","draft":false,"prerelease":false,"published_at":"2026-11-22T01:00:00Z","html_url":"https://github.com/x/y/releases/tag/cli-v0.1.13","assets":[
            {"name":"campus-bjtu-0.1.13-linux-x86_64.tar.gz","browser_download_url":"https://github.com/x/y/releases/download/cli-v0.1.13/campus-bjtu-0.1.13-linux-x86_64.tar.gz","size":4000000},
            {"name":"campus-bjtu_0.1.13_amd64.deb","browser_download_url":"https://github.com/x/y/releases/download/cli-v0.1.13/campus-bjtu_0.1.13_amd64.deb","size":3500000},
            {"name":"campus-bjtu-0.1.13-windows-x64.zip","browser_download_url":"https://github.com/x/y/releases/download/cli-v0.1.13/campus-bjtu-0.1.13-windows-x64.zip","size":3600000},
            {"name":"campus-bjtu-0.1.13-macos-arm64.tar.gz","browser_download_url":"https://github.com/x/y/releases/download/cli-v0.1.13/campus-bjtu-0.1.13-macos-arm64.tar.gz","size":3200000},
            {"name":"SHA256SUMS.txt","browser_download_url":"https://github.com/x/y/releases/download/cli-v0.1.13/SHA256SUMS.txt","size":500}
          ]}
        ]"#;
        let info = latest_cli_release(raw).unwrap().unwrap();
        assert_eq!(info.tag, "cli-v0.1.13");
        assert!(is_newer("0.1.12", &info.version));
        assert!(!is_newer("0.1.13", &info.version));
        assert_eq!(
            select_asset(&info, "linux", "x86_64", None).unwrap().name,
            "campus-bjtu-0.1.13-linux-x86_64.tar.gz"
        );
        assert_eq!(
            select_asset(&info, "linux", "x86_64", Some("deb"))
                .unwrap()
                .name,
            "campus-bjtu_0.1.13_amd64.deb"
        );
        assert_eq!(
            select_asset(&info, "windows", "x86_64", None).unwrap().name,
            "campus-bjtu-0.1.13-windows-x64.zip"
        );
        assert_eq!(
            select_asset(&info, "macos", "aarch64", None).unwrap().name,
            "campus-bjtu-0.1.13-macos-arm64.tar.gz"
        );
        let sums = "AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899  campus-bjtu-0.1.13-linux-x86_64.tar.gz\n";
        assert_eq!(
            find_sha256(sums, "campus-bjtu-0.1.13-linux-x86_64.tar.gz").as_deref(),
            Some("aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899")
        );
    }
}
