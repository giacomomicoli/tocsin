//! Heuristics that squeeze more information out of an addon folder than its
//! `.toc` admits to. Everything here is best-effort, read-only and bounded:
//! only a handful of small, well-known top-level files are ever opened.
//!
//! * A `github.com/owner/repo` link in `CHANGELOG`/`README` (Routine publishes
//!   on GitHub but its `.toc` only lists Wago and CurseForge).
//! * An SVN `$Revision: N $` keyword in `Ver.lua`/`Version.lua` (Zygor's
//!   `.toc` says `9.6`, the real build number lives there).

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::sources::{github, SourceRef};

/// Largest doc/version file we are willing to read.
const MAX_FILE_BYTES: u64 = 256 * 1024;

/// Files that commonly carry a project link, in order of trust.
const DOC_FILES: &[&str] = &["CHANGELOG", "CHANGES", "README", "HISTORY"];
const DOC_EXTS: &[&str] = &["md", "txt", "markdown", ""];

/// Files that commonly carry a build number.
const VERSION_FILES: &[&str] = &["Ver.lua", "Version.lua", "version.lua", "Revision.lua"];

/// What a directory entry is, taken from the listing itself, links not followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symlink, or on Windows a junction: never read through.
    Link,
    /// Anything else (sockets, devices, …).
    Other,
}

/// One top-level entry of an addon folder.
#[derive(Debug, Clone)]
pub struct ListedEntry {
    /// File name as on disk.
    pub name: String,
    /// `name`, ASCII-lowercased, for the case-insensitive lookups WoW addons need
    /// (`changelog.MD`, `CHANGELOG.md` and `Changelog.md` all occur in the wild).
    pub lower: String,
    /// Entry type, from the directory listing (no link is followed).
    pub kind: EntryKind,
}

/// The top level of one addon folder, listed once.
///
/// Scanning used to call `read_dir` once to pick the `.toc` and again for each
/// of the 20 well-known names the heuristics look for: ~6000 listings for 300
/// addons at startup, DBM folders with hundreds of files among them. Every
/// heuristic now works from this single listing.
#[derive(Debug, Clone)]
pub struct DirListing {
    dir: PathBuf,
    entries: Vec<ListedEntry>,
}

impl DirListing {
    /// List `dir` (top level only). Entries with non-UTF-8 names are skipped:
    /// none of the names we look for can be one.
    pub fn read(dir: &Path) -> std::io::Result<Self> {
        let entries = std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let kind = match e.file_type() {
                    Ok(t) if t.is_symlink() => EntryKind::Link,
                    Ok(t) if t.is_file() => EntryKind::File,
                    Ok(t) if t.is_dir() => EntryKind::Dir,
                    _ => EntryKind::Other,
                };
                let lower = name.to_ascii_lowercase();
                Some(ListedEntry { name, lower, kind })
            })
            .collect();
        Ok(Self {
            dir: dir.to_path_buf(),
            entries,
        })
    }

    /// The folder that was listed.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Entries in listing order.
    pub fn entries(&self) -> &[ListedEntry] {
        &self.entries
    }

    /// First entry whose name equals `name`, ignoring ASCII case.
    pub fn find_ci(&self, name: &str) -> Option<&ListedEntry> {
        let wanted = name.to_ascii_lowercase();
        self.entries.iter().find(|e| e.lower == wanted)
    }

    /// Read the regular file `name` (case-insensitive) if it is at most `max`
    /// bytes. Links, folders and oversized files give `None`.
    fn read_small_ci(&self, name: &str, max: u64) -> Option<String> {
        let entry = self.find_ci(name)?;
        if entry.kind != EntryKind::File {
            return None;
        }
        match read_regular_file(&self.dir.join(&entry.name), max).ok()? {
            Bounded::Read(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            Bounded::TooLarge(_) | Bounded::NotAFile => None,
        }
    }
}

/// Outcome of [`read_regular_file`].
#[derive(Debug)]
pub(crate) enum Bounded {
    /// The whole file, at most the cap.
    Read(Vec<u8>),
    /// Larger than the cap (size as seen).
    TooLarge(u64),
    /// A link, a folder or something else that is not a regular file.
    NotAFile,
}

/// Read a regular file of at most `max` bytes, refusing links.
///
/// The type comes from `symlink_metadata`, so a link is never followed to
/// wherever it points (a `.toc` symlinked to `~/.ssh/id_rsa` must not be
/// read). The read itself is capped too, so a file that grows between the
/// check and the read still cannot blow the cap. On Unix the opened file must
/// also be the very inode that was checked, which closes the swap-for-a-link
/// race; std offers no stable way to do the same on Windows.
pub(crate) fn read_regular_file(path: &Path, max: u64) -> std::io::Result<Bounded> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        return Ok(Bounded::NotAFile);
    }
    if meta.len() > max {
        return Ok(Bounded::TooLarge(meta.len()));
    }
    let file = std::fs::File::open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || !same_file(&meta, &opened) {
        return Ok(Bounded::NotAFile);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(opened.len().min(max)).unwrap_or(0));
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    let read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if read > max {
        return Ok(Bounded::TooLarge(read));
    }
    Ok(Bounded::Read(bytes))
}

#[cfg(unix)]
fn same_file(checked: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    checked.dev() == opened.dev() && checked.ino() == opened.ino()
}

#[cfg(not(unix))]
fn same_file(_: &std::fs::Metadata, _: &std::fs::Metadata) -> bool {
    true
}

/// Look for a GitHub repository link in the addon's top-level docs.
pub fn github_from_docs(dir: &Path) -> Option<SourceRef> {
    github_from_listing(&DirListing::read(dir).ok()?)
}

/// [`github_from_docs`] on a folder already listed.
pub fn github_from_listing(listing: &DirListing) -> Option<SourceRef> {
    for base in DOC_FILES {
        for ext in DOC_EXTS {
            let name = if ext.is_empty() {
                (*base).to_string()
            } else {
                format!("{base}.{ext}")
            };
            let Some(text) = listing.read_small_ci(&name, MAX_FILE_BYTES) else {
                continue;
            };
            if let Some(src) = first_github_link(&text) {
                return Some(src);
            }
        }
    }
    None
}

/// Extract the first `github.com/owner/repo` occurrence from free text.
pub fn first_github_link(text: &str) -> Option<SourceRef> {
    let mut rest = text;
    while let Some(at) = rest.find("github.com/") {
        let candidate = &rest[at..];
        // Up to the first whitespace or markdown delimiter.
        let end = candidate
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | ']' | '>' | '"' | '\'' | '<'))
            .unwrap_or(candidate.len());
        let url = format!("https://{}", &candidate[..end]);
        if let Some(src) = github::parse_repo_url(&url) {
            return Some(src);
        }
        rest = &rest[at + "github.com/".len()..];
    }
    None
}

/// Find an SVN-style `$Revision: 37349 $` in the addon's version file, if any.
pub fn svn_revision(dir: &Path) -> Option<u64> {
    svn_revision_in(&DirListing::read(dir).ok()?)
}

/// [`svn_revision`] on a folder already listed.
pub fn svn_revision_in(listing: &DirListing) -> Option<u64> {
    for name in VERSION_FILES {
        let Some(text) = listing.read_small_ci(name, MAX_FILE_BYTES) else {
            continue;
        };
        if let Some(rev) = parse_revision_keyword(&text) {
            return Some(rev);
        }
    }
    None
}

/// Parse `$Revision: N $` (the `N` may be absent in unexpanded keywords).
pub fn parse_revision_keyword(text: &str) -> Option<u64> {
    let at = text.find("$Revision:")?;
    let after = &text[at + "$Revision:".len()..];
    let digits: String = after
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// Create a file symlink for tests; `false` where the platform refuses
/// (Windows without Developer Mode or admin rights).
#[cfg(test)]
pub(crate) fn try_symlink_file(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    #[cfg(not(any(unix, windows)))]
    let made: std::io::Result<()> = Err(std::io::ErrorKind::Unsupported.into());
    made.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_github_link_in_markdown() {
        let md = "## [v12.0.351](https://github.com/LoyalFTW/Midnight-Routine/tree/v12.0.351) (2026-09-29)";
        assert_eq!(
            first_github_link(md),
            Some(SourceRef::Github {
                owner: "LoyalFTW".into(),
                repo: "Midnight-Routine".into(),
                tag_prefix: None
            })
        );
        assert_eq!(first_github_link("nothing here"), None);
        assert_eq!(first_github_link("see github.com/only-owner and more"), None);
    }

    #[test]
    fn parses_svn_keyword() {
        let lua = r#"ZGV.revision = tonumber(string.sub("$Revision: 37349 $", 12, -3))"#;
        assert_eq!(parse_revision_keyword(lua), Some(37349));
        assert_eq!(parse_revision_keyword("$Revision$"), None);
        assert_eq!(parse_revision_keyword("$Revision: $"), None);
    }

    #[test]
    fn reads_from_disk_case_insensitively() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("changelog.MD"), "https://github.com/a/b\n").unwrap();
        std::fs::write(tmp.path().join("Ver.lua"), "$Revision: 7 $").unwrap();
        assert!(matches!(
            github_from_docs(tmp.path()),
            Some(SourceRef::Github { .. })
        ));
        assert_eq!(svn_revision(tmp.path()), Some(7));
    }

    #[test]
    fn docs_keep_their_order_of_trust() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        std::fs::write(d.join("README.md"), "https://github.com/readme/repo\n").unwrap();
        std::fs::write(d.join("CHANGELOG.txt"), "https://github.com/txt/repo\n").unwrap();
        std::fs::write(d.join("CHANGELOG.md"), "https://github.com/md/repo\n").unwrap();
        let owner = |l: &DirListing| match github_from_listing(l) {
            Some(SourceRef::Github { owner, .. }) => owner,
            other => panic!("{other:?}"),
        };
        // CHANGELOG beats README, and .md beats .txt within CHANGELOG.
        assert_eq!(owner(&DirListing::read(d).unwrap()), "md");
        std::fs::remove_file(d.join("CHANGELOG.md")).unwrap();
        assert_eq!(owner(&DirListing::read(d).unwrap()), "txt");
        // A changelog without a link falls through to the README.
        std::fs::write(d.join("CHANGELOG.txt"), "- fixed things\n").unwrap();
        assert_eq!(owner(&DirListing::read(d).unwrap()), "readme");
    }

    #[test]
    fn oversized_and_folder_docs_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        // A CHANGELOG folder (some repos keep one) and a huge changelog are not read.
        std::fs::create_dir(d.join("CHANGELOG")).unwrap();
        let big = format!(
            "https://github.com/big/repo\n{}",
            "x".repeat(MAX_FILE_BYTES as usize)
        );
        std::fs::write(d.join("CHANGES.md"), big).unwrap();
        std::fs::write(d.join("Version.lua"), "$Revision: 12 $").unwrap();
        let l = DirListing::read(d).unwrap();
        assert_eq!(github_from_listing(&l), None);
        assert_eq!(svn_revision_in(&l), Some(12));
        assert!(matches!(
            read_regular_file(&d.join("CHANGES.md"), MAX_FILE_BYTES).unwrap(),
            Bounded::TooLarge(_)
        ));
        assert!(matches!(
            read_regular_file(&d.join("CHANGELOG"), MAX_FILE_BYTES).unwrap(),
            Bounded::NotAFile
        ));
    }

    #[test]
    fn symlinked_docs_are_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(
            outside.join("secret.txt"),
            "https://github.com/secret/repo\n$Revision: 99 $",
        )
        .unwrap();
        let d = tmp.path().join("Addon");
        std::fs::create_dir(&d).unwrap();
        if !try_symlink_file(&outside.join("secret.txt"), &d.join("CHANGELOG.md"))
            || !try_symlink_file(&outside.join("secret.txt"), &d.join("Ver.lua"))
        {
            eprintln!("symlinks not permitted here, skipping");
            return;
        }
        std::fs::write(d.join("README.md"), "https://github.com/real/repo\n").unwrap();
        let l = DirListing::read(&d).unwrap();
        assert_eq!(l.find_ci("ver.lua").map(|e| e.kind), Some(EntryKind::Link));
        assert!(matches!(
            github_from_listing(&l),
            Some(SourceRef::Github { owner, .. }) if owner == "real"
        ));
        assert_eq!(svn_revision_in(&l), None);
        assert!(matches!(
            read_regular_file(&d.join("Ver.lua"), MAX_FILE_BYTES).unwrap(),
            Bounded::NotAFile
        ));
    }
}
