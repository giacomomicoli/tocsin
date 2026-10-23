//! Put a downloaded release into `AddOns`, keep the previous version, undo.
//!
//! See `docs/install-design.md`. In short: extract into
//! `AddOns/.tocsin-staging/`, then per folder rename the old one into
//! `AddOns/.tocsin-backup/<key>@<version>/` and the new one into place.
//! Renames on one volume are atomic and instant; nothing is ever copied.
//! Exactly one previous generation is kept per family; rolling back swaps the
//! two, so a rollback can itself be undone.
//!
//! Every path this module deletes or renames is `AddOns/<plain name>` or lies
//! under one of its dot-folders. The install log says which, and it sits in
//! the game folder where anything can edit it, so it is validated on load:
//! a record naming `..`, an absolute path or anything outside
//! `.tocsin-backup/` is dropped before any caller sees it. The dot-folders
//! themselves, and every folder about to be moved, must be real directories:
//! a symlink or a Windows junction there would carry renames and deletions
//! outside `AddOns`.

use std::collections::BTreeMap;
use std::fs::Metadata;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::addon::{Compat, Flavor};
use crate::archive::{self, ArchiveError};
use crate::sources::SourceRef;
use crate::toc::Toc;

/// Folder inside `AddOns` where zips are unpacked before the swap.
pub const STAGING_DIR: &str = ".tocsin-staging";
/// Folder inside `AddOns` holding the previous generation of each family.
pub const BACKUP_DIR: &str = ".tocsin-backup";

/// The version recorded for a copy Tocsin did not install and whose version
/// the caller did not know. Never a real version: readers fall back to the
/// `.toc` when they see it.
pub const UNKNOWN_VERSION: &str = "unknown";
/// Largest install log accepted. A record is a few hundred bytes; a log of
/// thousands of addons stays far below this.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
/// Longest piece of a backup or staging folder name: keeps paths well under
/// Windows' `MAX_PATH` when a provider reports a very long version string.
const MAX_NAME_PART: usize = 64;

/// Errors while installing or rolling back.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    /// Zip problems.
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    /// The zip has nothing for this game flavor.
    #[error("this package is not for {0}")]
    WrongFlavor(&'static str),
    /// No previous version recorded for the family.
    #[error("nothing to roll back to")]
    NoBackup,
    /// The swap failed and was undone.
    #[error("install failed and was undone: {0}")]
    Reverted(String),
    /// This exact zip is what is on disk already.
    #[error("{0} is already installed")]
    AlreadyInstalled(String),
    /// The install log could not be read or written.
    #[error("install log: {0}")]
    Log(String),
    /// A folder Tocsin would write into or move is a symlink, a junction or
    /// a file: following it could reach outside `AddOns`.
    #[error("{0} is a link or not a folder; Tocsin will not touch it")]
    UnsafeDir(String),
    /// Disk failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// One kept generation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Generation {
    /// Version string as the provider reported it.
    pub version: String,
    /// Top-level folders that belong to this generation.
    pub folders: Vec<String>,
    /// Where the folders are (relative to `AddOns`), when not in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
}

/// What Tocsin installed for one family.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Record {
    /// Provider name (`github`, `curseforge`, …).
    pub provider: String,
    /// Currently installed generation (its folders are in `AddOns`).
    pub current: Generation,
    /// The one kept before it, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Generation>,
    /// Unix seconds of the last install or rollback.
    pub installed_at: u64,
    /// Hex SHA-256 of the zip that produced `current`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
    /// The source the user installed from. An addon found by search may not
    /// advertise it in its `.toc` (a WoWInterface upload with no
    /// `X-WoWI-ID`), and the user's pick should keep serving its updates
    /// either way.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::sources::deserialize_lenient"
    )]
    pub source: Option<SourceRef>,
}

/// `installs.toml`: one [`Record`] per family head folder, per game folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Log {
    /// Records keyed by family head folder.
    #[serde(default)]
    pub installs: BTreeMap<String, Record>,
}

impl Log {
    /// The log for a game folder lives inside it, next to the backups, so it
    /// moves with the installation: `AddOns/.tocsin-backup/installs.toml`.
    pub fn path_for(addons_dir: &Path) -> PathBuf {
        addons_dir.join(BACKUP_DIR).join("installs.toml")
    }

    /// Load, or empty when missing. The file is untrusted: it is size-capped,
    /// never read through a link, and records whose paths could point
    /// outside `AddOns` are dropped (with a warning) so that no caller acts
    /// on them.
    pub fn load(addons_dir: &Path) -> Result<Self, InstallError> {
        own_dir(addons_dir, BACKUP_DIR)?;
        let Some(text) = read_log(&Self::path_for(addons_dir))? else {
            return Ok(Self::default());
        };
        let mut log: Self = toml::from_str(&text).map_err(|e| InstallError::Log(e.to_string()))?;
        log.drop_unsafe();
        Ok(log)
    }

    /// Forget records that name anything but plain folders and backups
    /// under `.tocsin-backup/`. Tocsin never writes such a record; one that
    /// says `backup = "C:\\Users\\x"` or `folders = ["..\\.."]` was edited by
    /// hand or by something else, and acting on it would delete or move
    /// folders outside `AddOns` (`Path::join` with an absolute path replaces
    /// the base).
    fn drop_unsafe(&mut self) {
        self.installs.retain(|key, r| {
            let ok = is_plain_name(key)
                && is_safe_generation(&r.current)
                && r.previous.as_ref().map_or(true, is_safe_generation);
            if !ok {
                tracing::warn!(record = ?key, "install log: ignoring a record with unsafe paths");
            }
            ok
        });
    }

    /// Key of the record whose current generation contains `folder`, or
    /// `folder` itself when it is a key. A record is one release zip: the
    /// folders it shipped. A family can span several (Routine and RoutineData
    /// are separate releases of one bundle), so the folder, not the family
    /// head, says which record an action belongs to.
    pub fn key_covering(&self, folder: &str) -> Option<&str> {
        if let Some((k, _)) = self.installs.get_key_value(folder) {
            return Some(k.as_str());
        }
        self.installs
            .iter()
            .find(|(_, r)| r.current.folders.iter().any(|f| f == folder))
            .map(|(k, _)| k.as_str())
    }

    /// Save atomically: write a fresh temporary file, then rename it over
    /// the log. The temporary is created new (a leftover is removed first),
    /// so a link planted under its name is never written through.
    pub fn save(&self, addons_dir: &Path) -> Result<(), InstallError> {
        let dir = own_dir(addons_dir, BACKUP_DIR)?;
        std::fs::create_dir_all(&dir)?;
        let path = Self::path_for(addons_dir);
        let text = toml::to_string_pretty(self).map_err(|e| InstallError::Log(e.to_string()))?;
        let tmp = path.with_extension("toml.tmp");
        if std::fs::symlink_metadata(&tmp).is_ok() {
            std::fs::remove_file(&tmp)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// The log's text, `None` when there is none. Refuses a link, a non-file and
/// anything over [`MAX_LOG_BYTES`] (checked on the bytes read too, not only
/// on the size the file system reports).
fn read_log(path: &Path) -> Result<Option<String>, InstallError> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !meta.is_file() {
        return Err(InstallError::Log("not a regular file".into()));
    }
    let too_large = || InstallError::Log("file is too large".into());
    if meta.len() > MAX_LOG_BYTES {
        return Err(too_large());
    }
    let mut raw = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_LOG_BYTES + 1)
        .read_to_end(&mut raw)?;
    if raw.len() as u64 > MAX_LOG_BYTES {
        return Err(too_large());
    }
    String::from_utf8(raw)
        .map(Some)
        .map_err(|_| InstallError::Log("not valid text".into()))
}

/// Folders are plain names, and the backup (if any) is one folder directly
/// under `.tocsin-backup/`, exactly as [`install`] and [`remove`] write it.
fn is_safe_generation(g: &Generation) -> bool {
    g.folders.iter().all(|f| is_plain_name(f)) && g.backup.as_deref().map_or(true, is_backup_rel)
}

/// `.tocsin-backup/<plain name>`. Trailing dots and spaces are tolerated on
/// the name: Tocsin before this version did not strip them, so an author's
/// `## Version: 1.0.` gave `WA@1.0.` (which Windows silently stores as
/// `WA@1.0`). Such a name cannot leave `.tocsin-backup` — `..` and friends
/// trim to nothing and are refused — and dropping it would lose Rollback.
fn is_backup_rel(b: &str) -> bool {
    b.strip_prefix(BACKUP_DIR)
        .and_then(|rest| rest.strip_prefix('/'))
        .is_some_and(|name| is_plain_name(name.trim_end_matches(['.', ' '])))
}

/// What to install.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    /// The folder the user acted on: the key in the log. For an addon not
    /// installed yet (search), its name: it then only picks which of the
    /// zip's folders becomes the key, see [`main_folder`].
    pub key: &'a str,
    /// Provider name for the record.
    pub provider: &'a str,
    /// Source to remember for future checks, when known.
    pub source: Option<&'a SourceRef>,
    /// Version being installed.
    pub version: &'a str,
    /// Hex SHA-256 of the zip.
    pub sha256: &'a str,
    /// Game flavor of the target folder.
    pub flavor: Flavor,
    /// Game build, for the compatibility check.
    pub client_interface: Option<u32>,
    /// Version of what is on disk now, when Tocsin did not install it (from
    /// the scan); names the backup folder instead of `unknown`.
    pub replacing_version: Option<&'a str>,
}

/// Result of an install.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// Folders now in `AddOns`.
    pub installed: Vec<String>,
    /// Folders moved to the backup (the previous generation plus any stale ones).
    pub replaced: Vec<String>,
}

/// Install `zip` into `addons_dir`. Everything is validated before the first
/// rename; if any rename fails the ones already done are undone.
pub fn install(addons_dir: &Path, zip: &Path, req: &Request<'_>) -> Result<Outcome, InstallError> {
    let manifest = archive::inspect(zip)?;
    // At least one folder must be for this flavor (or declare nothing).
    let fits = manifest.folders.values().any(|toc| {
        !matches!(
            Compat::of(&toc.interface_versions(), req.flavor, req.client_interface),
            Compat::OtherFlavor
        )
    });
    if !fits {
        return Err(InstallError::WrongFlavor(req.flavor.label()));
    }

    let new_folders: Vec<String> = manifest.folders.keys().cloned().collect();
    // `archive` already refuses these; the swap below joins them to
    // `AddOns`, so it checks again rather than rely on another module.
    if let Some(bad) = new_folders.iter().find(|f| !is_plain_name(f)) {
        return Err(ArchiveError::UnsafeEntry(bad.clone()).into());
    }
    let staging_root = own_dir(addons_dir, STAGING_DIR)?;
    let disabled_root = own_dir(addons_dir, DISABLED_DIR)?;
    let mut log = Log::load(addons_dir)?;
    // The record this zip updates: the one already covering any of its
    // folders (updating WeakAuras from the WeakAurasOptions row must not open
    // a second record), else a new one under the requested key when the zip
    // ships that folder, else under the zip's main folder.
    // A requested key that already names a *different* release (the family
    // head, when the GUI once passed it for a member's update) is not reused:
    // its folders would be evicted as stale.
    let key: String = match new_folders.iter().find_map(|f| log.key_covering(f)) {
        Some(k) => k.to_string(),
        None if new_folders.iter().any(|f| f == req.key)
            && log
                .installs
                .get(req.key)
                .map_or(true, |r| r.current.folders.is_empty()) =>
        {
            req.key.to_string()
        }
        None => main_folder(&manifest.folders, req.key).unwrap_or_else(|| req.key.to_string()),
    };
    let old = log.installs.get(&key).cloned();
    // Installing the very zip that is on disk would only throw away the
    // previous generation. Seen with Routine: the GitHub build of v12.0.378
    // says `## Version: v12.0.372` in its .toc, so without this guard every
    // "Update" replaced the backup with a copy of the current folder.
    if let Some(r) = &old {
        let same = r.current.version == req.version && !r.sha256.is_empty() && r.sha256 == req.sha256;
        let in_place =
            !r.current.folders.is_empty() && r.current.folders.iter().all(|f| addons_dir.join(f).is_dir());
        if same && in_place {
            return Err(InstallError::AlreadyInstalled(req.version.to_string()));
        }
    }

    // Everything that must leave AddOns: folders in the zip that exist, plus
    // folders of the previous generation the new zip no longer ships. Found
    // (and refused if one is a link) before anything is written.
    let mut to_backup = present(addons_dir, &new_folders)?;
    if let Some(r) = &old {
        for f in present(addons_dir, &r.current.folders)? {
            if !to_backup.contains(&f) {
                to_backup.push(f);
            }
        }
    }
    // Updating a disabled addon re-enables it: the parked copy comes back
    // first so it is backed up like any other previous version.
    let parked: Vec<String> = present(&disabled_root, &new_folders)?
        .into_iter()
        .filter(|f| !to_backup.contains(f))
        .collect();

    let staging = fresh_dir(&staging_root, &key)?;
    let extracted = archive::extract(zip, &staging);
    if let Err(e) = extracted {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e.into());
    }

    if !parked.is_empty() {
        match enable(addons_dir, &parked) {
            Ok(back) => to_backup.extend(back),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(e);
            }
        }
    }

    let old_version = old
        .as_ref()
        .map(|r| r.current.version.clone())
        .or_else(|| req.replacing_version.map(str::to_string))
        .unwrap_or_else(|| UNKNOWN_VERSION.into());
    let backup_rel = format!("{BACKUP_DIR}/{}@{}", safe_name(&key), safe_name(&old_version));
    let backup = addons_dir.join(&backup_rel);
    if !to_backup.is_empty() {
        if let Err(e) = reset_dir(&backup) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    }

    // Swap, with undo on failure.
    let mut moved_out: Vec<String> = Vec::new();
    let mut moved_in: Vec<String> = Vec::new();
    let result = (|| -> Result<(), std::io::Error> {
        for f in &to_backup {
            std::fs::rename(addons_dir.join(f), backup.join(f))?;
            moved_out.push(f.clone());
        }
        for f in &new_folders {
            std::fs::rename(staging.join(f), addons_dir.join(f))?;
            moved_in.push(f.clone());
        }
        Ok(())
    })();
    if let Err(e) = result {
        for f in moved_in.iter().rev() {
            let _ = std::fs::rename(addons_dir.join(f), staging.join(f));
        }
        for f in moved_out.iter().rev() {
            let _ = std::fs::rename(backup.join(f), addons_dir.join(f));
        }
        let _ = std::fs::remove_dir_all(&staging);
        // Only goes when empty, i.e. when everything really moved back.
        let _ = std::fs::remove_dir(&backup);
        return Err(InstallError::Reverted(e.to_string()));
    }
    let _ = std::fs::remove_dir_all(&staging);

    // Drop the generation before the one we just backed up: one is kept.
    if let Some(prev) = old.as_ref().and_then(|r| r.previous.as_ref()) {
        if let Some(b) = &prev.backup {
            if b != &backup_rel {
                let _ = std::fs::remove_dir_all(addons_dir.join(b));
            }
        }
    }

    let previous = (!to_backup.is_empty()).then(|| Generation {
        version: old_version,
        folders: to_backup.clone(),
        backup: Some(backup_rel),
    });
    log.installs.insert(
        key,
        Record {
            provider: req.provider.to_string(),
            current: Generation {
                version: req.version.to_string(),
                folders: new_folders.clone(),
                backup: None,
            },
            previous,
            installed_at: now(),
            sha256: req.sha256.to_string(),
            source: req.source.cloned().or_else(|| old.and_then(|r| r.source)),
        },
    );
    log.save(addons_dir)?;
    Ok(Outcome {
        installed: new_folders,
        replaced: to_backup,
    })
}

/// Swap the current generation covering `folder` with its backup.
pub fn rollback(addons_dir: &Path, folder: &str) -> Result<Outcome, InstallError> {
    let mut log = Log::load(addons_dir)?;
    let key = log
        .key_covering(folder)
        .ok_or(InstallError::NoBackup)?
        .to_string();
    let key = key.as_str();
    let record = log.installs.get(key).cloned().ok_or(InstallError::NoBackup)?;
    let prev = record.previous.clone().ok_or(InstallError::NoBackup)?;
    let backup_rel = prev.backup.clone().ok_or(InstallError::NoBackup)?;
    let backup = addons_dir.join(&backup_rel);
    if !movable(&backup)? {
        return Err(InstallError::NoBackup);
    }

    // Park the current folders under a new backup name, bring the old ones in.
    let parked_rel = format!(
        "{BACKUP_DIR}/{}@{}",
        safe_name(key),
        safe_name(&record.current.version)
    );
    let parked = addons_dir.join(&parked_rel);
    if parked == backup {
        return Err(InstallError::Reverted(
            "backup and current share a version".into(),
        ));
    }
    let going = present(addons_dir, &record.current.folders)?;
    let coming = present(&backup, &prev.folders)?;
    reset_dir(&parked)?;

    let mut moved_out = Vec::new();
    let mut moved_in = Vec::new();
    let result = (|| -> Result<(), std::io::Error> {
        for f in &going {
            std::fs::rename(addons_dir.join(f), parked.join(f))?;
            moved_out.push(f.clone());
        }
        for f in &coming {
            std::fs::rename(backup.join(f), addons_dir.join(f))?;
            moved_in.push(f.clone());
        }
        Ok(())
    })();
    if let Err(e) = result {
        for f in moved_in.iter().rev() {
            let _ = std::fs::rename(addons_dir.join(f), backup.join(f));
        }
        for f in moved_out.iter().rev() {
            let _ = std::fs::rename(parked.join(f), addons_dir.join(f));
        }
        let _ = std::fs::remove_dir_all(&parked);
        return Err(InstallError::Reverted(e.to_string()));
    }
    let _ = std::fs::remove_dir_all(&backup);

    log.installs.insert(
        key.to_string(),
        Record {
            provider: record.provider,
            current: Generation {
                version: prev.version,
                folders: moved_in,
                backup: None,
            },
            previous: Some(Generation {
                version: record.current.version,
                folders: moved_out.clone(),
                backup: Some(parked_rel),
            }),
            installed_at: now(),
            sha256: String::new(),
            source: record.source,
        },
    );
    log.save(addons_dir)?;
    Ok(Outcome {
        installed: prev.folders,
        replaced: moved_out,
    })
}

/// Remove leftovers of interrupted installs. Call at startup.
pub fn clean_staging(addons_dir: &Path) {
    let _ = std::fs::remove_dir_all(addons_dir.join(STAGING_DIR));
}

/// Delete every backup and forget previous generations.
pub fn clear_backups(addons_dir: &Path) -> Result<(), InstallError> {
    let mut log = Log::load(addons_dir)?;
    for r in log.installs.values_mut() {
        if let Some(b) = r.previous.take().and_then(|p| p.backup) {
            let _ = std::fs::remove_dir_all(addons_dir.join(b));
        }
    }
    log.save(addons_dir)
}

fn fresh_dir(parent: &Path, key: &str) -> Result<PathBuf, InstallError> {
    let dir = parent.join(format!("{}-{}", safe_name(key), now()));
    reset_dir(&dir)?;
    Ok(dir)
}

/// Make `dir` an empty real directory. Whatever is there goes first, a link
/// included: `remove_dir_all` removes a symlink or junction itself, never
/// what it points to. The parent must already be checked with [`own_dir`].
fn reset_dir(dir: &Path) -> Result<(), InstallError> {
    if std::fs::symlink_metadata(dir).is_ok() {
        std::fs::remove_dir_all(dir)?;
    }
    std::fs::create_dir_all(dir)?;
    Ok(())
}

/// `addons_dir/<name>` for one of Tocsin's dot-folders, refused when it is a
/// symlink, a junction or a file: everything written there would land
/// wherever it points. Missing is fine; callers create it.
fn own_dir(addons_dir: &Path, name: &str) -> Result<PathBuf, InstallError> {
    let dir = addons_dir.join(name);
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if !is_real_dir_meta(&m) => Err(InstallError::UnsafeDir(name.to_string())),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(dir),
    }
}

/// Whether `path` is a directory itself, not a symlink or junction to one.
/// Never follows a link.
pub fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| is_real_dir_meta(&m))
}

fn is_real_dir_meta(m: &Metadata) -> bool {
    // On Windows `is_symlink` covers junctions too (name-surrogate reparse
    // points), while cloud placeholders (OneDrive) stay plain directories.
    m.is_dir() && !m.file_type().is_symlink()
}

/// Whether `path` is a folder to move: `false` when missing or a plain file
/// (no addon is one), an error when it is a link. Renaming a link would
/// carry it into the backup, where a later rollback or cleanup would act
/// through it; an addon developer's symlinked checkout is theirs to manage.
fn movable(path: &Path) -> Result<bool, InstallError> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(InstallError::UnsafeDir(
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        )),
        Ok(m) => Ok(m.is_dir()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Those of `folders` that are plain names and real folders under `root`;
/// an error if any of them is a link.
fn present(root: &Path, folders: &[String]) -> Result<Vec<String>, InstallError> {
    let mut found = Vec::new();
    for f in folders {
        if is_plain_name(f) && movable(&root.join(f))? && !found.contains(f) {
            found.push(f.clone());
        }
    }
    Ok(found)
}

/// Which of a zip's folders names its record when the request names none of
/// them, as for an addon installed from search. The folder named like the
/// addon wins (`ElvUI` beside `ElvUI_Libraries` and `ElvUI_Options`); else the
/// one the others depend on (*Deadly Boss Mods* is `DBM-Core` and its
/// modules); else the shortest name.
pub fn main_folder(folders: &BTreeMap<String, Toc>, name: &str) -> Option<String> {
    let simple = |s: &str| crate::search::simplify(s);
    let wanted = simple(name);
    if let Some(f) = folders.keys().find(|f| !wanted.is_empty() && simple(f) == wanted) {
        return Some(f.clone());
    }
    let dependents = |folder: &str| {
        folders
            .values()
            .filter(|toc| toc.dependencies().iter().any(|d| d.eq_ignore_ascii_case(folder)))
            .count()
    };
    folders
        .keys()
        .min_by(|a, b| {
            dependents(b)
                .cmp(&dependents(a))
                .then(a.len().cmp(&b.len()))
                .then(a.cmp(b))
        })
        .cloned()
}

/// Keep only characters safe in a folder name, at most [`MAX_NAME_PART`] of
/// them, and never a leading or trailing dot: the log validator refuses a
/// backup folder starting with one, and Windows drops a trailing one.
fn safe_name(s: &str) -> String {
    let mut cleaned: String = s
        .chars()
        .take(MAX_NAME_PART)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.starts_with('.') {
        cleaned.replace_range(..1, "_");
    }
    if cleaned.ends_with('.') {
        cleaned.pop();
        cleaned.push('_');
    }
    if cleaned.is_empty() {
        "x".into()
    } else {
        cleaned
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn make_zip(dir: &Path, name: &str, entries: &[(&str, &str)]) -> PathBuf {
        let path = dir.join(name);
        let mut w = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (n, body) in entries {
            w.start_file(*n, SimpleFileOptions::default()).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap();
        path
    }

    fn req<'a>(key: &'a str, version: &'a str) -> Request<'a> {
        Request {
            key,
            provider: "github",
            source: None,
            version,
            sha256: "abc",
            flavor: Flavor::Retail,
            client_interface: Some(120105),
            replacing_version: None,
        }
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap_or_default()
    }

    #[test]
    fn install_update_rollback_roll_forward() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        std::fs::create_dir(&addons).unwrap();
        // A hand-installed older version with a module the new release drops.
        std::fs::create_dir_all(addons.join("WA/x")).unwrap();
        std::fs::write(addons.join("WA/WA.toc"), "## Version: 1\n").unwrap();
        std::fs::write(addons.join("WA_Old/WA_Old.toc"), "## Version: 1\n").ok();
        std::fs::create_dir_all(addons.join("WA_Old")).unwrap();
        std::fs::write(addons.join("WA_Old/WA_Old.toc"), "## Version: 1\n").unwrap();

        let z1 = make_zip(
            tmp.path(),
            "v1.zip",
            &[
                ("WA/WA.toc", "## Interface: 120105\n## Version: 1\n"),
                ("WA_Old/WA_Old.toc", "## Version: 1\n"),
            ],
        );
        let o = install(&addons, &z1, &req("WA", "1")).unwrap();
        assert_eq!(o.installed, vec!["WA", "WA_Old"]);
        // The hand-installed folders went to backup under "unknown".
        assert!(addons.join(".tocsin-backup/WA@unknown/WA/x").is_dir());

        let z2 = make_zip(
            tmp.path(),
            "v2.zip",
            &[
                ("WA/WA.toc", "## Interface: 120105\n## Version: 2\n"),
                ("WA_Opt/WA_Opt.toc", "## Version: 2\n"),
            ],
        );
        let o = install(&addons, &z2, &req("WA", "2")).unwrap();
        assert_eq!(o.installed, vec!["WA", "WA_Opt"]);
        // Stale module removed from AddOns, kept in the backup of v1.
        assert!(!addons.join("WA_Old").exists());
        assert!(addons.join(".tocsin-backup/WA@1/WA_Old").is_dir());
        // Only one generation kept: the "unknown" backup is gone.
        assert!(!addons.join(".tocsin-backup/WA@unknown").exists());
        assert!(read(&addons.join("WA/WA.toc")).contains("Version: 2"));
        assert!(!addons.join(STAGING_DIR).join("WA-0").exists());

        let o = rollback(&addons, "WA").unwrap();
        assert_eq!(o.installed, vec!["WA", "WA_Old"]);
        assert!(read(&addons.join("WA/WA.toc")).contains("Version: 1"));
        assert!(addons.join("WA_Old").is_dir());
        assert!(!addons.join("WA_Opt").exists());
        let log = Log::load(&addons).unwrap();
        assert_eq!(log.installs["WA"].current.version, "1");
        assert_eq!(log.installs["WA"].previous.as_ref().unwrap().version, "2");

        // Rolling back again goes forward to v2.
        rollback(&addons, "WA").unwrap();
        assert!(read(&addons.join("WA/WA.toc")).contains("Version: 2"));
        assert!(addons.join("WA_Opt").is_dir());
    }

    #[test]
    fn reinstalling_the_same_zip_keeps_the_previous_generation() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        std::fs::create_dir(&addons).unwrap();
        let z1 = make_zip(
            tmp.path(),
            "v1.zip",
            &[("R/R.toc", "## Interface: 120105\n## Version: 1\n")],
        );
        install(&addons, &z1, &req("R", "1")).unwrap();
        // The author forgot to bump `## Version` in the v2 package.
        let z2 = make_zip(
            tmp.path(),
            "v2.zip",
            &[("R/R.toc", "## Interface: 120105\n## Version: 1\n")],
        );
        install(&addons, &z2, &req("R", "2")).unwrap();
        assert!(addons.join(".tocsin-backup/R@1").is_dir());

        let err = install(&addons, &z2, &req("R", "2")).unwrap_err();
        assert!(matches!(err, InstallError::AlreadyInstalled(v) if v == "2"));
        // Nothing moved: v1 is still the generation to roll back to.
        assert!(addons.join(".tocsin-backup/R@1").is_dir());
        assert!(!addons.join(".tocsin-backup/R@2").exists());
        assert_eq!(
            Log::load(&addons).unwrap().installs["R"]
                .previous
                .as_ref()
                .unwrap()
                .version,
            "1"
        );
    }

    #[test]
    fn companion_release_does_not_evict_its_family_head() {
        // Routine and RoutineData: one family, two release zips.
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        std::fs::create_dir(&addons).unwrap();
        let routine = make_zip(
            tmp.path(),
            "routine.zip",
            &[("MidnightRoutine/MidnightRoutine.toc", "## Interface: 120105\n")],
        );
        install(&addons, &routine, &req("MidnightRoutine", "12.0.378")).unwrap();
        let data = make_zip(
            tmp.path(),
            "data.zip",
            &[("RoutineData/RoutineData.toc", "## Interface: 120105\n")],
        );
        // The GUI used to pass the family head as key for a member's update.
        let o = install(&addons, &data, &req("MidnightRoutine", "12.0.14")).unwrap();
        assert_eq!(o.installed, vec!["RoutineData"]);
        assert!(o.replaced.is_empty());
        assert!(addons.join("MidnightRoutine").is_dir());
        assert!(addons.join("RoutineData").is_dir());
        let log = Log::load(&addons).unwrap();
        assert_eq!(log.installs["MidnightRoutine"].current.version, "12.0.378");
        assert_eq!(log.installs["RoutineData"].current.version, "12.0.14");

        // Updating the same release from any of its folders reuses its record.
        let data2 = make_zip(
            tmp.path(),
            "data2.zip",
            &[(
                "RoutineData/RoutineData.toc",
                "## Interface: 120105\n## Version: 15\n",
            )],
        );
        install(&addons, &data2, &req("MidnightRoutine", "12.0.15")).unwrap();
        let log = Log::load(&addons).unwrap();
        assert_eq!(log.installs.len(), 2);
        assert_eq!(log.installs["RoutineData"].current.version, "12.0.15");
        assert_eq!(
            log.installs["RoutineData"].previous.as_ref().unwrap().version,
            "12.0.14"
        );
        assert!(addons.join("MidnightRoutine").is_dir());

        // Rollback by any folder of the record.
        rollback(&addons, "RoutineData").unwrap();
        assert!(!read(&addons.join("RoutineData/RoutineData.toc")).contains("Version: 15"));
        assert!(addons.join("MidnightRoutine").is_dir());
        assert_eq!(
            Log::load(&addons).unwrap().installs["RoutineData"]
                .current
                .version,
            "12.0.14"
        );
    }

    #[test]
    fn fresh_install_from_search_keys_the_main_folder_and_keeps_the_source() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        std::fs::create_dir(&addons).unwrap();
        let z = make_zip(
            tmp.path(),
            "dbm.zip",
            &[
                ("DBM-Core/DBM-Core.toc", "## Interface: 120105\n"),
                (
                    "DBM-Challenges/DBM-Challenges.toc",
                    "## Interface: 120105\n## Dependencies: DBM-Core\n",
                ),
                (
                    "DBM-StatusBarTimers/DBM-StatusBarTimers.toc",
                    "## Interface: 120105\n## RequiredDeps: DBM-Core\n",
                ),
            ],
        );
        let source = SourceRef::WowInterface { id: 8814 };
        let r = Request {
            source: Some(&source),
            ..req("Deadly Boss Mods (DBM)", "12.0.1")
        };
        install(&addons, &z, &r).unwrap();
        let log = Log::load(&addons).unwrap();
        assert_eq!(log.installs.keys().collect::<Vec<_>>(), ["DBM-Core"]);
        assert_eq!(log.installs["DBM-Core"].source.as_ref(), Some(&source));

        // A later update that does not know the source keeps the recorded one.
        let z2 = make_zip(
            tmp.path(),
            "dbm2.zip",
            &[("DBM-Core/DBM-Core.toc", "## Interface: 120105\n## Version: 2\n")],
        );
        install(&addons, &z2, &req("DBM-Core", "12.0.2")).unwrap();
        assert_eq!(
            Log::load(&addons).unwrap().installs["DBM-Core"].source.as_ref(),
            Some(&source)
        );
    }

    #[test]
    fn main_folder_prefers_the_name_then_dependents() {
        let folders = |list: &[(&str, &str)]| -> BTreeMap<String, Toc> {
            list.iter()
                .map(|(f, toc)| (f.to_string(), Toc::parse(toc)))
                .collect()
        };
        let elvui = folders(&[
            ("ElvUI", "## Dependencies: ElvUI_Libraries\n"),
            ("ElvUI_Libraries", ""),
            ("ElvUI_Options", "## Dependencies: ElvUI, ElvUI_Libraries\n"),
        ]);
        assert_eq!(main_folder(&elvui, "ElvUI").as_deref(), Some("ElvUI"));
        // No name match: the most depended-upon folder.
        assert_eq!(main_folder(&elvui, "").as_deref(), Some("ElvUI_Libraries"));
        let plain = folders(&[("Bagnon_Config", ""), ("Bagnon", "")]);
        assert_eq!(main_folder(&plain, "Bagnon Bags").as_deref(), Some("Bagnon"));
        assert_eq!(main_folder(&BTreeMap::new(), "x"), None);
    }

    #[test]
    fn refuses_wrong_flavor_and_leaves_nothing_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        std::fs::create_dir(&addons).unwrap();
        let z = make_zip(
            tmp.path(),
            "cata.zip",
            &[("WA/WA_Cata.toc", "## Interface: 40402\n")],
        );
        let err = install(&addons, &z, &req("WA", "1")).unwrap_err();
        assert!(matches!(err, InstallError::WrongFlavor("Retail")));
        assert!(!addons.join("WA").exists());
        assert!(!addons.join(STAGING_DIR).exists());
    }

    #[test]
    fn rollback_without_backup_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(matches!(
            rollback(tmp.path(), "Nope"),
            Err(InstallError::NoBackup)
        ));
    }

    /// `AddOns` plus a sibling `outside` folder holding a file that no
    /// operation may move or delete.
    fn game_with_outside() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(addons.join(BACKUP_DIR)).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), "precious").unwrap();
        (tmp, addons, outside)
    }

    fn write_log(addons: &Path, text: &str) {
        std::fs::write(Log::path_for(addons), text).unwrap();
    }

    #[test]
    fn backups_named_by_older_versions_are_kept() {
        // An author's `## Version: 1.0.` gave `WA@1.0.` before names were
        // trimmed; the record must survive the validator.
        assert!(is_backup_rel(".tocsin-backup/WA@1.0."));
        assert!(is_backup_rel(".tocsin-backup/WA@1.0 "));
        assert!(!is_backup_rel(".tocsin-backup/..."));
        assert!(!is_backup_rel(".tocsin-backup/.."));
        assert!(!is_backup_rel(".tocsin-backup/a/b"));
    }

    #[test]
    fn tampered_log_records_are_dropped_and_nothing_outside_is_touched() {
        let (_tmp, addons, outside) = game_with_outside();
        std::fs::create_dir_all(addons.join("Good")).unwrap();
        std::fs::create_dir_all(addons.join("Abs")).unwrap();
        let abs = outside.display();
        // Literal TOML strings: no escaping of the Windows backslashes.
        write_log(
            &addons,
            &format!(
                r#"
[installs.Good]
provider = "github"
installed_at = 1
current = {{ version = "1", folders = ["Good"] }}

[installs.Abs]
provider = "github"
installed_at = 1
current = {{ version = "2", folders = ["Abs"] }}
previous = {{ version = "1", folders = ["keep.txt"], backup = '{abs}' }}

[installs.Trav]
provider = "github"
installed_at = 1
current = {{ version = "removed", folders = [] }}
previous = {{ version = "1", folders = ['..\..\outside'], backup = ".tocsin-backup/Trav@1" }}

[installs.Up]
provider = "github"
installed_at = 1
current = {{ version = "removed", folders = [] }}
previous = {{ version = "1", folders = ["x"], backup = ".tocsin-backup/../../outside" }}

[installs."..\\.."]
provider = "github"
installed_at = 1
current = {{ version = "1", folders = ["Good"] }}

[installs.Stream]
provider = "github"
installed_at = 1
current = {{ version = "1", folders = ["C:evil"] }}
"#
            ),
        );
        let log = Log::load(&addons).unwrap();
        assert_eq!(log.installs.keys().collect::<Vec<_>>(), ["Good"]);
        assert!(removed_in(&log).is_empty());

        for key in ["Abs", "Trav", "Up"] {
            assert!(matches!(rollback(&addons, key), Err(InstallError::NoBackup)));
        }
        remove(&addons, "Abs", &["Abs".into()], None).unwrap();
        clear_backups(&addons).unwrap();
        assert_eq!(read(&outside.join("keep.txt")), "precious");
        assert!(addons.join("Good").is_dir());
    }

    #[test]
    fn oversized_log_is_refused() {
        let (_tmp, addons, outside) = game_with_outside();
        let filler = "# padding\n".repeat((MAX_LOG_BYTES as usize / 10) + 1);
        write_log(&addons, &filler);
        assert!(matches!(Log::load(&addons), Err(InstallError::Log(_))));
        assert!(removed(&addons).is_empty());
        assert!(matches!(clear_backups(&addons), Err(InstallError::Log(_))));
        assert_eq!(read(&outside.join("keep.txt")), "precious");
    }

    #[test]
    fn backup_names_stay_short_and_valid() {
        let long = "v".repeat(500);
        assert_eq!(safe_name(&long).len(), MAX_NAME_PART);
        assert_eq!(safe_name(".hidden"), "_hidden");
        assert_eq!(safe_name("1.0."), "1.0_");
        assert_eq!(safe_name(""), "x");
        assert!(is_backup_rel(&format!(
            "{BACKUP_DIR}/{}@{}",
            safe_name("..\\.."),
            safe_name("..")
        )));
    }

    #[test]
    fn dot_folder_that_is_a_file_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        std::fs::create_dir_all(addons.join("WA")).unwrap();
        std::fs::write(addons.join(STAGING_DIR), "not a folder").unwrap();
        std::fs::write(addons.join(DISABLED_DIR), "not a folder").unwrap();
        let z = make_zip(tmp.path(), "wa.zip", &[("WA/WA.toc", "## Interface: 120105\n")]);
        assert!(matches!(
            install(&addons, &z, &req("WA", "1")),
            Err(InstallError::UnsafeDir(d)) if d == STAGING_DIR
        ));
        assert!(matches!(
            disable(&addons, &["WA".into()]),
            Err(InstallError::UnsafeDir(d)) if d == DISABLED_DIR
        ));
        assert!(addons.join("WA").is_dir());
    }

    /// A directory link: a symlink on Unix, a junction on Windows (which,
    /// unlike a symlink, needs no privilege, so it is what an attacker or a
    /// sync tool would leave). `false` when the platform refuses.
    fn link_dir(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(link)
                .arg(target)
                .output()
                .is_ok_and(|o| o.status.success())
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link);
            false
        }
    }

    fn outside_is_untouched(outside: &Path) {
        let names: Vec<_> = std::fs::read_dir(outside)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["keep.txt"], "something was written outside AddOns");
        assert_eq!(read(&outside.join("keep.txt")), "precious");
    }

    #[test]
    fn linked_dot_folders_are_refused() {
        for dot in [BACKUP_DIR, STAGING_DIR, DISABLED_DIR] {
            let (tmp, addons, outside) = game_with_outside();
            std::fs::remove_dir(addons.join(BACKUP_DIR)).unwrap();
            if !link_dir(&outside, &addons.join(dot)) {
                eprintln!("cannot create a directory link here; skipped");
                return;
            }
            std::fs::create_dir_all(addons.join("WA")).unwrap();
            std::fs::write(addons.join("WA/WA.toc"), "## Version: 0\n").unwrap();
            let z = make_zip(tmp.path(), "wa.zip", &[("WA/WA.toc", "## Interface: 120105\n")]);
            let err = install(&addons, &z, &req("WA", "1")).unwrap_err();
            assert!(
                matches!(&err, InstallError::UnsafeDir(d) if d == dot),
                "{dot}: {err:?}"
            );
            if dot == DISABLED_DIR {
                assert!(matches!(
                    disable(&addons, &["WA".into()]),
                    Err(InstallError::UnsafeDir(_))
                ));
                assert!(matches!(
                    remove(&addons, "WA", &["WA".into()], None),
                    Err(InstallError::UnsafeDir(_))
                ));
            }
            if dot == BACKUP_DIR {
                assert!(matches!(Log::load(&addons), Err(InstallError::UnsafeDir(_))));
                assert!(Log::default().save(&addons).is_err());
            }
            assert!(read(&addons.join("WA/WA.toc")).contains("Version: 0"));
            outside_is_untouched(&outside);
        }
    }

    #[test]
    fn linked_addon_folders_are_not_moved() {
        let (tmp, addons, outside) = game_with_outside();
        // An addon developer's checkout, linked into AddOns.
        if !link_dir(&outside, &addons.join("Dev")) {
            eprintln!("cannot create a directory link here; skipped");
            return;
        }
        assert!(!is_real_dir(&addons.join("Dev")));
        assert!(matches!(
            disable(&addons, &["Dev".into()]),
            Err(InstallError::UnsafeDir(d)) if d == "Dev"
        ));
        assert!(matches!(
            remove(&addons, "Dev", &["Dev".into()], None),
            Err(InstallError::UnsafeDir(_))
        ));
        let z = make_zip(
            tmp.path(),
            "dev.zip",
            &[("Dev/Dev.toc", "## Interface: 120105\n")],
        );
        assert!(matches!(
            install(&addons, &z, &req("Dev", "1")),
            Err(InstallError::UnsafeDir(_))
        ));
        assert!(addons.join("Dev/keep.txt").is_file(), "the link stays in place");
        outside_is_untouched(&outside);
    }
}

// ---------------------------------------------------------------- enable/disable/remove

/// Folder inside `AddOns` holding disabled addons. The game never looks in
/// dot-folders, so a disabled addon is simply out of its sight; its
/// `SavedVariables` in `WTF` are untouched and come back on enable.
pub const DISABLED_DIR: &str = ".tocsin-disabled";

/// Move `folders` out of the game's sight. Already-disabled ones are skipped.
pub fn disable(addons_dir: &Path, folders: &[String]) -> Result<Vec<String>, InstallError> {
    move_between(addons_dir, folders, "", DISABLED_DIR)
}

/// Bring `folders` back. Already-enabled ones are skipped.
pub fn enable(addons_dir: &Path, folders: &[String]) -> Result<Vec<String>, InstallError> {
    move_between(addons_dir, folders, DISABLED_DIR, "")
}

/// Rename each folder from `addons_dir/from/<f>` to `addons_dir/to/<f>`,
/// undoing on failure. Returns the folders actually moved.
fn move_between(
    addons_dir: &Path,
    folders: &[String],
    from: &str,
    to: &str,
) -> Result<Vec<String>, InstallError> {
    let root = |name: &str| {
        if name.is_empty() {
            Ok(addons_dir.to_path_buf())
        } else {
            own_dir(addons_dir, name)
        }
    };
    let src_root = root(from)?;
    let dst_root = root(to)?;
    let candidates: Vec<String> = present(&src_root, folders)?
        .into_iter()
        .filter(|f| std::fs::symlink_metadata(dst_root.join(f)).is_err())
        .collect();
    std::fs::create_dir_all(&dst_root)?;
    let mut moved = Vec::new();
    let result = (|| -> Result<(), std::io::Error> {
        for f in &candidates {
            std::fs::rename(src_root.join(f), dst_root.join(f))?;
            moved.push(f.clone());
        }
        Ok(())
    })();
    if let Err(e) = result {
        for f in moved.iter().rev() {
            let _ = std::fs::rename(dst_root.join(f), src_root.join(f));
        }
        return Err(InstallError::Reverted(e.to_string()));
    }
    Ok(moved)
}

/// Remove a family: its folders become the previous generation of `key`, so
/// [`rollback`] brings them back. `SavedVariables` are left alone.
pub fn remove(
    addons_dir: &Path,
    key: &str,
    folders: &[String],
    version: Option<&str>,
) -> Result<Outcome, InstallError> {
    let mut log = Log::load(addons_dir)?;
    let disabled_root = own_dir(addons_dir, DISABLED_DIR)?;
    let old = log.installs.get(key).cloned();
    let old_version = old
        .as_ref()
        .map(|r| r.current.version.clone())
        .or_else(|| version.map(str::to_string))
        .unwrap_or_else(|| UNKNOWN_VERSION.into());
    let backup_rel = format!("{BACKUP_DIR}/{}@{}", safe_name(key), safe_name(&old_version));
    let backup = addons_dir.join(&backup_rel);

    // Enabled and disabled folders alike go to the backup. Each is found
    // (and a link refused) before anything moves.
    let mut sources: Vec<(String, PathBuf)> = Vec::new();
    for f in folders {
        if !is_plain_name(f) || sources.iter().any(|(s, _)| s == f) {
            continue;
        }
        for root in [addons_dir, disabled_root.as_path()] {
            let src = root.join(f);
            if movable(&src)? {
                sources.push((f.clone(), src));
                break;
            }
        }
    }
    reset_dir(&backup)?;
    let mut moved: Vec<(String, PathBuf)> = Vec::new();
    let result = (|| -> Result<(), std::io::Error> {
        for (f, src) in &sources {
            std::fs::rename(src, backup.join(f))?;
            moved.push((f.clone(), src.clone()));
        }
        Ok(())
    })();
    if let Err(e) = result {
        for (f, src) in moved.iter().rev() {
            let _ = std::fs::rename(backup.join(f), src);
        }
        let _ = std::fs::remove_dir_all(&backup);
        return Err(InstallError::Reverted(e.to_string()));
    }
    if let Some(prev) = old.as_ref().and_then(|r| r.previous.as_ref()) {
        if let Some(b) = &prev.backup {
            if b != &backup_rel {
                let _ = std::fs::remove_dir_all(addons_dir.join(b));
            }
        }
    }
    let replaced: Vec<String> = moved.into_iter().map(|(f, _)| f).collect();
    // Companion releases (RoutineData under Routine) leave with the family;
    // their own records and backups would point at nothing, so they go too.
    let orphaned: Vec<String> = log
        .installs
        .iter()
        .filter(|(k, r)| k.as_str() != key && r.current.folders.iter().any(|f| replaced.contains(f)))
        .map(|(k, _)| k.clone())
        .collect();
    for k in orphaned {
        if let Some(b) = log
            .installs
            .remove(&k)
            .and_then(|r| r.previous)
            .and_then(|p| p.backup)
        {
            let _ = std::fs::remove_dir_all(addons_dir.join(b));
        }
    }
    log.installs.insert(
        key.to_string(),
        Record {
            provider: old
                .as_ref()
                .map(|r| r.provider.clone())
                .unwrap_or_else(|| "local".into()),
            current: Generation {
                version: "removed".into(),
                folders: Vec::new(),
                backup: None,
            },
            previous: Some(Generation {
                version: old_version,
                folders: replaced.clone(),
                backup: Some(backup_rel),
            }),
            installed_at: now(),
            sha256: String::new(),
            source: old.and_then(|r| r.source),
        },
    );
    log.save(addons_dir)?;
    Ok(Outcome {
        installed: Vec::new(),
        replaced,
    })
}

/// Families removed by Tocsin that can still be restored: key → version.
/// Loads the log; use [`removed_in`] when it is already loaded.
pub fn removed(addons_dir: &Path) -> Vec<(String, String)> {
    Log::load(addons_dir)
        .map(|log| removed_in(&log))
        .unwrap_or_default()
}

/// Families removed by Tocsin that can still be restored, from a loaded log:
/// key → version.
pub fn removed_in(log: &Log) -> Vec<(String, String)> {
    log.installs
        .iter()
        .filter(|(_, r)| r.current.version == "removed" && r.current.folders.is_empty())
        .filter_map(|(k, r)| r.previous.as_ref().map(|p| (k.clone(), p.version.clone())))
        .collect()
}

/// Folders that can roll back, from a loaded log: every folder of a release
/// with a kept generation, not only the record's key, so a companion's row
/// (RoutineData) offers it as well as its head's.
pub fn rollbackable_in(log: &Log) -> std::collections::BTreeSet<String> {
    log.installs
        .iter()
        .filter(|(_, r)| r.previous.as_ref().is_some_and(|p| p.backup.is_some()))
        .flat_map(|(k, r)| std::iter::once(k.clone()).chain(r.current.folders.iter().cloned()))
        .collect()
}

/// One folder name directly inside `AddOns` (or a dot-folder): no
/// separators, no `..`, no drive or stream (`:`), nothing Windows would
/// mangle (`NUL`, a trailing dot), and not a dot-name, which is Tocsin's own.
fn is_plain_name(f: &str) -> bool {
    !f.is_empty() && !f.starts_with('.') && archive::is_portable_component(f)
}

#[cfg(test)]
mod toggle_tests {
    use super::*;

    fn addon(root: &Path, folder: &str) {
        std::fs::create_dir_all(root.join(folder)).unwrap();
        std::fs::write(root.join(folder).join(format!("{folder}.toc")), "## Version: 1\n").unwrap();
    }

    #[test]
    fn disable_enable_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        addon(&addons, "WA");
        addon(&addons, "WA_Opt");
        let f = vec![
            "WA".to_string(),
            "WA_Opt".to_string(),
            "Nope".to_string(),
            "../x".to_string(),
        ];
        assert_eq!(disable(&addons, &f).unwrap(), vec!["WA", "WA_Opt"]);
        assert!(!addons.join("WA").exists());
        assert!(addons.join(DISABLED_DIR).join("WA/WA.toc").is_file());
        assert!(disable(&addons, &f).unwrap().is_empty());
        assert_eq!(enable(&addons, &f).unwrap(), vec!["WA", "WA_Opt"]);
        assert!(addons.join("WA/WA.toc").is_file());
    }

    #[test]
    fn remove_then_rollback_restores() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("AddOns");
        addon(&addons, "WA");
        addon(&addons, "WA_Opt");
        disable(&addons, &["WA_Opt".to_string()]).unwrap();
        let o = remove(&addons, "WA", &["WA".into(), "WA_Opt".into()], Some("5.22.1")).unwrap();
        assert_eq!(o.replaced, vec!["WA", "WA_Opt"]);
        assert!(!addons.join("WA").exists());
        assert!(!addons.join(DISABLED_DIR).join("WA_Opt").exists());
        assert_eq!(removed(&addons), vec![("WA".to_string(), "5.22.1".to_string())]);

        let o = rollback(&addons, "WA").unwrap();
        assert_eq!(o.installed, vec!["WA", "WA_Opt"]);
        assert!(addons.join("WA/WA.toc").is_file());
        assert!(addons.join("WA_Opt/WA_Opt.toc").is_file());
        assert!(removed(&addons).is_empty());
    }

    #[test]
    fn every_folder_of_a_release_with_a_backup_can_roll_back() {
        let generation = |version: &str, folders: &[&str], backup: Option<&str>| Generation {
            version: version.into(),
            folders: folders.iter().map(|f| f.to_string()).collect(),
            backup: backup.map(String::from),
        };
        let mut log = Log::default();
        // Routine's zip ships two folders; the companion rolls back with it.
        log.installs.insert(
            "MidnightRoutine".into(),
            Record {
                current: generation("12.0.378", &["MidnightRoutine", "RoutineData"], None),
                previous: Some(generation(
                    "12.0.372",
                    &["MidnightRoutine", "RoutineData"],
                    Some(".tocsin-backup/MidnightRoutine-12.0.372"),
                )),
                ..Default::default()
            },
        );
        // A first install keeps no generation to go back to.
        log.installs.insert(
            "Fresh".into(),
            Record {
                current: generation("1.0", &["Fresh"], None),
                ..Default::default()
            },
        );
        // A previous generation whose backup is gone is not offered either.
        log.installs.insert(
            "NoBackup".into(),
            Record {
                current: generation("2.0", &["NoBackup"], None),
                previous: Some(generation("1.0", &["NoBackup"], None)),
                ..Default::default()
            },
        );
        let got: Vec<String> = rollbackable_in(&log).into_iter().collect();
        assert_eq!(got, ["MidnightRoutine", "RoutineData"]);
    }
}
