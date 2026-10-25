//! Discovery of installed addons in an `Interface/AddOns` directory.

use std::path::{Path, PathBuf};

use crate::discover::{self, DirListing, EntryKind};
use crate::manifest::SourceOverride;
use crate::sources::SourceRef;
use crate::toc::Toc;
use crate::version::Version;

/// Which game client the `.toc` targets. Blizzard uses suffixes on the file
/// name (`Addon_Mainline.toc`, `Addon_Vanilla.toc`, …) for multi-flavor addons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Flavor {
    /// Retail (`_Mainline`).
    #[default]
    Retail,
    /// Classic Era (`_Vanilla`).
    Vanilla,
    /// Other classic progressions (`_Cata`, `_Mists`, `_TBC`, `_Wrath`, …).
    Classic,
    /// WoW Classic Forever (product `wowf`, builds `1.60.x`, game type `camelot`).
    Forever,
}

impl Flavor {
    /// `.toc` file-name suffixes this flavor loads, most specific first.
    fn suffixes(self) -> &'static [&'static str] {
        match self {
            Flavor::Retail => &["mainline", "retail", "standard"],
            Flavor::Vanilla => &["vanilla", "classic"],
            Flavor::Classic => &["cata", "mists", "mop", "wrath", "wotlk", "tbc", "bcc"],
            Flavor::Forever => &["forever", "camelot", "vanilla", "classic"],
        }
    }

    /// Game types for `[AllowLoadGameType …]` conditions inside a `.toc`.
    pub fn game_types(self) -> &'static [&'static str] {
        match self {
            Flavor::Retail => &["standard"],
            Flavor::Vanilla => &["vanilla", "classic"],
            Flavor::Classic => &["mists", "cata", "wrath", "tbc", "classic"],
            Flavor::Forever => &["camelot", "classic"],
        }
    }

    /// Flavor a `## Interface` code belongs to: `120105` → Retail, `11509` →
    /// Classic Era, `50504` → Classic (Mists), `16001` → Forever (1.60.x).
    pub fn of_interface(code: u32) -> Flavor {
        let major = code / 10000;
        let minor = (code / 100) % 100;
        match major {
            1 if minor >= 50 => Flavor::Forever,
            1 => Flavor::Vanilla,
            2..=9 => Flavor::Classic,
            _ => Flavor::Retail,
        }
    }

    /// Label for messages.
    pub fn label(self) -> &'static str {
        match self {
            Flavor::Retail => "Retail",
            Flavor::Vanilla => "Classic Era",
            Flavor::Classic => "Classic",
            Flavor::Forever => "Forever",
        }
    }
}

/// How an addon's `## Interface` list relates to the game it sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compat {
    /// Lists the client's exact build.
    Exact,
    /// Built for this flavor, but an older build (`120007` while the game is
    /// `120105`): the game shows it as "out of date" and still loads it.
    Older {
        /// Highest build listed for this flavor.
        built_for: u32,
    },
    /// Built for this flavor and a *newer* build than the installed game.
    Newer {
        /// Lowest build listed for this flavor.
        built_for: u32,
    },
    /// Lists builds, none of them for this flavor.
    OtherFlavor,
    /// No `## Interface` line, or no build known for the client.
    Unknown,
}

impl Compat {
    /// Compare an `## Interface` list against the client build.
    pub fn of(interfaces: &[u32], flavor: Flavor, client_interface: Option<u32>) -> Compat {
        if interfaces.is_empty() {
            return Compat::Unknown;
        }
        let mine: Vec<u32> = interfaces
            .iter()
            .copied()
            .filter(|&c| Flavor::of_interface(c) == flavor)
            .collect();
        if mine.is_empty() {
            return Compat::OtherFlavor;
        }
        let Some(current) = client_interface else {
            return Compat::Unknown;
        };
        if mine.contains(&current) {
            return Compat::Exact;
        }
        let max = mine.iter().copied().max().unwrap_or(0);
        let min = mine.iter().copied().min().unwrap_or(0);
        if max < current {
            Compat::Older { built_for: max }
        } else {
            Compat::Newer { built_for: min }
        }
    }
}

/// `120105` → `12.1.5`.
pub fn interface_to_string(code: u32) -> String {
    format!("{}.{}.{}", code / 10000, (code / 100) % 100, code % 100)
}

/// Something wrong with how an addon sits on disk. Found at scan time, no
/// network involved; the game would ignore or mis-load such an addon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// The folder has `.toc` files, but none for this game flavor. The
    /// metadata shown comes from another flavor's `.toc`.
    NotForFlavor {
        /// Flavor of the game folder being scanned.
        flavor: Flavor,
    },
    /// WoW requires `<Folder>/<Folder>.toc`; this folder is named differently
    /// (typically a GitHub "Source code" zip: `Addon-main/Addon.toc`).
    Misnamed {
        /// Folder name the `.toc` implies.
        expected: String,
    },
    /// The folder is a source checkout (`.pkgmeta`, `.git`), not a packaged
    /// release: libraries may be missing and the version is unknown.
    SourceCheckout,
    /// `## Interface` lists this flavor only at an older build than the game.
    /// Blizzard flags it "out of date"; it usually still works.
    OutOfDate {
        /// Highest build listed.
        built_for: u32,
        /// The game's build.
        game: u32,
    },
}

impl Issue {
    /// Short, user-facing description.
    pub fn describe(&self) -> String {
        match self {
            Issue::NotForFlavor { flavor } => {
                format!("No .toc for {}: the game will not load it", flavor.label())
            }
            Issue::Misnamed { expected } => {
                format!("Folder should be named {expected}: the game will not load it")
            }
            Issue::SourceCheckout => {
                "Source checkout, not a release: version unknown, libraries may be missing".into()
            }
            Issue::OutOfDate { built_for, game } => format!(
                "Built for {}, game is {}: loads as out of date",
                interface_to_string(*built_for),
                interface_to_string(*game)
            ),
        }
    }

    /// Whether this is a hard problem (red) rather than a note (muted).
    pub fn is_blocking(&self) -> bool {
        !matches!(self, Issue::OutOfDate { .. })
    }
}

/// An addon folder found on disk.
#[derive(Debug, Clone)]
pub struct InstalledAddon {
    /// Folder name inside `AddOns` (the stable identity of an addon).
    pub folder: String,
    /// Full path of the addon folder.
    pub path: PathBuf,
    /// The `.toc` used for metadata.
    pub toc_path: PathBuf,
    /// Parsed metadata.
    pub toc: Toc,
    /// Installed version, if the author declared one.
    pub installed: Option<Version>,
    /// Where to look for updates, best first. Resolved from the `.toc`,
    /// replaced entirely by the user's manifest when it declares a source.
    pub sources: Vec<SourceRef>,
    /// Optional user-facing name override.
    pub display_name: String,
    /// Problems with the installation itself (see [`Issue`]).
    pub issues: Vec<Issue>,
    /// Lives in `.tocsin-disabled`: the game does not see it.
    pub disabled: bool,
}

impl InstalledAddon {
    /// Build from a folder, picking the best `.toc` for `flavor`.
    pub fn from_dir(dir: &Path, flavor: Flavor) -> Option<Self> {
        Self::from_dir_for(dir, &crate::client::GameClient::flavor_only(flavor))
    }

    /// Build from a folder for a specific game client (flavor and, when
    /// known, build), so `## Interface` can be checked exactly.
    pub fn from_dir_for(dir: &Path, client: &crate::client::GameClient) -> Option<Self> {
        let flavor = client.flavor;
        let folder = dir.file_name()?.to_str()?.to_string();
        // One listing serves the .toc pick and every discover heuristic.
        let listing = DirListing::read(dir).ok()?;
        let picked = pick_toc(&listing, &folder, flavor)?;
        let toc_path = picked.path;
        let toc = Toc::from_path_for(&toc_path, flavor.game_types()).ok()?;

        let mut issues = Vec::new();
        // `## Interface` is the deterministic answer; the file-name suffix is
        // the fallback for .toc files that do not declare one.
        match Compat::of(&toc.interface_versions(), flavor, client.interface()) {
            Compat::OtherFlavor => issues.push(Issue::NotForFlavor { flavor }),
            Compat::Older { built_for } => {
                if let Some(game) = client.interface() {
                    issues.push(Issue::OutOfDate { built_for, game });
                }
            }
            Compat::Unknown if !picked.for_flavor => issues.push(Issue::NotForFlavor { flavor }),
            _ => {}
        }
        if let Some(expected) = picked.misnamed {
            issues.push(Issue::Misnamed { expected });
        }
        if is_source_checkout(&listing) {
            issues.push(Issue::SourceCheckout);
        }
        // `.toc` says `9.6`, Ver.lua says `$Revision: 37349 $` → `9.6.37349`.
        let installed = toc.version().map(|v| match discover::svn_revision_in(&listing) {
            Some(rev) if Version::parse(v).numbers().len() <= 2 => Version::parse(&format!("{v}.{rev}")),
            _ => Version::parse(v),
        });

        let mut sources = SourceRef::candidates_from_toc(&toc);
        // A GitHub link in the changelog beats keyed providers: no API key needed.
        if !sources.iter().any(|s| matches!(s, SourceRef::Github { .. })) {
            if let Some(gh) = discover::github_from_listing(&listing) {
                sources.insert(0, gh);
            }
        }
        let display_name = toc.title().unwrap_or_else(|| folder.clone());
        Some(Self {
            folder,
            path: dir.to_path_buf(),
            toc_path,
            toc,
            installed,
            sources,
            display_name,
            issues,
            disabled: false,
        })
    }

    /// The preferred source, if any.
    pub fn source(&self) -> Option<&SourceRef> {
        self.sources.first()
    }

    /// Put `source` first, dropping any other entry for the same project.
    pub fn prefer_source(&mut self, source: &SourceRef) {
        let key = source.project_key();
        self.sources
            .retain(|s| s != source && (key.is_none() || s.project_key() != key));
        self.sources.insert(0, source.clone());
    }

    /// Apply a user override from `tocsin.toml`.
    pub fn apply_override(&mut self, ov: &SourceOverride) {
        if let Some(src) = &ov.source {
            self.sources = vec![src.clone()];
        }
        if let Some(name) = &ov.name {
            self.display_name = name.clone();
        }
        if let Some(v) = &ov.version {
            self.installed = Some(Version::parse(v));
        }
    }
}

/// A `.pkgmeta` (BigWigs packager recipe) or a `.git` next to the `.toc` means
/// someone extracted a repository, not a packaged release (Details "Source
/// code" zips). Only presence matters, so a link counts without being followed.
fn is_source_checkout(listing: &DirListing) -> bool {
    listing
        .find_ci(".pkgmeta")
        .is_some_and(|e| e.kind != EntryKind::Dir)
        || listing.find_ci(".git").is_some()
}

/// Outcome of choosing a `.toc` in an addon folder.
struct PickedToc {
    path: PathBuf,
    /// Whether the file is meant for the requested flavor (or is flavor-less).
    for_flavor: bool,
    /// Folder name the `.toc` file name implies, when it differs from the real one.
    misnamed: Option<String>,
}

/// Choose the `.toc` that matches `flavor`, falling back sensibly.
///
/// Only regular files count: a `.toc` that is a symlink (or a folder named
/// `x.toc`) is never read, so it is not a candidate either and the next best
/// `.toc` wins.
fn pick_toc(listing: &DirListing, folder: &str, flavor: Flavor) -> Option<PickedToc> {
    let mut candidates: Vec<PathBuf> = listing
        .entries()
        .iter()
        .filter(|e| e.kind == EntryKind::File)
        .filter(|e| {
            Path::new(&e.name)
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("toc"))
        })
        .map(|e| listing.dir().join(&e.name))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort();

    let lower_folder = folder.to_ascii_lowercase();
    let stem = |p: &PathBuf| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default()
    };
    let all_suffixes: Vec<&str> = [Flavor::Retail, Flavor::Vanilla, Flavor::Classic]
        .iter()
        .flat_map(|f| f.suffixes().iter().copied())
        .collect();
    // "weakauras_cata" → ("weakauras", Some("cata")); "details" → ("details", None)
    let split = |s: &str| -> (String, Option<String>) {
        for suf in &all_suffixes {
            for sep in ['_', '-'] {
                if let Some(base) = s.strip_suffix(&format!("{sep}{suf}")) {
                    return (base.to_string(), Some((*suf).to_string()));
                }
            }
        }
        (s.to_string(), None)
    };

    // 1) Folder_Suffix.toc / Folder-Suffix.toc for the requested flavor.
    for suffix in flavor.suffixes() {
        if let Some(p) = candidates.iter().find(|p| {
            let s = stem(p);
            s == format!("{lower_folder}_{suffix}") || s == format!("{lower_folder}-{suffix}")
        }) {
            return Some(PickedToc {
                path: p.clone(),
                for_flavor: true,
                misnamed: None,
            });
        }
    }
    // 2) Plain Folder.toc: flavor-less, loads everywhere.
    if let Some(p) = candidates.iter().find(|p| stem(p) == lower_folder) {
        return Some(PickedToc {
            path: p.clone(),
            for_flavor: true,
            misnamed: None,
        });
    }
    // 3) Folder_OtherFlavor.toc: right addon, wrong game (WeakAuras shipped
    //    without a Retail .toc). Not for this flavor, but the name matches.
    if let Some(p) = candidates.iter().find(|p| split(&stem(p)).0 == lower_folder) {
        return Some(PickedToc {
            path: p.clone(),
            for_flavor: false,
            misnamed: None,
        });
    }
    // 4) Any .toc: the folder is misnamed (source zip "Details-Damage-Meter-x/Details.toc").
    //    Prefer the flavor-less or flavor-matching one to name the expected folder.
    let best = candidates
        .iter()
        .find(|p| split(&stem(p)).1.is_none())
        .or_else(|| {
            candidates.iter().find(|p| {
                split(&stem(p))
                    .1
                    .as_deref()
                    .is_some_and(|x| flavor.suffixes().contains(&x))
            })
        })
        .or_else(|| candidates.first())?;
    let original_stem = best.file_stem().and_then(|s| s.to_str()).unwrap_or(folder);
    let (base_lower, suffix) = split(&stem(best));
    // Keep the author's casing from the file name.
    let expected = original_stem[..base_lower.len()].to_string();
    let for_flavor = suffix.is_none() || suffix.as_deref().is_some_and(|x| flavor.suffixes().contains(&x));
    Some(PickedToc {
        path: best.clone(),
        for_flavor,
        misnamed: Some(expected),
    })
}

/// Scan an `AddOns` directory for the given flavor.
pub fn scan_addons_dir(addons_dir: &Path) -> anyhow::Result<Vec<InstalledAddon>> {
    scan_addons_dir_for(addons_dir, Flavor::Retail)
}

/// Scan an `AddOns` directory, choosing `.toc` files for `flavor`.
pub fn scan_addons_dir_for(addons_dir: &Path, flavor: Flavor) -> anyhow::Result<Vec<InstalledAddon>> {
    scan_addons_dir_for_client(addons_dir, &crate::client::GameClient::flavor_only(flavor))
}

/// Scan an `AddOns` directory for a specific game client.
pub fn scan_addons_dir_for_client(
    addons_dir: &Path,
    client: &crate::client::GameClient,
) -> anyhow::Result<Vec<InstalledAddon>> {
    scan_counting_skipped(addons_dir, client).map(|(addons, _)| addons)
}

/// [`scan_addons_dir_for_client`], plus how many folders were not addons (no
/// usable `.toc`). Folders and no addons means the wrong folder was picked.
pub(crate) fn scan_counting_skipped(
    addons_dir: &Path,
    client: &crate::client::GameClient,
) -> anyhow::Result<(Vec<InstalledAddon>, usize)> {
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(addons_dir)? {
        let entry = entry?;
        // Skip files and symlinks: a symlinked "addon" could point anywhere.
        // `DirEntry::file_type` does not follow links.
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        // Blizzard's own addons live in Interface/AddOns too on some setups;
        // dot-folders are ours (staging, backups) or someone's junk.
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("Blizzard_") || n.starts_with('.'))
        {
            continue;
        }
        dirs.push(path);
    }
    let mut out = build_all(&dirs, client);
    // Folder names differing only by case can coexist on Linux; the second key
    // keeps the order independent of which thread finished first.
    out.sort_by_cached_key(|a| (a.folder.to_lowercase(), a.folder.clone()));
    let skipped = dirs.len() - out.len();
    Ok((out, skipped))
}

/// Upper bound on scan threads. Reading ~300 small folders is latency-bound
/// (one listing and two or three tiny reads each); past a handful of threads
/// the disk, not the CPU, is the limit.
const MAX_SCAN_THREADS: usize = 8;

/// Build every addon in `dirs`, on up to [`MAX_SCAN_THREADS`] threads.
///
/// Workers pull the next folder from a shared counter, so one huge folder
/// (DBM, hundreds of files) does not hold back a whole pre-assigned chunk.
/// The calling thread works too: if no thread can be spawned the scan still
/// completes, just sequentially.
fn build_all(dirs: &[PathBuf], client: &crate::client::GameClient) -> Vec<InstalledAddon> {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let threads = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(MAX_SCAN_THREADS)
        .min(dirs.len());
    let next = AtomicUsize::new(0);
    let work = || {
        let mut found = Vec::new();
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(dir) = dirs.get(i) else { break found };
            if let Some(addon) = InstalledAddon::from_dir_for(dir, client) {
                found.push(addon);
            }
        }
    };
    std::thread::scope(|s| {
        let helpers: Vec<_> = (1..threads)
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(s, work).ok())
            .collect();
        let mut out = work();
        for helper in helpers {
            match helper.join() {
                Ok(found) => out.extend(found),
                // Same outcome as the sequential scan had: the panic surfaces.
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_flavor_toc_then_plain() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Foo");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("Foo.toc"), "## Version: 1\n").unwrap();
        std::fs::write(dir.join("Foo_Mainline.toc"), "## Version: 2\n").unwrap();
        std::fs::write(dir.join("Foo_Vanilla.toc"), "## Version: 3\n").unwrap();

        let a = InstalledAddon::from_dir(&dir, Flavor::Retail).unwrap();
        assert_eq!(a.installed.unwrap().as_str(), "2");
        let a = InstalledAddon::from_dir(&dir, Flavor::Vanilla).unwrap();
        assert_eq!(a.installed.unwrap().as_str(), "3");
        let a = InstalledAddon::from_dir(&dir, Flavor::Classic).unwrap();
        assert_eq!(a.installed.unwrap().as_str(), "1");
        assert!(a.issues.is_empty());
    }

    #[test]
    fn interface_decides_compatibility() {
        let f = Flavor::Retail;
        // BigWigs: every flavor, including the exact Retail build.
        let bw = [
            11508, 11509, 16001, 20505, 30405, 40402, 50504, 120100, 120105, 120107,
        ];
        assert_eq!(Compat::of(&bw, f, Some(120105)), Compat::Exact);
        assert_eq!(Compat::of(&bw, Flavor::Vanilla, Some(11509)), Compat::Exact);
        assert_eq!(Compat::of(&bw, Flavor::Forever, Some(16001)), Compat::Exact);
        assert_eq!(Compat::of(&bw, Flavor::Classic, Some(50504)), Compat::Exact);
        // WeakAuras package shipped without Retail.
        assert_eq!(
            Compat::of(&[40402, 50504, 11509], f, Some(120105)),
            Compat::OtherFlavor
        );
        // Routine built before 12.1.5.
        assert_eq!(
            Compat::of(&[120007, 120100], f, Some(120105)),
            Compat::Older { built_for: 120100 }
        );
        assert_eq!(
            Compat::of(&[120200], f, Some(120105)),
            Compat::Newer { built_for: 120200 }
        );
        // No build known: only the flavor can be judged.
        assert_eq!(Compat::of(&[120100], f, None), Compat::Unknown);
        assert_eq!(Compat::of(&[40402], f, None), Compat::OtherFlavor);
        assert_eq!(Compat::of(&[], f, Some(120105)), Compat::Unknown);
        assert_eq!(interface_to_string(120105), "12.1.5");
        assert_eq!(interface_to_string(16001), "1.60.1");
    }

    #[test]
    fn flags_missing_flavor_misnamed_and_source_checkouts() {
        let tmp = tempfile::tempdir().unwrap();
        // WeakAuras package without a Retail .toc.
        let wa = tmp.path().join("WeakAuras");
        std::fs::create_dir(&wa).unwrap();
        std::fs::write(wa.join("WeakAuras_Cata.toc"), "## Version: 5.22.1\n").unwrap();
        std::fs::write(wa.join("WeakAuras_Vanilla.toc"), "## Version: 5.22.1\n").unwrap();
        let a = InstalledAddon::from_dir(&wa, Flavor::Retail).unwrap();
        assert_eq!(
            a.issues,
            vec![Issue::NotForFlavor {
                flavor: Flavor::Retail
            }]
        );
        assert!(InstalledAddon::from_dir(&wa, Flavor::Classic)
            .unwrap()
            .issues
            .is_empty());

        // GitHub "Source code" zip of Details, extracted as-is.
        let d = tmp.path().join("Details-Damage-Meter-DetailsRetail.v9.1.0");
        std::fs::create_dir(&d).unwrap();
        std::fs::write(d.join("Details.toc"), "## Title: Details\n").unwrap();
        std::fs::write(d.join("Details-BCC.toc"), "## Title: Details\n").unwrap();
        std::fs::write(d.join(".pkgmeta"), "move-folders:\n").unwrap();
        let a = InstalledAddon::from_dir(&d, Flavor::Retail).unwrap();
        assert_eq!(
            a.issues,
            vec![
                Issue::Misnamed {
                    expected: "Details".into()
                },
                Issue::SourceCheckout
            ]
        );
        assert!(a.toc_path.ends_with("Details.toc"));
    }

    #[test]
    fn symlinked_toc_is_skipped_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let elsewhere = tmp.path().join("elsewhere.toc");
        std::fs::write(&elsewhere, "## Version: 666\n").unwrap();
        let dir = tmp.path().join("Foo");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("Foo.toc"), "## Version: 1\n").unwrap();
        if !discover::try_symlink_file(&elsewhere, &dir.join("Foo_Mainline.toc")) {
            eprintln!("symlinks not permitted here, skipping");
            return;
        }
        // The flavor .toc is a link: ignored, the plain Foo.toc wins.
        let a = InstalledAddon::from_dir(&dir, Flavor::Retail).unwrap();
        assert_eq!(a.installed.unwrap().as_str(), "1");
        assert!(a.toc_path.ends_with("Foo.toc"));

        // Only a linked .toc: no addon, and no crash.
        let only = tmp.path().join("Bar");
        std::fs::create_dir(&only).unwrap();
        assert!(discover::try_symlink_file(&elsewhere, &only.join("Bar.toc")));
        assert!(InstalledAddon::from_dir(&only, Flavor::Retail).is_none());
    }

    /// Junctions need no privilege on Windows, so this runs where symlinks
    /// cannot be made: a junctioned addon folder and a junction posing as a
    /// `.toc` are both ignored.
    #[cfg(windows)]
    #[test]
    fn junctions_are_links_on_windows() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("Evil.toc"), "## Version: 666\n").unwrap();
        let addons = tmp.path().join("AddOns");
        let foo = addons.join("Foo");
        std::fs::create_dir_all(&foo).unwrap();
        std::fs::write(foo.join("Foo.toc"), "## Version: 1\n").unwrap();
        let junction = |link: &Path| {
            std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(link)
                .arg(&target)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !junction(&addons.join("Evil")) || !junction(&foo.join("Foo_Mainline.toc")) {
            eprintln!("junctions not available here, skipping");
            return;
        }
        let listing = DirListing::read(&foo).unwrap();
        assert_eq!(
            listing.find_ci("foo_mainline.toc").map(|e| e.kind),
            Some(EntryKind::Link)
        );
        let list = scan_addons_dir(&addons).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].installed.as_ref().unwrap().as_str(), "1");
    }

    #[test]
    fn zygor_revision_and_routine_changelog_come_from_one_listing() {
        let tmp = tempfile::tempdir().unwrap();
        let z = tmp.path().join("ZygorGuidesViewer");
        std::fs::create_dir(&z).unwrap();
        std::fs::write(z.join("ZygorGuidesViewer.toc"), "## Version: 9.6\n").unwrap();
        std::fs::write(z.join("Ver.lua"), r#"ZGV.revision = "$Revision: 37349 $""#).unwrap();
        let a = InstalledAddon::from_dir(&z, Flavor::Retail).unwrap();
        assert_eq!(a.installed.unwrap().as_str(), "9.6.37349");

        let r = tmp.path().join("Routine");
        std::fs::create_dir(&r).unwrap();
        std::fs::write(
            r.join("Routine.toc"),
            "## Version: v12.0.376\n## X-Wago-ID: QN53j5KB\n",
        )
        .unwrap();
        std::fs::write(
            r.join("CHANGELOG.md"),
            "## [v12.0.351](https://github.com/LoyalFTW/Midnight-Routine/tree/v12.0.351)",
        )
        .unwrap();
        let a = InstalledAddon::from_dir(&r, Flavor::Retail).unwrap();
        assert!(matches!(a.source(), Some(SourceRef::Github { repo, .. }) if repo == "Midnight-Routine"));
        assert!(a.issues.is_empty());
    }

    #[test]
    fn parallel_scan_is_complete_and_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        // More folders than threads, mixed case, plus folders the scan must skip.
        let names: Vec<String> = (0..40)
            .map(|i| {
                if i % 2 == 0 {
                    format!("addon{i:02}")
                } else {
                    format!("Addon{i:02}")
                }
            })
            .collect();
        for n in &names {
            let d = tmp.path().join(n);
            std::fs::create_dir(&d).unwrap();
            std::fs::write(d.join(format!("{n}.toc")), "## Version: 1\n").unwrap();
        }
        std::fs::create_dir(tmp.path().join("NoToc")).unwrap();
        std::fs::create_dir(tmp.path().join(".tocsin-backup")).unwrap();

        let list = scan_addons_dir(tmp.path()).unwrap();
        let got: Vec<&str> = list.iter().map(|a| a.folder.as_str()).collect();
        assert_eq!(got, names.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(scan_addons_dir(&tmp.path().join("NoToc")).unwrap().is_empty());
    }

    #[test]
    fn scan_skips_files_and_blizzard() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("Blizzard_X")).unwrap();
        std::fs::write(tmp.path().join("Blizzard_X/Blizzard_X.toc"), "## Version: 1").unwrap();
        std::fs::write(tmp.path().join("stray.txt"), "").unwrap();
        let dir = tmp.path().join("Bar");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("Bar.toc"), "## Title: Bar\n## X-WoWI-ID: 12\n").unwrap();

        let list = scan_addons_dir(tmp.path()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].folder, "Bar");
        assert!(matches!(list[0].source(), Some(SourceRef::WowInterface { .. })));
    }
}
