//! Pictures of an addon page: its screenshots and the images of its
//! description. The same rules as [`crate::icons`], sized for pictures
//! instead of list icons.
//!
//! Addresses come from API answers and descriptions, the bytes from a CDN:
//! both are untrusted. A picture is fetched only from `https://` on one of
//! [`ALLOWED_HOSTS`] (every redirect hop included), with no credentials and
//! the default port; the body is capped at [`MAX_BYTES`]; the format is read
//! off the first bytes (PNG, JPEG or GIF, of which only the first frame);
//! the decoder refuses a picture over [`MAX_SIDE`] pixels a side or
//! [`MAX_DECODE_ALLOC`] bytes of memory; and what reaches the GUI is RGBA
//! scaled down to fit the box it is drawn in. Nothing is written to disk.

use std::io::Cursor;

use image::imageops::FilterType;
use image::{ImageFormat, ImageReader, Limits};

use crate::http::{Http, HttpError};

/// Where addon pages keep their pictures: CurseForge's CDN (screenshots and
/// the images of its descriptions), WoWInterface's, Tukui's storage, and
/// GitHub's hosts for READMEs: `camo` is GitHub's proxy for every outside
/// picture of a README, `github.com` the address of a picture in the
/// repository (redirected to `raw.githubusercontent.com`), the two
/// `*-images` hosts those of pictures pasted into it. `i.imgur.com` is the
/// one host that is not a provider's: CurseForge descriptions link their
/// pictures from it more than from anywhere else. Anything else is shown as
/// a link, never fetched.
pub const ALLOWED_HOSTS: &[&str] = &[
    "media.forgecdn.net",
    "cdn-wow.mmoui.com",
    "storage.tukui.org",
    "camo.githubusercontent.com",
    "github.com",
    "raw.githubusercontent.com",
    "user-images.githubusercontent.com",
    "private-user-images.githubusercontent.com",
    "i.imgur.com",
];

/// Largest picture file read. A CurseForge screenshot is 1–3 MB as a
/// 1920×1080 PNG; an animated GIF in a README can be larger, and is then
/// left as a link.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

/// Largest width or height decoded: a 4K screenshot.
pub const MAX_SIDE: u32 = 4096;

/// Most memory the decoder may take: a 3840×2160 screenshot is 32 MiB as
/// RGBA.
pub const MAX_DECODE_ALLOC: u64 = 48 * 1024 * 1024;

/// A decoded picture, RGBA, row by row.
#[derive(Clone)]
pub struct Picture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// Only the size: the pixels would flood any log that prints a message.
impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Picture({}x{})", self.width, self.height)
    }
}

/// Why a picture is not shown. The page shows a link to it instead.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// Not an `https://` URL on one of [`ALLOWED_HOSTS`].
    #[error("picture on a site Tocsin does not fetch from")]
    Untrusted,
    /// Transport, status or size problem.
    #[error(transparent)]
    Http(#[from] HttpError),
    /// Not PNG, JPEG or GIF.
    #[error("picture in a format Tocsin does not show")]
    Format,
    /// Corrupt, or over [`MAX_SIDE`] / [`MAX_DECODE_ALLOC`].
    #[error("picture could not be decoded: {0}")]
    Decode(String),
    /// The background decode did not finish.
    #[error("decoding the picture failed: {0}")]
    Task(String),
}

/// Whether pictures may come from `host`. Also applied to every redirect
/// hop (see [`crate::http::redirect_verdict`]).
pub fn is_allowed_host(host: &str) -> bool {
    ALLOWED_HOSTS.iter().any(|a| host.eq_ignore_ascii_case(a))
}

/// Whether `url` may be fetched as a page picture.
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

/// The host of `url`, for "Picture on example.com".
pub fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url).ok()?.host_str().map(str::to_string)
}

/// The format the bytes start with, among the three decoded.
fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(ImageFormat::Gif)
    } else {
        None
    }
}

/// Decode `bytes` within the limits above and scale the picture down to fit
/// `max_width` × `max_height` (never up). Blocking: call it off the async
/// workers.
pub fn decode(bytes: &[u8], max_width: u32, max_height: u32) -> Result<Picture, MediaError> {
    let format = sniff(bytes).ok_or(MediaError::Format)?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let picture = reader.decode().map_err(|e| MediaError::Decode(e.to_string()))?;
    let (w, h) = (picture.width(), picture.height());
    let (max_width, max_height) = (max_width.max(1), max_height.max(1));
    let picture = if w > max_width || h > max_height {
        picture.resize(max_width, max_height, FilterType::Triangle)
    } else {
        picture
    };
    let rgba = picture.into_rgba8();
    Ok(Picture {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// The height of a `width` × `height` picture once [`decode`] has fitted it
/// to `max_width` × `max_height` and the page has scaled it down (never up)
/// to a column `column` wide: what the page keeps for a picture whose
/// description declares its size, so that nothing below it moves when it
/// arrives. At least 1.
pub fn drawn_height(width: u32, height: u32, max_width: u32, max_height: u32, column: u32) -> u32 {
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let scale = [
        1.0,
        f64::from(max_width.max(1)) / w,
        f64::from(max_height.max(1)) / h,
        f64::from(column.max(1)) / w,
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min);
    // At most `height`, which is a u32: the cast cannot overflow.
    (h * scale).round().max(1.0) as u32
}

/// Fetch and decode the picture at `url`, fitted to `max_width` ×
/// `max_height`.
pub async fn fetch(http: &Http, url: &str, max_width: u32, max_height: u32) -> Result<Picture, MediaError> {
    if !is_allowed(url) {
        return Err(MediaError::Untrusted);
    }
    let bytes = http.get_media(url, MAX_BYTES).await?;
    tokio::task::spawn_blocking(move || decode(&bytes, max_width, max_height))
        .await
        .map_err(|e| MediaError::Task(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageEncoder, RgbaImage};

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, image::Rgba([200, 160, 60, 255]));
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    #[test]
    fn only_the_providers_hosts_and_imgur() {
        assert!(is_allowed("https://media.forgecdn.net/attachments/1/2/shot.png"));
        assert!(is_allowed("https://camo.githubusercontent.com/e488/6874"));
        assert!(is_allowed("https://i.imgur.com/wwbxeCG.jpeg"));
        assert!(!is_allowed("http://i.imgur.com/wwbxeCG.jpeg"));
        assert!(!is_allowed("https://imgur.com/gallery/abc"));
        assert!(!is_allowed("https://media.forgecdn.net:8443/a.png"));
        assert!(!is_allowed("https://user:pw@media.forgecdn.net/a.png"));
        assert!(!is_allowed("https://media.forgecdn.net.evil.example/a.png"));
        assert_eq!(
            host_of("https://i.imgur.com/a.png").as_deref(),
            Some("i.imgur.com")
        );
    }

    #[test]
    fn pictures_are_scaled_down_never_up() {
        let big = decode(&png(1920, 1080), 960, 1600).unwrap();
        assert_eq!((big.width, big.height), (960, 540));
        assert_eq!(big.rgba.len(), 960 * 540 * 4);
        let small = decode(&png(40, 20), 960, 1600).unwrap();
        assert_eq!((small.width, small.height), (40, 20));
        let tall = decode(&png(100, 4000), 960, 1600).unwrap();
        assert_eq!(tall.height, 1600);
    }

    #[test]
    fn the_height_kept_for_a_picture_follows_its_drawn_size() {
        // A CurseForge banner narrower than the 840 column: as declared.
        assert_eq!(drawn_height(600, 300, 1280, 1600, 840), 300);
        // A 1920×1080 screenshot: fitted to 1280 wide, then to the column.
        assert_eq!(drawn_height(1920, 1080, 1280, 1600, 840), 473);
        // A tall strip: the box's height caps it first.
        assert_eq!(drawn_height(400, 4000, 1280, 1600, 840), 1600);
        // A 1×1 tracking pixel, or nonsense: never 0.
        assert_eq!(drawn_height(1, 1, 1280, 1600, 840), 1);
        assert_eq!(drawn_height(4096, 1, 1280, 1600, 840), 1);
        assert_eq!(drawn_height(0, 0, 0, 0, 0), 1);
        // The same fit as decode's.
        let shot = decode(&png(1920, 1080), 1280, 1600).unwrap();
        let column = 840.0 / f64::from(shot.width);
        assert_eq!(
            drawn_height(1920, 1080, 1280, 1600, 840),
            (f64::from(shot.height) * column).round() as u32
        );
    }

    #[test]
    fn gifs_give_their_first_frame_and_svgs_are_refused() {
        let mut gif = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut gif);
            let frame = image::Frame::new(RgbaImage::from_pixel(8, 4, image::Rgba([1, 2, 3, 255])));
            enc.encode_frame(frame).unwrap();
        }
        let first = decode(&gif, 960, 1600).unwrap();
        assert_eq!((first.width, first.height), (8, 4));
        assert!(matches!(
            decode(b"<svg xmlns='http://www.w3.org/2000/svg'/>", 960, 1600),
            Err(MediaError::Format)
        ));
    }

    #[test]
    fn oversized_pictures_are_refused_before_decoding() {
        // A PNG header claiming 20000×20000: the limits stop it, however
        // small the file.
        let mut bytes = png(1, 1);
        bytes[16..20].copy_from_slice(&20_000u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&20_000u32.to_be_bytes());
        assert!(matches!(decode(&bytes, 960, 1600), Err(MediaError::Decode(_))));
    }
}
