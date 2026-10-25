//! Tocsin core: discover installed World of Warcraft addons, read their `.toc`
//! metadata and ask the original source (GitHub, CurseForge, WoWInterface, …) whether a
//! newer version exists.
//!
//! The crate is GUI-agnostic and has no global state; the GUI crate (and tests)
//! drive it through [`Library`] and [`checker::check_all`].

#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::all)]

pub mod addon;
pub mod archive;
pub mod checker;
pub mod client;
pub mod detect;
pub mod discover;
pub mod download;
pub mod family;
pub mod http;
pub mod icons;
pub mod install;
pub mod manifest;
pub mod paths;
pub mod search;
pub mod secrets;
pub mod sources;
pub mod toc;
pub mod updater;
pub mod version;

pub use addon::{scan_addons_dir, InstalledAddon};
pub use checker::{check_all, UpdateReport, UpdateStatus};
pub use manifest::{Manifest, SourceOverride};
pub use sources::SourceRef;
pub use toc::Toc;
pub use version::Version;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One look at an installation: the library and what the install log says
/// can be undone.
#[derive(Debug, Clone)]
pub struct Scan {
    /// The addons on disk.
    pub library: Library,
    /// Folders with a previous generation to roll back to.
    pub rollbackable: BTreeSet<String>,
    /// Families removed by Tocsin that can be restored: (key, version).
    pub removed: Vec<(String, String)>,
    /// Why the install log could not be read; the scan goes on without it.
    pub log_error: Option<String>,
}

/// Scan `addons_dir` (blocking). Clears what an interrupted install left in
/// staging first, then reads the install log once for the library, the
/// rollback list and the removed list.
pub fn scan(addons_dir: &Path, manifest: &Manifest) -> anyhow::Result<Scan> {
    install::clean_staging(addons_dir);
    let (log, log_error) = match install::Log::load(addons_dir) {
        Ok(log) => (log, None),
        Err(e) => {
            tracing::warn!(error = %e, "install log unreadable; ignoring it");
            (install::Log::default(), Some(e.to_string()))
        }
    };
    let library = Library::load_with_log(addons_dir, manifest, &log)?;
    Ok(Scan {
        library,
        rollbackable: install::rollbackable_in(&log),
        removed: install::removed_in(&log),
        log_error,
    })
}

/// Everything the app knows about one WoW installation: where the AddOns
/// folder is and which addons live in it.
#[derive(Debug, Clone)]
pub struct Library {
    /// The `Interface/AddOns` directory.
    pub addons_dir: PathBuf,
    /// Addons discovered on disk, sorted by folder name.
    pub addons: Vec<InstalledAddon>,
    /// The game client this folder belongs to.
    pub client: client::GameClient,
    /// Folders in `AddOns` that are not addons (no usable `.toc`). Many of
    /// them and no addons means the folder holds something else.
    pub skipped: usize,
}

impl Library {
    /// Scan `addons_dir` and apply the overrides in `manifest`. Loads the
    /// install log itself; see [`Library::load_with_log`] to share one.
    pub fn load(addons_dir: &Path, manifest: &Manifest) -> anyhow::Result<Self> {
        let log = install::Log::load(addons_dir).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "install log unreadable; ignoring it");
            install::Log::default()
        });
        Self::load_with_log(addons_dir, manifest, &log)
    }

    /// [`Library::load`] with an install log the caller already loaded, so
    /// one read serves the scan, the rollback list and the removed list.
    pub fn load_with_log(addons_dir: &Path, manifest: &Manifest, log: &install::Log) -> anyhow::Result<Self> {
        let client = client::GameClient::detect(addons_dir);
        let (mut addons, skipped) = addon::scan_counting_skipped(addons_dir, &client)?;
        // Disabled addons are parked in a dot-folder; list them too, flagged.
        // Only when it is a real folder: a link there would have the scan
        // read whatever it points to.
        let disabled_dir = addons_dir.join(install::DISABLED_DIR);
        let disabled_ok = install::is_real_dir(&disabled_dir);
        if disabled_ok {
            for mut a in addon::scan_addons_dir_for_client(&disabled_dir, &client)? {
                if !addons.iter().any(|x| x.folder == a.folder) {
                    a.disabled = true;
                    addons.push(a);
                }
            }
            // Same order as the scan itself, whether or not this folder exists.
            addons.sort_by_cached_key(|a| (a.folder.to_lowercase(), a.folder.clone()));
        }
        // What Tocsin installed is known exactly from the install log; the
        // `.toc` is only a hint the author may have forgotten to bump (Routine's
        // GitHub build of v12.0.378 still says v12.0.372). Trusting the log
        // keeps a freshly updated addon from showing "update available" again.
        // Likewise the source the user installed from comes first: picking
        // WoWInterface in search means updates come from there, whatever the
        // `.toc` says.
        let mut pinned: Vec<String> = Vec::new();
        for record in log.installs.values() {
            let gen = &record.current;
            let present =
                |f: &String| addons_dir.join(f).is_dir() || (disabled_ok && disabled_dir.join(f).is_dir());
            if gen.folders.is_empty() || !gen.folders.iter().all(present) {
                continue;
            }
            for addon in addons.iter_mut().filter(|a| gen.folders.contains(&a.folder)) {
                // `unknown` is what the log says of a hand-installed copy that
                // was replaced and then rolled back to: the `.toc` knows better
                // (a rolled-back DBM-Core showed "unknown" instead of its 1.0).
                if gen.version != install::UNKNOWN_VERSION {
                    addon.installed = Some(Version::parse(&gen.version));
                }
                if let Some(src) = &record.source {
                    addon.prefer_source(src);
                    pinned.push(addon.folder.clone());
                }
            }
        }
        for addon in &mut addons {
            if let Some(ov) = manifest.overrides.get(&addon.folder) {
                addon.apply_override(ov);
            }
        }
        // Companions published with their family head (RoutineData ← Routine,
        // ElvUI_Options ← ElvUI) inherit its bundle-wide, key-free source
        // (GitHub repository or Tukui project), so they need no API key either.
        // Only when the author matches: a third-party HandyNotes plugin depends
        // on HandyNotes but is not released from its repository. User
        // overrides are left alone.
        for fam in family::group(&addons) {
            let head = &addons[fam.head];
            let Some(head_src) = head
                .sources
                .iter()
                .find(|s| matches!(s, SourceRef::Github { .. } | SourceRef::Tukui { .. }))
                .cloned()
            else {
                continue;
            };
            let head_author = head.toc.author().map(str::to_ascii_lowercase);
            for i in fam.members {
                let a = &mut addons[i];
                // Same author, or an anonymous module with no source of its own
                // (BigWigs_* ship no `## Author` and no ids at all).
                let same_author = a.toc.author().map(str::to_ascii_lowercase) == head_author
                    || (a.toc.author().is_none() && a.sources.is_empty());
                let overridden = manifest
                    .overrides
                    .get(&a.folder)
                    .is_some_and(|o| o.source.is_some());
                let already = a
                    .sources
                    .iter()
                    .any(|s| matches!(s, SourceRef::Github { .. } | SourceRef::Tukui { .. }));
                if same_author && !overridden && !already && !pinned.contains(&a.folder) {
                    a.sources.insert(0, head_src.clone());
                }
            }
        }
        Ok(Self {
            addons_dir: addons_dir.to_path_buf(),
            addons,
            client,
            skipped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_addon(root: &Path, folder: &str, toc: &str) {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{folder}.toc")), toc).unwrap();
    }

    #[test]
    fn counts_folders_that_are_not_addons() {
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "Details", "## Title: Details\n");
        // Elder Scrolls Online addons carry a .txt manifest, not a .toc.
        let eso = tmp.path().join("LibAddonMenu-2.0");
        std::fs::create_dir_all(&eso).unwrap();
        std::fs::write(eso.join("LibAddonMenu-2.0.txt"), "## Title: LibAddonMenu\n").unwrap();
        let lib = Library::load(tmp.path(), &Manifest::default()).unwrap();
        assert_eq!(lib.addons.len(), 1);
        assert_eq!(lib.skipped, 1);
    }

    #[test]
    fn companions_inherit_github_only_from_the_same_author() {
        let tmp = tempfile::tempdir().unwrap();
        write_addon(
            tmp.path(),
            "HandyNotes",
            "## Title: HandyNotes\n## Author: Nevcairiel\n## X-Website: https://github.com/Nevcairiel/HandyNotes\n",
        );
        write_addon(
            tmp.path(),
            "HandyNotes_Extras",
            "## Title: HandyNotes: Extras\n## Author: Nevcairiel\n## Dependencies: HandyNotes\n## X-Curse-Project-ID: 1\n",
        );
        write_addon(
            tmp.path(),
            "HandyNotes_Anon",
            "## Title: HandyNotes: Anonymous module\n## Dependencies: HandyNotes\n",
        );
        write_addon(
            tmp.path(),
            "HandyNotes_ThirdParty",
            "## Title: HandyNotes: Third Party\n## Author: Someone Else\n## Dependencies: HandyNotes\n## X-Curse-Project-ID: 2\n",
        );
        let lib = Library::load(tmp.path(), &Manifest::default()).unwrap();
        let by = |f: &str| lib.addons.iter().find(|a| a.folder == f).unwrap();
        assert!(matches!(
            by("HandyNotes_Extras").source(),
            Some(SourceRef::Github { .. })
        ));
        assert!(matches!(
            by("HandyNotes_ThirdParty").source(),
            Some(SourceRef::CurseForge { .. })
        ));
        assert!(matches!(
            by("HandyNotes_Anon").source(),
            Some(SourceRef::Github { .. })
        ));
    }

    #[test]
    fn install_log_overrides_a_stale_toc_version() {
        let tmp = tempfile::tempdir().unwrap();
        write_addon(
            tmp.path(),
            "MidnightRoutine",
            "## Title: Routine\n## Version: v12.0.372\n",
        );
        write_addon(tmp.path(), "Other", "## Title: Other\n## Version: 1.0\n");
        let mut log = install::Log::default();
        log.installs.insert(
            "MidnightRoutine".into(),
            install::Record {
                provider: "GitHub".into(),
                current: install::Generation {
                    version: "12.0.378".into(),
                    folders: vec!["MidnightRoutine".into()],
                    backup: None,
                },
                ..Default::default()
            },
        );
        // A record whose folders are gone must not touch anything.
        log.installs.insert(
            "Gone".into(),
            install::Record {
                current: install::Generation {
                    version: "9".into(),
                    folders: vec!["Other".into(), "Missing".into()],
                    backup: None,
                },
                ..Default::default()
            },
        );
        log.save(tmp.path()).unwrap();

        let lib = Library::load(tmp.path(), &Manifest::default()).unwrap();
        let by = |f: &str| lib.addons.iter().find(|a| a.folder == f).unwrap();
        assert_eq!(by("MidnightRoutine").installed, Some(Version::parse("12.0.378")));
        assert_eq!(by("Other").installed, Some(Version::parse("1.0")));
    }

    #[test]
    fn disabled_addons_are_listed_but_not_through_a_link() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        write_addon(&addons, "WA", "## Title: WA\n");
        write_addon(
            &addons.join(install::DISABLED_DIR),
            "Parked",
            "## Title: Parked\n",
        );
        let lib = Library::load_with_log(&addons, &Manifest::default(), &install::Log::default()).unwrap();
        assert!(lib.addons.iter().any(|a| a.folder == "Parked" && a.disabled));

        // The same folder reached through a junction/symlink is not scanned.
        let elsewhere = tmp.path().join("elsewhere");
        write_addon(&elsewhere, "Ghost", "## Title: Ghost\n");
        std::fs::remove_dir_all(addons.join(install::DISABLED_DIR)).unwrap();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&elsewhere, addons.join(install::DISABLED_DIR)).is_ok();
        #[cfg(windows)]
        let linked = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(addons.join(install::DISABLED_DIR))
            .arg(&elsewhere)
            .output()
            .is_ok_and(|o| o.status.success());
        #[cfg(not(any(unix, windows)))]
        let linked = false;
        if !linked {
            eprintln!("cannot create a directory link here; skipped");
            return;
        }
        let lib = Library::load(&addons, &Manifest::default()).unwrap();
        assert!(lib.addons.iter().all(|a| a.folder != "Ghost"));
        assert!(lib.addons.iter().any(|a| a.folder == "WA"));
    }

    #[test]
    fn scan_cleans_staging_and_survives_a_broken_log() {
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "WA", "## Title: WA\n## Version: 5.0\n");
        // Left by an install that crashed mid-unpack.
        write_addon(&tmp.path().join(install::STAGING_DIR), "Half", "## Title: Half\n");
        let log = install::Log::path_for(tmp.path());
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, "this is [[ not toml").unwrap();

        let scan = scan(tmp.path(), &Manifest::default()).unwrap();
        assert!(!tmp.path().join(install::STAGING_DIR).exists());
        assert!(scan.log_error.is_some());
        assert!(scan.rollbackable.is_empty() && scan.removed.is_empty());
        let folders: Vec<&str> = scan.library.addons.iter().map(|a| a.folder.as_str()).collect();
        assert_eq!(folders, ["WA"]);
    }

    #[test]
    fn scan_reads_rollback_and_removed_from_the_log() {
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "WA", "## Title: WA\n");
        let mut log = install::Log::default();
        log.installs.insert(
            "WA".into(),
            install::Record {
                current: install::Generation {
                    version: "2".into(),
                    folders: vec!["WA".into()],
                    backup: None,
                },
                previous: Some(install::Generation {
                    version: "1".into(),
                    folders: vec!["WA".into()],
                    backup: Some(".tocsin-backup/WA-1".into()),
                }),
                ..Default::default()
            },
        );
        log.installs.insert(
            "Gone".into(),
            install::Record {
                current: install::Generation {
                    version: "removed".into(),
                    folders: Vec::new(),
                    backup: None,
                },
                previous: Some(install::Generation {
                    version: "3.1".into(),
                    folders: vec!["Gone".into()],
                    backup: Some(".tocsin-backup/Gone-3.1".into()),
                }),
                ..Default::default()
            },
        );
        log.save(tmp.path()).unwrap();

        let scan = scan(tmp.path(), &Manifest::default()).unwrap();
        assert_eq!(scan.log_error, None);
        assert!(scan.rollbackable.contains("WA"));
        assert_eq!(scan.removed, [("Gone".to_string(), "3.1".to_string())]);
    }

    #[test]
    fn an_unknown_logged_version_defers_to_the_toc() {
        // A hand-installed DBM-Core was replaced from search, then rolled
        // back: the log knows the folders but not their version.
        let tmp = tempfile::tempdir().unwrap();
        write_addon(tmp.path(), "DBM-Core", "## Title: DBM\n## Version: 1.0\n");
        let mut log = install::Log::default();
        log.installs.insert(
            "DBM-Core".into(),
            install::Record {
                provider: "WoWInterface".into(),
                current: install::Generation {
                    version: install::UNKNOWN_VERSION.into(),
                    folders: vec!["DBM-Core".into()],
                    backup: None,
                },
                ..Default::default()
            },
        );
        let lib = Library::load_with_log(tmp.path(), &Manifest::default(), &log).unwrap();
        assert_eq!(lib.addons[0].installed, Some(Version::parse("1.0")));
    }

    #[test]
    fn the_source_installed_from_comes_first() {
        let tmp = tempfile::tempdir().unwrap();
        // The .toc prefers GitHub; the user installed from WoWInterface
        // through search.
        write_addon(
            tmp.path(),
            "Bagnon",
            "## Title: Bagnon\n## X-Website: https://github.com/Jaliborc/Bagnon\n## X-WoWI-ID: 4459\n",
        );
        write_addon(
            tmp.path(),
            "Bagnon_Config",
            "## Title: Bagnon Config\n## Dependencies: Bagnon\n",
        );
        let wowi = SourceRef::WowInterface { id: 4459 };
        let mut log = install::Log::default();
        log.installs.insert(
            "Bagnon".into(),
            install::Record {
                provider: "WoWInterface".into(),
                current: install::Generation {
                    version: "12.0.1".into(),
                    folders: vec!["Bagnon".into(), "Bagnon_Config".into()],
                    backup: None,
                },
                source: Some(wowi.clone()),
                ..Default::default()
            },
        );
        log.save(tmp.path()).unwrap();

        let lib = Library::load(tmp.path(), &Manifest::default()).unwrap();
        let by = |f: &str| lib.addons.iter().find(|a| a.folder == f).unwrap();
        assert_eq!(by("Bagnon").source(), Some(&wowi));
        assert_eq!(by("Bagnon").sources.len(), 2, "no duplicate WoWInterface entry");
        // The companion is pinned too, and not handed the head's GitHub repo.
        assert_eq!(by("Bagnon_Config").source(), Some(&wowi));
    }
}
