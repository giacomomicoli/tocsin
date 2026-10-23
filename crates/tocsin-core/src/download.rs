//! Download a release zip into the per-user cache.
//!
//! The URL comes from an API answer, i.e. untrusted input, so before a single
//! byte is fetched the host must be one a provider is known to serve files
//! from. The body streams to disk under a size cap and is hashed on the way;
//! when the provider published a digest, a mismatch deletes the file.

use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt;

use crate::http::Http;
use crate::sources::Digest;

/// Largest zip we accept (Details and ElvUI are ~20 MiB; this is generous).
pub const MAX_ZIP_BYTES: u64 = 256 * 1024 * 1024;

/// Hosts that providers hand out download links on. Anything else is refused.
const ALLOWED_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
    "codeload.github.com",
    "edge.forgecdn.net",
    "mediafilez.forgecdn.net",
    "api.tukui.org",
    "cdn.wowinterface.com",
    "www.wowinterface.com",
];

/// Errors while downloading.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// Host is not a known provider CDN.
    #[error("refusing to download from {0}")]
    UntrustedHost(String),
    /// Not an `https://` URL or unparsable.
    #[error("invalid download url")]
    InvalidUrl,
    /// Larger than [`MAX_ZIP_BYTES`].
    #[error("file larger than {} MiB", MAX_ZIP_BYTES / 1024 / 1024)]
    TooLarge,
    /// Provider digest did not match what was downloaded.
    #[error("checksum mismatch: the download is corrupt or tampered")]
    DigestMismatch,
    /// Transport failure.
    #[error("download failed: {0}")]
    Http(String),
    /// Disk failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A finished download.
#[derive(Debug, Clone)]
pub struct Downloaded {
    /// Where the zip was written.
    pub path: PathBuf,
    /// Hex SHA-256 of the bytes on disk.
    pub sha256: String,
    /// Size in bytes.
    pub bytes: u64,
}

/// Whether `url` points at a host we accept zips from.
pub fn is_trusted(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https" && parsed.host_str().is_some_and(is_trusted_host)
}

/// Whether zips may come from `host`. Also applied to every redirect hop of
/// a download (see [`crate::http::redirect_verdict`]).
pub fn is_trusted_host(host: &str) -> bool {
    ALLOWED_HOSTS.iter().any(|a| host.eq_ignore_ascii_case(a))
}

/// Download `url` into `dir/<file_name>`, verifying `expected` when given.
/// `file_name` must be a plain name (the caller builds it from provider,
/// id and version, never from the URL).
pub async fn fetch(
    http: &Http,
    url: &str,
    dir: &Path,
    file_name: &str,
    expected: Option<&Digest>,
) -> Result<Downloaded, DownloadError> {
    if !is_trusted(url) {
        let host = reqwest::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .ok_or(DownloadError::InvalidUrl)?;
        return Err(DownloadError::UntrustedHost(host));
    }
    if file_name.is_empty() || file_name.contains(['/', '\\']) || file_name.starts_with('.') {
        return Err(DownloadError::InvalidUrl);
    }
    tokio::fs::create_dir_all(dir).await?;
    // The fallback cache is under the shared temp dir, where anyone can plant
    // a link: neither the folder nor the `.part` file is ever written through
    // one. A leftover `.part` (or a link in its place) is removed, then the
    // file is created new.
    if tokio::fs::symlink_metadata(dir).await?.file_type().is_symlink() {
        return Err(DownloadError::Io(std::io::Error::other(
            "the download folder is a link",
        )));
    }
    let path = dir.join(file_name);
    let tmp = dir.join(format!("{file_name}.part"));
    if tokio::fs::symlink_metadata(&tmp).await.is_ok() {
        tokio::fs::remove_file(&tmp).await?;
    }

    let mut resp = http
        .get_bytes(url)
        .await
        .map_err(|e| DownloadError::Http(e.to_string()))?;
    if resp.content_length().is_some_and(|len| len > MAX_ZIP_BYTES) {
        return Err(DownloadError::TooLarge);
    }

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .await?;
    let mut hasher = Hashers::new(expected);
    let mut total: u64 = 0;
    let result: Result<(), DownloadError> = async {
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| DownloadError::Http(e.to_string()))?
        {
            total += chunk.len() as u64;
            if total > MAX_ZIP_BYTES {
                return Err(DownloadError::TooLarge);
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(())
    }
    .await;
    drop(file);
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }

    let (sha256, matches) = hasher.finish(expected);
    if !matches {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(DownloadError::DigestMismatch);
    }
    tokio::fs::rename(&tmp, &path).await?;
    Ok(Downloaded {
        path,
        sha256,
        bytes: total,
    })
}

/// The digests computed while a download streams: always SHA-256 (our own
/// install record), plus the one algorithm the provider's digest uses.
///
/// SHA-1/MD5 used to be a second pass that read the whole zip back into
/// memory (up to [`MAX_ZIP_BYTES`]), on every WoWInterface download since it
/// publishes MD5. Hashing the chunks as they arrive costs no extra I/O.
enum Extra {
    None,
    Sha1(sha1_smol::Sha1),
    Md5(md5::Context),
}

struct Hashers {
    sha256: Sha256,
    extra: Extra,
}

impl Hashers {
    fn new(expected: Option<&Digest>) -> Self {
        let extra = match expected {
            Some(Digest::Sha1(_)) => Extra::Sha1(sha1_smol::Sha1::new()),
            Some(Digest::Md5(_)) => Extra::Md5(md5::Context::new()),
            Some(Digest::Sha256(_)) | None => Extra::None,
        };
        Self {
            sha256: Sha256::new(),
            extra,
        }
    }

    fn update(&mut self, chunk: &[u8]) {
        self.sha256.update(chunk);
        match &mut self.extra {
            Extra::None => {}
            Extra::Sha1(h) => h.update(chunk),
            Extra::Md5(h) => h.consume(chunk),
        }
    }

    /// Hex SHA-256, and whether the bytes match `expected` (true without one).
    fn finish(self, expected: Option<&Digest>) -> (String, bool) {
        let sha256 = hex(&self.sha256.finalize());
        let matches = match (expected, self.extra) {
            (None, _) => true,
            (Some(Digest::Sha256(h)), _) => h.eq_ignore_ascii_case(&sha256),
            (Some(Digest::Sha1(h)), Extra::Sha1(s)) => hex(&s.digest().bytes()).eq_ignore_ascii_case(h),
            (Some(Digest::Md5(h)), Extra::Md5(m)) => hex(&m.finalize().0).eq_ignore_ascii_case(h),
            // `new` and `finish` were given different digests: refuse rather
            // than accept an unverified file.
            (Some(_), _) => false,
        };
        (sha256, matches)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusts_only_provider_hosts() {
        assert!(is_trusted(
            "https://github.com/WeakAuras/WeakAuras2/releases/download/5.22.1/WeakAuras-5.22.1.zip"
        ));
        assert!(is_trusted("https://edge.forgecdn.net/files/1/2/x.zip"));
        assert!(is_trusted("https://api.tukui.org/v1/download/elvui-16.00.zip"));
        assert!(!is_trusted("http://github.com/x.zip"));
        assert!(!is_trusted("https://evil.example.com/github.com/x.zip"));
        assert!(!is_trusted("https://github.com.evil.example/x.zip"));
        assert!(!is_trusted("file:///etc/passwd"));
    }

    /// Feed `data` in uneven chunks, as a network body arrives.
    fn hash_in_chunks(data: &[u8], expected: Option<&Digest>) -> (String, bool) {
        let mut h = Hashers::new(expected);
        for chunk in data.chunks(7) {
            h.update(chunk);
        }
        h.finish(expected)
    }

    #[test]
    fn streamed_digests_match_known_vectors() {
        let data = b"The quick brown fox jumps over the lazy dog";
        let sha256 = "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592";
        let sha1 = Digest::Sha1("2FD4E1C67A2D28FCED849EE1BB76E7391B93EB12".into());
        let md5 = Digest::Md5("9e107d9d372bb6826bd81d3542a419d6".into());

        for expected in [None, Some(&sha1), Some(&md5)] {
            let (got, ok) = hash_in_chunks(data, expected);
            assert_eq!(got, sha256);
            assert!(ok, "{expected:?}");
        }
        let (_, ok) = hash_in_chunks(data, Some(&Digest::Sha256(sha256.to_uppercase())));
        assert!(ok);

        // One byte off: every algorithm must refuse it.
        let tampered = b"The quick brown fox jumps over the lazy cog";
        for expected in [&sha1, &md5, &Digest::Sha256(sha256.into())] {
            assert!(!hash_in_chunks(tampered, Some(expected)).1, "{expected:?}");
        }

        // Empty input, MD5's best-known vector.
        let empty_md5 = Digest::Md5("d41d8cd98f00b204e9800998ecf8427e".into());
        assert!(hash_in_chunks(b"", Some(&empty_md5)).1);
    }

    #[tokio::test]
    async fn refuses_untrusted_before_any_io() {
        let tmp = tempfile::tempdir().unwrap();
        let err = fetch(
            &Http::new(),
            "https://example.com/x.zip",
            tmp.path(),
            "x.zip",
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::UntrustedHost(_)));
        assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none());
    }
}
