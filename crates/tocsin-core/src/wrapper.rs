//! A folder with no `.toc` of its own that holds addon folders one level down.
//!
//! Windows' "Extract all" unpacks `Details-Details.20261104….zip` into a
//! folder named after the zip, so `Details/`, `Details_Compare2/`, … end up
//! one level too deep: copied into `AddOns` like that, the game loads none of
//! them. Some packages are zipped the same way, with a wrapper folder around
//! their addon folders. A GitHub "Source code" zip looks alike from afar
//! (`Repo-main/Addon/…`) but holds a repository, not a release: libraries
//! missing, `@project-version@` placeholders. One rule tells them apart, used
//! for zips ([`crate::archive`]) and for folders on disk ([`on_disk`]):
//!
//! * a **wrapper** has only addon folders inside (each with its own
//!   `<Folder>.toc` or `<Folder>_<game>.toc`, as the game requires), plus
//!   docs at most;
//! * a **source checkout** has a `.toc` directly inside, a `.pkgmeta`, a
//!   `.git…` entry, or loose files beside the folders that are not docs;
//! * anything else is neither, and is not treated as either.
//!
//! Only one level is ever looked at: the game itself loads
//! `AddOns/<Folder>/<Folder>.toc` and nothing deeper, and real packages have
//! zero wrapper folders or one.

use std::path::Path;

use crate::archive::is_toc_for;
use crate::discover::{DirListing, EntryKind};

/// Most folders looked into inside one candidate wrapper. Details ships 8,
/// DBM a few dozen; a folder with more is not a wrapper Tocsin handles.
const MAX_INNER_DIRS: usize = 64;

/// One entry directly inside a candidate wrapper.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Inner<'a> {
    /// A file.
    File(&'a str),
    /// A folder, and whether it has its own `.toc`.
    Dir {
        /// Folder name.
        name: &'a str,
        /// `<name>/<name>.toc` or `<name>/<name>_<game>.toc` exists.
        has_toc: bool,
    },
    /// A link or anything else that is neither a file nor a folder.
    Other(&'a str),
}

/// What a folder without a `.toc` of its own holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
    /// Only addon folders (and docs): they belong one level up. Sorted.
    Wrapper(Vec<String>),
    /// A repository: a `.toc` inside, a `.pkgmeta`, a `.git…` entry or loose
    /// files that are not docs.
    SourceCheckout,
    /// Addon folders beside folders that are not addons. Both sorted.
    Mixed {
        /// The folders with their own `.toc`.
        addons: Vec<String>,
        /// The others (no `.toc`, or a link).
        others: Vec<String>,
    },
    /// No addon folder inside (or more folders than Tocsin looks into).
    NoAddons,
}

/// Apply the rule to the entries directly inside a folder.
pub(crate) fn classify<'a>(inner: impl IntoIterator<Item = Inner<'a>>) -> Layout {
    let mut addons = Vec::new();
    let mut others = Vec::new();
    for entry in inner {
        let name = match entry {
            Inner::File(n) | Inner::Dir { name: n, .. } | Inner::Other(n) => n,
        };
        if is_repository_mark(name) {
            return Layout::SourceCheckout;
        }
        if name.starts_with('.') {
            // `.DS_Store` from a Mac-made zip, an editor's folder: not the
            // game's, not moved, no verdict either way.
            continue;
        }
        match entry {
            Inner::File(n) if is_toc_file(n) || !is_doc(n) => return Layout::SourceCheckout,
            Inner::File(_) => {}
            Inner::Dir { name, has_toc: true } => addons.push(name.to_string()),
            Inner::Dir { name, has_toc: false } | Inner::Other(name) => others.push(name.to_string()),
        }
    }
    addons.sort();
    others.sort();
    match (addons.is_empty(), others.is_empty()) {
        (true, _) => Layout::NoAddons,
        (false, true) => Layout::Wrapper(addons),
        (false, false) => Layout::Mixed { addons, others },
    }
}

/// Look one level into `dir`, a folder in `AddOns` with no `.toc` of its
/// own. Reads directory listings only (the folder's and those of the folders
/// inside it, at most [`MAX_INNER_DIRS`]), never a file, never through a link.
pub fn on_disk(dir: &Path) -> Layout {
    let Ok(listing) = DirListing::read(dir) else {
        return Layout::NoAddons;
    };
    let dirs = listing
        .entries()
        .iter()
        .filter(|e| e.kind == EntryKind::Dir && !e.name.starts_with('.'))
        .count();
    if dirs > MAX_INNER_DIRS {
        return Layout::NoAddons;
    }
    let has_toc = |name: &str| {
        DirListing::read(&dir.join(name)).is_ok_and(|inner| {
            inner
                .entries()
                .iter()
                .any(|e| e.kind == EntryKind::File && is_toc_for(&e.name, name))
        })
    };
    classify(listing.entries().iter().map(|e| match e.kind {
        EntryKind::File => Inner::File(&e.name),
        EntryKind::Dir => Inner::Dir {
            name: &e.name,
            // Dot-folders are skipped by `classify` (or end it): no listing.
            has_toc: !e.name.starts_with('.') && has_toc(&e.name),
        },
        EntryKind::Link | EntryKind::Other => Inner::Other(&e.name),
    }))
}

/// Marks of a repository checkout: the packager's recipe and Git's files
/// (`.git`, `.github`, `.gitignore`, `.gitattributes`, `.gitmodules`).
fn is_repository_mark(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".pkgmeta" || lower.starts_with(".git")
}

fn is_toc_file(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".toc")
}

/// Text that comes with a package and that nothing loads: `README.md`,
/// `LICENSE`, `CHANGES.txt`. A `.lua` or `.xml` beside the addon folders is
/// code, which only a repository has there.
pub(crate) fn is_doc(name: &str) -> bool {
    const EXTS: &[&str] = &["md", "txt", "markdown"];
    const NAMES: &[&str] = &[
        "readme",
        "license",
        "licence",
        "changelog",
        "changes",
        "history",
        "copying",
        "authors",
    ];
    let lower = name.to_ascii_lowercase();
    let (stem, ext) = lower.rsplit_once('.').unwrap_or((lower.as_str(), ""));
    EXTS.contains(&ext) || (NAMES.contains(&stem) && (ext.is_empty() || EXTS.contains(&ext)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> Inner<'_> {
        Inner::Dir { name, has_toc: true }
    }

    /// The folder Windows' "Extract all" makes from Details' CurseForge zip.
    const DETAILS: &[&str] = &[
        "Details",
        "Details_Compare2",
        "Details_DataStorage",
        "Details_EncounterDetails",
        "Details_RaidCheck",
        "Details_Streamer",
        "Details_TinyThreat",
        "Details_Vanguard",
    ];

    #[test]
    fn only_addon_folders_make_a_wrapper() {
        let layout = classify(DETAILS.iter().map(|n| dir(n)));
        assert_eq!(
            layout,
            Layout::Wrapper(DETAILS.iter().map(|s| s.to_string()).collect())
        );
        // Docs and a Mac's `.DS_Store` beside them change nothing.
        let with_docs = DETAILS.iter().map(|n| dir(n)).chain([
            Inner::File("README.md"),
            Inner::File("LICENSE"),
            Inner::File(".DS_Store"),
        ]);
        assert!(matches!(classify(with_docs), Layout::Wrapper(v) if v.len() == 8));
    }

    #[test]
    fn a_repository_is_a_source_checkout() {
        // GitHub "Source code" zip of Details: the `.toc` sits in the wrapper.
        let details_main = [
            Inner::File("Details.toc"),
            Inner::File("boot.lua"),
            Inner::Dir {
                name: "core",
                has_toc: false,
            },
        ];
        assert_eq!(classify(details_main), Layout::SourceCheckout);
        // WeakAuras2-main: addon folders, but the packager's recipe beside them.
        let weakauras_main = [dir("WeakAuras"), dir("WeakAurasOptions"), Inner::File(".pkgmeta")];
        assert_eq!(classify(weakauras_main), Layout::SourceCheckout);
        let with_git = [
            dir("WeakAuras"),
            Inner::Dir {
                name: ".github",
                has_toc: false,
            },
        ];
        assert_eq!(classify(with_git), Layout::SourceCheckout);
        // Loose code beside the addon folders.
        assert_eq!(
            classify([dir("Foo"), Inner::File("build.lua")]),
            Layout::SourceCheckout
        );
    }

    #[test]
    fn anything_in_between_is_neither() {
        let mixed = [
            dir("Foo"),
            Inner::Dir {
                name: "Libs",
                has_toc: false,
            },
            Inner::Other("Linked"),
        ];
        assert_eq!(
            classify(mixed),
            Layout::Mixed {
                addons: vec!["Foo".into()],
                others: vec!["Libs".into(), "Linked".into()],
            }
        );
        assert_eq!(classify([Inner::File("readme.txt")]), Layout::NoAddons);
        assert_eq!(classify([]), Layout::NoAddons);
    }

    #[test]
    fn docs_are_text_nothing_loads() {
        for doc in [
            "README.md",
            "CHANGES.txt",
            "LICENSE",
            "license.md",
            "Notes.markdown",
            "COPYING",
        ] {
            assert!(is_doc(doc), "{doc}");
        }
        for code in [
            "boot.lua",
            "Bindings.xml",
            "Details.toc",
            "install.exe",
            "license.lua",
        ] {
            assert!(!is_doc(code), "{code}");
        }
    }

    #[test]
    fn on_disk_reads_one_level() {
        let tmp = tempfile::tempdir().unwrap();
        let w = tmp.path().join("Details-Details.20261104.15400.172-1-gcec582c");
        for name in DETAILS {
            std::fs::create_dir_all(w.join(name)).unwrap();
            std::fs::write(w.join(name).join(format!("{name}.toc")), "## Title: x\n").unwrap();
        }
        // The plugins ship one `.toc` per game: `<Folder>_<game>.toc` counts.
        std::fs::remove_file(w.join("Details_Vanguard/Details_Vanguard.toc")).unwrap();
        std::fs::write(w.join("Details_Vanguard/Details_Vanguard_Mists.toc"), "").unwrap();
        assert!(matches!(on_disk(&w), Layout::Wrapper(v) if v.len() == 8));

        // Deeper is not looked at: a folder whose `.toc` is two levels down
        // is not an addon folder.
        std::fs::create_dir_all(w.join("Extra/Extra/")).unwrap();
        std::fs::write(w.join("Extra/Extra/Extra.toc"), "").unwrap();
        assert!(matches!(on_disk(&w), Layout::Mixed { others, .. } if others == ["Extra"]));
        assert_eq!(on_disk(&tmp.path().join("missing")), Layout::NoAddons);
    }

    #[test]
    fn on_disk_does_not_look_into_a_crowd() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..=MAX_INNER_DIRS {
            let name = format!("A{i}");
            std::fs::create_dir_all(tmp.path().join(&name)).unwrap();
            std::fs::write(tmp.path().join(&name).join(format!("{name}.toc")), "").unwrap();
        }
        assert_eq!(on_disk(tmp.path()), Layout::NoAddons);
    }
}
