//! Addon icons, for search results and the library: where one may come
//! from, how much of it is read, how it is decoded, and the library's disk
//! cache.
//!
//! The URL comes from an API answer and the bytes from a CDN: both are
//! untrusted. A URL is used only when it is `https://` on one of
//! [`ALLOWED_HOSTS`], with no credentials and the default port; the body is
//! capped at [`MAX_ICON_BYTES`]; the format is read off the first bytes (PNG
//! or JPEG, never what the URL or the server claim); the decoder refuses a
//! picture over [`MAX_SIDE`] pixels a side or [`MAX_DECODE_ALLOC`] bytes of
//! memory, so a small file that unpacks into a huge one goes nowhere; and
//! what reaches the GUI is a [`SIDE`]-pixel square of RGBA, so the GUI
//! decodes nothing.
//!
//! Library icons are also kept on disk ([`fetch_cached`]), so a launch does
//! not download a logo per addon again: about 3 MB for 30 CurseForge logos.
//! The file kept is Tocsin's own 64px PNG, named after the URL's SHA-256,
//! refreshed after [`CACHE_MAX_AGE`]; reading it back goes through the same
//! size cap and decoder limits as a download. Search icons stay in memory.

use std::collections::{BTreeSet, HashMap};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{ExtendedColorType, ImageEncoder, ImageFormat, ImageReader, Limits};
use sha2::{Digest as _, Sha256};

use crate::addon::InstalledAddon;
use crate::http::{Http, HttpError};
use crate::sources::SourceRef;

/// Hosts the providers serve the pictures from: CurseForge's project logos
/// and WoWInterface's screenshot thumbnails. Anything else is refused.
pub const ALLOWED_HOSTS: &[&str] = &["media.forgecdn.net", "cdn-wow.mmoui.com"];

/// Largest icon file read. WoWInterface thumbnails were at most 25 KB in a
/// sample of 40, CurseForge's 256px logos 34–136 KB in a sample of 8
/// (October 2026).
pub const MAX_ICON_BYTES: usize = 256 * 1024;

/// How long a cached library icon is used before it is fetched again, and
/// after which [`prune_cache`] deletes it: an author rarely changes a logo.
pub const CACHE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Largest width or height decoded. Both providers serve far smaller.
pub const MAX_SIDE: u32 = 512;

/// Most memory the decoder may take: a [`MAX_SIDE`] square of RGBA is 1 MiB.
pub const MAX_DECODE_ALLOC: u64 = 4 * 1024 * 1024;

/// Side of the square handed to the GUI: twice the 32px icon slot, so it
/// stays sharp on a high-density screen.
pub const SIDE: u32 = 64;

/// A decoded icon: a [`SIDE`]-pixel square, RGBA, row by row.
#[derive(Clone, PartialEq, Eq)]
pub struct Icon {
    /// Width and height in pixels.
    pub side: u32,
    /// `side * side * 4` bytes.
    pub rgba: Vec<u8>,
}

/// Only the size: the pixels would flood any log that prints a message.
impl std::fmt::Debug for Icon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Icon({0}x{0})", self.side)
    }
}

/// Why an icon is not shown. The GUI falls back to the monogram either way.
#[derive(Debug, thiserror::Error)]
pub enum IconError {
    /// Not an `https://` URL on one of [`ALLOWED_HOSTS`].
    #[error("icon from an unexpected address")]
    Untrusted,
    /// Transport, status or size problem.
    #[error(transparent)]
    Http(#[from] HttpError),
    /// Neither PNG nor JPEG.
    #[error("icon is neither PNG nor JPEG")]
    Format,
    /// Corrupt, or over [`MAX_SIDE`] / [`MAX_DECODE_ALLOC`].
    #[error("icon could not be decoded: {0}")]
    Decode(String),
    /// The background decode did not finish.
    #[error("decoding the icon failed: {0}")]
    Task(String),
}

/// Whether icons may come from `host`. Also applied to every redirect hop
/// (see [`crate::http::redirect_verdict`]).
pub fn is_allowed_host(host: &str) -> bool {
    ALLOWED_HOSTS.iter().any(|a| host.eq_ignore_ascii_case(a))
}

/// Whether `url` may be fetched as an icon.
pub fn is_allowed(url: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    u.scheme() == "https"
        && u.username().is_empty()
        && u.password().is_none()
        && u.port().is_none()
        && u.host_str().is_some_and(is_allowed_host)
}

/// The format the bytes start with, among the two decoded.
fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else {
        None
    }
}

/// Decode `bytes` within the limits above and fill a [`SIDE`] square with
/// it, cropping the longer side: WoWInterface thumbnails are screenshots,
/// rarely square. Blocking: call it off the async workers.
pub fn decode(bytes: &[u8]) -> Result<Icon, IconError> {
    let format = sniff(bytes).ok_or(IconError::Format)?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let picture = reader.decode().map_err(|e| IconError::Decode(e.to_string()))?;
    let square = picture
        .resize_to_fill(SIDE, SIDE, FilterType::Triangle)
        .into_rgba8();
    Ok(Icon {
        side: SIDE,
        rgba: square.into_raw(),
    })
}

/// Fetch and decode the icon at `url` (a [`crate::search::Hit::icon`]).
pub async fn fetch(http: &Http, url: &str) -> Result<Icon, IconError> {
    if !is_allowed(url) {
        return Err(IconError::Untrusted);
    }
    let bytes = http.get_image(url, MAX_ICON_BYTES).await?;
    tokio::task::spawn_blocking(move || decode(&bytes))
        .await
        .map_err(|e| IconError::Task(e.to_string()))?
}

/// [`fetch`] through the disk cache: a fresh cached copy is used without a
/// request, and a downloaded icon is stored for the next launch. A cache
/// that cannot be read or written only costs the download.
pub async fn fetch_cached(http: &Http, url: &str) -> Result<Icon, IconError> {
    if !is_allowed(url) {
        return Err(IconError::Untrusted);
    }
    let dir = cache_dir();
    let key = url.to_string();
    let cached = {
        let (dir, key) = (dir.clone(), key.clone());
        tokio::task::spawn_blocking(move || read_cached(&dir, &key, SystemTime::now()))
            .await
            .ok()
            .flatten()
    };
    if let Some(icon) = cached {
        return Ok(icon);
    }
    let bytes = http.get_image(url, MAX_ICON_BYTES).await?;
    tokio::task::spawn_blocking(move || {
        let icon = decode(&bytes)?;
        if let Err(e) = write_cached(&dir, &key, &icon) {
            tracing::debug!(error = %e, "could not cache an icon");
        }
        Ok(icon)
    })
    .await
    .map_err(|e| IconError::Task(e.to_string()))?
}

/// Per-user folder of cached library icons, next to the downloads.
pub fn cache_dir() -> PathBuf {
    crate::paths::project_dirs()
        .map(|d| d.cache_dir().join("icons"))
        .unwrap_or_else(|| std::env::temp_dir().join("tocsin-icons"))
}

/// Delete cached icons older than [`CACHE_MAX_AGE`]. Top level only; links
/// and folders are left alone. Blocking: call it off the UI thread.
pub fn prune_cache() -> usize {
    crate::updater::prune_dir(&cache_dir(), CACHE_MAX_AGE, SystemTime::now())
}

/// The cache file of `url`: a hex SHA-256, so nothing of the URL reaches
/// the file name.
fn cache_name(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{hex}.png")
}

/// The cached icon of `url`, when there is a fresh one: a plain file (never
/// a link), at most [`MAX_ICON_BYTES`], younger than [`CACHE_MAX_AGE`] at
/// `now`, that decodes within the usual limits.
fn read_cached(dir: &Path, url: &str, now: SystemTime) -> Option<Icon> {
    let path = dir.join(cache_name(url));
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_ICON_BYTES as u64 {
        return None;
    }
    // Written after `now` (the clock moved back, or it was just stored):
    // as fresh as it gets.
    let age = now
        .duration_since(meta.modified().ok()?)
        .unwrap_or(Duration::ZERO);
    if age > CACHE_MAX_AGE {
        return None;
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    std::fs::File::open(&path)
        .ok()?
        .take(MAX_ICON_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    decode(&bytes).ok()
}

/// Store `icon` as `url`'s cache file: a fresh temporary file, then a
/// rename over the old one, never written through a link (the fallback
/// folder is under the shared temp dir, where anyone can plant one).
fn write_cached(dir: &Path, url: &str, icon: &Icon) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    if std::fs::symlink_metadata(dir)?.file_type().is_symlink() {
        return Err(std::io::Error::other("the icon cache folder is a link"));
    }
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&icon.rgba, icon.side, icon.side, ExtendedColorType::Rgba8)
        .map_err(std::io::Error::other)?;
    let name = cache_name(url);
    let path = dir.join(&name);
    let tmp = dir.join(format!("{name}.tmp"));
    if std::fs::symlink_metadata(&tmp).is_ok() {
        std::fs::remove_file(&tmp)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    file.write_all(&png)?;
    drop(file);
    std::fs::rename(&tmp, &path)
}

/// The projects whose pictures stand for installed addons: every
/// CurseForge id and every WoWInterface id among their sources, once each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryIds {
    /// For `POST /v1/mods` (needs the CurseForge key).
    pub curseforge: Vec<u64>,
    /// For the WoWInterface catalog, when it is loaded.
    pub wowinterface: Vec<u64>,
}

impl LibraryIds {
    /// Collect the ids of `addons`. A companion's own ids only: a family
    /// head's logo on its modules would say they are the same project.
    pub fn of(addons: &[InstalledAddon]) -> Self {
        let mut curseforge = BTreeSet::new();
        let mut wowinterface = BTreeSet::new();
        for a in addons {
            curseforge.extend(curseforge_id(a));
            wowinterface.extend(wowinterface_id(a));
        }
        Self {
            curseforge: curseforge.into_iter().collect(),
            wowinterface: wowinterface.into_iter().collect(),
        }
    }
}

fn curseforge_id(a: &InstalledAddon) -> Option<u64> {
    a.sources.iter().find_map(|s| match s {
        SourceRef::CurseForge { id } => Some(*id),
        _ => None,
    })
}

fn wowinterface_id(a: &InstalledAddon) -> Option<u64> {
    a.sources.iter().find_map(|s| match s {
        SourceRef::WowInterface { id } => Some(*id),
        _ => None,
    })
}

/// The icon URL of each installed addon, by folder: its CurseForge logo,
/// else its WoWInterface thumbnail. Addons with neither are left out.
pub fn library_urls(
    addons: &[InstalledAddon],
    logos: &HashMap<u64, String>,
    thumbs: &HashMap<u64, String>,
) -> HashMap<String, String> {
    addons
        .iter()
        .filter_map(|a| {
            let logo = curseforge_id(a).and_then(|id| logos.get(&id));
            let url = logo.or_else(|| wowinterface_id(a).and_then(|id| thumbs.get(&id)))?;
            Some((a.folder.clone(), url.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(width, height)
            .write_to(&mut out, format)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn only_https_on_the_icon_hosts() {
        assert!(is_allowed(
            "https://media.forgecdn.net/avatars/thumbnails/9/9/256/256/1.png"
        ));
        assert!(is_allowed("https://CDN-WOW.mmoui.com/preview/tiny/pvw20498.png"));
        assert!(!is_allowed("http://cdn-wow.mmoui.com/preview/tiny/pvw20498.png"));
        assert!(!is_allowed("https://evil.example/pvw20498.png"));
        // Reads as the CDN, goes to another host.
        assert!(!is_allowed("https://media.forgecdn.net@evil.example/x.png"));
        assert!(!is_allowed("https://user@media.forgecdn.net/x.png"));
        assert!(!is_allowed("https://media.forgecdn.net:8443/x.png"));
        assert!(!is_allowed("file:///C:/Windows/x.png"));
        assert!(!is_allowed("not a url"));
    }

    #[test]
    fn png_and_jpeg_become_a_square() {
        // A WoWInterface-shaped thumbnail: 100px wide, not square.
        for (w, h, format) in [(100, 56, ImageFormat::Png), (100, 18, ImageFormat::Jpeg)] {
            let icon = decode(&encoded(w, h, format)).unwrap();
            assert_eq!(icon.side, SIDE);
            assert_eq!(icon.rgba.len(), (SIDE * SIDE * 4) as usize);
        }
    }

    #[test]
    fn the_format_comes_from_the_bytes() {
        assert!(matches!(
            decode(b"GIF89a\x01\x00\x01\x00"),
            Err(IconError::Format)
        ));
        assert!(matches!(
            decode(b"<html>not an image</html>"),
            Err(IconError::Format)
        ));
        assert!(matches!(decode(&[]), Err(IconError::Format)));
        // Starts like a PNG, then nothing: corrupt, not a crash.
        let mut cut = encoded(100, 56, ImageFormat::Png);
        cut.truncate(40);
        assert!(matches!(decode(&cut), Err(IconError::Decode(_))));
    }

    #[test]
    fn oversized_pictures_are_refused() {
        // Tiny file, wide picture: refused on its header, before the pixels.
        let wide = encoded(MAX_SIDE + 1, 1, ImageFormat::Png);
        assert!(wide.len() < MAX_ICON_BYTES);
        assert!(matches!(decode(&wide), Err(IconError::Decode(_))));
        assert!(decode(&encoded(MAX_SIDE, 1, ImageFormat::Png)).is_ok());
    }

    const LOGO: &str = "https://media.forgecdn.net/avatars/thumbnails/1/2/256/256/3.png";

    #[test]
    fn the_cache_gives_back_what_it_stored_while_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("icons");
        let icon = decode(&encoded(100, 56, ImageFormat::Jpeg)).unwrap();
        write_cached(&dir, LOGO, &icon).unwrap();
        let name = cache_name(LOGO);
        assert!(name.len() == 64 + 4 && name.ends_with(".png"));
        assert!(!dir.join(format!("{name}.tmp")).exists());

        let now = SystemTime::now();
        assert_eq!(read_cached(&dir, LOGO, now), Some(icon.clone()));
        // A week and a day later it is fetched again.
        let later = now + CACHE_MAX_AGE + Duration::from_secs(24 * 60 * 60);
        assert_eq!(read_cached(&dir, LOGO, later), None);
        // Another URL, another file.
        assert_eq!(read_cached(&dir, "https://cdn-wow.mmoui.com/x.png", now), None);
        // Writing again replaces it.
        write_cached(&dir, LOGO, &icon).unwrap();
        assert_eq!(read_cached(&dir, LOGO, now), Some(icon));
    }

    #[test]
    fn a_bad_cache_file_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(cache_name(LOGO));
        let now = SystemTime::now();
        std::fs::write(&path, b"not a picture").unwrap();
        assert_eq!(read_cached(tmp.path(), LOGO, now), None);
        std::fs::write(&path, vec![0u8; MAX_ICON_BYTES + 1]).unwrap();
        assert_eq!(read_cached(tmp.path(), LOGO, now), None);

        // A link in the file's place is not followed.
        std::fs::remove_file(&path).unwrap();
        let elsewhere = tmp.path().join("elsewhere.png");
        std::fs::write(&elsewhere, encoded(64, 64, ImageFormat::Png)).unwrap();
        if crate::discover::try_symlink_file(&elsewhere, &path) {
            assert_eq!(read_cached(tmp.path(), LOGO, now), None);
        } else {
            eprintln!("cannot create a file link here; skipped");
        }
    }

    fn addon(folder: &str, sources: Vec<SourceRef>) -> InstalledAddon {
        InstalledAddon {
            folder: folder.into(),
            path: PathBuf::new(),
            toc_path: PathBuf::new(),
            installed: None,
            sources,
            display_name: folder.into(),
            issues: Vec::new(),
            disabled: false,
            toc: crate::Toc::parse(""),
        }
    }

    #[test]
    fn library_icons_prefer_the_curseforge_logo() {
        let gh = SourceRef::Github {
            owner: "Nevcairiel".into(),
            repo: "GatherMate2".into(),
            tag_prefix: None,
        };
        let addons = vec![
            // GitHub answers its updates; its CurseForge id still has a logo.
            addon(
                "DBM-Core",
                vec![
                    gh.clone(),
                    SourceRef::WowInterface { id: 8814 },
                    SourceRef::CurseForge { id: 3358 },
                ],
            ),
            // No logo on CurseForge: the WoWInterface thumbnail.
            addon(
                "GatherMate2",
                vec![
                    SourceRef::WowInterface { id: 21293 },
                    SourceRef::CurseForge { id: 9 },
                ],
            ),
            // Same project as another addon: asked once.
            addon("DBM-Raids", vec![SourceRef::CurseForge { id: 3358 }]),
            // Nothing to show.
            addon(
                "Zygor",
                vec![SourceRef::Manual {
                    url: "https://zygorguides.com/".into(),
                }],
            ),
            addon("Plain", vec![gh]),
        ];
        let ids = LibraryIds::of(&addons);
        assert_eq!(ids.curseforge, [9, 3358]);
        assert_eq!(ids.wowinterface, [8814, 21293]);

        let logos = HashMap::from([(3358, LOGO.to_string())]);
        let thumbs = HashMap::from([
            (8814, "https://cdn-wow.mmoui.com/a.png".to_string()),
            (21293, "https://cdn-wow.mmoui.com/b.png".to_string()),
        ]);
        let urls = library_urls(&addons, &logos, &thumbs);
        assert_eq!(urls.len(), 3);
        assert_eq!(urls["DBM-Core"], LOGO);
        assert_eq!(urls["DBM-Raids"], LOGO);
        assert_eq!(urls["GatherMate2"], "https://cdn-wow.mmoui.com/b.png");
    }
}
