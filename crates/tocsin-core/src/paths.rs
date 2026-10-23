//! Where Tocsin keeps its per-user files (settings, download and icon
//! caches), and the one-time move of those files from where early builds put
//! them.
//!
//! On Windows the settings live in `%APPDATA%\tocsin\config` and the caches
//! in `%LOCALAPPDATA%\tocsin\cache`; on Linux in `~/.config/tocsin` and
//! `~/.cache/tocsin`; on macOS under `Library/Application Support/tocsin` and
//! `Library/Caches/tocsin`.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::install::is_real_dir;

/// The application's name in the per-user folders. No qualifier and no
/// organization: Tocsin is not published under anybody's name.
const APP: &str = "tocsin";

/// The identity early builds used (qualifier, organization, application),
/// which put everything under the developer's handle on Windows
/// (`%APPDATA%\<org>\tocsin`) and macOS (`dev.<org>.tocsin`). Kept only so
/// [`migrate_legacy`] can find those folders and move them; nothing is ever
/// written there.
const LEGACY_IDENTITY: (&str, &str, &str) = ("dev", "fakejack", "tocsin");

/// Tocsin's per-user directories; `None` when the OS gives no home folder.
pub fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", APP)
}

/// The directories of early builds (see [`LEGACY_IDENTITY`]).
fn legacy_project_dirs() -> Option<ProjectDirs> {
    let (qualifier, organization, application) = LEGACY_IDENTITY;
    ProjectDirs::from(qualifier, organization, application)
}

/// What [`migrate_root`] did with one folder.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Old and new location are the same (Linux ignores the organization).
    Same,
    /// Nothing at the old location.
    NoLegacy,
    /// The new location exists already: neither side is touched.
    NewExists,
    /// The old folder, or its parent, is a link (or not a folder): refused.
    Unsafe,
    /// Renamed.
    Moved,
    /// The rename failed (another volume, a file in use, permissions).
    Failed(String),
}

/// Move the per-user folders of early builds to where this build looks, once:
/// only when the old folder exists and the new one does not, by rename (both
/// sit under the same `AppData` root), never through a link. Afterwards the
/// old organization folder is removed when it is left empty. Each folder
/// fails on its own and only into the log: the app starts either way.
///
/// Blocking, a few metadata reads and renames: call it at start-up, before
/// the settings are read and before the window opens.
pub fn migrate_legacy() {
    let (Some(legacy), Some(new)) = (legacy_project_dirs(), project_dirs()) else {
        return;
    };
    let pairs = [
        ("settings", legacy.config_dir(), new.config_dir()),
        ("cache", legacy.cache_dir(), new.cache_dir()),
    ];
    for (what, old_dir, new_dir) in pairs {
        let roots = (
            project_root(old_dir, legacy.project_path()),
            project_root(new_dir, new.project_path()),
        );
        let (Some(old_root), Some(new_root)) = roots else {
            continue;
        };
        match migrate_root(&old_root, &new_root) {
            Step::Moved => {
                tracing::info!("moved the {what} of an earlier build to the new location");
                remove_empty_parent(&old_root, legacy.project_path());
            }
            Step::NewExists => {
                tracing::info!("{what} from an earlier build left alone: the new location exists")
            }
            Step::Unsafe => tracing::warn!("{what} from an earlier build not moved: it is a link"),
            Step::Failed(e) => tracing::warn!(error = %e, "could not move the {what} of an earlier build"),
            Step::Same | Step::NoLegacy => {}
        }
    }
}

/// The folder named after the project that contains `dir`: on Windows the
/// config dir is `%APPDATA%\<project>\config`, so the project folder is its
/// parent; elsewhere it is the dir itself.
fn project_root(dir: &Path, project_path: &Path) -> Option<PathBuf> {
    if project_path.as_os_str().is_empty() {
        return None;
    }
    dir.ancestors()
        .find(|a| a.ends_with(project_path))
        .map(Path::to_path_buf)
}

/// Rename `legacy` to `new` when that is safe: `legacy` and its parent are
/// real folders (a link would carry its target along, or move something that
/// is not Tocsin's), and nothing at all sits at `new` (not even a link).
fn migrate_root(legacy: &Path, new: &Path) -> Step {
    if legacy == new {
        return Step::Same;
    }
    match std::fs::symlink_metadata(legacy) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Step::NoLegacy,
        Err(e) => return Step::Failed(e.to_string()),
        Ok(_) => {}
    }
    let parent_ok = legacy.parent().is_some_and(is_real_dir);
    if !is_real_dir(legacy) || !parent_ok {
        return Step::Unsafe;
    }
    match std::fs::symlink_metadata(new) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // There, or unreadable: either way not ours to overwrite.
        _ => return Step::NewExists,
    }
    match std::fs::rename(legacy, new) {
        Ok(()) => Step::Moved,
        Err(e) => Step::Failed(e.to_string()),
    }
}

/// After a move, drop the organization folder that held `old_root`
/// (`%APPDATA%\<org>`) if nothing else is in it. Only when the project path
/// has an organization part: on macOS the parent is `Application Support`
/// itself. `remove_dir` refuses a folder that is not empty.
fn remove_empty_parent(old_root: &Path, project_path: &Path) {
    if project_path.components().count() < 2 {
        return;
    }
    let Some(parent) = old_root.parent() else {
        return;
    };
    if is_real_dir(parent) {
        if let Err(e) = std::fs::remove_dir(parent) {
            tracing::debug!(error = %e, "old organization folder kept");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory link at `link` to `target`: a symlink on Unix, a junction
    /// on Windows (no privilege needed). `false` when the platform refuses.
    fn dir_link(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(target, link).is_ok();
        #[cfg(windows)]
        let linked = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success());
        #[cfg(not(any(unix, windows)))]
        let linked = {
            let _ = (target, link);
            false
        };
        linked
    }

    /// `%APPDATA%`-shaped tree: `<root>/org/tocsin/config/tocsin.toml`.
    fn legacy_tree(root: &Path) -> PathBuf {
        let old = root.join("org").join("tocsin");
        std::fs::create_dir_all(old.join("config")).unwrap();
        std::fs::write(old.join("config").join("tocsin.toml"), "[settings]\n").unwrap();
        old
    }

    #[test]
    fn the_new_identity_has_no_organization() {
        if let Some(dirs) = project_dirs() {
            assert_eq!(dirs.project_path(), Path::new("tocsin"));
            assert!(dirs.config_dir().ancestors().any(|a| a.ends_with("tocsin")));
        }
    }

    #[test]
    fn the_project_root_is_the_folder_named_after_the_project() {
        let win = Path::new("C:/Users/a/AppData/Roaming/org/tocsin/config");
        assert_eq!(
            project_root(win, &Path::new("org").join("tocsin")),
            Some(PathBuf::from("C:/Users/a/AppData/Roaming/org/tocsin"))
        );
        let lin = Path::new("/home/a/.config/tocsin");
        assert_eq!(project_root(lin, Path::new("tocsin")), Some(lin.to_path_buf()));
        assert_eq!(project_root(lin, Path::new("other")), None);
        assert_eq!(project_root(lin, Path::new("")), None);
    }

    #[test]
    fn a_legacy_folder_moves_when_the_new_one_is_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let old = legacy_tree(tmp.path());
        let new = tmp.path().join("tocsin");
        assert_eq!(migrate_root(&old, &new), Step::Moved);
        assert!(!old.exists());
        let toml = std::fs::read_to_string(new.join("config").join("tocsin.toml")).unwrap();
        assert_eq!(toml, "[settings]\n");

        remove_empty_parent(&old, &Path::new("org").join("tocsin"));
        assert!(
            !tmp.path().join("org").exists(),
            "empty organization folder removed"
        );
    }

    #[test]
    fn nothing_moves_when_both_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let old = legacy_tree(tmp.path());
        let new = tmp.path().join("tocsin");
        std::fs::create_dir_all(new.join("config")).unwrap();
        std::fs::write(new.join("config").join("tocsin.toml"), "new\n").unwrap();
        assert_eq!(migrate_root(&old, &new), Step::NewExists);
        assert!(old.join("config").join("tocsin.toml").is_file());
        let toml = std::fs::read_to_string(new.join("config").join("tocsin.toml")).unwrap();
        assert_eq!(toml, "new\n");
    }

    #[test]
    fn nothing_to_do_without_a_legacy_folder_or_on_the_same_path() {
        let tmp = tempfile::tempdir().unwrap();
        let new = tmp.path().join("tocsin");
        assert_eq!(
            migrate_root(&tmp.path().join("org").join("tocsin"), &new),
            Step::NoLegacy
        );
        std::fs::create_dir_all(&new).unwrap();
        assert_eq!(migrate_root(&new, &new), Step::Same);
        assert!(new.is_dir());
    }

    #[test]
    fn a_non_empty_organization_folder_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let old = legacy_tree(tmp.path());
        std::fs::write(tmp.path().join("org").join("other.txt"), "x").unwrap();
        assert_eq!(migrate_root(&old, &tmp.path().join("tocsin")), Step::Moved);
        remove_empty_parent(&old, &Path::new("org").join("tocsin"));
        assert!(tmp.path().join("org").join("other.txt").is_file());

        // A one-part project path (macOS) never touches the parent.
        let solo = tmp.path().join("solo");
        std::fs::create_dir_all(solo.join("empty")).unwrap();
        remove_empty_parent(&solo.join("empty").join("tocsin"), Path::new("tocsin"));
        assert!(solo.join("empty").is_dir());
    }

    #[test]
    fn a_linked_legacy_folder_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(elsewhere.join("config")).unwrap();
        std::fs::create_dir_all(tmp.path().join("org")).unwrap();
        let old = tmp.path().join("org").join("tocsin");
        if !dir_link(&elsewhere, &old) {
            eprintln!("cannot create a directory link here; skipped");
            return;
        }
        let new = tmp.path().join("tocsin");
        assert_eq!(migrate_root(&old, &new), Step::Unsafe);
        assert!(!new.exists());
        assert!(elsewhere.join("config").is_dir());
    }

    #[test]
    fn a_legacy_folder_under_a_linked_parent_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(elsewhere.join("tocsin").join("config")).unwrap();
        let org = tmp.path().join("org");
        if !dir_link(&elsewhere, &org) {
            eprintln!("cannot create a directory link here; skipped");
            return;
        }
        let new = tmp.path().join("tocsin");
        assert_eq!(migrate_root(&org.join("tocsin"), &new), Step::Unsafe);
        assert!(!new.exists());
        assert!(elsewhere.join("tocsin").join("config").is_dir());
    }
}
