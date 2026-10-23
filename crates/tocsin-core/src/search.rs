//! Find addons that are not installed yet, across every provider at once.
//!
//! Each provider answers with its own list ([`Hit`]s, best first). The lists
//! are merged into [`Found`] entries, one per addon: Details! on CurseForge,
//! on WoWInterface and on GitHub is one entry with three hits, and the user
//! picks which one to install from. CurseForge needs a key; without one it is
//! skipped and reported, and search still covers WoWInterface, Tukui and
//! GitHub.

use std::collections::HashSet;

use crate::addon::{Flavor, InstalledAddon};
use crate::http::Http;
use crate::sources::wowinterface::{self, Catalog};
use crate::sources::{curseforge, github, tukui, Credentials, SourceError, SourceRef};

/// Most results asked of each provider. Twenty per provider is two screens of
/// results and keeps every answer far below the body cap.
pub const PER_PROVIDER: usize = 20;

/// Shortest and longest query sent to a provider.
const QUERY_CHARS: std::ops::RangeInclusive<usize> = 2..=64;

/// How popular a hit is, in the provider's own unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Popularity {
    /// Total downloads (CurseForge, WoWInterface, Tukui).
    Downloads(u64),
    /// Repository stars (GitHub).
    Stars(u64),
}

impl Popularity {
    /// `1.2M downloads`, `228 stars`.
    pub fn describe(self) -> String {
        match self {
            Self::Downloads(n) => format!("{} downloads", compact(n)),
            Self::Stars(n) => format!("{} stars", compact(n)),
        }
    }
}

fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// One provider's answer for one addon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Where to install it from.
    pub source: SourceRef,
    /// Name as the provider shows it.
    pub name: String,
    /// One-line description.
    pub summary: String,
    /// Author names (GitHub: the repository owner).
    pub authors: Vec<String>,
    /// Downloads or stars, when the provider says.
    pub popularity: Option<Popularity>,
    /// Project page.
    pub url: String,
    /// `owner/repo`, lowercase, of the GitHub repository the project links
    /// to. The strongest signal that two providers list the same addon.
    pub repo: Option<String>,
    /// Top-level folders the download installs, when the provider lists
    /// them (WoWInterface, CurseForge, Tukui). Tells which results are
    /// already on disk, and helps recognise the same addon elsewhere.
    pub folders: Vec<String>,
    /// Year of the last update, when known: on WoWInterface especially, a
    /// 2019 upload of an addon now maintained elsewhere is common, and the
    /// year is what lets the user pick the live source.
    pub updated: Option<u16>,
    /// Picture the provider shows for it, when any: CurseForge's project
    /// logo, or WoWInterface's first screenshot thumbnail. Untrusted: only
    /// [`crate::icons::fetch`] may act on it.
    pub icon: Option<String>,
}

/// One addon and every provider that offers it. Never empty; the first hit is
/// the best-ranked one and names the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// One hit per provider, at most.
    pub hits: Vec<Hit>,
}

impl Found {
    /// The hit that names this entry.
    pub fn primary(&self) -> &Hit {
        &self.hits[0]
    }

    /// Every author named by any provider, deduplicated.
    pub fn authors(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for a in self.hits.iter().flat_map(|h| &h.authors) {
            if !out.iter().any(|o| simplify(o) == simplify(a)) {
                out.push(a);
            }
        }
        out
    }

    /// The picture to show for this entry: CurseForge's logo first, as it
    /// is drawn to be the project's icon; WoWInterface's thumbnail only
    /// failing that, since it is a cropped screenshot.
    pub fn icon(&self) -> Option<&str> {
        let of = |pick: fn(&SourceRef) -> bool| {
            self.hits
                .iter()
                .filter(|h| pick(&h.source))
                .find_map(|h| h.icon.as_deref())
        };
        of(|s| matches!(s, SourceRef::CurseForge { .. }))
            .or_else(|| of(|s| matches!(s, SourceRef::WowInterface { .. })))
    }
}

/// What is on disk, as search results are matched against it.
#[derive(Debug, Clone, Default)]
pub struct Installed {
    /// [`SourceRef::project_key`] of every source of every installed addon.
    projects: HashSet<String>,
    /// Lowercase folder names in `AddOns` (parked ones included).
    folders: HashSet<String>,
}

impl Installed {
    /// Index `addons` (a library scan, disabled addons included).
    pub fn new(addons: &[InstalledAddon]) -> Self {
        Self {
            projects: addons
                .iter()
                .flat_map(|a| &a.sources)
                .filter_map(SourceRef::project_key)
                .collect(),
            folders: addons.iter().map(|a| a.folder.to_ascii_lowercase()).collect(),
        }
    }

    /// Whether `found` is on disk already: one of its sources is an
    /// installed addon's, or every folder a hit ships is in `AddOns`, so a
    /// hand-installed copy with no ids in its `.toc` is recognised too.
    pub fn contains(&self, found: &Found) -> bool {
        found.hits.iter().any(|h| {
            h.source.project_key().is_some_and(|k| self.projects.contains(&k))
                || (!h.folders.is_empty()
                    && h.folders
                        .iter()
                        .all(|d| self.folders.contains(&d.to_ascii_lowercase())))
        })
    }
}

/// What a search produced.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// Merged results, most relevant first.
    pub results: Vec<Found>,
    /// Providers not asked because their key is missing.
    pub skipped: Vec<&'static str>,
    /// Providers that failed, as `Provider: reason`.
    pub errors: Vec<String>,
}

/// Trim and collapse a user query; `None` when it is too short or too long to
/// be worth a request. Control characters are dropped: the query ends up in a
/// URL (encoded, but there is no reason to send them at all).
pub fn clean_query(raw: &str) -> Option<String> {
    let q = raw
        .split_whitespace()
        .map(|w| w.chars().filter(|c| !c.is_control()).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    QUERY_CHARS.contains(&q.chars().count()).then_some(q)
}

/// Ask every provider we can for `query`, concurrently (four requests at
/// most, under the checker's bound), and merge the answers. WoWInterface is
/// searched in `catalog`, downloaded on first use.
pub async fn search(
    http: &Http,
    creds: &Credentials,
    catalog: &Catalog,
    query: &str,
    flavor: Flavor,
) -> Outcome {
    let Some(query) = clean_query(query) else {
        return Outcome::default();
    };
    let q = query.as_str();
    let (cf, wi, gh, tk) = tokio::join!(
        async {
            match creds.curseforge.as_deref() {
                Some(key) => Some(curseforge::search(http, key, q, flavor).await),
                None => None,
            }
        },
        wowinterface::search(http, catalog, q, flavor),
        github::search(http, creds, q),
        tukui::search(http, q),
    );

    let mut out = Outcome::default();
    let mut lists = Vec::with_capacity(4);
    let answers = [
        ("Tukui", Some(tk)),
        ("CurseForge", cf),
        ("WoWInterface", Some(wi)),
        ("GitHub", Some(gh)),
    ];
    for (provider, answer) in answers {
        match answer {
            None => out.skipped.push(provider),
            Some(Ok(hits)) => lists.push(hits),
            Some(Err(e)) => {
                tracing::warn!(provider, error = %e, "search failed");
                out.errors.push(format!("{provider}: {}", describe(&e)));
            }
        }
    }
    out.results = merge(lists);
    out
}

/// Short, user-facing reason for a failed search.
fn describe(e: &SourceError) -> String {
    match e {
        SourceError::Http(crate::http::HttpError::Status {
            status: 403 | 429, ..
        }) => "too many searches, try again in a minute".into(),
        SourceError::Http(crate::http::HttpError::Status { status: 401, .. }) => {
            "the API key was refused".into()
        }
        other => other.to_string(),
    }
}

/// Merge per-provider lists (each best first) into one list of addons.
///
/// Hits are taken round-robin by rank, so the first result of every provider
/// is placed before anyone's second: no provider drowns the others. A hit
/// joins an existing entry when that entry has no hit from the same provider
/// yet and one of its hits is [the same addon](same_addon).
///
/// Then entries a catalog lists (CurseForge, WoWInterface, Tukui) go before
/// those only GitHub knows. Searching "details" showed why: an exact name
/// match is no signal on GitHub, where `coatless-quarto/details` (a Quarto
/// extension) and a dozen private-server ports of Details! sort above the
/// real thing, while the catalogs rank by downloads.
pub fn merge(lists: Vec<Vec<Hit>>) -> Vec<Found> {
    let mut iters: Vec<_> = lists.into_iter().map(Vec::into_iter).collect();
    let mut found: Vec<Found> = Vec::new();
    loop {
        let mut any = false;
        for hit in iters.iter_mut().filter_map(Iterator::next) {
            any = true;
            place(&mut found, hit);
        }
        if !any {
            break;
        }
    }
    for f in &mut found {
        // The catalog's name and summary are the better written ones.
        f.hits.sort_by_key(|h| provider_rank(&h.source));
    }
    found.sort_by_key(|f| matches!(f.primary().source, SourceRef::Github { .. }));
    found
}

/// Preferred provider order within an entry: catalogs first.
fn provider_rank(source: &SourceRef) -> u8 {
    match source {
        SourceRef::Tukui { .. } => 0,
        SourceRef::CurseForge { .. } => 1,
        SourceRef::WowInterface { .. } => 2,
        SourceRef::Github { .. } => 3,
        SourceRef::Manual { .. } => 4,
    }
}

fn place(found: &mut Vec<Found>, hit: Hit) {
    let provider = hit.source.provider();
    let home = found.iter_mut().find(|f| {
        !f.hits.iter().any(|h| h.source.provider() == provider) && f.hits.iter().any(|h| same_addon(h, &hit))
    });
    match home {
        Some(f) => f.hits.push(hit),
        None => found.push(Found { hits: vec![hit] }),
    }
}

/// Whether two hits from different providers are the same addon.
///
/// * Both link the same GitHub repository: yes. CurseForge's *Details!
///   Damage Meter* points its source link at `Tercioo/Details-Damage-Meter`,
///   which is also the GitHub hit.
/// * Both link a repository, different ones: no (a fork, or a namesake).
/// * Otherwise the names must match once decoration is gone (`Deadly Boss
///   Mods (DBM)` = `Deadly Boss Mods`) or both must install a folder of the
///   same name (`DBM-Core`), and the authors must not contradict each other:
///   `xorann/BigWigs` is a namesake of BigWigs on CurseForge, not the same
///   addon.
pub fn same_addon(a: &Hit, b: &Hit) -> bool {
    if let (Some(x), Some(y)) = (&a.repo, &b.repo) {
        return x == y;
    }
    let name = simplify_name(&a.name);
    let same_name = !name.is_empty() && name == simplify_name(&b.name);
    let shared_folder = a
        .folders
        .iter()
        .any(|f| b.folders.iter().any(|g| f.eq_ignore_ascii_case(g)));
    (same_name || shared_folder) && authors_agree(&a.authors, &b.authors)
}

/// Unknown authors on either side agree with anything; known ones must share
/// at least one name.
fn authors_agree(a: &[String], b: &[String]) -> bool {
    a.is_empty() || b.is_empty() || a.iter().any(|x| b.iter().any(|y| simplify(x) == simplify(y)))
}

/// Lowercase alphanumerics only: `Details!` → `details`.
pub(crate) fn simplify(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// An addon name without its decoration: WoW color codes (`|cff00ff00`,
/// `|r`) that some authors put in their titles, and parenthesised or
/// bracketed asides (`(DBM)`, `[Classic]`). Falls back to the plain
/// simplification when nothing is left.
pub(crate) fn simplify_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut depth = 0u32;
    let mut chars = name.chars();
    while let Some(c) = chars.next() {
        match c {
            // `|cAARRGGBB` opens a color, `|r` closes it.
            '|' => {
                if let Some('c' | 'C') = chars.next() {
                    chars.by_ref().take(8).for_each(drop);
                }
            }
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    match simplify(&out) {
        s if s.is_empty() => simplify(name),
        s => s,
    }
}

/// `owner/repo` in lowercase, from any GitHub URL a provider hands out.
pub(crate) fn repo_of(url: Option<&str>) -> Option<String> {
    match url.and_then(github::parse_repo_url)? {
        SourceRef::Github { owner, repo, .. } => Some(format!("{owner}/{repo}").to_ascii_lowercase()),
        _ => None,
    }
}

/// Author names from a lenient JSON list: plain strings or `{ "name": … }`.
pub(crate) fn author_names(list: &[serde_json::Value]) -> Vec<String> {
    list.iter()
        .filter_map(|v| v.as_str().or_else(|| v.get("name").and_then(|n| n.as_str())))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Year of an ISO-8601 date (`2026-10-13T…`), when it looks like one.
pub(crate) fn year_of_iso(date: Option<&str>) -> Option<u16> {
    date?.get(..4)?.parse().ok().filter(|y| (1990..=2200).contains(y))
}

/// Year of a Unix time in milliseconds (WoWInterface's `UIDate`).
pub(crate) fn year_of_millis(ms: u64) -> Option<u16> {
    // Civil-from-days (H. Hinnant), enough for a year.
    let days = i64::try_from(ms / 86_400_000).ok()? + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month_index = (5 * doy + 2) / 153; // March-based
    let year = yoe + era * 400 + i64::from(month_index >= 10);
    u16::try_from(year).ok()
}

/// A provider's download count, which may come as a float (`1.2E8`).
pub(crate) fn downloads(n: Option<f64>) -> Option<Popularity> {
    n.filter(|n| n.is_finite() && *n >= 0.0)
        .map(|n| Popularity::Downloads(n as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(source: SourceRef, name: &str, authors: &[&str], repo: Option<&str>) -> Hit {
        Hit {
            source,
            name: name.into(),
            summary: String::new(),
            authors: authors.iter().map(|a| a.to_string()).collect(),
            popularity: None,
            url: String::new(),
            repo: repo.map(str::to_string),
            folders: Vec::new(),
            updated: None,
            icon: None,
        }
    }

    fn gh(owner: &str, repo: &str) -> Hit {
        hit(
            SourceRef::Github {
                owner: owner.into(),
                repo: repo.into(),
                tag_prefix: None,
            },
            repo,
            &[owner],
            Some(&format!("{owner}/{repo}").to_ascii_lowercase()),
        )
    }

    fn cf(id: u64, name: &str, authors: &[&str], repo: Option<&str>) -> Hit {
        hit(SourceRef::CurseForge { id }, name, authors, repo)
    }

    fn wowi(id: u64, name: &str, authors: &[&str], folders: &[&str]) -> Hit {
        Hit {
            folders: folders.iter().map(|f| f.to_string()).collect(),
            ..hit(SourceRef::WowInterface { id }, name, authors, None)
        }
    }

    fn on_disk(folder: &str, sources: Vec<SourceRef>) -> InstalledAddon {
        InstalledAddon {
            folder: folder.into(),
            path: Default::default(),
            toc_path: Default::default(),
            installed: None,
            sources,
            display_name: folder.into(),
            issues: Vec::new(),
            disabled: false,
            toc: crate::Toc::parse(""),
        }
    }

    #[test]
    fn installed_matches_by_project_or_by_every_folder() {
        let installed = Installed::new(&[
            on_disk("Details", vec![SourceRef::CurseForge { id: 61284 }]),
            // Hand-installed, no ids in its `.toc`.
            on_disk("Bagnon", Vec::new()),
            on_disk("Bagnon_Config", Vec::new()),
        ]);
        let found = |hits: Vec<Hit>| Found { hits };
        // Same project on another provider's hit list.
        assert!(installed.contains(&found(vec![
            wowi(1, "Details", &[], &[]),
            cf(61284, "Details!", &[], None)
        ])));
        // Every folder it ships is there, whatever the case.
        assert!(installed.contains(&found(vec![wowi(
            4459,
            "Bagnon",
            &[],
            &["bagnon", "Bagnon_Config"]
        )])));
        // Only some of them: a different download.
        assert!(!installed.contains(&found(vec![wowi(
            4459,
            "Bagnon",
            &[],
            &["Bagnon", "Bagnon_Bank"]
        )])));
        // No folders listed and an unknown project.
        assert!(!installed.contains(&found(vec![gh("Jaliborc", "Bagnon")])));
    }

    #[test]
    fn the_curseforge_logo_wins_over_a_wowinterface_thumbnail() {
        let with = |h: Hit, url: &str| Hit {
            icon: Some(url.into()),
            ..h
        };
        let thumb = "https://cdn-wow.mmoui.com/preview/tiny/pvw20498.png";
        let logo = "https://media.forgecdn.net/avatars/thumbnails/1/2/256/256/3.png";
        // WoWInterface ranks first, CurseForge's logo is still the icon.
        let dbm = Found {
            hits: vec![
                with(wowi(8814, "Deadly Boss Mods", &[], &[]), thumb),
                with(cf(3358, "DBM", &[], None), logo),
            ],
        };
        assert_eq!(dbm.icon(), Some(logo));
        // No CurseForge logo: the thumbnail.
        let bagnon = Found {
            hits: vec![
                cf(1, "Bagnon", &[], None),
                with(wowi(4459, "Bagnon", &[], &[]), thumb),
            ],
        };
        assert_eq!(bagnon.icon(), Some(thumb));
        // Nothing usable: the monogram.
        assert_eq!(
            Found {
                hits: vec![gh("Jaliborc", "Bagnon")]
            }
            .icon(),
            None
        );
    }

    #[test]
    fn queries_are_cleaned_and_bounded() {
        assert_eq!(
            clean_query("  deadly   boss\tmods ").as_deref(),
            Some("deadly boss mods")
        );
        assert_eq!(clean_query("we\u{0}ak").as_deref(), Some("weak"));
        assert_eq!(clean_query("a"), None);
        assert_eq!(clean_query("   "), None);
        assert_eq!(clean_query(&"x".repeat(65)), None);
    }

    #[test]
    fn names_lose_their_decoration() {
        assert_eq!(simplify_name("Details! Damage Meter"), "detailsdamagemeter");
        assert_eq!(simplify_name("Details-Damage-Meter"), "detailsdamagemeter");
        assert_eq!(simplify_name("Deadly Boss Mods (DBM)"), "deadlybossmods");
        assert_eq!(simplify_name("|cff00ccffRareScanner|r"), "rarescanner");
        assert_eq!(simplify_name("(DBM)"), "dbm");
    }

    #[test]
    fn same_addon_by_repo_link_or_by_name_and_author() {
        // CurseForge links its GitHub source: the owner need not be an author.
        let details_cf = cf(
            61284,
            "Details! Damage Meter",
            &["Terciob"],
            Some("tercioo/details-damage-meter"),
        );
        assert!(same_addon(&details_cf, &gh("Tercioo", "Details-Damage-Meter")));
        // Same name, both link a repository, different ones: a fork.
        assert!(!same_addon(&details_cf, &gh("Bunny67", "Details-Damage-Meter")));
        // No links: the name and a shared author decide.
        let dbm_cf = cf(3358, "Deadly Boss Mods (DBM)", &["MysticalOS"], None);
        assert!(same_addon(
            &dbm_cf,
            &wowi(8814, "Deadly Boss Mods", &["mysticalos"], &[])
        ));
        // A namesake by someone else.
        let bw_cf = cf(2382, "BigWigs", &["Funkydude"], None);
        assert!(!same_addon(&bw_cf, &gh("xorann", "BigWigs")));
        // Unknown authors do not veto a name match.
        assert!(same_addon(&bw_cf, &wowi(1, "BigWigs", &[], &[])));
    }

    #[test]
    fn same_addon_by_shared_folder() {
        let dbm_cf = Hit {
            folders: vec!["DBM-Core".into(), "DBM-Raids-Midnight".into()],
            ..cf(3358, "DBM - Deadly Boss Mods", &["MysticalOS"], None)
        };
        assert!(same_addon(
            &dbm_cf,
            &wowi(8814, "DBM Core", &["MysticalOS"], &["dbm-core"])
        ));
        // A shared folder by another author is a plugin or a fork, not it.
        assert!(!same_addon(
            &dbm_cf,
            &wowi(9, "DBM skin", &["someone"], &["DBM-Core"])
        ));
    }

    #[test]
    fn years_from_dates() {
        assert_eq!(year_of_iso(Some("2026-10-13T00:00:00Z")), Some(2026));
        assert_eq!(year_of_iso(Some("soon")), None);
        assert_eq!(year_of_millis(1_553_216_401_000), Some(2019));
        assert_eq!(year_of_millis(1_735_689_599_000), Some(2024)); // 2024-12-31T23:59:59Z
        assert_eq!(year_of_millis(1_735_689_600_000), Some(2025));
    }

    #[test]
    fn merges_across_providers_and_never_within_one() {
        let lists = vec![
            vec![
                cf(
                    61284,
                    "Details! Damage Meter",
                    &["Terciob"],
                    Some("tercioo/details-damage-meter"),
                ),
                cf(99, "Details! Streamer", &["Terciob"], None),
            ],
            vec![wowi(4000, "Details! Damage Meter", &["Terciob"], &["Details"])],
            vec![
                gh("Bunny67", "Details-WotLK"),
                gh("Tercioo", "Details-Damage-Meter"),
                gh("Tercioo", "Details-Framework"),
            ],
        ];
        let found = merge(lists);
        let details = found
            .iter()
            .find(|f| f.primary().name == "Details! Damage Meter")
            .unwrap();
        let providers: Vec<_> = details.hits.iter().map(|h| h.source.provider()).collect();
        assert_eq!(providers, ["CurseForge", "WoWInterface", "GitHub"]);
        // Every hit lands exactly once.
        assert_eq!(found.iter().map(|f| f.hits.len()).sum::<usize>(), 6);
        // Two CurseForge projects with the same name stay two entries.
        let same = merge(vec![vec![
            cf(1, "Bagnon", &["Jaliborc"], None),
            cf(2, "Bagnon", &["Jaliborc"], None),
        ]]);
        assert_eq!(same.len(), 2);
    }

    #[test]
    fn catalog_entries_come_before_github_only_ones() {
        // GitHub answered first and its hit came first in the round-robin.
        let lists = vec![
            vec![
                gh("coatless-quarto", "details"),
                gh("Tercioo", "Details-Damage-Meter"),
            ],
            vec![
                cf(
                    61284,
                    "Details! Damage Meter",
                    &["Terciob"],
                    Some("tercioo/details-damage-meter"),
                ),
                cf(2, "Details! Streamer", &["Terciob"], None),
            ],
        ];
        let found = merge(lists);
        let names: Vec<_> = found.iter().map(|f| f.primary().name.as_str()).collect();
        assert_eq!(names, ["Details! Damage Meter", "Details! Streamer", "details"]);
        // The catalog's hit names the entry and is listed first.
        assert_eq!(found[0].hits[1].source.provider(), "GitHub");
    }

    #[test]
    fn popularity_reads_well() {
        assert_eq!(Popularity::Downloads(950).describe(), "950 downloads");
        assert_eq!(Popularity::Stars(1_234).describe(), "1.2k stars");
        assert_eq!(downloads(Some(1.2e8)), Some(Popularity::Downloads(120_000_000)));
        assert_eq!(downloads(Some(f64::NAN)), None);
    }

    #[test]
    fn authors_from_lenient_json() {
        let v: Vec<serde_json::Value> = serde_json::from_str(r#"["Elv", {"name":"Simpy"}, 3, " "]"#).unwrap();
        assert_eq!(author_names(&v), ["Elv", "Simpy"]);
    }
}
