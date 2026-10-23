//! Application state, update logic and the window frame (header, footer).
//! Each page's view lives in its own child module.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use iced::futures::stream::{self, StreamExt};
use iced::widget::{button, column, container, image, row, space, text};
use iced::{Alignment, Element, Fill, Padding, Subscription, Task, Theme};

use tocsin_core::addon::Flavor;
use tocsin_core::family::{self, Family};
use tocsin_core::http::{self, Http};
use tocsin_core::icons;
use tocsin_core::search::{self, Found, Installed};
use tocsin_core::secrets::{self, KeyStatus, Secret};
use tocsin_core::sources::wowinterface::Catalog;
use tocsin_core::sources::{curseforge, Credentials};
use tocsin_core::updater::{self, Prepared, Target};
use tocsin_core::{checker, detect, InstalledAddon, Manifest, SourceRef, UpdateReport, UpdateStatus};

/// Downloaded zips older than this are swept from the cache at startup.
const CACHE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

use crate::theme::{self, Tone};

// One module per page. Children of `app`, so they read `App`'s private
// state without it being widened; views stay pure functions of `&self`.
mod discover;
mod library;
mod settings;
mod widgets;

/// Which page is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Library,
    /// Search for addons that are not installed.
    Discover,
    Settings,
}

/// Where the WoWInterface catalog stands: it is a few megabytes, fetched
/// once per session when the "Get addons" page first opens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum CatalogState {
    #[default]
    Idle,
    Loading,
    /// Number of files it lists.
    Ready(usize),
    Failed(String),
}

/// State of the "Get addons" page.
#[derive(Debug, Default)]
struct Discover {
    catalog: CatalogState,
    query: String,
    /// The query the shown results answer.
    searched: Option<String>,
    searching: bool,
    /// Bumped per search, so a slow answer to an older query is dropped.
    seq: u64,
    outcome: search::Outcome,
    /// The source an install is running from, for its button.
    installing: Option<SourceRef>,
    /// A downloaded result that would replace folders already on disk,
    /// waiting for the user to say "Replace" or "Cancel".
    confirm: Option<Box<PendingInstall>>,
    /// The icon downloads of the shown results, cancelled as soon as a newer
    /// search replaces them.
    icon_batch: Option<IconBatch>,
}

/// Where one icon stands. A failed one is not asked for again this session:
/// its row keeps the monogram.
#[derive(Debug, Clone)]
enum IconState {
    Loading,
    Ready(image::Handle),
    Failed,
}

/// Icon downloads started together. The task stops when this is dropped;
/// [`cancel_icons`] also forgets the icons it was still loading, so a later
/// batch asks for them again instead of waiting on nothing.
#[derive(Debug)]
struct IconBatch {
    _task: iced::task::Handle,
    urls: Vec<String>,
}

/// Drop `batch` and the entries of `icons` it had not delivered yet.
fn cancel_icons(icons: &mut HashMap<String, IconState>, batch: IconBatch) {
    for url in &batch.urls {
        if matches!(icons.get(url), Some(IconState::Loading)) {
            icons.remove(url);
        }
    }
}

/// Decoded icons kept in memory, search and library together: 64x64 RGBA
/// is 16 KiB each, so 300 is under 5 MiB. Past it, icons no longer shown
/// anywhere are dropped.
const MAX_ICONS: usize = 300;

/// Icon downloads in flight at once, below the update checks' bound.
const ICON_CONCURRENCY: usize = 4;

/// A search install between download and unpack. It remembers where it was
/// meant to go, so switching installation meanwhile changes nothing.
#[derive(Debug, Clone)]
pub struct PendingInstall {
    source: SourceRef,
    name: String,
    dir: PathBuf,
    flavor: Flavor,
    client_interface: Option<u32>,
    prepared: Prepared,
    /// A folder the zip must contain: set for a missing dependency, so the
    /// head's own zip (CurseForge and WoWInterface ignore which folder was
    /// asked for) is not reinstalled in its place.
    needs: Option<String>,
}

/// A key as the user types it. `Message` derives `Debug`; this keeps any
/// future `?message` log from printing keystrokes of an API key.
#[derive(Clone)]
pub struct Typed(String);

impl std::fmt::Debug for Typed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Typed(..)")
    }
}

/// Which addons a check covers.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CheckScope {
    All,
    Folders(Vec<String>),
}

impl CheckScope {
    /// One scope covering both: a full check wins over any partial one.
    fn merge(self, other: CheckScope) -> CheckScope {
        match (self, other) {
            (CheckScope::Folders(mut a), CheckScope::Folders(b)) => {
                a.extend(b);
                a.sort();
                a.dedup();
                CheckScope::Folders(a)
            }
            _ => CheckScope::All,
        }
    }
}

/// Status tallies the toolbar and the notice show, recomputed when statuses
/// or the library change instead of on every frame.
#[derive(Debug, Clone, Copy, Default)]
struct Counts {
    outdated: usize,
    manual: usize,
    errors: usize,
    /// Everything the "Needs attention" filter shows.
    attention: usize,
    /// Hard dependencies not installed.
    missing_deps: usize,
}

/// What the library list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Outdated,
    Attention,
}

/// What the user typed in the key fields, until "Save". Stored keys are
/// never loaded into it: the form only ever holds new values.
#[derive(Debug, Clone, Default)]
struct SettingsForm {
    input: BTreeMap<Secret, String>,
}

impl SettingsForm {
    fn dirty(&self) -> bool {
        self.input.values().any(|v| !v.trim().is_empty())
    }
}

/// Application state.
pub struct App {
    manifest_path: Option<PathBuf>,
    manifest: Manifest,
    http: Http,
    screen: Screen,
    /// Index into `manifest.settings.game_folders` of the installation shown.
    active_folder: usize,
    /// "Retail 12.1.5", from the last scan.
    client: String,
    /// "12.1.5", from the last scan, when the build is known.
    build: Option<String>,
    flavor: Flavor,
    client_interface: Option<u32>,
    addons: Arc<Vec<InstalledAddon>>,
    families: Vec<Family>,
    /// Family head folder of every member folder (heads are not keys).
    head_of: HashMap<String, String>,
    /// Hard dependencies not installed, by addon index.
    missing: BTreeMap<usize, Vec<String>>,
    /// Family heads with a previous generation to roll back to.
    rollbackable: BTreeSet<String>,
    /// Families removed by Tocsin that can be restored: (key, version).
    removed: Vec<(String, String)>,
    /// Folder currently being updated, for the row's spinner text.
    installing: Option<String>,
    statuses: BTreeMap<String, UpdateStatus>,
    counts: Counts,
    /// Bumped per scan: a check answering for an older library is dropped.
    lib_gen: u64,
    /// Folders to re-check after the next scan, set when an install or a
    /// move succeeded; `None` means the next scan checks everything.
    recheck: Option<Vec<String>>,
    /// API keys, read once off the UI thread; `None` until they arrive.
    creds: Option<Arc<Credentials>>,
    /// Where each key comes from; empty until the keys are read.
    key_status: BTreeMap<Secret, KeyStatus>,
    /// A check asked for before the keys were read: it runs when they arrive,
    /// never with empty credentials.
    pending_check: Option<CheckScope>,
    filter: Filter,
    search: String,
    busy: bool,
    notice: Option<(bool, String)>, // (is_error, text)
    form: SettingsForm,
    discover: Discover,
    /// Shared with every search; downloaded once per session.
    catalog: Catalog,
    /// What is on disk, so search results already there show as installed.
    installed: Installed,
    /// Animation frame for loaders, advanced only while something loads.
    tick: usize,
    /// Tocsin Void (dark) or Tocsin Sunwell (light).
    dark: bool,
    /// The sidebar logo, decoded once: a handle made in `view` would get a
    /// new id, and the PNG decoded again, on every frame.
    lockup: image::Handle,
    /// Search and library icons by URL (at most [`MAX_ICONS`]).
    icons: HashMap<String, IconState>,
    /// Icon URL of each library addon, by folder.
    library_icons: HashMap<String, String>,
    /// CurseForge logo thumbnails by project id, kept across scans.
    logos: HashMap<u64, String>,
    /// CurseForge ids already asked for a logo this session, found or not,
    /// so a rescan after an install asks only for new projects.
    logos_asked: HashSet<u64>,
    /// WoWInterface thumbnails by file id, from the catalog once loaded.
    thumbs: HashMap<u64, String>,
    /// The library's icon downloads; cancelled when another installation
    /// is shown.
    library_batches: Vec<IconBatch>,
}

/// UI events.
#[derive(Debug, Clone)]
pub enum Message {
    // navigation
    Show(Screen),
    // library
    Scan,
    Scanned(Result<Box<tocsin_core::Scan>, String>),
    /// The API keys were read (at boot, and again after saving keys).
    CredsLoaded(Arc<Credentials>, BTreeMap<Secret, KeyStatus>),
    /// Check the whole library.
    CheckUpdates,
    /// Reports for the library of scan number `.0`.
    Checked(u64, Vec<UpdateReport>),
    /// Download and install the release found for this folder.
    Update(String),
    /// Install a missing dependency `(dep_folder, head_folder)` from the
    /// head's source: the provider is asked for the file named after the dep.
    InstallMissing(String, String),
    /// Swap this family back to its previous generation.
    Rollback(String),
    /// Park the family (head + members) out of the game's sight.
    Disable(String),
    /// Bring a parked family back.
    Enable(String),
    /// Move the family to the backup; Rollback (shown as Restore) undoes it.
    Remove(String),
    /// An update or rollback finished: (folder, result).
    Installed(String, Result<Vec<String>, String>),
    // discover
    DiscoverQuery(String),
    /// Run the search (on Enter or the button, never per keystroke: GitHub
    /// allows ten unauthenticated searches a minute).
    DiscoverSearch,
    /// Results for search number `.0`.
    DiscoverFound(u64, search::Outcome),
    /// An icon arrived (or will not): (its URL, the decoded picture).
    IconLoaded(String, Result<icons::Icon, String>),
    /// CurseForge logos of library projects: (ids asked, (id, URL) found).
    LogosFound(Vec<u64>, Vec<(u64, String)>),
    /// WoWInterface thumbnails of library files, from the loaded catalog.
    ThumbsFound(Vec<(u64, String)>),
    /// Install a search result from the source the user picked: (source, name).
    /// Downloads and inspects first; see [`Message::Prepared`].
    InstallFrom(SourceRef, String),
    /// A search result is downloaded: (name, result). Installed right away
    /// unless it would replace folders already there.
    Prepared(String, Result<Box<PendingInstall>, String>),
    /// The user accepts replacing the folders of the pending install.
    ConfirmInstall,
    /// The user drops the pending install; its zip is deleted.
    CancelInstall,
    /// Fetch the WoWInterface catalog ahead of the first search.
    LoadCatalog,
    /// The catalog arrived (number of files) or could not be fetched.
    CatalogLoaded(Result<usize, String>),
    /// Next loader animation frame.
    Tick,
    FilterChanged(Filter),
    SearchChanged(String),
    OpenUrl(String),
    DismissNotice,
    SelectFolder(usize),
    // settings
    AddGameFolder,
    GameFolderPicked(Option<PathBuf>),
    RemoveGameFolder(usize),
    KeyChanged(Secret, Typed),
    SaveSettings,
    /// Delete the key from the OS credential store.
    RemoveKey(Secret),
    /// Keys written: (go back to the library, result).
    SettingsSaved(bool, Result<(), String>),
    /// Switch to the dark (`true`) or light theme.
    SetDark(bool),
}

impl App {
    /// Load configuration, resolve the AddOns folder and scan it right away.
    pub fn boot(start_on_settings: bool) -> (Self, Task<Message>) {
        let manifest_path = Manifest::default_path();
        let manifest = manifest_path
            .as_deref()
            .map(Manifest::load)
            .transpose()
            .unwrap_or_else(|e| {
                tracing::warn!("could not read config: {e}");
                None
            })
            .unwrap_or_default();

        let mut app = Self {
            manifest_path,
            manifest,
            http: Http::new(),
            screen: Screen::Library,
            active_folder: 0,
            client: String::new(),
            build: None,
            flavor: Flavor::Retail,
            client_interface: None,
            addons: Arc::new(Vec::new()),
            families: Vec::new(),
            head_of: HashMap::new(),
            missing: BTreeMap::new(),
            rollbackable: BTreeSet::new(),
            removed: Vec::new(),
            installing: None,
            statuses: BTreeMap::new(),
            counts: Counts::default(),
            lib_gen: 0,
            recheck: None,
            creds: None,
            key_status: BTreeMap::new(),
            pending_check: None,
            filter: Filter::All,
            search: String::new(),
            busy: false,
            notice: None,
            form: SettingsForm::default(),
            discover: Discover::default(),
            catalog: Catalog::default(),
            installed: Installed::default(),
            tick: 0,
            dark: true,
            lockup: image::Handle::from_bytes(theme::LOCKUP_PNG),
            icons: HashMap::new(),
            library_icons: HashMap::new(),
            logos: HashMap::new(),
            logos_asked: HashSet::new(),
            thumbs: HashMap::new(),
            library_batches: Vec::new(),
        };

        if start_on_settings {
            app.screen = Screen::Settings;
        }
        // Keys and scan run side by side; the check the scan asks for waits
        // for the keys (see `pending_check`).
        let scan = if app.manifest.settings.game_folders.is_empty() {
            app.screen = Screen::Settings;
            app.notice = Some((false, "Add your World of Warcraft folder to get started.".into()));
            Task::none()
        } else {
            app.update(Message::Scan)
        };
        let task = Task::batch([app.load_creds(), prune_cache(), scan]);
        (app, task)
    }

    pub fn title(&self) -> String {
        "Tocsin".to_string()
    }

    pub fn theme(&self) -> Theme {
        if self.dark {
            theme::void()
        } else {
            theme::sunwell()
        }
    }

    /// A timer for the loaders, running only while one is on screen: an idle
    /// app wakes up for nothing.
    pub fn subscription(&self) -> Subscription<Message> {
        let d = &self.discover;
        if d.catalog == CatalogState::Loading || d.searching || d.installing.is_some() {
            iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Show(screen) => {
                self.screen = screen;
                self.notice = None;
                // Half-typed keys do not outlive the page.
                if screen != Screen::Settings {
                    self.form.input.clear();
                }
                // Start the catalog download while the user types the query.
                let idle = matches!(
                    self.discover.catalog,
                    CatalogState::Idle | CatalogState::Failed(_)
                );
                if screen == Screen::Discover && idle {
                    return self.update(Message::LoadCatalog);
                }
                Task::none()
            }

            // ---------------------------------------------------------- library
            Message::Scan => {
                let Some(dir) = self.active_dir().cloned() else {
                    self.recheck = None;
                    self.screen = Screen::Settings;
                    self.notice = Some((true, "Add a game folder first.".into()));
                    return Task::none();
                };
                self.busy = true;
                self.notice = None;
                let manifest = self.manifest.clone();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            tocsin_core::scan(&dir, &manifest)
                                .map(Box::new)
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                    },
                    Message::Scanned,
                )
            }
            Message::Scanned(Ok(scan)) => {
                let tocsin_core::Scan {
                    library,
                    rollbackable,
                    removed,
                    log_error,
                } = *scan;
                let addons = library.addons;
                self.lib_gen += 1;
                self.removed = removed;
                self.busy = false;
                self.client = library.client.describe();
                self.build = library
                    .client
                    .version
                    .map(|(ma, mi, pa)| format!("{ma}.{mi}.{pa}"));
                self.flavor = library.client.flavor;
                self.client_interface = library.client.interface();
                self.rollbackable = rollbackable;
                self.families = family::group(&addons);
                self.head_of = family::heads(&addons, &self.families);
                self.missing = family::missing_dependencies(&addons);
                self.installed = Installed::new(&addons);
                self.addons = Arc::new(addons);
                let scope = self.scope_after_scan();
                self.recount();
                let missing = self.counts.missing_deps;
                self.notice = Some(match (&log_error, missing) {
                    (Some(e), _) => (true, format!("Install history unreadable: {e}")),
                    (None, 0) => (false, format!("Found {} addons.", self.addons.len())),
                    (None, n) => (
                        true,
                        format!("Found {} addons · {n} missing dependencies.", self.addons.len()),
                    ),
                });
                // Check right away: that is what the user opened the app for.
                let check = match scope {
                    Some(scope) => self.start_check(scope),
                    None => Task::none(),
                };
                Task::batch([check, self.resolve_library_icons()])
            }
            Message::Scanned(Err(e)) => {
                self.recheck = None;
                self.busy = false;
                self.notice = Some((true, format!("Scan failed: {e}")));
                Task::none()
            }
            Message::CredsLoaded(creds, status) => {
                self.creds = Some(creds);
                self.key_status = status;
                let check = match self.pending_check.take() {
                    Some(scope) => self.start_check(scope),
                    None => Task::none(),
                };
                // A scan that came first could not ask CurseForge for logos.
                Task::batch([check, self.resolve_library_icons()])
            }
            Message::CheckUpdates => {
                if self.addons.is_empty() || self.busy {
                    return Task::none();
                }
                self.start_check(CheckScope::All)
            }
            Message::Checked(gen, reports) => {
                if gen != self.lib_gen {
                    // A newer scan replaced the library this answers for.
                    return Task::none();
                }
                self.busy = false;
                for r in reports {
                    self.statuses.insert(r.folder, r.status);
                }
                self.recount();
                let Counts {
                    outdated,
                    errors,
                    manual,
                    missing_deps: missing,
                    ..
                } = self.counts;
                let mut parts = vec![match outdated {
                    0 => "No updates".to_string(),
                    1 => "1 update available".to_string(),
                    n => format!("{n} updates available"),
                }];
                if manual > 0 {
                    parts.push(format!("{manual} to check manually"));
                }
                if self.github_token_refused() {
                    parts.push("GitHub refused your token, see Settings".to_string());
                }
                if errors > 0 {
                    parts.push(format!("{errors} could not be checked"));
                }
                if missing > 0 {
                    parts.push(format!("{missing} missing dependencies"));
                }
                self.notice = Some((errors > 0 || missing > 0, parts.join(" · ") + "."));
                Task::none()
            }
            Message::FilterChanged(f) => {
                self.filter = f;
                Task::none()
            }
            Message::SearchChanged(s) => {
                self.search = s;
                Task::none()
            }
            Message::Update(folder) => {
                let (Some(dir), Some(addon)) = (
                    self.active_dir().cloned(),
                    self.addons.iter().find(|a| a.folder == folder),
                ) else {
                    return Task::none();
                };
                let Some(UpdateStatus::Outdated(release) | UpdateStatus::UpToDate(release)) =
                    self.statuses.get(&folder).cloned()
                else {
                    return Task::none();
                };
                let Some(source) = addon.source().cloned() else {
                    return Task::none();
                };
                // One record per release zip: the folder names it. Routine and
                // RoutineData are one family but two releases, so the family
                // head must not be the key (it made RoutineData's update evict
                // MidnightRoutine as a "stale folder").
                let key = folder.clone();
                self.busy = true;
                self.installing = Some(folder.clone());
                self.notice = Some((false, format!("Downloading {}…", addon.display_name)));
                let http = self.http.clone();
                let (flavor, client_interface) = (self.flavor, self.client_interface);
                let replacing = addon.installed.as_ref().map(|v| v.as_str().to_string());
                let done_folder = folder.clone();
                Task::perform(
                    async move {
                        let target = tocsin_core::updater::Target {
                            addons_dir: &dir,
                            key: &key,
                            flavor,
                            client_interface,
                            replacing_version: replacing.as_deref(),
                        };
                        tocsin_core::updater::update(&http, &target, &source, &release)
                            .await
                            .map(|o| o.installed)
                            .map_err(|e| e.to_string())
                    },
                    move |r| Message::Installed(done_folder.clone(), r),
                )
            }
            Message::InstallMissing(dep, head) => {
                let (Some(dir), Some(head_addon)) = (
                    self.active_dir().cloned(),
                    self.addons.iter().find(|a| a.folder == head),
                ) else {
                    return Task::none();
                };
                let Some(source) = head_addon.source().cloned() else {
                    return Task::none();
                };
                if self.busy || self.discover.confirm.is_some() {
                    return Task::none();
                }
                let Some(creds) = self.creds.clone() else {
                    self.notice = Some((false, KEYS_LOADING.into()));
                    return Task::none();
                };
                self.busy = true;
                self.installing = Some(dep.clone());
                self.notice = Some((false, format!("Looking for {dep}…")));
                // Same two steps as a search install: the zip may replace
                // folders already on disk, and the user is asked first.
                self.prepare(creds, dir, source, dep.clone(), Some(dep))
            }
            Message::Disable(ref folder) | Message::Enable(ref folder) | Message::Remove(ref folder) => {
                let folder = folder.clone();
                let Some(dir) = self.active_dir().cloned() else {
                    return Task::none();
                };
                let folders = self.family_folders(&folder);
                let version = self
                    .addons
                    .iter()
                    .find(|a| a.folder == folder)
                    .and_then(|a| a.installed.as_ref().map(|v| v.as_str().to_string()));
                let action = match &message {
                    Message::Disable(_) => "Disabled",
                    Message::Enable(_) => "Enabled",
                    _ => "Removed",
                };
                self.busy = true;
                self.installing = Some(folder.clone());
                let done = folder.clone();
                let key = folder.clone();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || match action {
                            "Disabled" => tocsin_core::install::disable(&dir, &folders),
                            "Enabled" => tocsin_core::install::enable(&dir, &folders),
                            _ => tocsin_core::install::remove(&dir, &key, &folders, version.as_deref())
                                .map(|o| o.replaced),
                        })
                        .await
                        .unwrap_or_else(|e| Err(tocsin_core::install::InstallError::Log(e.to_string())))
                        .map_err(|e| e.to_string())
                    },
                    move |r| Message::Installed(done.clone(), r),
                )
            }
            Message::Rollback(key) => {
                let Some(dir) = self.active_dir().cloned() else {
                    return Task::none();
                };
                self.busy = true;
                self.installing = Some(key.clone());
                self.notice = Some((false, "Rolling back…".into()));
                let done = key.clone();
                Task::perform(
                    async move {
                        tocsin_core::updater::rollback(&dir, &key)
                            .await
                            .map(|o| o.installed)
                            .map_err(|e| e.to_string())
                    },
                    move |r| Message::Installed(done.clone(), r),
                )
            }
            Message::DiscoverQuery(q) => {
                self.discover.query = q;
                Task::none()
            }
            Message::DiscoverSearch => {
                let Some(query) = search::clean_query(&self.discover.query) else {
                    self.notice = Some((false, "Type at least two characters to search.".into()));
                    return Task::none();
                };
                let Some(creds) = self.creds.clone() else {
                    self.notice = Some((false, KEYS_LOADING.into()));
                    return Task::none();
                };
                self.discover.seq += 1;
                self.discover.searching = true;
                self.discover.searched = Some(query.clone());
                self.notice = None;
                let seq = self.discover.seq;
                let http = self.http.clone();
                let catalog = self.catalog.clone();
                let flavor = self.flavor;
                Task::perform(
                    async move { search::search(&http, &creds, &catalog, &query, flavor).await },
                    move |out| Message::DiscoverFound(seq, out),
                )
            }
            Message::DiscoverFound(seq, outcome) => {
                if seq != self.discover.seq {
                    return Task::none();
                }
                self.discover.searching = false;
                self.discover.outcome = outcome;
                self.load_icons()
            }
            Message::IconLoaded(url, result) => {
                // Not waited for any more: dropped from the cache meanwhile.
                let Some(state) = self.icons.get_mut(&url) else {
                    return Task::none();
                };
                *state = match result {
                    Ok(icon) => IconState::Ready(image::Handle::from_rgba(icon.side, icon.side, icon.rgba)),
                    Err(e) => {
                        // The URL is a public CDN address from an API answer.
                        tracing::debug!(%url, error = %e, "no icon for an addon");
                        IconState::Failed
                    }
                };
                Task::none()
            }
            Message::LogosFound(asked, found) => {
                self.logos_asked.extend(asked);
                self.logos.extend(found);
                self.refresh_library_icons()
            }
            Message::ThumbsFound(found) => {
                self.thumbs.extend(found);
                self.refresh_library_icons()
            }
            Message::LoadCatalog => {
                if self.discover.catalog == CatalogState::Loading {
                    return Task::none();
                }
                self.discover.catalog = CatalogState::Loading;
                let http = self.http.clone();
                let catalog = self.catalog.clone();
                Task::perform(
                    async move { catalog.preload(&http).await.map_err(|e| e.to_string()) },
                    Message::CatalogLoaded,
                )
            }
            Message::CatalogLoaded(result) => {
                self.discover.catalog = match result {
                    Ok(n) => CatalogState::Ready(n),
                    Err(e) => CatalogState::Failed(e),
                };
                // Its thumbnails can stand in for library addons now.
                match self.discover.catalog {
                    CatalogState::Ready(_) => self.resolve_library_icons(),
                    _ => Task::none(),
                }
            }
            Message::Tick => {
                self.tick = self.tick.wrapping_add(1);
                Task::none()
            }
            Message::InstallFrom(source, name) => {
                let Some(dir) = self.active_dir().cloned() else {
                    return Task::none();
                };
                if self.busy || self.discover.confirm.is_some() {
                    return Task::none();
                }
                let Some(creds) = self.creds.clone() else {
                    self.notice = Some((false, KEYS_LOADING.into()));
                    return Task::none();
                };
                self.busy = true;
                self.installing = Some(name.clone());
                self.discover.installing = Some(source.clone());
                self.notice = Some((false, format!("Downloading {name} from {}…", source.provider())));
                self.prepare(creds, dir, source, name, None)
            }
            Message::Prepared(name, Ok(pending)) => {
                if let Some(needed) = pending.needs.as_deref() {
                    if !pending.prepared.ships(needed) {
                        // The provider served the head's zip again: installing
                        // it would replace the head and still miss the dependency.
                        let error =
                            format!("the release from {} does not ship it", pending.source.provider());
                        let discard = Task::future(async move {
                            let _ = tokio::task::spawn_blocking(move || updater::discard(&pending.prepared))
                                .await;
                        })
                        .discard();
                        return Task::batch([discard, self.update(Message::Installed(name, Err(error)))]);
                    }
                }
                if pending.prepared.conflicts.is_empty() {
                    return self.install_pending(*pending);
                }
                // Many uploads ship their libraries as top-level folders that
                // may belong to another addon: ask before replacing them.
                self.busy = false;
                self.installing = None;
                self.discover.installing = None;
                self.notice = Some((false, format!("{name} would replace folders you already have.")));
                self.discover.confirm = Some(pending);
                Task::none()
            }
            Message::Prepared(name, Err(e)) => self.update(Message::Installed(name, Err(e))),
            Message::ConfirmInstall => match self.discover.confirm.take() {
                Some(pending) if !self.busy => self.install_pending(*pending),
                other => {
                    self.discover.confirm = other;
                    Task::none()
                }
            },
            Message::CancelInstall => {
                let Some(pending) = self.discover.confirm.take() else {
                    return Task::none();
                };
                self.notice = Some((false, format!("Did not install {}.", pending.name)));
                Task::future(async move {
                    let _ = tokio::task::spawn_blocking(move || updater::discard(&pending.prepared)).await;
                })
                .discard()
            }
            Message::Installed(folder, result) => {
                self.busy = false;
                self.installing = None;
                self.discover.installing = None;
                match result {
                    Ok(folders) => {
                        self.notice = Some((
                            false,
                            format!("Installed {} ({} folders). Rescanning…", folder, folders.len()),
                        ));
                        // Re-check only what changed: the folders the operation
                        // touched and the family of the one acted on.
                        let mut touched = self.family_folders(&folder);
                        touched.extend(folders);
                        touched.push(folder);
                        self.recheck = Some(touched);
                        self.update(Message::Scan)
                    }
                    Err(e) => {
                        self.notice = Some((true, format!("{folder}: {e}")));
                        Task::none()
                    }
                }
            }
            Message::OpenUrl(url) => {
                // Only ever hand https:// links to the OS: no file://, no custom schemes.
                match http::browser_url(&url) {
                    Some(safe) => {
                        if let Err(e) = open::that_detached(&safe) {
                            self.notice = Some((true, format!("Could not open browser: {e}")));
                        }
                    }
                    None => {
                        self.notice = Some((true, format!("Refusing to open non-HTTPS link: {url}")));
                    }
                }
                Task::none()
            }
            Message::DismissNotice => {
                self.notice = None;
                Task::none()
            }

            // --------------------------------------------------------- settings
            // Not while an operation runs: the new folder's scan would clean
            // a staging dir another install may be unpacking into.
            Message::SelectFolder(_) | Message::RemoveGameFolder(_) if self.busy => Task::none(),
            Message::SelectFolder(i) => {
                if i < self.manifest.settings.game_folders.len() && i != self.active_folder {
                    self.active_folder = i;
                    self.clear_library();
                    return self.update(Message::Scan);
                }
                Task::none()
            }
            Message::AddGameFolder => Task::perform(
                async move {
                    rfd::AsyncFileDialog::new()
                        .set_title("Select your World of Warcraft folder (or its Interface folder)")
                        .pick_folder()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                Message::GameFolderPicked,
            ),
            Message::GameFolderPicked(Some(path)) => {
                match detect::normalize(&path) {
                    Some(dir) => {
                        let folders = &mut self.manifest.settings.game_folders;
                        if folders.contains(&dir) {
                            self.notice = Some((false, "That folder is already in the list.".into()));
                        } else {
                            folders.push(dir);
                            let first = folders.len() == 1;
                            self.save_manifest();
                            self.notice = Some((false, "Game folder added.".into()));
                            if first {
                                self.active_folder = 0;
                                self.screen = Screen::Library;
                                return self.update(Message::Scan);
                            }
                        }
                    }
                    None => {
                        self.notice = Some((
                            true,
                            "No Interface\\AddOns found there. Pick the game folder (e.g. _retail_) or its Interface folder."
                                .into(),
                        ));
                    }
                }
                Task::none()
            }
            Message::GameFolderPicked(None) => Task::none(),
            Message::RemoveGameFolder(i) => {
                let folders = &mut self.manifest.settings.game_folders;
                if i < folders.len() {
                    folders.remove(i);
                    let len = folders.len();
                    self.save_manifest();
                    if self.active_folder >= len {
                        self.active_folder = 0;
                    }
                    self.clear_library();
                    self.notice = Some((false, "Game folder removed.".into()));
                }
                Task::none()
            }
            Message::KeyChanged(secret, Typed(value)) => {
                if self.key_status.get(&secret) != Some(&KeyStatus::Env) {
                    self.form.input.insert(secret, value);
                }
                Task::none()
            }
            Message::SaveSettings => {
                // Only what the user typed: an untouched field keeps its key.
                // The values leave the form now, whatever the outcome.
                let keys: Vec<(Secret, String)> = std::mem::take(&mut self.form.input)
                    .into_iter()
                    .filter(|(s, v)| !v.trim().is_empty() && self.key_status.get(s) != Some(&KeyStatus::Env))
                    .collect();
                if keys.is_empty() {
                    return Task::none();
                }
                self.busy = true;
                store_keys(keys, true)
            }
            Message::RemoveKey(secret) => {
                if self.busy || self.key_status.get(&secret) != Some(&KeyStatus::Stored) {
                    return Task::none();
                }
                self.busy = true;
                // An empty value deletes the entry (`secrets::store`).
                store_keys(vec![(secret, String::new())], false)
            }
            Message::SetDark(dark) => {
                self.dark = dark;
                Task::none()
            }
            Message::SettingsSaved(leave, result) => {
                self.busy = false;
                match result {
                    Ok(()) => {
                        self.notice =
                            Some((false, if leave { "Keys saved." } else { "Key removed." }.into()));
                        if leave {
                            self.screen = Screen::Library;
                        }
                        // Read the keys again; the check below waits for them.
                        self.creds = None;
                        let then = if self.addons.is_empty() {
                            self.update(Message::Scan)
                        } else {
                            self.start_check(CheckScope::All)
                        };
                        Task::batch([self.load_creds(), then])
                    }
                    Err(e) => {
                        self.notice = Some((true, format!("Could not save keys: {e}")));
                        Task::none()
                    }
                }
            }
        }
    }

    /// GitHub answered 401 to the stored token during this session.
    fn github_token_refused(&self) -> bool {
        self.creds.as_ref().is_some_and(|c| c.github_token_refused())
    }

    /// Whether the user has a key for `secret`, from any origin. Unknown
    /// until the keys are read, and then assumed present so no hint flashes.
    fn has_key(&self, secret: Secret) -> bool {
        self.key_status.get(&secret) != Some(&KeyStatus::NotSet)
    }

    /// Read the API keys on a blocking thread: the OS credential store is
    /// synchronous and may prompt (Secret Service on Linux).
    fn load_creds(&self) -> Task<Message> {
        let manifest = self.manifest.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let creds = checker::credentials(&manifest);
                    let status = secrets::key_statuses(&manifest, &creds);
                    (Arc::new(creds), status)
                })
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!("could not read API keys: {e}");
                    Default::default()
                })
            },
            |(creds, status)| Message::CredsLoaded(creds, status),
        )
    }

    /// Start checking `scope`, or queue it until the keys are read.
    fn start_check(&mut self, scope: CheckScope) -> Task<Message> {
        if self.addons.is_empty() {
            self.busy = false;
            return Task::none();
        }
        // Busy while waiting too: the rows say "Checking…", the buttons wait.
        self.busy = true;
        let Some(creds) = self.creds.clone() else {
            self.pending_check = Some(match self.pending_check.take() {
                Some(queued) => queued.merge(scope),
                None => scope,
            });
            return Task::none();
        };
        let http = self.http.clone();
        let manifest = self.manifest.clone();
        let addons = Arc::clone(&self.addons);
        let flavor = self.flavor;
        let gen = self.lib_gen;
        Task::perform(
            async move {
                match scope {
                    CheckScope::All => checker::check_all(&http, &manifest, &creds, &addons, flavor).await,
                    CheckScope::Folders(folders) => {
                        checker::check_folders(&http, &manifest, &creds, &addons, flavor, &folders).await
                    }
                }
            },
            move |reports| Message::Checked(gen, reports),
        )
    }

    /// Download and inspect `source` for a fresh install (a search result, or
    /// a missing dependency when `needs` names it), answering `Prepared`.
    fn prepare(
        &self,
        creds: Arc<Credentials>,
        dir: PathBuf,
        source: SourceRef,
        name: String,
        needs: Option<String>,
    ) -> Task<Message> {
        let http = self.http.clone();
        let (flavor, client_interface) = (self.flavor, self.client_interface);
        let done = name.clone();
        Task::perform(
            async move {
                // The name picks which of the zip's folders keys the install
                // record (see `install::main_folder`) and which release asset
                // a multi-addon GitHub release offers.
                let target = Target {
                    addons_dir: &dir,
                    key: &name,
                    flavor,
                    client_interface,
                    replacing_version: None,
                };
                let prepared = updater::prepare_new(&http, &creds, &target, &source).await;
                prepared
                    .map(|prepared| {
                        Box::new(PendingInstall {
                            source,
                            name,
                            dir,
                            flavor,
                            client_interface,
                            prepared,
                            needs,
                        })
                    })
                    .map_err(|e| e.to_string())
            },
            move |r| Message::Prepared(done.clone(), r),
        )
    }

    /// Start fetching the icons of the shown results, memory only. The
    /// downloads of the previous search are cancelled.
    fn load_icons(&mut self) -> Task<Message> {
        if let Some(old) = self.discover.icon_batch.take() {
            cancel_icons(&mut self.icons, old);
        }
        let urls: Vec<String> = self
            .discover
            .outcome
            .results
            .iter()
            .filter_map(Found::icon)
            .map(String::from)
            .collect();
        let Some((task, batch)) = self.fetch_icons(urls, false) else {
            return Task::none();
        };
        self.discover.icon_batch = Some(batch);
        task
    }

    /// Ask for the pictures of the library: CurseForge logos of projects
    /// not asked yet this session (needs the key), and WoWInterface
    /// thumbnails from the catalog when it is in memory (never downloaded
    /// for this). Then fetch what is already known.
    fn resolve_library_icons(&mut self) -> Task<Message> {
        if self.addons.is_empty() {
            return Task::none();
        }
        let ids = icons::LibraryIds::of(&self.addons);
        let key = self.creds.as_ref().and_then(|c| c.curseforge.clone());
        let new: Vec<u64> = ids
            .curseforge
            .into_iter()
            .filter(|id| !self.logos_asked.contains(id))
            .collect();
        let logos = match key {
            Some(key) if !new.is_empty() => {
                // Asked now, so a scan meanwhile does not ask again.
                self.logos_asked.extend(&new);
                let http = self.http.clone();
                Task::perform(
                    async move {
                        let found = curseforge::logos(&http, &key, &new).await;
                        (new, found)
                    },
                    |(asked, found)| Message::LogosFound(asked, found),
                )
            }
            _ => Task::none(),
        };
        let catalog = self.catalog.clone();
        let wowi = ids.wowinterface;
        let thumbs = Task::perform(
            async move { catalog.loaded_thumbs(&wowi).await },
            Message::ThumbsFound,
        );
        Task::batch([logos, thumbs, self.refresh_library_icons()])
    }

    /// Recompute which picture each library addon shows and fetch the new
    /// ones, through the disk cache.
    fn refresh_library_icons(&mut self) -> Task<Message> {
        self.library_icons = icons::library_urls(&self.addons, &self.logos, &self.thumbs);
        let urls: Vec<String> = self.library_icons.values().cloned().collect();
        match self.fetch_icons(urls, true) {
            Some((task, batch)) => {
                // Batches with nothing left to deliver are done.
                let icons = &self.icons;
                self.library_batches.retain(|b| {
                    b.urls
                        .iter()
                        .any(|u| matches!(icons.get(u), Some(IconState::Loading)))
                });
                self.library_batches.push(batch);
                task
            }
            None => Task::none(),
        }
    }

    /// Start downloading those of `urls` not known yet, [`ICON_CONCURRENCY`]
    /// at a time, through the disk cache when `disk` is set; `None` when
    /// there is nothing new. Past [`MAX_ICONS`], icons no longer shown in
    /// search or the library are dropped first.
    fn fetch_icons(&mut self, urls: Vec<String>, disk: bool) -> Option<(Task<Message>, IconBatch)> {
        let mut wanted: Vec<String> = urls.into_iter().filter(|u| !self.icons.contains_key(u)).collect();
        wanted.sort();
        wanted.dedup();
        if wanted.is_empty() {
            return None;
        }
        if self.icons.len() + wanted.len() > MAX_ICONS {
            let shown: HashSet<&str> = self
                .discover
                .outcome
                .results
                .iter()
                .filter_map(Found::icon)
                .chain(self.library_icons.values().map(String::as_str))
                .collect();
            self.icons
                .retain(|u, s| matches!(s, IconState::Loading) || shown.contains(u.as_str()));
        }
        for u in &wanted {
            self.icons.insert(u.clone(), IconState::Loading);
        }
        let http = self.http.clone();
        let downloads = stream::iter(wanted.clone())
            .map(move |url| {
                let http = http.clone();
                async move {
                    let icon = if disk {
                        icons::fetch_cached(&http, &url).await
                    } else {
                        icons::fetch(&http, &url).await
                    };
                    (url, icon.map_err(|e| e.to_string()))
                }
            })
            .buffer_unordered(ICON_CONCURRENCY);
        let (task, handle) = Task::run(downloads, |(url, icon)| Message::IconLoaded(url, icon)).abortable();
        let batch = IconBatch {
            _task: handle.abort_on_drop(),
            urls: wanted,
        };
        Some((task, batch))
    }

    /// The decoded picture behind `url`, once it has arrived.
    fn icon(&self, url: Option<&str>) -> Option<image::Handle> {
        match self.icons.get(url?) {
            Some(IconState::Ready(handle)) => Some(handle.clone()),
            _ => None,
        }
    }

    /// Unpack a downloaded search result.
    fn install_pending(&mut self, pending: PendingInstall) -> Task<Message> {
        self.busy = true;
        self.installing = Some(pending.name.clone());
        self.discover.installing = Some(pending.source.clone());
        self.notice = Some((false, format!("Installing {}…", pending.name)));
        let done = pending.name.clone();
        Task::perform(
            async move {
                let p = pending;
                let target = Target {
                    addons_dir: &p.dir,
                    key: &p.name,
                    flavor: p.flavor,
                    client_interface: p.client_interface,
                    replacing_version: None,
                };
                updater::install_prepared(&target, &p.source, &p.prepared)
                    .await
                    .map(|o| o.installed)
                    .map_err(|e| e.to_string())
            },
            move |r| Message::Installed(done.clone(), r),
        )
    }

    /// After a scan: what to check, with statuses adjusted to match. A plain
    /// scan checks everything afresh; one following an install keeps every
    /// other folder's status and checks only the folders it touched.
    fn scope_after_scan(&mut self) -> Option<CheckScope> {
        let Some(touched) = self.recheck.take() else {
            self.statuses.clear();
            return Some(CheckScope::All);
        };
        let present: HashSet<&str> = self.addons.iter().map(|a| a.folder.as_str()).collect();
        self.statuses.retain(|f, _| present.contains(f.as_str()));
        // The family as it is now: an install may have added members.
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        for f in &touched {
            wanted.extend(self.family_folders(f));
        }
        wanted.retain(|f| present.contains(f.as_str()));
        for f in &wanted {
            self.statuses.remove(f);
        }
        (!wanted.is_empty()).then(|| CheckScope::Folders(wanted.into_iter().collect()))
    }

    /// Recompute [`Counts`] from statuses and the library.
    fn recount(&mut self) {
        let mut c = Counts {
            missing_deps: self.missing.values().map(Vec::len).sum(),
            ..Counts::default()
        };
        for s in self.statuses.values() {
            match s {
                UpdateStatus::Outdated(_) => c.outdated += 1,
                UpdateStatus::Manual(_) => c.manual += 1,
                UpdateStatus::Error(_) => c.errors += 1,
                _ => {}
            }
            if s.needs_attention() {
                c.attention += 1;
            }
        }
        c.attention += self.missing.len()
            + self
                .addons
                .iter()
                .filter(|a| a.issues.iter().any(|i| i.is_blocking()))
                .count();
        self.counts = c;
    }

    /// Forget the shown library (another installation is about to be shown).
    fn clear_library(&mut self) {
        self.addons = Arc::new(Vec::new());
        self.families.clear();
        self.head_of.clear();
        self.missing.clear();
        self.statuses.clear();
        self.library_icons.clear();
        for batch in std::mem::take(&mut self.library_batches) {
            cancel_icons(&mut self.icons, batch);
        }
        self.recount();
    }

    fn save_manifest(&self) {
        if let Some(path) = &self.manifest_path {
            if let Err(e) = self.manifest.save(path) {
                tracing::warn!("could not save config: {e}");
            }
        }
    }

    /// Every folder of the family `folder` belongs to (head first).
    fn family_folders(&self, folder: &str) -> Vec<String> {
        self.families
            .iter()
            .find(|f| {
                self.addons[f.head].folder == folder
                    || f.members.iter().any(|&i| self.addons[i].folder == folder)
            })
            .map(|f| {
                std::iter::once(f.head)
                    .chain(f.members.iter().copied())
                    .map(|i| self.addons[i].folder.clone())
                    .collect()
            })
            .unwrap_or_else(|| vec![folder.to_string()])
    }

    fn active_dir(&self) -> Option<&PathBuf> {
        self.manifest.settings.game_folders.get(self.active_folder)
    }

    // ------------------------------------------------------------------ views

    /// Two columns: the fixed sidebar, and the page of the current screen
    /// with the notice under it.
    pub fn view(&self) -> Element<'_, Message> {
        let page: Element<'_, Message> = match self.screen {
            Screen::Library => self.library_page(),
            Screen::Discover => self.discover(),
            Screen::Settings => self.settings(),
        };
        // A pending "replace these folders?" question can come from search or
        // from a missing dependency's Install: shown on every page until
        // answered (the Get addons page places it under its search bar).
        let page: Element<'_, Message> = match &self.discover.confirm {
            Some(p) if self.screen != Screen::Discover => column![self.confirm_card(p), page]
                .spacing(theme::space::PAGE_GAP)
                .into(),
            _ => page,
        };
        let content = column![page, self.footer()]
            .spacing(theme::space::PAGE_GAP)
            .padding(theme::space::CONTENT_PAD)
            .width(Fill)
            .height(Fill);
        container(row![
            self.sidebar(),
            container(space::vertical())
                .width(1)
                .height(Fill)
                .style(theme::divider),
            content,
        ])
        .width(Fill)
        .height(Fill)
        .style(theme::app)
        .into()
    }

    /// Logo, the installations to switch between, and the page nav.
    fn sidebar(&self) -> Element<'_, Message> {
        let lockup = container(image(self.lockup.clone()).height(theme::space::LOCKUP_H))
            .padding(Padding::ZERO.left(4));
        let installs = self
            .manifest
            .settings
            .game_folders
            .iter()
            .enumerate()
            .fold(column![].spacing(6), |col, (i, f)| {
                col.push(self.install_item(i, f))
            });
        let add = button(
            text("+ Add game folder")
                .size(theme::size::META)
                .font(theme::UI_MEDIUM)
                .width(Fill)
                .align_x(Alignment::Center),
        )
        .padding(theme::space::BTN_SM)
        .width(Fill)
        .style(theme::add_folder)
        .on_press(Message::AddGameFolder);
        let nav = column![
            self.nav("Library", Screen::Library),
            self.nav("Get addons", Screen::Discover),
            self.nav("Settings", Screen::Settings),
        ]
        .spacing(4);

        container(
            column![
                lockup,
                widgets::rule(),
                container(
                    text("INSTALLATIONS")
                        .size(theme::size::XS)
                        .font(theme::DISPLAY)
                        .style(theme::gold)
                )
                .padding(Padding::ZERO.left(4)),
                installs,
                add,
                space::vertical(),
                widgets::rule(),
                nav,
            ]
            .spacing(14),
        )
        .width(theme::space::SIDEBAR_W)
        .height(Fill)
        .padding(widgets::pad4(theme::space::SIDEBAR_PAD))
        .style(theme::sidebar)
        .into()
    }

    /// One game folder in the sidebar. Build, addon count and updates are
    /// known only for the one scanned, which is the active one.
    fn install_item<'a>(&'a self, i: usize, dir: &'a Path) -> Element<'a, Message> {
        let active = i == self.active_folder;
        let label = detect::label_of(dir);
        let initials: String = label
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .collect();
        let icon = container(
            text(initials)
                .size(theme::size::CONTROL)
                .font(theme::DISPLAY_BOLD)
                .style(theme::gold),
        )
        .center(theme::space::ICON_HEAD)
        .style(theme::icon_slot);

        let mut info = column![text(label).size(theme::size::CONTROL).font(theme::DISPLAY)].spacing(1);
        if active && !self.addons.is_empty() {
            let meta = match &self.build {
                Some(build) => format!("{build} · {} addons", self.addons.len()),
                None => format!("{} addons", self.addons.len()),
            };
            info = info.push(text(meta).size(theme::size::META).style(theme::muted));
        }
        info = info.push(widgets::one_line(
            text(short_path(dir))
                .size(10)
                .font(theme::MONO)
                .style(theme::faint),
        ));
        // Clipped: the path is cut at the edge instead of wrapping.
        let info = container(info).width(Fill).clip(true);

        let mut item = row![icon, info].spacing(10).align_y(Alignment::Center);
        let updates = self.counts.outdated;
        if active && updates > 0 {
            item = item.push(
                container(
                    // Mono, not Cinzel: Cinzel's "1" reads as an "I".
                    text(updates.to_string()).size(theme::size::XS).font(theme::MONO),
                )
                .padding([1, 5])
                .style(theme::badge(Tone::Update)),
            );
        }
        button(item)
            .padding([8, 10])
            .width(Fill)
            .style(theme::install_item(active))
            .on_press_maybe((!self.busy).then_some(Message::SelectFolder(i)))
            .into()
    }

    fn nav(&self, label: &'static str, screen: Screen) -> Element<'_, Message> {
        button(text(label).size(theme::size::CONTROL).font(theme::DISPLAY))
            .padding([7, 10])
            .width(Fill)
            .style(theme::nav_item(self.screen == screen))
            .on_press(Message::Show(screen))
            .into()
    }

    fn footer(&self) -> Element<'_, Message> {
        let Some((is_error, msg)) = &self.notice else {
            return container(text("")).height(0).into();
        };
        let style: fn(&Theme) -> text::Style = if *is_error {
            |t| theme::tone(Tone::Error)(t)
        } else {
            theme::muted
        };
        row![
            text(msg).size(theme::size::CONTROL).style(style).width(Fill),
            widgets::ghost("×", theme::size::CONTROL, [2, 8]).on_press(Message::DismissNotice),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }
}

/// Shown when an action needs the API keys before they have been read.
const KEYS_LOADING: &str = "Still reading your API keys; try again in a moment.";

/// Write `keys` to the OS credential store on a blocking thread; an empty
/// value deletes the entry.
fn store_keys(keys: Vec<(Secret, String)>, leave: bool) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                for (s, v) in keys {
                    secrets::store(s, &v).map_err(|e| format!("{}: {e}", s.label()))?;
                }
                Ok(())
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        },
        move |r| Message::SettingsSaved(leave, r),
    )
}

/// A game folder as the sidebar shows it: the game's own folder, without
/// `Interface\AddOns`, cut to its last two components (iced does not
/// ellipsize text).
fn short_path(addons_dir: &Path) -> String {
    let game = Some(addons_dir)
        // `/` separates components on Windows too.
        .filter(|d| d.ends_with("Interface/AddOns"))
        .and_then(|d| d.parent()?.parent())
        .unwrap_or(addons_dir);
    let tail: Vec<_> = game.iter().rev().take(2).collect();
    let shown: PathBuf = tail.iter().rev().collect();
    if game.iter().count() > 2 {
        format!("…{}{}", std::path::MAIN_SEPARATOR, shown.display())
    } else {
        shown.display().to_string()
    }
}

/// Sweep old downloads and icons from the cache, fire and forget: a crash
/// or a cancelled install can leave zips behind, and a removed addon its icon.
fn prune_cache() -> Task<Message> {
    Task::future(async {
        let removed =
            tokio::task::spawn_blocking(|| updater::prune_cache(CACHE_MAX_AGE) + icons::prune_cache())
                .await
                .unwrap_or(0);
        tracing::debug!(removed, "pruned the cache");
    })
    .discard()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_path_shows_the_game_folder_tail() {
        let sep = std::path::MAIN_SEPARATOR;
        let dir: PathBuf = ["games", "World of Warcraft", "_retail_", "Interface", "AddOns"]
            .iter()
            .collect();
        assert_eq!(short_path(&dir), format!("…{sep}World of Warcraft{sep}_retail_"));
        let plain: PathBuf = ["wow", "AddOns"].iter().collect();
        assert_eq!(short_path(&plain), format!("wow{sep}AddOns"));
    }
}
