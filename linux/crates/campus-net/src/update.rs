//! Secure GitHub Releases client, SHA-256 verification, and tar.gz / zip binary extraction.
use campus_core::update::{
    GITHUB_RELEASES_API, ReleaseAsset, ReleaseInfo, find_sha256, is_newer, latest_cli_release,
};
use campus_core::{Error, Result};
use flate2::Crc;
use flate2::read::{DeflateDecoder, GzDecoder};
use reqwest::{Client, header, redirect};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::time::Duration;
use url::Url;

const MAX_METADATA_BYTES: usize = 2 * 1024 * 1024;
const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
const ALLOWED_GITHUB_HOSTS: &[&str] = &[
    "api.github.com",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
    "codeload.github.com",
];

fn allows_github_url(url: &Url, allow_local_http: bool) -> bool {
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    if allow_local_http && url.scheme() == "http" && matches!(host, "127.0.0.1" | "localhost") {
        return true;
    }
    url.scheme() == "https" && ALLOWED_GITHUB_HOSTS.contains(&host)
}

fn build_client() -> Result<Client> {
    Client::builder()
        .redirect(redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .user_agent(concat!("campus-bjtu/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| Error::Network)
}

async fn get_bounded(url: &str, max_bytes: usize, allow_local_http: bool) -> Result<Vec<u8>> {
    let client = build_client()?;
    let mut current = Url::parse(url).map_err(|_| Error::SecureChannel)?;
    for _ in 0..8 {
        if !allows_github_url(&current, allow_local_http) {
            return Err(Error::SecureChannel);
        }
        let mut req = client.get(current.clone());
        if current.host_str() == Some("api.github.com") {
            req = req.header(header::ACCEPT, "application/vnd.github+json");
        }
        let resp = req.send().await.map_err(|_| Error::Network)?;
        let status = resp.status();
        if status.is_redirection() {
            let loc = resp
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or(Error::Malformed("update redirect"))?;
            if loc.chars().any(char::is_control) {
                return Err(Error::SecureChannel);
            }
            current = current.join(loc).map_err(|_| Error::SecureChannel)?;
            continue;
        }
        if !status.is_success() {
            return Err(Error::Network);
        }
        if let Some(len) = resp.content_length()
            && len as usize > max_bytes
        {
            return Err(Error::Rejected("release asset exceeds size limit"));
        }
        let bytes = resp.bytes().await.map_err(|_| Error::Network)?;
        if bytes.len() > max_bytes {
            return Err(Error::Rejected("release asset exceeds size limit"));
        }
        return Ok(bytes.to_vec());
    }
    Err(Error::Rejected("too many update redirects"))
}

/// Checks GitHub Releases and returns the latest stable `cli-v*` release if newer than `current_version`.
pub async fn check_latest(current_version: &str) -> Result<Option<ReleaseInfo>> {
    check_latest_from(GITHUB_RELEASES_API, current_version, false).await
}

async fn check_latest_from(
    api_url: &str,
    current_version: &str,
    allow_local_http: bool,
) -> Result<Option<ReleaseInfo>> {
    let raw = get_bounded(api_url, MAX_METADATA_BYTES, allow_local_http).await?;
    let text = String::from_utf8(raw).map_err(|_| Error::Malformed("github releases utf8"))?;
    let Some(latest) = latest_cli_release(&text)? else {
        return Ok(None);
    };
    if is_newer(current_version, &latest.version) {
        Ok(Some(latest))
    } else {
        Ok(None)
    }
}

/// Downloads `asset` and `sums_asset`, verifying the SHA-256 digest before returning bytes.
pub async fn download_verified(asset: &ReleaseAsset, sums_asset: &ReleaseAsset) -> Result<Vec<u8>> {
    download_verified_with_policy(asset, sums_asset, false).await
}

async fn download_verified_with_policy(
    asset: &ReleaseAsset,
    sums_asset: &ReleaseAsset,
    allow_local_http: bool,
) -> Result<Vec<u8>> {
    let sums_bytes = get_bounded(&sums_asset.url, MAX_METADATA_BYTES, allow_local_http).await?;
    let sums_text =
        String::from_utf8(sums_bytes).map_err(|_| Error::Malformed("checksums utf8"))?;
    let expected =
        find_sha256(&sums_text, &asset.name).ok_or(Error::Malformed("release checksum missing"))?;

    let payload = get_bounded(&asset.url, MAX_ASSET_BYTES, allow_local_http).await?;
    let actual = hex::encode(Sha256::digest(&payload));
    if actual != expected {
        return Err(Error::Rejected("release checksum mismatch"));
    }
    Ok(payload)
}

/// Extracts a regular file whose basename matches `target_name` (e.g. `campus`) from a `.tar.gz` archive.
pub fn extract_tar_gz_binary(archive: &[u8], target_name: &str) -> Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(archive);
    let mut header = [0u8; 512];
    loop {
        match read_exact_or_eof(&mut decoder, &mut header)? {
            0 => break,
            512 => {}
            _ => return Err(Error::Malformed("truncated tar header")),
        }
        if header.iter().all(|&b| b == 0) {
            break;
        }
        verify_tar_checksum(&header)?;
        let size = parse_tar_octal(&header[124..136])?;
        if size > MAX_ASSET_BYTES {
            return Err(Error::Rejected("tar entry exceeds size limit"));
        }
        let typeflag = header[156];
        let name = parse_tar_name(&header);
        let basename = name.rsplit('/').next().unwrap_or(&name);
        let is_regular = matches!(typeflag, 0 | b'0');
        let padded = size.div_ceil(512) * 512;
        if is_regular && basename == target_name {
            let mut content = vec![0u8; size];
            decoder
                .read_exact(&mut content)
                .map_err(|_| Error::Malformed("truncated tar payload"))?;
            return Ok(content);
        }
        let mut sink = [0u8; 512];
        let mut remaining = padded;
        while remaining > 0 {
            let chunk = remaining.min(sink.len());
            decoder
                .read_exact(&mut sink[..chunk])
                .map_err(|_| Error::Malformed("truncated tar padding"))?;
            remaining -= chunk;
        }
    }
    Err(Error::Malformed("archive missing campus binary"))
}

fn read_exact_or_eof(reader: &mut impl Read, buf: &mut [u8]) -> Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        let n = reader
            .read(&mut buf[total..])
            .map_err(|_| Error::Malformed("corrupt gzip stream"))?;
        if n == 0 {
            return Ok(total);
        }
        total += n;
    }
    Ok(total)
}

fn verify_tar_checksum(header: &[u8; 512]) -> Result<()> {
    let expected = parse_tar_octal(&header[148..156])?;
    let actual: usize = header
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            if (148..156).contains(&i) {
                b' ' as usize
            } else {
                b as usize
            }
        })
        .sum();
    if actual != expected {
        return Err(Error::Malformed("invalid tar header checksum"));
    }
    Ok(())
}

fn parse_tar_octal(field: &[u8]) -> Result<usize> {
    let text = std::str::from_utf8(field)
        .map_err(|_| Error::Malformed("invalid tar octal field"))?
        .trim_matches(|c: char| c == '\0' || c.is_ascii_whitespace());
    if text.is_empty() {
        return Ok(0);
    }
    usize::from_str_radix(text, 8).map_err(|_| Error::Malformed("invalid tar octal number"))
}

/// Extracts a regular file whose basename matches `target_name` (e.g. `campus.exe`) from a `.zip` archive.
pub fn extract_zip_binary(archive: &[u8], target_name: &str) -> Result<Vec<u8>> {
    let eocd = find_zip_eocd(archive)?;
    let cd_entries = u16::from_le_bytes([archive[eocd + 10], archive[eocd + 11]]) as usize;
    let cd_size = u32::from_le_bytes([
        archive[eocd + 12],
        archive[eocd + 13],
        archive[eocd + 14],
        archive[eocd + 15],
    ]) as usize;
    let cd_offset = u32::from_le_bytes([
        archive[eocd + 16],
        archive[eocd + 17],
        archive[eocd + 18],
        archive[eocd + 19],
    ]) as usize;
    let cd_end = cd_offset
        .checked_add(cd_size)
        .filter(|&end| end <= eocd)
        .ok_or(Error::Malformed("invalid zip central directory"))?;

    let mut pos = cd_offset;
    for _ in 0..cd_entries {
        if pos + 46 > cd_end || &archive[pos..pos + 4] != b"PK\x01\x02" {
            return Err(Error::Malformed("invalid zip central directory header"));
        }
        let method = u16::from_le_bytes([archive[pos + 10], archive[pos + 11]]);
        let expected_crc = u32::from_le_bytes([
            archive[pos + 16],
            archive[pos + 17],
            archive[pos + 18],
            archive[pos + 19],
        ]);
        let comp_size = u32::from_le_bytes([
            archive[pos + 20],
            archive[pos + 21],
            archive[pos + 22],
            archive[pos + 23],
        ]) as usize;
        let uncomp_size = u32::from_le_bytes([
            archive[pos + 24],
            archive[pos + 25],
            archive[pos + 26],
            archive[pos + 27],
        ]) as usize;
        let name_len = u16::from_le_bytes([archive[pos + 28], archive[pos + 29]]) as usize;
        let extra_len = u16::from_le_bytes([archive[pos + 30], archive[pos + 31]]) as usize;
        let comment_len = u16::from_le_bytes([archive[pos + 32], archive[pos + 33]]) as usize;
        let local_offset = u32::from_le_bytes([
            archive[pos + 42],
            archive[pos + 43],
            archive[pos + 44],
            archive[pos + 45],
        ]) as usize;

        let name_start = pos + 46;
        let name_end = name_start + name_len;
        let next_pos = name_end + extra_len + comment_len;
        if next_pos > cd_end {
            return Err(Error::Malformed("truncated zip central directory entry"));
        }
        let name = std::str::from_utf8(&archive[name_start..name_end])
            .map_err(|_| Error::Malformed("invalid zip entry name"))?;
        pos = next_pos;

        let safe_path = !name.starts_with(['/', '\\'])
            && !name.ends_with(['/', '\\'])
            && !name.split(['/', '\\']).any(|seg| seg == "..");
        let basename = name.rsplit(['/', '\\']).next().unwrap_or("");
        if !safe_path || basename != target_name {
            continue;
        }
        if uncomp_size > MAX_ASSET_BYTES {
            return Err(Error::Rejected("zip entry exceeds size limit"));
        }
        if local_offset + 30 > cd_offset
            || &archive[local_offset..local_offset + 4] != b"PK\x03\x04"
        {
            return Err(Error::Malformed("invalid zip local file header"));
        }
        let l_name_len =
            u16::from_le_bytes([archive[local_offset + 26], archive[local_offset + 27]]) as usize;
        let l_extra_len =
            u16::from_le_bytes([archive[local_offset + 28], archive[local_offset + 29]]) as usize;
        let data_start = local_offset + 30 + l_name_len + l_extra_len;
        let data_end = data_start
            .checked_add(comp_size)
            .filter(|&end| end <= cd_offset)
            .ok_or(Error::Malformed("truncated zip entry payload"))?;
        let compressed = &archive[data_start..data_end];
        let out = match method {
            0 => {
                if compressed.len() != uncomp_size {
                    return Err(Error::Malformed("invalid stored zip entry size"));
                }
                compressed.to_vec()
            }
            8 => {
                let mut decoder =
                    DeflateDecoder::new(compressed).take((MAX_ASSET_BYTES + 1) as u64);
                let mut buf = Vec::with_capacity(uncomp_size);
                decoder
                    .read_to_end(&mut buf)
                    .map_err(|_| Error::Malformed("corrupt deflate zip stream"))?;
                if buf.len() > MAX_ASSET_BYTES {
                    return Err(Error::Rejected("zip entry exceeds size limit"));
                }
                if buf.len() != uncomp_size {
                    return Err(Error::Malformed("unexpected zip entry size"));
                }
                buf
            }
            _ => return Err(Error::Malformed("unsupported zip compression")),
        };
        let mut crc = Crc::new();
        crc.update(&out);
        if crc.sum() != expected_crc {
            return Err(Error::Malformed("invalid zip entry checksum"));
        }
        return Ok(out);
    }

    Err(Error::Malformed("update zip missing target binary"))
}

fn find_zip_eocd(archive: &[u8]) -> Result<usize> {
    if archive.len() < 22 {
        return Err(Error::Malformed("truncated zip archive"));
    }
    let min_offset = archive.len().saturating_sub(22 + u16::MAX as usize);
    for i in (min_offset..=archive.len() - 22).rev() {
        if &archive[i..i + 4] == b"PK\x05\x06" {
            let comment_len = u16::from_le_bytes([archive[i + 20], archive[i + 21]]) as usize;
            if i + 22 + comment_len == archive.len() {
                return Ok(i);
            }
        }
    }
    Err(Error::Malformed("invalid zip end of central directory"))
}

fn parse_tar_name(header: &[u8; 512]) -> String {
    let raw_str = |slice: &[u8]| {
        let end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
        String::from_utf8_lossy(&slice[..end]).into_owned()
    };
    let name = raw_str(&header[0..100]);
    let prefix = raw_str(&header[345..500]);
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    fn make_tar_gz(filename: &str, payload: &[u8]) -> Vec<u8> {
        let mut tar = Vec::new();
        let mut header = [0u8; 512];
        header[..filename.len()].copy_from_slice(filename.as_bytes());
        header[100..108].copy_from_slice(b"0000755\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        let size_str = format!("{:011o}\0", payload.len());
        header[124..136].copy_from_slice(size_str.as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[148..156].fill(b' ');
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum: usize = header.iter().map(|&b| b as usize).sum();
        let sum_str = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(sum_str.as_bytes());
        tar.extend_from_slice(&header);
        tar.extend_from_slice(payload);
        let rem = payload.len() % 512;
        if rem != 0 {
            tar.extend(std::iter::repeat_n(0u8, 512 - rem));
        }
        tar.extend(std::iter::repeat_n(0u8, 1024));
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&tar).unwrap();
        gz.finish().unwrap()
    }

    #[tokio::test]
    async fn downloads_verifies_sha256_extracts_and_blocks_foreign_redirect() {
        let server = MockServer::start().await;
        let payload = b"#!/bin/sh\necho campus-0.1.13\n";
        let tgz = make_tar_gz("campus", payload);
        let digest = hex::encode(Sha256::digest(&tgz));
        let sums = format!("{digest}  campus-bjtu-0.1.13-linux-x86_64.tar.gz\n");

        let api_body = serde_json::json!([
            {
                "tag_name": "cli-v0.1.13",
                "draft": false,
                "prerelease": false,
                "published_at": "2026-11-22T01:00:00Z",
                "html_url": "https://github.com/Tak1naa/BJTUselfService-Lite/releases/tag/cli-v0.1.13",
                "assets": [
                    {
                        "name": "campus-bjtu-0.1.13-linux-x86_64.tar.gz",
                        "browser_download_url": format!("{}/dl/archive.tar.gz", server.uri()),
                        "size": tgz.len()
                    },
                    {
                        "name": "SHA256SUMS.txt",
                        "browser_download_url": format!("{}/dl/SHA256SUMS.txt", server.uri()),
                        "size": sums.len()
                    }
                ]
            }
        ]);

        Mock::given(method("GET"))
            .and(path("/releases"))
            .respond_with(ResponseTemplate::new(200).set_body_json(api_body))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/dl/SHA256SUMS.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sums))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/dl/archive.tar.gz"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/cdn/archive.tar.gz", server.uri())),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/cdn/archive.tar.gz"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(tgz.clone()))
            .mount(&server)
            .await;

        let info = check_latest_from(&format!("{}/releases", server.uri()), "0.1.12", true)
            .await
            .unwrap()
            .unwrap();
        let asset = campus_core::update::select_asset(&info, "linux", "x86_64", None).unwrap();
        let sums_asset = campus_core::update::checksums_asset(&info).unwrap();
        let verified = download_verified_with_policy(asset, sums_asset, true)
            .await
            .unwrap();
        assert_eq!(extract_tar_gz_binary(&verified, "campus").unwrap(), payload);

        // Tampered digest fails verification
        let bad_sums = ReleaseAsset {
            name: "SHA256SUMS.txt".into(),
            url: format!("{}/dl/bad.txt", server.uri()),
            size: 10,
        };
        Mock::given(method("GET"))
            .and(path("/dl/bad.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "0000000000000000000000000000000000000000000000000000000000000000  campus-bjtu-0.1.13-linux-x86_64.tar.gz\n",
            ))
            .mount(&server)
            .await;
        assert!(matches!(
            download_verified_with_policy(asset, &bad_sums, true).await,
            Err(Error::Rejected("release checksum mismatch"))
        ));

        // Untrusted redirect domain is blocked
        Mock::given(method("GET"))
            .and(path("/evil"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", "https://evil.example/bin"),
            )
            .mount(&server)
            .await;
        assert!(matches!(
            get_bounded(&format!("{}/evil", server.uri()), 1024, true).await,
            Err(Error::SecureChannel)
        ));
    }

    fn make_zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        use flate2::{Compression, write::DeflateEncoder};
        let mut archive = Vec::new();
        let mut central = Vec::new();
        for &(name, data, deflate) in entries {
            let mut crc = Crc::new();
            crc.update(data);
            let sum = crc.sum();
            let comp = if deflate {
                let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
                enc.write_all(data).unwrap();
                enc.finish().unwrap()
            } else {
                data.to_vec()
            };
            let method: u16 = if deflate { 8 } else { 0 };
            let offset = archive.len() as u32;
            let nbytes = name.as_bytes();
            let nlen = nbytes.len() as u16;
            archive.extend_from_slice(b"PK\x03\x04");
            archive.extend_from_slice(&20u16.to_le_bytes());
            archive.extend_from_slice(&0u16.to_le_bytes());
            archive.extend_from_slice(&method.to_le_bytes());
            archive.extend_from_slice(&0u16.to_le_bytes());
            archive.extend_from_slice(&0u16.to_le_bytes());
            archive.extend_from_slice(&sum.to_le_bytes());
            archive.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            archive.extend_from_slice(&(data.len() as u32).to_le_bytes());
            archive.extend_from_slice(&nlen.to_le_bytes());
            archive.extend_from_slice(&0u16.to_le_bytes());
            archive.extend_from_slice(nbytes);
            archive.extend_from_slice(&comp);

            central.extend_from_slice(b"PK\x01\x02");
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&sum.to_le_bytes());
            central.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&nlen.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(nbytes);
        }
        let cd_offset = archive.len() as u32;
        let cd_size = central.len() as u32;
        archive.extend_from_slice(&central);
        archive.extend_from_slice(b"PK\x05\x06");
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        archive.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        archive.extend_from_slice(&cd_size.to_le_bytes());
        archive.extend_from_slice(&cd_offset.to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive
    }

    #[test]
    fn extracts_zip_stored_and_deflated_and_rejects_traversal_and_bad_crc() {
        let exe = b"MZ-synthetic-campus-exe";
        let ico = b"ICO-synthetic";
        let zip = make_zip(&[
            ("../campus.exe", b"evil", false),
            ("campus.ico", ico, false),
            ("nested/campus.exe", exe, true),
        ]);
        assert_eq!(extract_zip_binary(&zip, "campus.exe").unwrap(), exe);
        assert_eq!(extract_zip_binary(&zip, "campus.ico").unwrap(), ico);
        assert!(matches!(
            extract_zip_binary(&zip, "missing.exe"),
            Err(Error::Malformed("update zip missing target binary"))
        ));

        let mut bad_crc = make_zip(&[("campus.exe", exe, false)]);
        let last = bad_crc.len() - 32;
        bad_crc[last] ^= 0xff;
        assert!(extract_zip_binary(&bad_crc, "campus.exe").is_err());
    }
}
