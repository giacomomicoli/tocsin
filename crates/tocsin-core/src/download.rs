//! Download a release zip into the per-user cache.
//!
//! The URL comes from an API answer, i.e. untrusted input, so before a single
//! byte is fetched the host must be one a provider is known to serve files
//! from. The body streams to disk under a size cap and is hashed on the way;
//! when the provider published a digest, a mismatch deletes the file.
//!
//! The cap is [`MAX_ZIP_BYTES`] unless the provider declared the zip's size
//! before the download (see [`limit`]): voice-over packs are 300–500 MB of
//! audio, and CurseForge says so in the file's `fileLength`. A connection
//! that drops half way is picked up where it stopped with a `Range` request,
//! within the same download only (see [`fetch`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderMap, CONTENT_ENCODING, CONTENT_RANGE, ETAG, LAST_MODIFIED};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::disk::{self, human_size};
use crate::http::{Http, RangeFrom};
use crate::sources::Digest;

/// Largest zip accepted when the provider does not declare its size up front
/// (Details and ElvUI are ~20 MiB; this is generous).
pub const MAX_ZIP_BYTES: u64 = 256 * 1024 * 1024;

/// Largest zip accepted at all, even with a declared size. The Spoken
/// voice-over packs are 320–475 MB; their zips are stored, not compressed,
/// so this matches the 2 GiB `archive` lets a zip unpack to.
pub const MAX_DECLARED_ZIP_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Times a dropped connection is picked up again within one download.
const MAX_RESUMES: u32 = 3;

/// Slowest line a download's deadline allows for: 1 Mbit/s, so a 453 MB
/// pack gets an hour. The idle timeout ([`crate::http::DOWNLOAD_IDLE`])
/// catches a dead connection long before.
const MIN_RATE: u64 = 128 * 1024;

/// Shortest deadline, whatever the size: what every download had before.
const MIN_DEADLINE: Duration = Duration::from_secs(300);

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
    /// Larger than the cap that applies (see [`limit`]).
    #[error("file larger than {}", human_size(*.0))]
    TooLarge(u64),
    /// The body ended before the size the provider or the server announced.
    #[error("the download ended early; try again")]
    Incomplete,
    /// The server's file is not the size the provider declared.
    #[error("the file is not the size the site lists")]
    SizeMismatch,
    /// Provider digest did not match what was downloaded.
    #[error("checksum mismatch: the download is corrupt or tampered")]
    DigestMismatch,
    /// Not enough room for the zip, found before downloading.
    #[error(transparent)]
    NoSpace(#[from] disk::NoSpace),
    /// The disk filled up while the zip was written.
    #[error("the disk is full")]
    DiskFull,
    /// Transport failure.
    #[error("download failed: {0}")]
    Http(String),
    /// Disk failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A disk error, with "disk full" told apart.
fn io_error(e: std::io::Error) -> DownloadError {
    if disk::is_full(&e) {
        DownloadError::DiskFull
    } else {
        DownloadError::Io(e)
    }
}

/// A transport error for the user: no URL, and a stall said plainly.
fn transport(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "the server stopped sending".into()
    } else {
        e.without_url().to_string()
    }
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

/// What the provider said about the file before it was downloaded.
#[derive(Debug, Clone, Copy, Default)]
pub struct Expected<'a> {
    /// Its digest, checked once the file is on disk.
    pub digest: Option<&'a Digest>,
    /// Its size in bytes (see [`limit`]).
    pub size: Option<u64>,
}

/// How many bytes a download may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limit {
    /// Never more than this.
    pub max: u64,
    /// The file must be exactly `max` bytes: its size was declared, and is
    /// what let it past [`MAX_ZIP_BYTES`].
    pub exact: bool,
}

/// The cap for a file whose provider declared `declared` bytes (or nothing).
///
/// Up to [`MAX_ZIP_BYTES`] nothing changes, declared or not: an addon that
/// downloaded before still does, even if its provider's size is off by a
/// byte. Past it, the declared size is the cap, and the file must be exactly
/// that size; past [`MAX_DECLARED_ZIP_BYTES`] it is refused before any
/// request.
pub fn limit(declared: Option<u64>) -> Result<Limit, DownloadError> {
    match declared {
        Some(n) if n > MAX_DECLARED_ZIP_BYTES => Err(DownloadError::TooLarge(MAX_DECLARED_ZIP_BYTES)),
        Some(n) if n > MAX_ZIP_BYTES => Ok(Limit { max: n, exact: true }),
        _ => Ok(Limit {
            max: MAX_ZIP_BYTES,
            exact: false,
        }),
    }
}

/// Whether a file of `declared` bytes is larger than the usual cap, so the
/// user is asked before it is downloaded for a first install.
pub fn is_large(declared: Option<u64>) -> bool {
    declared.is_some_and(|n| n > MAX_ZIP_BYTES)
}

/// The deadline of one request for a file of at most `max` bytes.
fn deadline(max: u64) -> Duration {
    MIN_DEADLINE.max(Duration::from_secs(max / MIN_RATE))
}

/// How far a download has come, shared with whoever shows it. Cloning
/// shares the same counters; reading them never blocks the download.
#[derive(Debug, Clone, Default)]
pub struct Progress(Arc<Counters>);

#[derive(Debug, Default)]
struct Counters {
    done: AtomicU64,
    /// 0 while the size is unknown.
    total: AtomicU64,
    finished: AtomicBool,
}

impl Progress {
    /// Fresh counters, at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes received, and the file's size when it is known.
    pub fn get(&self) -> (u64, Option<u64>) {
        let total = self.0.total.load(Ordering::Relaxed);
        (self.0.done.load(Ordering::Relaxed), (total > 0).then_some(total))
    }

    /// Share received, from 0 to 1, when the size is known.
    pub fn fraction(&self) -> Option<f32> {
        let (done, total) = self.get();
        total.map(|t| (done as f64 / t as f64).clamp(0.0, 1.0) as f32)
    }

    /// Whether the zip is on disk and verified: what follows is the install.
    pub fn finished(&self) -> bool {
        self.0.finished.load(Ordering::Relaxed)
    }

    /// "120 MB of 453 MB", or "12 MB" while the size is unknown.
    pub fn label(&self) -> String {
        match self.get() {
            (done, Some(total)) => format!("{} of {}", human_size(done), human_size(total)),
            (done, None) => human_size(done),
        }
    }

    fn start(&self, total: Option<u64>) {
        self.0.done.store(0, Ordering::Relaxed);
        self.0.total.store(total.unwrap_or(0), Ordering::Relaxed);
    }

    fn set_done(&self, done: u64) {
        self.0.done.store(done, Ordering::Relaxed);
    }

    fn finish(&self) {
        self.0.finished.store(true, Ordering::Relaxed);
    }
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

/// Download `url` into `dir/<file_name>`, under the cap [`limit`] sets for
/// `expected.size` and verifying `expected.digest` when given. `file_name`
/// must be a plain name (the caller builds it from provider, id and version,
/// never from the URL). `progress` follows the bytes as they arrive.
///
/// A connection that drops is picked up where it stopped, at most
/// [`MAX_RESUMES`] times, from the URL that answered (already vetted) and
/// only for the same file (`If-Range`). The `.part` file stays open
/// throughout and is never reopened: a resume only ever appends to bytes
/// this call wrote and hashed. A `.part` left by an earlier run is deleted,
/// never resumed: it may sit in the shared temp dir, where anyone could have
/// written it.
pub async fn fetch(
    http: &Http,
    url: &str,
    dir: &Path,
    file_name: &str,
    expected: Expected<'_>,
    progress: &Progress,
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
    let limit = limit(expected.size)?;
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

    let deadline = deadline(limit.max);
    let resp = http
        .get_bytes(url, None, deadline)
        .await
        .map_err(|e| DownloadError::Http(e.to_string()))?;
    let announced = resp.content_length();
    if announced.is_some_and(|len| len > limit.max) {
        return Err(DownloadError::TooLarge(limit.max));
    }
    // A file past the usual cap must be the one the provider described: a
    // server announcing another size is not worth downloading 450 MB for.
    if limit.exact && announced.is_some_and(|len| len != limit.max) {
        return Err(DownloadError::SizeMismatch);
    }
    let size = if limit.exact { Some(limit.max) } else { announced };
    ensure_space(dir, size.unwrap_or(0)).await?;
    progress.start(size);

    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .await?;
    let mut sink = Sink {
        file,
        hasher: Hashers::new(expected.digest),
        total: 0,
        size,
    };
    let origin = Origin::of(&resp);
    let result = receive(http, resp, &mut sink, &origin, limit, expected, progress).await;
    let Sink {
        file, hasher, total, ..
    } = sink;
    drop(file);
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }

    let (sha256, matches) = hasher.finish(expected.digest);
    if !matches {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(DownloadError::DigestMismatch);
    }
    tokio::fs::rename(&tmp, &path).await?;
    progress.finish();
    Ok(Downloaded {
        path,
        sha256,
        bytes: total,
    })
}

/// Refuse a download of `needed` bytes that `dir`'s disk has no room for.
async fn ensure_space(dir: &Path, needed: u64) -> Result<(), DownloadError> {
    let dir = dir.to_path_buf();
    match tokio::task::spawn_blocking(move || disk::ensure_space(&dir, needed)).await {
        Ok(checked) => Ok(checked?),
        // The check itself died: the write goes ahead, a full disk is still caught.
        Err(_) => Ok(()),
    }
}

/// Where a dropped download can be picked up: the URL the first answer came
/// from, and the validator that keeps it to the same file. `None` when the
/// first answer cannot be resumed byte for byte.
struct Origin {
    resume: Option<(String, Option<String>)>,
}

impl Origin {
    fn of(resp: &reqwest::Response) -> Self {
        // A body decoded on the way (Content-Encoding) has offsets that are
        // not the server's: resume only a plain body of known length.
        let plain = resp.content_length().is_some() && !resp.headers().contains_key(CONTENT_ENCODING);
        Self {
            resume: plain.then(|| (resp.url().to_string(), validator(resp.headers()))),
        }
    }
}

/// The first answer's strong `ETag`, else its `Last-Modified`, for
/// `If-Range`. A weak ETag (`W/"…"`) is not allowed there.
fn validator(headers: &HeaderMap) -> Option<String> {
    let get = |name| headers.get(name).and_then(|v| v.to_str().ok());
    get(ETAG)
        .filter(|e| !e.starts_with("W/"))
        .or_else(|| get(LAST_MODIFIED))
        .map(str::to_string)
}

/// The `.part` file being written, with what it holds so far.
struct Sink {
    file: tokio::fs::File,
    hasher: Hashers,
    total: u64,
    /// Size the file must reach, when announced or declared.
    size: Option<u64>,
}

/// Why a body stopped.
enum Stop {
    /// The connection: it may be picked up again.
    Transport(String),
    /// Anything else: the download is over.
    Fatal(DownloadError),
}

impl Sink {
    /// Write `resp`'s body after what is already there.
    async fn drain(
        &mut self,
        resp: &mut reqwest::Response,
        limit: Limit,
        progress: &Progress,
    ) -> Result<(), Stop> {
        while let Some(chunk) = resp.chunk().await.map_err(|e| Stop::Transport(transport(e)))? {
            self.total += chunk.len() as u64;
            if self.total > limit.max {
                return Err(Stop::Fatal(DownloadError::TooLarge(limit.max)));
            }
            self.hasher.update(&chunk);
            self.file
                .write_all(&chunk)
                .await
                .map_err(|e| Stop::Fatal(io_error(e)))?;
            progress.set_done(self.total);
        }
        Ok(())
    }

    /// Start over from an empty file, for a server that sent the whole file
    /// again instead of the rest.
    async fn restart(&mut self, digest: Option<&Digest>, size: Option<u64>) -> Result<(), DownloadError> {
        self.file.set_len(0).await.map_err(io_error)?;
        self.file.seek(std::io::SeekFrom::Start(0)).await?;
        self.hasher = Hashers::new(digest);
        self.total = 0;
        self.size = size;
        Ok(())
    }
}

/// Stream `resp` into `sink`, resuming a dropped connection, then check the
/// size it reached.
async fn receive(
    http: &Http,
    mut resp: reqwest::Response,
    sink: &mut Sink,
    origin: &Origin,
    limit: Limit,
    expected: Expected<'_>,
    progress: &Progress,
) -> Result<(), DownloadError> {
    let mut resumes = 0;
    loop {
        let why = match sink.drain(&mut resp, limit, progress).await {
            Ok(()) => break,
            Err(Stop::Fatal(e)) => return Err(e),
            Err(Stop::Transport(why)) => why,
        };
        let Some((url, if_range)) = &origin.resume else {
            return Err(DownloadError::Http(why));
        };
        // A connection that keeps failing, or a server that answers the
        // resume with something else, ends the download with the first error.
        let mut next = None;
        while next.is_none() && resumes < MAX_RESUMES {
            resumes += 1;
            tokio::time::sleep(Duration::from_secs(u64::from(resumes))).await;
            let range = RangeFrom {
                offset: sink.total,
                if_range: if_range.as_deref(),
            };
            next = http.get_bytes(url, Some(range), deadline(limit.max)).await.ok();
        }
        let Some(next) = next else {
            return Err(DownloadError::Http(why));
        };
        let content_range = next.headers().get(CONTENT_RANGE).and_then(|v| v.to_str().ok());
        let verdict = resume_verdict(next.status().as_u16(), content_range, sink.total, limit);
        tracing::info!("download dropped at {} bytes ({why}): {verdict:?}", sink.total);
        match verdict {
            Resume::Continue => {}
            Resume::Restart => {
                let size = next.content_length();
                if size.is_some_and(|n| n > limit.max) {
                    return Err(DownloadError::TooLarge(limit.max));
                }
                let size = if limit.exact { Some(limit.max) } else { size };
                sink.restart(expected.digest, size).await?;
                progress.start(size);
            }
            Resume::Refuse => return Err(DownloadError::Http(why)),
        }
        resp = next;
    }
    sink.file.flush().await.map_err(io_error)?;
    match sink.size {
        Some(size) if sink.total != size => Err(DownloadError::Incomplete),
        _ => Ok(()),
    }
}

/// What to do with the answer to a resume request.
#[derive(Debug, PartialEq, Eq)]
enum Resume {
    /// `206` with exactly the rest of the file: append it.
    Continue,
    /// `200`: the server sent the whole file (it ignores `Range`, or the
    /// file changed and `If-Range` said so): start over with it.
    Restart,
    /// Anything else: give up.
    Refuse,
}

/// Judge the answer to `Range: bytes=<offset>-` for a file under `limit`.
/// The piece must start at `offset` and run to the end of the file, and the
/// file must be the size it was (the declared one, under an exact limit).
fn resume_verdict(status: u16, content_range: Option<&str>, offset: u64, limit: Limit) -> Resume {
    match status {
        200 => Resume::Restart,
        206 => {
            let Some((start, end, total)) = content_range.and_then(parse_content_range) else {
                return Resume::Refuse;
            };
            let fits = start == offset && end < limit.max;
            let whole_rest = match total {
                Some(t) => end + 1 == t && (!limit.exact || t == limit.max),
                None => !limit.exact,
            };
            if fits && whole_rest {
                Resume::Continue
            } else {
                Resume::Refuse
            }
        }
        _ => Resume::Refuse,
    }
}

/// `bytes 1000-1999/2000` → `(1000, 1999, Some(2000))`; a `*` total is
/// `None`. Anything else, or an end before the start, is `None`.
fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes ")?;
    let (span, total) = rest.split_once('/')?;
    let (start, end) = span.split_once('-')?;
    let (start, end): (u64, u64) = (start.trim().parse().ok()?, end.trim().parse().ok()?);
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    (end >= start).then_some((start, end, total))
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

    /// Spoken Audio: Readables 2.3.1, as CurseForge declares it.
    const READABLES: u64 = 475_482_457;

    #[test]
    fn declared_size_lifts_the_cap_up_to_the_ceiling() {
        let usual = Limit {
            max: MAX_ZIP_BYTES,
            exact: false,
        };
        // Undeclared, or declared under the usual cap: exactly as before.
        assert_eq!(limit(None).unwrap(), usual);
        assert_eq!(limit(Some(7_207_095)).unwrap(), usual);
        assert_eq!(limit(Some(MAX_ZIP_BYTES)).unwrap(), usual);
        // A voice-over pack: its own size, exactly.
        assert_eq!(
            limit(Some(READABLES)).unwrap(),
            Limit {
                max: READABLES,
                exact: true
            }
        );
        assert!(limit(Some(MAX_DECLARED_ZIP_BYTES)).is_ok());
        let err = limit(Some(MAX_DECLARED_ZIP_BYTES + 1)).unwrap_err();
        assert!(matches!(err, DownloadError::TooLarge(MAX_DECLARED_ZIP_BYTES)));
        assert_eq!(err.to_string(), "file larger than 2.0 GB");

        assert!(is_large(Some(READABLES)));
        assert!(!is_large(Some(MAX_ZIP_BYTES)));
        assert!(!is_large(None));
    }

    #[test]
    fn deadline_grows_with_the_file() {
        assert_eq!(deadline(1024), MIN_DEADLINE);
        assert_eq!(deadline(MAX_ZIP_BYTES), Duration::from_secs(2048));
        // An hour for the largest Spoken pack: a 1 Mbit/s line makes it.
        assert_eq!(deadline(READABLES), Duration::from_secs(3627));
    }

    #[test]
    fn content_range_is_parsed_strictly() {
        assert_eq!(
            parse_content_range("bytes 1000-1999/2000"),
            Some((1000, 1999, Some(2000)))
        );
        assert_eq!(parse_content_range("bytes 0-0/*"), Some((0, 0, None)));
        assert_eq!(parse_content_range("bytes 5-4/10"), None);
        assert_eq!(parse_content_range("bytes */2000"), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
        assert_eq!(parse_content_range("bytes -1-5/10"), None);
        assert_eq!(parse_content_range(""), None);
    }

    #[test]
    fn resume_takes_only_the_rest_of_the_same_file() {
        let exact = Limit {
            max: READABLES,
            exact: true,
        };
        let at = 120_000_000;
        let rest = format!("bytes {at}-{}/{READABLES}", READABLES - 1);
        assert_eq!(resume_verdict(206, Some(&rest), at, exact), Resume::Continue);
        // The server ignored Range, or the file changed: the whole file again.
        assert_eq!(resume_verdict(200, None, at, exact), Resume::Restart);
        // Another offset, a piece short of the end, another file size.
        let wrong_start = format!("bytes {}-{}/{READABLES}", at - 1, READABLES - 1);
        assert_eq!(resume_verdict(206, Some(&wrong_start), at, exact), Resume::Refuse);
        let short = format!("bytes {at}-{}/{READABLES}", READABLES - 2);
        assert_eq!(resume_verdict(206, Some(&short), at, exact), Resume::Refuse);
        let grown = format!("bytes {at}-{READABLES}/{}", READABLES + 1);
        assert_eq!(resume_verdict(206, Some(&grown), at, exact), Resume::Refuse);
        // A declared size needs the total to compare with.
        let unknown = format!("bytes {at}-{}/*", READABLES - 1);
        assert_eq!(resume_verdict(206, Some(&unknown), at, exact), Resume::Refuse);
        assert_eq!(resume_verdict(206, None, at, exact), Resume::Refuse);
        assert_eq!(resume_verdict(416, None, at, exact), Resume::Refuse);

        // Undeclared: any size under the usual cap.
        let usual = limit(None).unwrap();
        assert_eq!(
            resume_verdict(206, Some("bytes 10-99/100"), 10, usual),
            Resume::Continue
        );
        assert_eq!(
            resume_verdict(206, Some("bytes 10-99/*"), 10, usual),
            Resume::Continue
        );
        let past_cap = format!("bytes 10-{MAX_ZIP_BYTES}/{}", MAX_ZIP_BYTES + 1);
        assert_eq!(resume_verdict(206, Some(&past_cap), 10, usual), Resume::Refuse);
    }

    #[test]
    fn validator_is_a_strong_etag_or_the_date() {
        let mut h = HeaderMap::new();
        assert_eq!(validator(&h), None);
        h.insert(LAST_MODIFIED, "Wed, 05 Nov 2026 08:25:00 GMT".parse().unwrap());
        assert_eq!(validator(&h).as_deref(), Some("Wed, 05 Nov 2026 08:25:00 GMT"));
        h.insert(ETAG, "W/\"abc\"".parse().unwrap());
        assert_eq!(validator(&h).as_deref(), Some("Wed, 05 Nov 2026 08:25:00 GMT"));
        h.insert(ETAG, "\"abc\"".parse().unwrap());
        assert_eq!(validator(&h).as_deref(), Some("\"abc\""));
    }

    #[test]
    fn progress_reads_like_the_footer_shows_it() {
        let p = Progress::new();
        assert_eq!(p.fraction(), None);
        p.start(None);
        p.set_done(12 * 1024 * 1024);
        assert_eq!(p.label(), "12 MB");
        p.start(Some(READABLES));
        p.set_done(READABLES / 4);
        assert_eq!(p.label(), "113 MB of 453 MB");
        assert!((p.fraction().unwrap() - 0.25).abs() < 0.001);
        // A clone is the same counters, as the GUI holds it.
        let shown = p.clone();
        p.set_done(READABLES);
        assert_eq!(shown.fraction(), Some(1.0));
        assert!(!shown.finished());
        p.finish();
        assert!(shown.finished());
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
            Expected::default(),
            &Progress::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::UntrustedHost(_)));
        assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn refuses_a_declared_size_past_the_ceiling_before_any_io() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("downloads");
        let expected = Expected {
            digest: None,
            size: Some(MAX_DECLARED_ZIP_BYTES + 1),
        };
        let err = fetch(
            &Http::new(),
            "https://edge.forgecdn.net/files/1/2/x.zip",
            &dir,
            "x.zip",
            expected,
            &Progress::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::TooLarge(_)));
        assert!(!dir.exists());
    }
}
