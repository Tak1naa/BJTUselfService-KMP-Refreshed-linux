//! Resumable media I/O. Never downloads keys or encrypted HLS streams.
use crate::transport::{SchoolTransport, same_origin};
use campus_core::{Error, Result};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::{
    io::{Read, Write},
    path::Path,
};
use url::Url;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, GetFileInformationByHandle,
};
fn url_hash(url: &Url) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(url.as_str().as_bytes()))
}
fn io(e: impl std::fmt::Display) -> Error {
    Error::Storage(e.to_string())
}
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Playlist {
    pub segments: Vec<Url>,
}
pub fn playlist(body: &str, base: &Url) -> Result<Playlist> {
    if !body.trim_start().starts_with("#EXTM3U") {
        return Err(Error::Malformed("HLS header"));
    }
    let mut segments = Vec::new();
    for line in body.lines().map(str::trim).filter(|s| !s.is_empty()) {
        if line.starts_with("#EXT-X-KEY:")
            && !line
                .split_once(':')
                .unwrap()
                .1
                .split(',')
                .any(|s| s == "METHOD=NONE")
        {
            return Err(Error::Rejected("encrypted HLS is unsupported"));
        }
        if line.starts_with("#EXT-X-MAP")
            || line.starts_with("#EXT-X-BYTERANGE")
            || line.starts_with("#EXT-X-STREAM-INF")
        {
            return Err(Error::Rejected(
                "HLS master, fMP4 or byte-range playlist requires a media playlist",
            ));
        }
        if !line.starts_with('#') {
            let url = base.join(line).map_err(|_| Error::Malformed("HLS URL"))?;
            if !same_origin(base, &url) {
                return Err(Error::SecureChannel);
            }
            segments.push(url);
        }
    }
    if !body.contains("#EXT-X-ENDLIST") || segments.is_empty() {
        return Err(Error::Rejected("only completed VOD playlists are accepted"));
    }
    Ok(Playlist { segments })
}
fn private_open(path: &Path, append: bool) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .append(append);
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    #[cfg(windows)]
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    let f = options.open(path).map_err(io)?;
    let m = f.metadata().map_err(io)?;
    if !m.is_file() || m.file_type().is_symlink() {
        return Err(io("unsafe download file type"));
    }
    #[cfg(unix)]
    if m.nlink() != 1
        || m.uid() != rustix::process::getuid().as_raw()
        || m.permissions().mode() & 0o077 != 0
    {
        return Err(io("unsafe download file permissions or links"));
    }
    #[cfg(windows)]
    {
        use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
        let mut info = MaybeUninit::uninit();
        // The handle belongs to the live file and Windows initializes info on success.
        if unsafe { GetFileInformationByHandle(f.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(io(std::io::Error::last_os_error()));
        }
        let info = unsafe { info.assume_init() };
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || info.nNumberOfLinks != 1 {
            return Err(io("unsafe download reparse point or links"));
        }
    }
    if !append {
        f.set_len(0).map_err(io)?;
    }
    Ok(f)
}
fn suffix(path: &Path, ending: &str) -> std::path::PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(ending);
    p.into()
}
impl SchoolTransport {
    fn media_allowed(&self, url: &Url) -> bool {
        self.endpoints.school_allowed(url)
            || (url.scheme() == "https"
                && url.port_or_known_default() == Some(8880)
                && url.username().is_empty()
                && url.password().is_none()
                && url
                    .host_str()
                    .is_some_and(|h| h.starts_with("lmt") && h.ends_with(".bjtu.edu.cn")))
    }
    /// Range checkpoints store an ETag and the URL, so a different resource cannot append to old data.
    pub async fn download(&self, url: Url, path: &Path) -> Result<u64> {
        let allowed = if self.legacy_courseware() {
            self.legacy_download_allowed(&url)
        } else {
            self.media_allowed(&url)
        };
        if !allowed {
            return Err(Error::SecureChannel);
        }
        if path.exists() {
            return Err(io("output exists; refusing to overwrite"));
        }
        let part = suffix(path, ".campus-part");
        let meta = suffix(path, ".campus-resume");
        let previous: Option<serde_json::Value> = if meta.exists() {
            let mut b = Vec::new();
            private_open(&meta, true)?
                .take(1024 * 1024)
                .read_to_end(&mut b)
                .map_err(io)?;
            serde_json::from_slice(&b).ok()
        } else {
            None
        };
        // Persist only a URL digest; signed query parameters never enter checkpoint files.
        let mut offset = if previous
            .as_ref()
            .is_some_and(|v| v["url_hash"].as_str() == Some(url_hash(&url).as_str()))
        {
            std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0)
        } else {
            0
        };
        let mut file = private_open(&part, offset > 0)?;
        let mut etag = previous.and_then(|v| v["etag"].as_str().map(String::from));
        loop {
            let end = offset + 4 * 1024 * 1024 - 1;
            let mut req = self
                .media_client()
                .get(url.clone())
                .timeout(std::time::Duration::from_secs(900))
                .header(reqwest::header::RANGE, format!("bytes={offset}-{end}"));
            if let Some(tag) = &etag {
                req = req.header(reqwest::header::IF_RANGE, tag);
            }
            let mut response = req.send().await.map_err(|_| Error::Network)?;
            // Without a validator a multi-range transfer cannot safely join versions.
            if offset == 0
                && response.status() == reqwest::StatusCode::PARTIAL_CONTENT
                && response.headers().get(reqwest::header::ETAG).is_none()
            {
                response = self
                    .media_client()
                    .get(url.clone())
                    .timeout(std::time::Duration::from_secs(900))
                    .send()
                    .await
                    .map_err(|_| Error::Network)?;
                if response.status() != reqwest::StatusCode::OK {
                    return Err(Error::Rejected(
                        "server cannot provide a validated range or full response",
                    ));
                }
            }
            let status = response.status();
            if self.legacy_courseware()
                && response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.starts_with("text/html") || v.contains("json"))
            {
                return Err(Error::Rejected("download returned a web page or error"));
            }
            if status.is_redirection() {
                return Err(Error::SecureChannel);
            }
            if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
                return Err(Error::Rejected("range checkpoint no longer valid"));
            }
            if status != reqwest::StatusCode::OK && status != reqwest::StatusCode::PARTIAL_CONTENT {
                return Err(Error::Network);
            }
            let received_etag = response
                .headers()
                .get(reqwest::header::ETAG)
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            if offset > 0
                && (status == reqwest::StatusCode::OK || etag.is_none() || etag != received_etag)
            {
                return Err(Error::Rejected(
                    "resource changed or cannot safely resume; remove campus-part and campus-resume files",
                ));
            }
            if etag.is_none() {
                etag = received_etag;
            }
            let range = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            let total = range
                .as_deref()
                .and_then(|s| s.rsplit('/').next())
                .and_then(|s| s.parse::<u64>().ok());
            if status == reqwest::StatusCode::PARTIAL_CONTENT
                && !range
                    .as_deref()
                    .is_some_and(|s| s.starts_with(&format!("bytes {offset}-")))
            {
                return Err(Error::Malformed("Content-Range"));
            }
            let snapshot = serde_json::json!({"url_hash":url_hash(&url),"etag":etag});
            let mut m = private_open(&meta, false)?;
            m.write_all(snapshot.to_string().as_bytes()).map_err(io)?;
            m.sync_all().map_err(io)?;
            use futures_util::StreamExt;
            let mut stream = response.bytes_stream();
            let start = offset;
            while let Some(chunk) = stream.next().await {
                let bytes = chunk.map_err(|_| Error::Network)?;
                file.write_all(&bytes).map_err(io)?;
                offset += bytes.len() as u64;
            }
            file.sync_all().map_err(io)?;
            if status == reqwest::StatusCode::OK || total == Some(offset) {
                break;
            }
            if offset == start || total.is_none() {
                return Err(Error::Malformed("incomplete range response"));
            }
        }
        drop(file);
        std::fs::hard_link(&part, path).map_err(io)?;
        std::fs::remove_file(part).map_err(io)?;
        let _ = std::fs::remove_file(meta);
        Ok(offset)
    }
    async fn vod_playlist(&self, mut url: Url) -> Result<Playlist> {
        for _ in 0..4 {
            if !self.media_allowed(&url) {
                return Err(Error::SecureChannel);
            }
            let r = self
                .media_client()
                .get(url.clone())
                .send()
                .await
                .map_err(|_| Error::Network)?;
            if r.status() != reqwest::StatusCode::OK {
                return Err(Error::Network);
            }
            use futures_util::StreamExt;
            let mut stream = r.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(b) = stream.next().await {
                let b = b.map_err(|_| Error::Network)?;
                if bytes.len() + b.len() > 4 * 1024 * 1024 {
                    return Err(Error::Malformed("playlist size"));
                }
                bytes.extend_from_slice(&b);
            }
            let body =
                std::str::from_utf8(&bytes).map_err(|_| Error::Malformed("playlist encoding"))?;
            if body.contains("#EXT-X-SESSION-KEY") {
                return Err(Error::Rejected("encrypted HLS is unsupported"));
            }
            if body.contains("#EXT-X-STREAM-INF:") {
                let line = body
                    .lines()
                    .map(str::trim)
                    .find(|s| !s.is_empty() && !s.starts_with('#'))
                    .ok_or(Error::Malformed("master playlist"))?;
                let next = url.join(line).map_err(|_| Error::SecureChannel)?;
                if !same_origin(&url, &next) {
                    return Err(Error::SecureChannel);
                }
                url = next;
                continue;
            }
            return playlist(body, &url);
        }
        Err(Error::Rejected("playlist nesting limit"))
    }
    pub async fn record(&self, url: Url, path: &Path, remux: bool) -> Result<usize> {
        if !self.media_allowed(&url) {
            return Err(Error::SecureChannel);
        }
        let list = self.vod_playlist(url).await?;
        let dir = suffix(path, ".segments");
        let builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        builder
            .create(&dir)
            .or_else(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(e)
                }
            })
            .map_err(io)?;
        let metadata = std::fs::symlink_metadata(&dir).map_err(io)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(io("unsafe segment directory"));
        }
        #[cfg(unix)]
        if metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(io("unsafe segment directory"));
        }
        #[cfg(windows)]
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io("unsafe segment directory reparse point"));
        }
        let manifest = dir.join("manifest.json");
        let encoded = serde_json::to_vec(&list.segments.iter().map(url_hash).collect::<Vec<_>>())
            .map_err(io)?;
        if manifest.exists() {
            let mut existing = Vec::new();
            private_open(&manifest, true)?
                .take(4 * 1024 * 1024)
                .read_to_end(&mut existing)
                .map_err(io)?;
            if existing != encoded {
                return Err(Error::Rejected("recording playlist changed"));
            }
        } else {
            private_open(&manifest, false)?
                .write_all(&encoded)
                .map_err(io)?;
        }
        use futures_util::{StreamExt, stream};
        let results = stream::iter(list.segments.iter().enumerate().map(|(i, u)| {
            let p = dir.join(format!("{i:08}.ts"));
            async move {
                if p.exists() {
                    return Ok(());
                }
                self.download(u.clone(), &p).await.map(|_| ())
            }
        }))
        .buffer_unordered(4)
        .collect::<Vec<_>>()
        .await;
        for r in results {
            r?;
        }
        let joined = suffix(path, ".campus-join");
        let mut target = private_open(&joined, false)?;
        for i in 0..list.segments.len() {
            let mut f = private_open(&dir.join(format!("{i:08}.ts")), true)?;
            std::io::copy(&mut f, &mut target).map_err(io)?;
        }
        target.sync_all().map_err(io)?;
        drop(target);
        std::fs::hard_link(&joined, path).map_err(io)?;
        std::fs::remove_file(joined).map_err(io)?;
        if remux {
            let mp4 = path.with_extension("mp4");
            let status = std::process::Command::new("ffmpeg")
                .args(["-nostdin", "-n", "-i"])
                .arg(path)
                .args(["-c", "copy"])
                .arg(&mp4)
                .status()
                .map_err(io)?;
            if !status.success() {
                return Err(io("ffmpeg remux failed; TS output was retained"));
            }
        }
        Ok(list.segments.len())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_rejected() {
        let url = "https://lmt1.bjtu.edu.cn:8880/a.m3u8".parse().unwrap();
        assert!(
            playlist(
                "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=key\na.ts\n#EXT-X-ENDLIST",
                &url
            )
            .is_err()
        );
    }
    #[test]
    fn order_and_origin() {
        let u = "https://lmt1.bjtu.edu.cn:8880/v/a.m3u8".parse().unwrap();
        let p = playlist("#EXTM3U\n2.ts\n1.ts\n#EXT-X-ENDLIST", &u).unwrap();
        assert!(p.segments[0].path().ends_with("2.ts"));
        assert!(playlist("#EXTM3U\nhttps://evil.example/a.ts\n#EXT-X-ENDLIST", &u).is_err());
    }
    #[tokio::test]
    async fn resumes_a_validated_range_and_never_overwrites_output() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, path},
        };
        let server = MockServer::start().await;
        Mock::given(path("/blob"))
            .and(header("range", "bytes=3-4194306"))
            .respond_with(
                ResponseTemplate::new(206)
                    .insert_header("etag", "\"fixture\"")
                    .insert_header("content-range", "bytes 3-5/6")
                    .set_body_bytes(b"def"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let transport =
            SchoolTransport::new(crate::Endpoints::for_test(&server.uri()), None).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("fixture.bin");
        let url: Url = format!("{}/blob", server.uri()).parse().unwrap();
        private_open(&suffix(&output, ".campus-part"), false)
            .unwrap()
            .write_all(b"abc")
            .unwrap();
        private_open(&suffix(&output, ".campus-resume"), false)
            .unwrap()
            .write_all(
                serde_json::json!({"url_hash":url_hash(&url),"etag":"\"fixture\""})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        assert_eq!(transport.download(url.clone(), &output).await.unwrap(), 6);
        assert_eq!(std::fs::read(&output).unwrap(), b"abcdef");
        assert!(transport.download(url, &output).await.is_err());
        server.verify().await;
    }
    #[tokio::test]
    async fn master_playlist_and_join_preserve_segment_order() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let server = MockServer::start().await;
        for (p, body) in [
            (
                "/master.m3u8",
                "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nmedia.m3u8",
            ),
            ("/media.m3u8", "#EXTM3U\n2.ts\n1.ts\n#EXT-X-ENDLIST"),
            ("/2.ts", "first"),
            ("/1.ts", "second"),
        ] {
            Mock::given(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(&server)
                .await;
        }
        let t = SchoolTransport::new(crate::Endpoints::for_test(&server.uri()), None).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("fixture.ts");
        assert_eq!(
            t.record(
                format!("{}/master.m3u8", server.uri()).parse().unwrap(),
                &output,
                false
            )
            .await
            .unwrap(),
            2
        );
        assert_eq!(std::fs::read(output).unwrap(), b"firstsecond");
    }
}
