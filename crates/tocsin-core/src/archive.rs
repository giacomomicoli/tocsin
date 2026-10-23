//! Inspect and extract addon zips.
//!
//! A WoW addon zip has only folders at its root, each with its own `.toc`.
//! Everything else — a wrapper folder from a GitHub "Source code" zip, loose
//! files, symlinks, `..`, absolute paths, Windows device names, absurd sizes —
//! is refused before a byte is written. Extraction goes to a staging directory
//! the caller owns; this module never touches `AddOns` itself.
//!
//! Sizes in the zip's directory are written by whoever made the zip, so they
//! only serve as an early refusal in [`inspect`]; [`extract`] counts the
//! bytes it really writes and stops at the same caps.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use zip::ZipArchive;

use crate::toc::Toc;

/// Caps against zip bombs. ElvUI is ~3 000 files / 60 MiB unpacked.
const MAX_ENTRIES: usize = 20_000;
const MAX_UNPACKED: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILE: u64 = 256 * 1024 * 1024;
/// A `.toc` is a few KiB; anything larger is not one we want to parse.
const MAX_TOC: u64 = 1024 * 1024;

/// Errors while inspecting or extracting.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    /// Not a zip we can read.
    #[error("not a valid zip file")]
    Invalid,
    /// An entry would escape the target or is otherwise unacceptable.
    #[error("unsafe entry in zip: {0}")]
    UnsafeEntry(String),
    /// Too many entries or too much data.
    #[error("zip is too large to be an addon")]
    TooLarge,
    /// A file at the root, or a folder without a `.toc`.
    #[error("not an addon package: {0}")]
    NotAnAddon(String),
    /// The wrapper-folder layout of a source zip.
    #[error("this is a source zip ({0}/…), not a release: download the release zip instead")]
    SourceZip(String),
    /// Disk failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// What a zip contains at its root.
#[derive(Debug, Clone)]
pub struct Manifest {
    /// Top-level folders, each with its parsed `.toc` (Retail game types).
    pub folders: BTreeMap<String, Toc>,
    /// Total unpacked size.
    pub unpacked: u64,
    /// Number of entries.
    pub entries: usize,
}

/// Read the zip's directory and validate it. No data is decompressed.
pub fn inspect(zip_path: &Path) -> Result<Manifest, ArchiveError> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = ZipArchive::new(file).map_err(|_| ArchiveError::Invalid)?;
    if archive.len() > MAX_ENTRIES {
        return Err(ArchiveError::TooLarge);
    }

    let mut unpacked: u64 = 0;
    let mut tops: BTreeMap<String, bool> = BTreeMap::new(); // name → has <Name>*.toc
    let mut toc_bodies: BTreeMap<String, String> = BTreeMap::new();

    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i).map_err(|_| ArchiveError::Invalid)?;
        let name = entry_name(&entry)?;
        let rel = safe_relative(&name)?;
        if entry.is_symlink() {
            return Err(ArchiveError::UnsafeEntry(name));
        }
        unpacked = unpacked.saturating_add(entry.size());
        if entry.size() > MAX_FILE || unpacked > MAX_UNPACKED {
            return Err(ArchiveError::TooLarge);
        }
        let mut comps = rel.components();
        let Some(Component::Normal(top)) = comps.next() else {
            continue; // empty name
        };
        let top = top.to_string_lossy().into_owned();
        let is_root_file = comps.clone().next().is_none() && !entry.is_dir();
        if is_root_file {
            // `.pkgmeta`, README at root: tolerate dotfiles/docs, refuse the rest.
            if top.starts_with('.') || top.to_ascii_lowercase().ends_with(".md") {
                continue;
            }
            return Err(ArchiveError::NotAnAddon(format!("loose file {top} at the root")));
        }
        if top.starts_with('.') {
            // The game ignores dot-folders, and Tocsin's own live in AddOns:
            // a zip shipping `.tocsin-backup/installs.toml` must never be able
            // to replace the install log. Such folders are skipped, here and
            // in `extract`.
            continue;
        }
        let has_toc = tops.entry(top.clone()).or_insert(false);
        // `<Top>/<Top>.toc` or `<Top>/<Top>_Mainline.toc`, one level deep.
        let depth = rel.components().count();
        if depth == 2 && !entry.is_dir() {
            let fname = rel.file_name().and_then(|f| f.to_str()).unwrap_or("");
            if is_toc_for(fname, &top) {
                *has_toc = true;
                if entry.size() <= MAX_TOC && !toc_bodies.contains_key(&top) {
                    // Read this one small file now; we need its metadata.
                    drop(entry);
                    let f = archive.by_index(i).map_err(|_| ArchiveError::Invalid)?;
                    let mut raw = Vec::new();
                    // The declared size is the zip author's word: a `.toc`
                    // listed as 1 KiB may inflate to gigabytes.
                    copy_capped(f, &mut raw, MAX_TOC)?;
                    toc_bodies.insert(top.clone(), String::from_utf8_lossy(&raw).into_owned());
                }
            }
        }
    }

    if tops.is_empty() {
        return Err(ArchiveError::NotAnAddon("empty zip".into()));
    }
    // A single folder with no .toc of its own but addon folders inside:
    // GitHub's "Source code" layout.
    if tops.len() == 1 {
        let (only, has_toc) = tops
            .iter()
            .next()
            .map(|(k, v)| (k.clone(), *v))
            .unwrap_or_default();
        if !has_toc && looks_like_wrapper(&mut archive, &only) {
            return Err(ArchiveError::SourceZip(only));
        }
    }
    let mut folders = BTreeMap::new();
    for (top, has_toc) in tops {
        if !has_toc {
            return Err(ArchiveError::NotAnAddon(format!("folder {top} has no .toc")));
        }
        let toc = toc_bodies.get(&top).map(|b| Toc::parse(b)).unwrap_or_default();
        folders.insert(top, toc);
    }
    Ok(Manifest {
        folders,
        unpacked,
        entries: archive.len(),
    })
}

/// Extract every entry into `staging` (created if missing). Entry paths are
/// re-validated; the result is `staging/<Top>/…` for each top-level folder.
///
/// Each file may hold no more than its declared size (and [`MAX_FILE`]), and
/// all of them together no more than [`MAX_UNPACKED`]: the bytes written are
/// counted, so a zip lying about its sizes stops at the caps, partial output
/// included. On error the caller removes `staging`.
pub fn extract(zip_path: &Path, staging: &Path) -> Result<(), ArchiveError> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = ZipArchive::new(file).map_err(|_| ArchiveError::Invalid)?;
    if archive.len() > MAX_ENTRIES {
        return Err(ArchiveError::TooLarge);
    }
    std::fs::create_dir_all(staging)?;
    let mut written: u64 = 0;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|_| ArchiveError::Invalid)?;
        let name = entry_name(&entry)?;
        let rel = safe_relative(&name)?;
        if rel.as_os_str().is_empty() || under_hidden_top(&rel) {
            continue;
        }
        if entry.is_symlink() {
            return Err(ArchiveError::UnsafeEntry(name));
        }
        let target = staging.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let limit = entry.size().min(MAX_FILE).min(MAX_UNPACKED - written);
        if entry.size() > limit {
            return Err(ArchiveError::TooLarge);
        }
        let mut out = std::fs::File::create(&target)?;
        written += copy_capped(entry, &mut out, limit)?;
    }
    Ok(())
}

/// Copy at most `limit` bytes from `from` to `to`, and fail with
/// [`ArchiveError::TooLarge`] if `from` has more. Never writes past `limit`.
///
/// The zip crate also refuses an entry that inflates past its declared size
/// or fails its CRC; it reports that as `InvalidData`, which is the zip's
/// fault, not the disk's, so it becomes [`ArchiveError::Invalid`].
fn copy_capped(mut from: impl Read, to: &mut impl Write, limit: u64) -> Result<u64, ArchiveError> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let room = usize::try_from(limit - total)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        // At the limit, ask for one more byte: it tells "exactly full" from "too much".
        let n = match from.read(&mut buf[..room.max(1)]) {
            Ok(0) => return Ok(total),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => return Err(ArchiveError::Invalid),
            Err(e) => return Err(e.into()),
        };
        if room == 0 {
            return Err(ArchiveError::TooLarge);
        }
        to.write_all(&buf[..n])?;
        total += n as u64;
    }
}

/// Under a top-level dot-name (`.pkgmeta`, `.github/…`), which nothing needs
/// and [`inspect`] skips.
fn under_hidden_top(rel: &Path) -> bool {
    matches!(rel.components().next(), Some(Component::Normal(t)) if t.to_string_lossy().starts_with('.'))
}

/// The entry's name as stored; the zip crate refuses invalid encodings.
fn entry_name<R: Read>(entry: &zip::read::ZipFile<'_, R>) -> Result<String, ArchiveError> {
    entry
        .name()
        .map(|n| n.into_owned())
        .map_err(|_| ArchiveError::Invalid)
}

/// Turn a zip entry name into a relative, normal-components-only path.
fn safe_relative(name: &str) -> Result<PathBuf, ArchiveError> {
    // Zip names use '/', but Windows-made archives sometimes carry '\'.
    let normalized = name.replace('\\', "/");
    if normalized.contains('\0') {
        return Err(ArchiveError::UnsafeEntry(name.to_string()));
    }
    let mut out = PathBuf::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => continue,
            p if !is_portable_component(p) => return Err(ArchiveError::UnsafeEntry(name.to_string())),
            p => out.push(p),
        }
    }
    // Reject anything Path would not treat as a plain relative name.
    if out.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(ArchiveError::UnsafeEntry(name.to_string()));
    }
    Ok(out)
}

/// Whether one path component is a name Windows stores as written. The zips
/// are for a Windows game, so these rules apply on every platform: `aux.lua`
/// opens the AUX device instead of a file, `Core.` silently becomes `Core`
/// (two entries can then collide, or a check made on one name applies to
/// another), and `:` opens an alternate data stream.
pub(crate) fn is_portable_component(part: &str) -> bool {
    const FORBIDDEN: &[char] = &['<', '>', ':', '"', '|', '?', '*', '/', '\\'];
    if part == ".." || part.ends_with(['.', ' ']) {
        return false;
    }
    if part.chars().any(|c| c.is_control() || FORBIDDEN.contains(&c)) {
        return false;
    }
    !is_reserved_device(part)
}

/// `CON`, `NUL`, `COM1`, `LPT¹`… with any extension (`aux.lua`, `nul.tar.gz`),
/// in any case. Windows ignores what follows the first dot and trailing
/// spaces before it when it resolves these names.
fn is_reserved_device(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).trim_end_matches(' ');
    let stem = stem.to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|n| {
            let mut chars = n.chars();
            matches!(
                (chars.next(), chars.next()),
                (Some('1'..='9' | '¹' | '²' | '³'), None)
            )
        })
    };
    numbered("COM") || numbered("LPT")
}

/// `WeakAuras.toc`, `WeakAuras_Mainline.toc`, `weakauras-cata.toc` for folder `WeakAuras`.
fn is_toc_for(file_name: &str, folder: &str) -> bool {
    let f = file_name.to_ascii_lowercase();
    let Some(stem) = f.strip_suffix(".toc") else {
        return false;
    };
    let folder = folder.to_ascii_lowercase();
    stem == folder
        || stem
            .strip_prefix(&folder)
            .is_some_and(|r| r.starts_with(['_', '-']))
}

/// Does `wrapper/` contain `wrapper/<X>/<X>*.toc` or `wrapper/<X>.toc`? Then
/// it is a repository checkout, not a package.
fn looks_like_wrapper<R: Read + std::io::Seek>(archive: &mut ZipArchive<R>, wrapper: &str) -> bool {
    let prefix = format!("{wrapper}/");
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index_raw(i) else {
            continue;
        };
        let Ok(name) = entry_name(&entry) else {
            continue;
        };
        let name = name.replace('\\', "/");
        let Some(rest) = name.strip_prefix(&prefix) else {
            continue;
        };
        let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
        match parts.as_slice() {
            [f] if f.to_ascii_lowercase().ends_with(".toc") => return true,
            [d, f] if is_toc_for(f, d) => return true,
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn make_zip(entries: &[(&str, &str)]) -> tempfile::NamedTempFile {
        let f = tempfile::NamedTempFile::new().unwrap();
        let mut w = zip::ZipWriter::new(f.reopen().unwrap());
        for (name, body) in entries {
            if name.ends_with('/') {
                w.add_directory(*name, SimpleFileOptions::default()).unwrap();
            } else {
                w.start_file(*name, SimpleFileOptions::default()).unwrap();
                w.write_all(body.as_bytes()).unwrap();
            }
        }
        w.finish().unwrap();
        f
    }

    #[test]
    fn accepts_a_multi_folder_release() {
        let z = make_zip(&[
            ("WeakAuras/", ""),
            (
                "WeakAuras/WeakAuras_Mainline.toc",
                "## Title: WeakAuras\n## Version: 5.22.1\n",
            ),
            ("WeakAuras/WeakAuras.lua", "x"),
            ("WeakAurasOptions/WeakAurasOptions.toc", "## Title: Options\n"),
            ("WeakAurasOptions/Libs/AceGUI/AceGUI.lua", "y"),
        ]);
        let m = inspect(z.path()).unwrap();
        assert_eq!(
            m.folders.keys().cloned().collect::<Vec<_>>(),
            vec!["WeakAuras", "WeakAurasOptions"]
        );
        assert_eq!(m.folders["WeakAuras"].version(), Some("5.22.1"));

        let staging = tempfile::tempdir().unwrap();
        extract(z.path(), staging.path()).unwrap();
        assert!(staging
            .path()
            .join("WeakAurasOptions/Libs/AceGUI/AceGUI.lua")
            .is_file());
    }

    #[test]
    fn rejects_source_zip_wrapper() {
        let z = make_zip(&[
            ("Details-Damage-Meter-main/Details.toc", "## Title: Details\n"),
            ("Details-Damage-Meter-main/boot.lua", ""),
            ("Details-Damage-Meter-main/.pkgmeta", ""),
        ]);
        assert!(
            matches!(inspect(z.path()), Err(ArchiveError::SourceZip(w)) if w == "Details-Damage-Meter-main")
        );
    }

    #[test]
    fn rejects_traversal_and_loose_files() {
        let z = make_zip(&[("../evil.lua", "")]);
        assert!(matches!(inspect(z.path()), Err(ArchiveError::UnsafeEntry(_))));
        let z = make_zip(&[("Addon/Addon.toc", ""), ("Addon/../../x.lua", "")]);
        assert!(matches!(inspect(z.path()), Err(ArchiveError::UnsafeEntry(_))));
        let z = make_zip(&[("Addon/Addon.toc", ""), ("install.exe", "")]);
        assert!(matches!(inspect(z.path()), Err(ArchiveError::NotAnAddon(_))));
        let z = make_zip(&[("Addon/Addon.toc", ""), ("C:/Windows/x", "")]);
        assert!(matches!(inspect(z.path()), Err(ArchiveError::UnsafeEntry(_))));
        let z = make_zip(&[("Lonely/readme.txt", "")]);
        assert!(matches!(inspect(z.path()), Err(ArchiveError::NotAnAddon(_))));
    }

    #[test]
    fn tolerates_root_docs_and_backslashes() {
        let z = make_zip(&[
            ("README.md", "hi"),
            ("Addon\\Addon.toc", "## Title: A\n"),
            ("Addon\\core.lua", ""),
        ]);
        let m = inspect(z.path()).unwrap();
        assert_eq!(m.folders.len(), 1);
        let staging = tempfile::tempdir().unwrap();
        extract(z.path(), staging.path()).unwrap();
        assert!(staging.path().join("Addon/core.lua").is_file());
    }

    /// A stored (uncompressed) zip whose directory claims `name` is
    /// `declared` bytes long, whatever it really holds: the zip-bomb trick of
    /// lying in the header that `inspect` reads.
    fn lying_zip(entries: &[(&str, &[u8])], name: &str, declared: u32) -> tempfile::NamedTempFile {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (n, body) in entries {
            w.start_file(*n, stored).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap();
        let mut bytes = buf.into_inner();
        let u16_at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]) as usize;
        // (signature, offset of the uncompressed size, of the name length, of the name)
        for (sig, size_at, len_at, name_at) in [(b"PK\x03\x04", 22, 26, 30), (b"PK\x01\x02", 24, 28, 46)] {
            let mut i = 0;
            while i + name_at < bytes.len() {
                if &bytes[i..i + 4] == sig {
                    let len = u16_at(&bytes, i + len_at);
                    if bytes.get(i + name_at..i + name_at + len) == Some(name.as_bytes()) {
                        bytes[i + size_at..i + size_at + 4].copy_from_slice(&declared.to_le_bytes());
                    }
                }
                i += 1;
            }
        }
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(), bytes).unwrap();
        f
    }

    #[test]
    fn extract_stops_at_the_declared_size() {
        let big = vec![b'x'; 64 * 1024];
        let z = lying_zip(
            &[("Addon/Addon.toc", b"## Title: A\n"), ("Addon/big.lua", &big)],
            "Addon/big.lua",
            10,
        );
        // The directory looks harmless…
        let m = inspect(z.path()).unwrap();
        assert!(m.unpacked < 100);
        // …but the data is not what it says, and extraction refuses it
        // (our cap or the zip crate's own check, whichever trips first)
        // without writing past the declared size.
        let staging = tempfile::tempdir().unwrap();
        let r = extract(z.path(), staging.path());
        assert!(
            matches!(r, Err(ArchiveError::TooLarge | ArchiveError::Invalid)),
            "{r:?}"
        );
        let written = std::fs::metadata(staging.path().join("Addon/big.lua")).map_or(0, |m| m.len());
        assert!(written <= 10, "wrote {written} bytes");
    }

    #[test]
    fn inspect_reads_at_most_a_mebibyte_of_toc() {
        let mut toc = b"## Title: A\n".to_vec();
        toc.resize(MAX_TOC as usize + 10, b'#');
        let z = lying_zip(&[("Addon/Addon.toc", &toc)], "Addon/Addon.toc", 12);
        // Refused either by our cap or by the zip crate's own size check.
        let r = inspect(z.path());
        assert!(
            matches!(r, Err(ArchiveError::TooLarge | ArchiveError::Invalid)),
            "{r:?}"
        );
    }

    #[test]
    fn copy_capped_counts_real_bytes() {
        let mut out = Vec::new();
        assert_eq!(copy_capped(&b"hello"[..], &mut out, 5).unwrap(), 5);
        assert_eq!(out, b"hello");
        let mut out = Vec::new();
        assert!(matches!(
            copy_capped(&b"hello!"[..], &mut out, 5),
            Err(ArchiveError::TooLarge)
        ));
        assert_eq!(out.len(), 5, "never more than the limit");
        let mut out = Vec::new();
        assert_eq!(copy_capped(&b""[..], &mut out, 0).unwrap(), 0);
        assert!(copy_capped(&b"x"[..], &mut out, 0).is_err());
    }

    #[test]
    fn dot_folders_in_a_zip_are_skipped() {
        // A package must not be able to plant Tocsin's own folders (or a
        // forged install log) in AddOns.
        let z = make_zip(&[
            ("Addon/Addon.toc", "## Title: A\n"),
            (".tocsin-backup/.tocsin-backup.toc", ""),
            (".tocsin-backup/installs.toml", "[installs]\n"),
            (".github/workflows/x.yml", ""),
        ]);
        let m = inspect(z.path()).unwrap();
        assert_eq!(m.folders.keys().collect::<Vec<_>>(), ["Addon"]);
        let staging = tempfile::tempdir().unwrap();
        extract(z.path(), staging.path()).unwrap();
        assert!(staging.path().join("Addon/Addon.toc").is_file());
        assert!(!staging.path().join(".tocsin-backup").exists());
        assert!(!staging.path().join(".github").exists());
    }

    #[test]
    fn rejects_names_windows_would_mangle() {
        for bad in [
            "Addon/aux.lua",
            "Addon/AUX",
            "Addon/con.txt",
            "Addon/Nul.tar.gz",
            "Addon/com1.lua",
            "Addon/LPT9",
            "Addon/COM¹.lua",
            "Addon/AUX .lua",
            "CONIN$/x.lua",
            "Addon/Core./x.lua",
            "Addon/Core /x.lua",
            "Addon/a<b.lua",
            "Addon/a>b.lua",
            "Addon/a\"b.lua",
            "Addon/a|b.lua",
            "Addon/a?b.lua",
            "Addon/a*b.lua",
            "Addon/a\u{1}b.lua",
            "Addon/a\tb.lua",
            "Addon/file.lua:stream",
            "Addon/...",
        ] {
            assert!(safe_relative(bad).is_err(), "{bad:?} accepted");
        }
        for good in [
            "Addon/Libs/AceGUI-3.0/AceGUI-3.0.lua",
            "Addon/console.lua",
            "Addon/auxiliary.lua",
            "Addon/COM10.lua",
            "Addon/LPT0.lua",
            "Addon/.pkgmeta",
            "Addon/media/font (bold).ttf",
            "Addon/Locales/frFR.lua",
        ] {
            assert!(safe_relative(good).is_ok(), "{good:?} refused");
        }
        let z = make_zip(&[("Addon/Addon.toc", ""), ("Addon/nul.lua", "")]);
        assert!(matches!(inspect(z.path()), Err(ArchiveError::UnsafeEntry(_))));
    }

    #[test]
    fn toc_name_matching() {
        assert!(is_toc_for("WeakAuras.toc", "WeakAuras"));
        assert!(is_toc_for("WeakAuras_Mainline.toc", "WeakAuras"));
        assert!(is_toc_for("weakauras-cata.toc", "WeakAuras"));
        assert!(!is_toc_for("WeakAurasOptions.toc", "WeakAuras"));
        assert!(!is_toc_for("Other.toc", "WeakAuras"));
    }
}
