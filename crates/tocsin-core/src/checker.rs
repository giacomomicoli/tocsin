//! Check every addon in a library for updates, concurrently but politely.

use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::addon::{Flavor, InstalledAddon};
use crate::http::Http;
use crate::manifest::Manifest;
use crate::sources::{fetch_latest, Credentials, Release, SourceError, SourceRef};

/// At most this many requests in flight at once, so we never hammer an API.
const MAX_CONCURRENCY: usize = 6;

/// Result of checking one addon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Installed version is the newest known.
    UpToDate(Release),
    /// A newer release exists.
    Outdated(Release),
    /// Installed is newer than what the provider reports (dev build, or
    /// provider lagging behind).
    Ahead(Release),
    /// Provider reachable, but the installed version is unknown so we can only
    /// show what's out there.
    Unknown(Release),
    /// Source cannot be queried automatically; here is the page to visit.
    Manual(String),
    /// The `.toc` carries no source and the user declared none.
    NoSource,
    /// User asked to ignore this addon.
    Ignored,
    /// Something went wrong (network, missing key, …).
    Error(String),
}

impl UpdateStatus {
    /// Short label for list views.
    pub fn label(&self) -> &'static str {
        match self {
            Self::UpToDate(_) => "Up to date",
            Self::Outdated(_) => "Update available",
            Self::Ahead(_) => "Ahead of source",
            Self::Unknown(_) => "Version unknown",
            Self::Manual(_) => "Check manually",
            Self::NoSource => "No source",
            Self::Ignored => "Ignored",
            Self::Error(_) => "Error",
        }
    }

    /// The release the provider answered with, when it answered one.
    pub fn release(&self) -> Option<&Release> {
        match self {
            Self::UpToDate(r) | Self::Outdated(r) | Self::Ahead(r) | Self::Unknown(r) => Some(r),
            Self::Manual(_) | Self::NoSource | Self::Ignored | Self::Error(_) => None,
        }
    }

    /// The one line about the addon its provider gave with the release, for
    /// the library row (see [`Release::summary`]).
    pub fn summary(&self) -> Option<&str> {
        self.release()?.summary.as_deref()
    }

    /// Whether the user has something to do about it: a page to visit, a
    /// source to declare, an error, or a version Tocsin cannot compare.
    /// What the "Needs attention" filter shows.
    pub fn needs_attention(&self) -> bool {
        matches!(
            self,
            Self::Manual(_) | Self::NoSource | Self::Error(_) | Self::Unknown(_)
        )
    }
}

/// One addon's outcome.
#[derive(Debug, Clone)]
pub struct UpdateReport {
    /// Folder name of the addon.
    pub folder: String,
    /// Status after checking.
    pub status: UpdateStatus,
}

/// Build [`Credentials`]: environment, OS credential store, then manifest.
///
/// Blocking: the OS credential store is a synchronous call (D-Bus on Linux,
/// which may show an unlock prompt). Call it from `spawn_blocking`, once,
/// and hand the result to [`check_all`] / [`check_folders`].
pub fn credentials(manifest: &Manifest) -> Credentials {
    crate::secrets::credentials(manifest)
}

/// Check one addon. `client_interface` is the game's build when known: a
/// release that says which build each of its zips is for is read for it.
pub async fn check_one(
    http: &Http,
    creds: &Credentials,
    addon: &InstalledAddon,
    ignored: bool,
    flavor: Flavor,
    client_interface: Option<u32>,
) -> UpdateStatus {
    if ignored {
        return UpdateStatus::Ignored;
    }
    if addon.sources.is_empty() {
        return UpdateStatus::NoSource;
    }

    // Try each advertised source in order; providers we have no key for are
    // skipped, so a CurseForge+WoWInterface addon still resolves without the
    // CurseForge key. Remember the first real failure for the error message.
    let mut first_error: Option<String> = None;
    let mut manual: Option<String> = None;
    let mut only_missing = true;
    let mut no_key: Vec<&'static str> = Vec::new();
    let mut set_aside: Option<Release> = None;
    for source in &addon.sources {
        if let SourceRef::Manual { .. } = source {
            manual.get_or_insert_with(|| source.web_url());
            continue;
        }
        if !source.is_usable(creds) {
            tracing::debug!(addon = %addon.folder, provider = source.provider(), "skipped: no API key");
            if !no_key.contains(&source.provider()) {
                no_key.push(source.provider());
            }
            continue;
        }
        let hint = crate::sources::github::Hint {
            folder: &addon.folder,
            installed: addon.installed.as_ref(),
            flavor,
            client_interface,
        };
        match fetch_latest(http, creds, source, hint).await {
            Ok(release) if is_stale_guess(addon, source, &release) => {
                tracing::debug!(addon = %addon.folder, "guessed repository is behind and has no zip; trying the next source");
                set_aside.get_or_insert(release);
            }
            Ok(release) => return compare(addon, release),
            Err(SourceError::Manual(url)) => {
                manual.get_or_insert(url);
            }
            Err(e) => {
                tracing::warn!(addon = %addon.folder, provider = source.provider(), error = %e, "update check failed");
                only_missing &= matches!(e, SourceError::NoRelease | SourceError::NotFound);
                first_error.get_or_insert_with(|| format!("{}: {e}", source.provider()));
            }
        }
    }
    match set_aside {
        // Nothing else to say about it: the guess is all there is.
        Some(release) if first_error.is_none() && manual.is_none() && no_key.is_empty() => {
            compare(addon, release)
        }
        _ => settle(first_error, manual, only_missing, &no_key),
    }
}

/// A release not worth believing: from a GitHub repository the addon's own
/// `.toc` does not name (found in its docs, or taken from its family head),
/// with no zip to download, and older than what is installed. Details links
/// its repository in its docs; the newest release there is a 2021 tag with no
/// files, so a 2026 build read as "Ahead of source" while the addon is
/// published on CurseForge. Such a release is set aside for the next source.
fn is_stale_guess(addon: &InstalledAddon, source: &SourceRef, release: &Release) -> bool {
    let SourceRef::Github { .. } = source else {
        return false;
    };
    let declared = SourceRef::candidates_from_toc(&addon.toc)
        .iter()
        .any(|c| c.project_key() == source.project_key());
    let behind = addon
        .installed
        .as_ref()
        .is_some_and(|installed| release.version.is_older_than(installed));
    !declared && release.download.is_none() && behind
}

/// The status of an addon no source gave a usable release for. "No release"
/// or "not found" from every API asked (Ace3's GitHub mirror has no
/// releases; the repository in Auctionator's changelog is gone) is not a
/// failure worth a red badge when a page to check by hand exists. `no_key`
/// lists the providers skipped for want of a key.
fn settle(
    first_error: Option<String>,
    manual: Option<String>,
    only_missing: bool,
    no_key: &[&str],
) -> UpdateStatus {
    match (first_error, manual) {
        (Some(_), Some(url)) if only_missing => UpdateStatus::Manual(url),
        (Some(e), _) => UpdateStatus::Error(e),
        (None, Some(url)) => UpdateStatus::Manual(url),
        // Only keyed providers were left and we hold none of the keys.
        (None, None) => {
            UpdateStatus::Error(format!("API key needed for {} (see README)", no_key.join(" or ")))
        }
    }
}

fn compare(addon: &InstalledAddon, release: Release) -> UpdateStatus {
    match &addon.installed {
        None => UpdateStatus::Unknown(release),
        Some(installed) if installed.is_older_than(&release.version) => UpdateStatus::Outdated(release),
        Some(installed) if release.version.is_older_than(installed) => UpdateStatus::Ahead(release),
        Some(_) => UpdateStatus::UpToDate(release),
    }
}

/// Check all addons, at most [`MAX_CONCURRENCY`] at a time. Results come back
/// in the same order as `addons`.
///
/// `creds` comes from the caller, loaded once with [`credentials`] off the
/// async workers: reading the OS credential store blocks (and may prompt on
/// Linux), so it must not happen on every check.
pub async fn check_all(
    http: &Http,
    manifest: &Manifest,
    creds: &Credentials,
    addons: &Arc<Vec<InstalledAddon>>,
    flavor: Flavor,
    client_interface: Option<u32>,
) -> Vec<UpdateReport> {
    let all = (0..addons.len()).collect();
    run(http, manifest, creds, addons, flavor, client_interface, all).await
}

/// Check only the addons whose folder is in `folders`, with the same bound
/// and semantics as [`check_all`]. Results follow the order of `addons`;
/// folders not in the library are skipped.
///
/// After installing one addon the GUI re-checks just that one (and its
/// family): re-checking the whole library would spend GitHub's 60
/// unauthenticated requests an hour for nothing.
pub async fn check_folders(
    http: &Http,
    manifest: &Manifest,
    creds: &Credentials,
    addons: &Arc<Vec<InstalledAddon>>,
    flavor: Flavor,
    client_interface: Option<u32>,
    folders: &[String],
) -> Vec<UpdateReport> {
    let wanted = addons
        .iter()
        .enumerate()
        .filter(|(_, a)| folders.contains(&a.folder))
        .map(|(i, _)| i)
        .collect();
    run(http, manifest, creds, addons, flavor, client_interface, wanted).await
}

/// One check run over `addons[i]` for every `i` in `indices`. Each task gets
/// the shared `Arc` and an index: cloning an addon would copy its whole
/// `.toc` map once per check.
async fn run(
    http: &Http,
    manifest: &Manifest,
    creds: &Credentials,
    addons: &Arc<Vec<InstalledAddon>>,
    flavor: Flavor,
    client_interface: Option<u32>,
    indices: Vec<usize>,
) -> Vec<UpdateReport> {
    // A new run asks every endpoint afresh.
    http.clear_cache();
    let creds = Arc::new(creds.clone());
    let sem = Arc::new(Semaphore::new(MAX_CONCURRENCY));
    let mut handles = Vec::with_capacity(indices.len());

    for &i in &indices {
        let http = http.clone();
        let creds = Arc::clone(&creds);
        let sem = Arc::clone(&sem);
        let addons = Arc::clone(addons);
        let ignored = manifest
            .overrides
            .get(&addons[i].folder)
            .is_some_and(|o| o.ignore);
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore closed");
            let addon = &addons[i];
            let status = check_one(&http, &creds, addon, ignored, flavor, client_interface).await;
            UpdateReport {
                folder: addon.folder.clone(),
                status,
            }
        }));
    }

    let mut out = Vec::with_capacity(handles.len());
    for (h, i) in handles.into_iter().zip(indices) {
        match h.await {
            Ok(r) => out.push(r),
            Err(e) => out.push(UpdateReport {
                folder: addons[i].folder.clone(),
                status: UpdateStatus::Error(e.to_string()),
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toc::Toc;
    use std::path::PathBuf;

    fn addon(toc: &str) -> InstalledAddon {
        let toc = Toc::parse(toc);
        InstalledAddon {
            folder: "X".into(),
            path: PathBuf::new(),
            toc_path: PathBuf::new(),
            installed: toc.version().map(crate::Version::parse),
            sources: SourceRef::candidates_from_toc(&toc),
            display_name: "X".into(),
            issues: Vec::new(),
            disabled: false,
            toc,
        }
    }

    #[tokio::test]
    async fn keyed_only_sources_without_keys_explain_themselves() {
        let a = addon("## Version: 1\n## X-Wago-ID: abc\n## X-Curse-Project-ID: 5\n");
        let s = check_one(
            &Http::new(),
            &Credentials::default(),
            &a,
            false,
            Flavor::Retail,
            None,
        )
        .await;
        assert_eq!(
            s,
            UpdateStatus::Error("API key needed for CurseForge (see README)".into())
        );
    }

    fn named(folder: &str, toc: &str) -> InstalledAddon {
        InstalledAddon {
            folder: folder.into(),
            ..addon(toc)
        }
    }

    /// Only sources that need no request (manual, none, ignored), so the
    /// runs below never touch the network.
    fn library() -> Arc<Vec<InstalledAddon>> {
        Arc::new(vec![
            named("Zygor", "## X-Website: https://zygorguides.com/\n"),
            named("Bare", "## Title: x\n"),
            named("Quiet", "## X-Website: https://example.com/\n"),
        ])
    }

    #[tokio::test]
    async fn check_all_keeps_library_order() {
        let mut manifest = Manifest::default();
        manifest.overrides.entry("Quiet".into()).or_default().ignore = true;
        let r = check_all(
            &Http::new(),
            &manifest,
            &Credentials::default(),
            &library(),
            Flavor::Retail,
            None,
        )
        .await;
        let got: Vec<(&str, &UpdateStatus)> = r.iter().map(|r| (r.folder.as_str(), &r.status)).collect();
        assert_eq!(
            got,
            vec![
                ("Zygor", &UpdateStatus::Manual("https://zygorguides.com/".into())),
                ("Bare", &UpdateStatus::NoSource),
                ("Quiet", &UpdateStatus::Ignored),
            ]
        );
    }

    #[tokio::test]
    async fn check_folders_checks_only_those_in_library_order() {
        let wanted = ["Bare".to_string(), "Zygor".to_string(), "Gone".to_string()];
        let r = check_folders(
            &Http::new(),
            &Manifest::default(),
            &Credentials::default(),
            &library(),
            Flavor::Retail,
            None,
            &wanted,
        )
        .await;
        let folders: Vec<&str> = r.iter().map(|r| r.folder.as_str()).collect();
        assert_eq!(folders, ["Zygor", "Bare"]);
        assert_eq!(r[1].status, UpdateStatus::NoSource);
        let none = check_folders(
            &Http::new(),
            &Manifest::default(),
            &Credentials::default(),
            &library(),
            Flavor::Retail,
            None,
            &[],
        )
        .await;
        assert!(none.is_empty());
    }

    #[test]
    fn a_source_that_is_gone_falls_back_to_the_page() {
        // Auctionator 341 without a CurseForge key: CurseForge skipped, the
        // changelog's GitHub repository answered 404, the page is left.
        let page = "https://www.curseforge.com/wow/addons/auctionator".to_string();
        let gone = Some("GitHub: project not found".to_string());
        assert_eq!(
            settle(gone.clone(), Some(page.clone()), true, &[]),
            UpdateStatus::Manual(page.clone())
        );
        // A real failure (timeout, 5xx) still says so.
        assert_eq!(
            settle(gone.clone(), Some(page), false, &[]),
            UpdateStatus::Error("GitHub: project not found".into())
        );
        // Nothing to fall back on: the user still learns why.
        assert_eq!(
            settle(gone, None, true, &[]),
            UpdateStatus::Error("GitHub: project not found".into())
        );
    }

    #[tokio::test]
    async fn keyed_sources_are_named_once() {
        // No key: nothing is fetched, both CurseForge entries are skipped.
        let mut a = addon("## X-Curse-Project-ID: 5\n");
        a.sources.push(SourceRef::CurseForgeSlug { slug: "x".into() });
        assert_eq!(
            check_one(
                &Http::new(),
                &Credentials::default(),
                &a,
                false,
                Flavor::Retail,
                None
            )
            .await,
            UpdateStatus::Error("API key needed for CurseForge (see README)".into())
        );
    }

    /// Details' repository, as its docs link it: the newest release is
    /// `DetailsRetail.v9.1.0.8888.145` from 2021, with no files.
    fn details_repo() -> SourceRef {
        SourceRef::Github {
            owner: "Tercioo".into(),
            repo: "Details-Damage-Meter".into(),
            tag_prefix: None,
        }
    }

    fn release(version: &str, download: Option<&str>) -> Release {
        Release {
            version: crate::Version::parse(version),
            url: String::new(),
            published_at: None,
            download: download.map(str::to_string),
            digest: None,
            provider: "GitHub".into(),
            interfaces: None,
            size: None,
            summary: None,
        }
    }

    #[test]
    fn a_guessed_repository_behind_the_install_is_set_aside() {
        let details =
            addon("## Version: #Details.20261104.15400.172-1-gcec582c\n## X-Curse-Project-ID: 61284\n");
        let old = release("v9.0.5.8357.144", None);
        assert!(is_stale_guess(&details, &details_repo(), &old));
        // A zip to install, a newer release or no installed version: believed.
        assert!(!is_stale_guess(
            &details,
            &details_repo(),
            &release("v9.0.5", Some("https://x"))
        ));
        assert!(!is_stale_guess(
            &details,
            &details_repo(),
            &release("Details.20261107.15402.172", None)
        ));
        let unversioned = addon("## X-Curse-Project-ID: 61284\n");
        assert!(!is_stale_guess(&unversioned, &details_repo(), &old));
        // The `.toc` names the repository itself: its answer stands.
        let declared = addon(
            "## Version: #Details.20261104.15400.172\n## X-Website: https://github.com/Tercioo/Details-Damage-Meter\n",
        );
        assert!(!is_stale_guess(&declared, &details_repo(), &old));
        // Other providers are not guesses.
        assert!(!is_stale_guess(
            &details,
            &SourceRef::CurseForge { id: 61284 },
            &old
        ));
    }

    #[test]
    fn a_set_aside_release_gives_way_to_the_key_it_needs() {
        // Details without a CurseForge key: CurseForge skipped, the docs'
        // repository set aside. The truth is the missing key, not "Ahead".
        assert_eq!(
            settle(None, None, true, &["CurseForge"]),
            UpdateStatus::Error("API key needed for CurseForge (see README)".into())
        );
    }

    #[test]
    fn attention_is_for_what_the_user_must_act_on() {
        let release = Release {
            version: crate::Version::parse("1.0"),
            url: String::new(),
            published_at: None,
            download: None,
            digest: None,
            provider: String::new(),
            interfaces: None,
            size: None,
            summary: None,
        };
        assert!(UpdateStatus::Manual("https://zygorguides.com/".into()).needs_attention());
        assert!(UpdateStatus::NoSource.needs_attention());
        assert!(UpdateStatus::Error("CurseForge: timeout".into()).needs_attention());
        assert!(UpdateStatus::Unknown(release.clone()).needs_attention());
        // An update is its own filter; the rest is fine as it is.
        assert!(!UpdateStatus::Outdated(release.clone()).needs_attention());
        assert!(!UpdateStatus::UpToDate(release.clone()).needs_attention());
        assert!(!UpdateStatus::Ahead(release).needs_attention());
        assert!(!UpdateStatus::Ignored.needs_attention());
    }

    #[test]
    fn the_summary_comes_with_the_release() {
        let mut r = release("1.0", None);
        assert_eq!(UpdateStatus::UpToDate(r.clone()).summary(), None);
        r.summary = Some("Computes damage.".into());
        assert_eq!(UpdateStatus::Outdated(r).summary(), Some("Computes damage."));
        assert_eq!(UpdateStatus::Error("x".into()).summary(), None);
    }

    #[tokio::test]
    async fn manual_and_ignored_and_none() {
        let http = Http::new();
        let c = Credentials::default();
        let a = addon("## X-Website: https://zygorguides.com/\n");
        assert_eq!(
            check_one(&http, &c, &a, false, Flavor::Retail, None).await,
            UpdateStatus::Manual("https://zygorguides.com/".into())
        );
        assert_eq!(
            check_one(&http, &c, &a, true, Flavor::Retail, None).await,
            UpdateStatus::Ignored
        );
        assert_eq!(
            check_one(&http, &c, &addon("## Title: x\n"), false, Flavor::Retail, None).await,
            UpdateStatus::NoSource
        );
    }
}
