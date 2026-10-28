//! One call that does "update this addon": download, verify, install.
//!
//! A fresh install from search goes in two steps, [`prepare_new`] then
//! [`install_prepared`] (or [`discard`]), so the GUI can ask before a zip
//! replaces folders that are already there: many WoWInterface uploads ship
//! their libraries (`Ace3`, `!BugGrabber`, `LibStub`) as top-level folders,
//! and those may belong to another addon.
//!
//! A downloaded zip is deleted once it has been installed (or failed to):
//! rollback uses the backup folders, never the zip. [`prune_cache`] sweeps
//! what a crash or a cancelled dialog left behind.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::addon::Flavor;
use crate::archive;
use crate::download::{self, DownloadError, Downloaded};
use crate::http::Http;
use crate::install::{self, InstallError, Outcome, Request};
use crate::sources::{fetch_latest, github::Hint, Credentials, Release, SourceError, SourceRef};

/// Longest piece of a download's file name; a search install keys it by the
/// addon's display name, which can be long.
const MAX_NAME_PART: usize = 64;

/// A `.part` file older than this is a leftover: [`download::fetch`] never
/// resumes one, and no download runs anywhere near this long.
const STALE_PART: Duration = Duration::from_secs(60 * 60);

/// Errors from an update.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// The provider gave no direct download link.
    #[error("no download link; open the page and install by hand")]
    NoDownload,
    /// Download problems.
    #[error(transparent)]
    Download(#[from] DownloadError),
    /// Install problems.
    #[error(transparent)]
    Install(#[from] InstallError),
    /// The provider could not say what to download.
    #[error(transparent)]
    Source(#[from] SourceError),
    /// Background task failure.
    #[error("install task failed: {0}")]
    Task(String),
}

impl UpdateError {
    /// Whether the source has nothing an app can download, so trying it
    /// again is pointless and only its page can help: no download link, or
    /// no release at all (a GitHub repository that never published one).
    pub fn is_site_only(&self) -> bool {
        matches!(self, Self::NoDownload | Self::Source(SourceError::NoRelease))
    }
}

/// Per-user cache directory for downloaded zips.
pub fn cache_dir() -> PathBuf {
    crate::paths::project_dirs()
        .map(|d| d.cache_dir().join("downloads"))
        .unwrap_or_else(|| std::env::temp_dir().join("tocsin-downloads"))
}

/// Where an update goes.
#[derive(Debug, Clone)]
pub struct Target<'a> {
    /// The `AddOns` folder.
    pub addons_dir: &'a Path,
    /// Folder the user acted on (key in the install log); for an addon not
    /// installed yet, its name (see [`Request::key`]).
    pub key: &'a str,
    /// Game flavor of the folder.
    pub flavor: Flavor,
    /// Game build, when known.
    pub client_interface: Option<u32>,
    /// Version on disk now, when Tocsin did not install it.
    pub replacing_version: Option<&'a str>,
}

/// A fresh install, downloaded and inspected but not yet unpacked.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// The release that was downloaded.
    pub release: Release,
    /// The zip in the cache.
    pub zip: PathBuf,
    /// Hex SHA-256 of the zip.
    pub sha256: String,
    /// The zip's top-level folders, i.e. what lands in `AddOns`.
    pub folders: Vec<String>,
    /// Those of `folders` already in `AddOns` or `AddOns/.tocsin-disabled`:
    /// installing replaces them (they go to the backup, so Rollback undoes it).
    pub conflicts: Vec<String>,
}

impl Prepared {
    /// Whether the zip installs `folder` (case-insensitively). Asked when
    /// installing a missing dependency from its head's source: CurseForge
    /// and WoWInterface ignore which folder was asked for and may serve the
    /// head's zip again, which would replace the head and still miss the
    /// dependency.
    pub fn ships(&self, folder: &str) -> bool {
        self.folders.iter().any(|f| f.eq_ignore_ascii_case(folder))
    }
}

/// Download `release` and install it at `target`. The zip is deleted
/// afterwards, whether the install worked or not.
pub async fn update(
    http: &Http,
    target: &Target<'_>,
    source: &SourceRef,
    release: &Release,
) -> Result<Outcome, UpdateError> {
    let downloaded = fetch_zip(http, target, source, release).await?;
    install_zip(target, source, release, downloaded.path, downloaded.sha256).await
}

/// First step of installing an addon that is not on disk yet (a search
/// result): ask `source` for its latest release for the target's flavor,
/// download it and list its folders and which of them already exist. Follow
/// with [`install_prepared`], or [`discard`] when the user cancels.
pub async fn prepare_new(
    http: &Http,
    creds: &Credentials,
    target: &Target<'_>,
    source: &SourceRef,
) -> Result<Prepared, UpdateError> {
    // The requested name doubles as the folder to match release assets
    // against: a missing dependency (`RoutineData`) picks its own zip from a
    // release that ships two. A search result's display name rarely matches
    // an asset, and then the plain family and the flavor's asset win as before.
    let hint = Hint {
        folder: target.key,
        installed: None,
        flavor: target.flavor,
    };
    let release = fetch_latest(http, creds, source, hint).await?;
    let Downloaded { path, sha256, .. } = fetch_zip(http, target, source, &release).await?;

    let addons_dir = target.addons_dir.to_path_buf();
    let zip = path.clone();
    let inspected = tokio::task::spawn_blocking(move || {
        let manifest = archive::inspect(&zip).map_err(InstallError::from)?;
        let folders: Vec<String> = manifest.folders.into_keys().collect();
        let conflicts = conflicts(&addons_dir, &folders);
        Ok::<_, UpdateError>((folders, conflicts))
    })
    .await
    .map_err(|e| UpdateError::Task(e.to_string()))
    .and_then(|r| r);
    match inspected {
        Ok((folders, conflicts)) => Ok(Prepared {
            release,
            zip: path,
            sha256,
            folders,
            conflicts,
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            Err(e)
        }
    }
}

/// Second step: unpack a [`Prepared`] zip at `target`, remembering `source`
/// for future updates as [`update`] does. The zip is deleted afterwards, so
/// a `Prepared` is installed once.
pub async fn install_prepared(
    target: &Target<'_>,
    source: &SourceRef,
    prepared: &Prepared,
) -> Result<Outcome, UpdateError> {
    install_zip(
        target,
        source,
        &prepared.release,
        prepared.zip.clone(),
        prepared.sha256.clone(),
    )
    .await
}

/// The user cancelled a prepared install: drop its zip.
pub fn discard(prepared: &Prepared) {
    let _ = std::fs::remove_file(&prepared.zip);
}

/// Install an addon that is not on disk yet in one go: [`prepare_new`] then
/// [`install_prepared`], replacing any conflicting folders without asking.
/// For callers with no one to ask (the headless example); the GUI uses the
/// two steps.
pub async fn install_new(
    http: &Http,
    creds: &Credentials,
    target: &Target<'_>,
    source: &SourceRef,
) -> Result<Outcome, UpdateError> {
    let prepared = prepare_new(http, creds, target, source).await?;
    install_prepared(target, source, &prepared).await
}

/// Roll `key` back to its previous generation.
pub async fn rollback(addons_dir: &Path, key: &str) -> Result<Outcome, UpdateError> {
    let addons_dir = addons_dir.to_path_buf();
    let key = key.to_string();
    tokio::task::spawn_blocking(move || install::rollback(&addons_dir, &key).map_err(UpdateError::from))
        .await
        .map_err(|e| UpdateError::Task(e.to_string()))?
}

/// Delete files in [`cache_dir`] older than `max_age`, and `.part` files of
/// downloads that never finished. Top level only; links and folders are left
/// alone. Returns how many files were removed. Blocking: call it off the UI
/// thread.
pub fn prune_cache(max_age: Duration) -> usize {
    prune_dir(&cache_dir(), max_age, SystemTime::now())
}

/// [`prune_cache`] for any cache folder (the icon cache shares it).
pub(crate) fn prune_dir(dir: &Path, max_age: Duration, now: SystemTime) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        // `DirEntry::file_type` does not follow links: a link is not a file here.
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let is_part = path.extension().is_some_and(|e| e == "part");
        let limit = if is_part { max_age.min(STALE_PART) } else { max_age };
        let age = std::fs::symlink_metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok());
        if age.is_some_and(|a| a > limit) && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Which of `folders` already exist in `AddOns` (in any form, a stray file
/// or a link included) or among the disabled addons.
fn conflicts(addons_dir: &Path, folders: &[String]) -> Vec<String> {
    let disabled = addons_dir.join(install::DISABLED_DIR);
    let disabled_ok = install::is_real_dir(&disabled);
    folders
        .iter()
        .filter(|f| {
            std::fs::symlink_metadata(addons_dir.join(f)).is_ok()
                || (disabled_ok && std::fs::symlink_metadata(disabled.join(f)).is_ok())
        })
        .cloned()
        .collect()
}

/// Download `release` into the cache under a name built from the provider,
/// the key and the version (never from the URL).
async fn fetch_zip(
    http: &Http,
    target: &Target<'_>,
    source: &SourceRef,
    release: &Release,
) -> Result<Downloaded, UpdateError> {
    let url = release.download.as_deref().ok_or(UpdateError::NoDownload)?;
    let file_name = format!(
        "{}-{}-{}.zip",
        safe(&source.provider().to_ascii_lowercase()),
        safe(target.key),
        safe(release.version.as_str())
    );
    Ok(download::fetch(http, url, &cache_dir(), &file_name, release.digest.as_ref()).await?)
}

/// Install `zip` on a blocking thread, then delete it.
async fn install_zip(
    target: &Target<'_>,
    source: &SourceRef,
    release: &Release,
    zip: PathBuf,
    sha256: String,
) -> Result<Outcome, UpdateError> {
    let addons_dir = target.addons_dir.to_path_buf();
    let key = target.key.to_string();
    let flavor = target.flavor;
    let client_interface = target.client_interface;
    let replacing = target.replacing_version.map(str::to_string);
    let provider = source.provider().to_string();
    // Remember the source only when it is the one that answered: a fallback
    // provider may have served this release instead.
    let remembered = (release.provider.is_empty() || release.provider == provider).then(|| source.clone());
    let version = release.version.as_str().to_string();
    let cleanup = zip.clone();
    let result = tokio::task::spawn_blocking(move || {
        let req = Request {
            key: &key,
            provider: &provider,
            source: remembered.as_ref(),
            version: &version,
            sha256: &sha256,
            flavor,
            client_interface,
            replacing_version: replacing.as_deref(),
        };
        let outcome = install::install(&addons_dir, &zip, &req).map_err(UpdateError::from);
        let _ = std::fs::remove_file(&zip);
        outcome
    })
    .await;
    match result {
        Ok(outcome) => outcome,
        Err(e) => {
            // The task died before it could clean up.
            let _ = std::fs::remove_file(&cleanup);
            Err(UpdateError::Task(e.to_string()))
        }
    }
}

/// A file-name piece: ASCII letters, digits, `.`, `_`, `-`, at most
/// [`MAX_NAME_PART`] of them, so the cache path stays well under Windows'
/// `MAX_PATH` whatever the display name or version string.
fn safe(s: &str) -> String {
    s.chars()
        .take(MAX_NAME_PART)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_pieces_are_short_and_plain() {
        assert_eq!(safe("Deadly Boss Mods (DBM)"), "Deadly_Boss_Mods__DBM_");
        assert_eq!(safe(&"x".repeat(300)).len(), MAX_NAME_PART);
        assert_eq!(safe("../..\\C:"), ".._.._C_");
    }

    #[test]
    fn only_a_missing_file_makes_a_source_site_only() {
        // Azta'rec Helper: CurseForge without a link, GitHub without releases.
        assert!(UpdateError::NoDownload.is_site_only());
        assert!(UpdateError::Source(SourceError::NoRelease).is_site_only());
        // A failure that may pass on the next try.
        assert!(!UpdateError::Task("cancelled".into()).is_site_only());
        assert!(!UpdateError::Source(SourceError::MissingCredentials("CurseForge")).is_site_only());
    }

    #[test]
    fn a_zip_ships_a_dependency_only_if_it_has_its_folder() {
        let prepared = Prepared {
            release: crate::sources::Release {
                version: crate::Version::parse("12.0.378"),
                url: String::new(),
                published_at: None,
                download: None,
                digest: None,
                provider: "CurseForge".into(),
            },
            zip: PathBuf::new(),
            sha256: String::new(),
            // The head's zip, served again when RoutineData was asked for.
            folders: vec!["MidnightRoutine".into()],
            conflicts: Vec::new(),
        };
        assert!(!prepared.ships("RoutineData"));
        assert!(prepared.ships("midnightroutine"));
    }

    #[test]
    fn conflicts_lists_folders_already_there() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path();
        // A WoWInterface zip shipping its libraries as top-level folders.
        std::fs::create_dir_all(addons.join("Ace3")).unwrap();
        std::fs::create_dir_all(addons.join(install::DISABLED_DIR).join("!BugGrabber")).unwrap();
        let folders: Vec<String> = ["!BugGrabber", "Ace3", "LibStub", "MyAddon"]
            .map(String::from)
            .to_vec();
        assert_eq!(conflicts(addons, &folders), ["!BugGrabber", "Ace3"]);
    }

    #[test]
    fn prune_removes_old_files_and_leftover_parts_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let touch = |name: &str, age: Duration| {
            let p = dir.join(name);
            let f = std::fs::File::create(&p).unwrap();
            f.set_modified(SystemTime::now() - age).unwrap();
        };
        let day = Duration::from_secs(24 * 60 * 60);
        touch("github-WA-1.zip", 10 * day);
        touch("github-WA-2.zip", Duration::from_secs(60));
        touch("github-WA-3.zip.part", 2 * STALE_PART);
        touch("github-WA-4.zip.part", Duration::from_secs(60));
        std::fs::create_dir(dir.join("old-folder")).unwrap();

        assert_eq!(prune_dir(dir, 7 * day, SystemTime::now()), 2);
        let mut left: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["github-WA-2.zip", "github-WA-4.zip.part", "old-folder"]);
        assert_eq!(prune_dir(&dir.join("missing"), day, SystemTime::now()), 0);
    }
}
