//! GitHub Releases source. Public, no key required (60 req/h per IP; a token
//! raises that to 5000).

use reqwest::Url;
use serde::Deserialize;

use serde::de::DeserializeOwned;

use super::{Credentials, Release, SourceError, SourceRef};
use crate::addon::Flavor;
use crate::http::Http;
use crate::http::HttpError;
use crate::search::{year_of_iso, Hit, Popularity, PER_PROVIDER};
use crate::version::Version;

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    #[serde(default)]
    browser_download_url: Option<String>,
    #[serde(default)]
    digest: Option<String>,
}

/// Suffixes release authors put on classic-flavor zips (`WeakAuras-5.22.1-cata.zip`).
const CLASSIC_SUFFIXES: &[(&str, Flavor)] = &[
    ("-vanilla", Flavor::Vanilla),
    ("-classic", Flavor::Vanilla),
    ("-era", Flavor::Vanilla),
    ("-bcc", Flavor::Classic),
    ("-tbc", Flavor::Classic),
    ("-wrath", Flavor::Classic),
    ("-wotlk", Flavor::Classic),
    ("-cata", Flavor::Classic),
    ("-mists", Flavor::Classic),
    ("-mop", Flavor::Classic),
    ("-forever", Flavor::Forever),
];

/// Flavor a zip asset name advertises, if any (`None` = plain/retail build).
fn asset_flavor(name: &str) -> Option<Flavor> {
    let stem = name.to_ascii_lowercase();
    let stem = stem.strip_suffix(".zip").unwrap_or(&stem);
    CLASSIC_SUFFIXES
        .iter()
        .find(|(suf, _)| stem.ends_with(suf))
        .map(|(_, f)| *f)
}

/// Choose the zip to download for `flavor`, preferring one named after the
/// addon folder when a release carries several addons.
fn pick_asset<'a>(assets: &'a [GhAsset], folder: &str, flavor: Flavor) -> Option<&'a GhAsset> {
    let folder = folder.to_ascii_lowercase();
    let zips: Vec<&GhAsset> = assets
        .iter()
        .filter(|a| a.browser_download_url.is_some())
        .filter(|a| a.name.to_ascii_lowercase().ends_with(".zip"))
        .filter(|a| !a.name.to_ascii_lowercase().contains("nolib"))
        .collect();
    let for_flavor = |a: &&GhAsset| match (asset_flavor(&a.name), flavor) {
        (None, Flavor::Retail) => true,
        (Some(f), g) => f == g,
        // A plain zip usually bundles every flavor's .toc; accept it when
        // nothing flavor-specific exists.
        (None, _) => !zips.iter().any(|z| asset_flavor(&z.name) == Some(flavor)),
    };
    let candidates: Vec<&GhAsset> = zips.iter().copied().filter(for_flavor).collect();
    candidates
        .iter()
        .find(|a| !folder.is_empty() && a.name.to_ascii_lowercase().starts_with(&folder))
        .or_else(|| candidates.first())
        .copied()
}

/// What we know about the installed addon, used to pick the right release
/// family when one repository publishes several addons.
#[derive(Debug, Clone, Copy)]
pub struct Hint<'a> {
    /// Addon folder name, e.g. `RoutineData`.
    pub folder: &'a str,
    /// Installed version, if known.
    pub installed: Option<&'a Version>,
    /// Game flavor of the folder being checked (providers that publish one
    /// file per flavor pick accordingly).
    pub flavor: Flavor,
}

impl Default for Hint<'_> {
    fn default() -> Self {
        Self {
            folder: "",
            installed: None,
            flavor: Flavor::Retail,
        }
    }
}

/// Fetch the newest non-draft, non-prerelease release for this addon.
///
/// Some repositories publish several addons from one repo with different tag
/// prefixes (e.g. `v12.0.376` for Routine and `dv12.0.13` for Routine Data), so
/// `/releases/latest` is not enough. We list recent releases, group them by tag
/// *family* (the non-numeric prefix: `v`, `dv`, `` …) and choose the family
/// with an explicit `tag_prefix` when given, otherwise automatically:
///
/// 1. a family whose release assets are named after the addon folder;
/// 2. the family whose latest version shares the most leading components with
///    the installed version;
/// 3. the plain `v`/no-prefix family.
pub async fn latest(
    http: &Http,
    creds: &Credentials,
    owner: &str,
    repo: &str,
    tag_prefix: Option<&str>,
    hint: Hint<'_>,
) -> Result<Release, SourceError> {
    if !valid_segment(owner) || !valid_segment(repo) {
        return Err(SourceError::NoRelease);
    }
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases?per_page=30");
    let releases: Vec<GhRelease> = get(http, creds, &url, Cache::PerRun).await?;
    pick(releases, tag_prefix, hint).ok_or(SourceError::NoRelease)
}

/// Whether a GitHub answer may be served from the per-run cache.
#[derive(Clone, Copy)]
enum Cache {
    PerRun,
    Never,
}

/// GET from api.github.com with the user's token when there is one.
///
/// A token is optional on GitHub: it only raises the rate limit. An expired
/// or revoked one makes GitHub answer 401 to everything, which turned every
/// GitHub-sourced addon into an error (seen with a stale token in October
/// 2026), although the very same requests succeed anonymously. So a 401 to a
/// request that carried a token marks the token refused for the session and
/// the request is repeated once without it.
async fn get<T: DeserializeOwned>(
    http: &Http,
    creds: &Credentials,
    url: &str,
    cache: Cache,
) -> Result<T, HttpError> {
    let token = creds.github_token();
    match get_with(http, url, token, cache).await {
        Err(e) if retry_anonymously(&e, token.is_some()) => {
            creds.refuse_github_token();
            tracing::warn!("GitHub refused the token; continuing without it");
            get_with(http, url, None, cache).await
        }
        other => other,
    }
}

/// A 401 to an authenticated request: the token, not the request, is at fault.
fn retry_anonymously(e: &HttpError, sent_token: bool) -> bool {
    sent_token && matches!(e, HttpError::Status { status: 401, .. })
}

async fn get_with<T: DeserializeOwned>(
    http: &Http,
    url: &str,
    token: Option<&str>,
    cache: Cache,
) -> Result<T, HttpError> {
    let mut headers: Vec<(&str, &str)> = vec![("X-GitHub-Api-Version", "2022-11-28")];
    let auth;
    if let Some(t) = token {
        auth = format!("Bearer {t}");
        headers.push(("Authorization", &auth));
    }
    match cache {
        Cache::PerRun => http.get_json(url, &headers).await,
        Cache::Never => http.get_json_uncached(url, &headers).await,
    }
}

/// One tag family: its prefix, newest release and whether any of its assets
/// is named after the addon folder.
struct Family {
    prefix: String,
    best: Release,
    asset_match: bool,
}

fn pick(releases: Vec<GhRelease>, tag_prefix: Option<&str>, hint: Hint<'_>) -> Option<Release> {
    let folder = hint.folder.to_ascii_lowercase();
    // A user writes the prefix as it appears in the tag ("dv", "data-v");
    // normalise it the same way tags are so both spellings match.
    let wanted = tag_prefix.map(normalize_prefix);
    let mut families: Vec<Family> = Vec::new();

    for r in releases.into_iter().filter(|r| !r.draft && !r.prerelease) {
        let Some((prefix, rest)) = split_tag(&r.tag_name) else {
            continue;
        };
        if wanted.is_some_and(|p| p != prefix) {
            continue;
        }
        let asset_match = !folder.is_empty()
            && r.assets
                .iter()
                .any(|a| a.name.to_ascii_lowercase().starts_with(&folder));
        let asset = pick_asset(&r.assets, hint.folder, hint.flavor);
        let release = Release {
            version: Version::parse(rest),
            url: r.html_url,
            published_at: r.published_at,
            download: asset.and_then(|a| a.browser_download_url.clone()),
            digest: asset
                .and_then(|a| a.digest.as_deref())
                .and_then(|d| d.strip_prefix("sha256:"))
                .map(|h| super::Digest::Sha256(h.to_string())),
            provider: String::new(),
        };
        match families.iter_mut().find(|f| f.prefix == prefix) {
            Some(f) => {
                if release.version > f.best.version {
                    f.best = release;
                }
                f.asset_match |= asset_match;
            }
            None => families.push(Family {
                prefix: prefix.to_string(),
                best: release,
                asset_match,
            }),
        }
    }
    if families.is_empty() {
        return None;
    }
    if families.len() == 1 || tag_prefix.is_some() {
        return families
            .into_iter()
            .max_by(|a, b| a.best.version.cmp(&b.best.version))
            .map(|f| f.best);
    }

    // 1) asset named after the folder
    if let Some(f) = families.iter().find(|f| f.asset_match) {
        return Some(f.best.clone());
    }
    // 2) closest to the installed version
    if let Some(installed) = hint.installed {
        let score = |f: &Family| {
            f.best
                .version
                .numbers()
                .iter()
                .zip(installed.numbers())
                .take_while(|(a, b)| a == b)
                .count()
        };
        if let Some(f) = families.iter().max_by_key(|f| score(f)) {
            if score(f) > 0 {
                return Some(f.best.clone());
            }
        }
    }
    // 3) plain versions
    families
        .iter()
        .find(|f| f.prefix.is_empty() || f.prefix == "v")
        .or_else(|| families.first())
        .map(|f| f.best.clone())
}

/// Split `dv12.0.13` into `("dv", "12.0.13")`, `v1.2` into `("", "1.2")`
/// (a lone `v` is not a family of its own), `1.2` into `("", "1.2")`.
/// Tags with no digits (`nightly`) yield `None`.
fn split_tag(tag: &str) -> Option<(&str, &str)> {
    let at = tag.find(|c: char| c.is_ascii_digit())?;
    let (prefix, rest) = tag.split_at(at);
    Some((normalize_prefix(prefix), rest))
}

/// `dv` → `d`, `data-v` → `data`, `v` → ``: the family name without the
/// conventional `v` and separators.
fn normalize_prefix(prefix: &str) -> &str {
    let p = prefix.strip_suffix('v').unwrap_or(prefix);
    p.trim_end_matches(['-', '_', '/'])
}

#[derive(Debug, Deserialize)]
struct RepoSearch {
    #[serde(default)]
    items: Vec<Repo>,
}

#[derive(Debug, Deserialize)]
struct Repo {
    name: String,
    owner: Owner,
    #[serde(default)]
    description: Option<String>,
    html_url: String,
    #[serde(default)]
    stargazers_count: u64,
    #[serde(default)]
    pushed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Owner {
    login: String,
}

/// The search qualifiers that make GitHub's repository search useful for
/// addons without a key. Topics were tried first and miss too much: Details!
/// and Bagnon carry no `wow-addon` topic, and GitHub search has no OR between
/// qualifiers. Every addon is Lua, and matching the name only keeps
/// libraries that merely mention the addon out of the way.
const QUALIFIERS: &str = "in:name language:lua archived:false";

/// Search repositories by name, most starred first.
///
/// `GET /search/repositories?q=…&sort=stars`. Unauthenticated, GitHub allows
/// ten searches a minute per IP; the GUI searches on Enter, never per
/// keystroke. Whether a hit really ships a release zip is only known at
/// install time.
pub async fn search(http: &Http, creds: &Credentials, query: &str) -> Result<Vec<Hit>, SourceError> {
    let mut url = Url::parse("https://api.github.com/search/repositories").expect("static url");
    url.query_pairs_mut()
        .append_pair("q", &format!("{query} {QUALIFIERS}"))
        .append_pair("sort", "stars")
        .append_pair("per_page", &PER_PROVIDER.to_string());
    // Uncached: one URL per query, the per-run cache would only grow.
    let found: RepoSearch = get(http, creds, url.as_str(), Cache::Never).await?;
    Ok(found.items.into_iter().filter_map(repo_hit).collect())
}

fn repo_hit(r: Repo) -> Option<Hit> {
    if !valid_segment(&r.owner.login) || !valid_segment(&r.name) {
        return None;
    }
    Some(Hit {
        repo: Some(format!("{}/{}", r.owner.login, r.name).to_ascii_lowercase()),
        source: SourceRef::Github {
            owner: r.owner.login.clone(),
            repo: r.name.clone(),
            tag_prefix: None,
        },
        summary: r.description.unwrap_or_default(),
        authors: vec![r.owner.login],
        popularity: Some(Popularity::Stars(r.stargazers_count)),
        // Folders are only known from a release zip, not from search.
        folders: Vec::new(),
        updated: year_of_iso(r.pushed_at.as_deref()),
        // The owner's avatar is a person or an organisation, not the addon.
        icon: None,
        // Whether a release zip exists is only known at install time.
        site_only: false,
        url: r.html_url,
        name: r.name,
    })
}

/// Owner/repo names: GitHub allows alphanumerics, `-`, `_` and `.`.
fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && s != "."
        && s != ".."
}

/// Recognise `https://github.com/owner/repo[/...]` in a `## X-Website` line.
pub fn parse_repo_url(url: &str) -> Option<SourceRef> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("https://www.github.com/"))?;
    let mut parts = rest.split('/').filter(|p| !p.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?.trim_end_matches(".git");
    if !valid_segment(owner) || !valid_segment(repo) {
        return None;
    }
    Some(SourceRef::Github {
        owner: owner.to_string(),
        repo: repo.to_string(),
        tag_prefix: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls() {
        let r = parse_repo_url("https://github.com/LoyalFTW/Midnight-Routine/releases").unwrap();
        assert_eq!(
            r,
            SourceRef::Github {
                owner: "LoyalFTW".into(),
                repo: "Midnight-Routine".into(),
                tag_prefix: None,
            }
        );
        assert!(parse_repo_url("https://github.com/only-owner").is_none());
        assert!(parse_repo_url("https://gitlab.com/a/b").is_none());
        assert!(parse_repo_url("https://github.com/a/../etc").is_none());
        assert!(parse_repo_url("https://github.com/a/b%2F..").is_none());
    }

    #[test]
    fn decodes_search() {
        let json = r#"{"total_count":2,"incomplete_results":false,"items":[
            {"name":"Details-Damage-Meter","full_name":"Tercioo/Details-Damage-Meter",
             "owner":{"login":"Tercioo"},"html_url":"https://github.com/Tercioo/Details-Damage-Meter",
             "description":null,"stargazers_count":228,"topics":[]},
            {"name":"..","owner":{"login":"x"},"html_url":"https://github.com/x/.."}]}"#;
        let s: RepoSearch = serde_json::from_str(json).unwrap();
        let hits: Vec<Hit> = s.items.into_iter().filter_map(repo_hit).collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].repo.as_deref(), Some("tercioo/details-damage-meter"));
        assert_eq!(hits[0].authors, ["Tercioo"]);
        assert_eq!(hits[0].popularity, Some(Popularity::Stars(228)));
        assert_eq!(hits[0].summary, "");
    }

    #[test]
    fn a_refused_token_is_dropped_for_everyone_holding_the_credentials() {
        let status = |status| HttpError::Status {
            host: "api.github.com".into(),
            status,
        };
        // Only a 401 to a request that carried a token retries.
        assert!(retry_anonymously(&status(401), true));
        assert!(!retry_anonymously(&status(401), false));
        assert!(!retry_anonymously(&status(403), true)); // rate limit: the token is fine
        assert!(!retry_anonymously(&status(404), true));

        let creds = Credentials {
            github: Some("ghp_expired".into()),
            ..Default::default()
        };
        let held_by_gui = creds.clone();
        assert_eq!(creds.github_token(), Some("ghp_expired"));
        creds.refuse_github_token();
        assert_eq!(held_by_gui.github_token(), None);
        assert!(held_by_gui.github_token_refused());
        assert!(format!("{held_by_gui:?}").contains("refused"));
        // Without a token there is nothing to refuse.
        let none = Credentials::default();
        none.refuse_github_token();
        assert!(!none.github_token_refused());
    }

    #[test]
    fn splits_tags() {
        assert_eq!(split_tag("v12.0.376"), Some(("", "12.0.376")));
        assert_eq!(split_tag("12.0.376"), Some(("", "12.0.376")));
        assert_eq!(split_tag("dv12.0.13"), Some(("d", "12.0.13")));
        assert_eq!(split_tag("data-v1.2"), Some(("data", "1.2")));
        assert_eq!(split_tag("nightly"), None);
    }

    fn rel(tag: &str, assets: &[&str]) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            html_url: format!("https://github.com/x/y/releases/tag/{tag}"),
            published_at: None,
            draft: false,
            prerelease: false,
            assets: assets
                .iter()
                .map(|a| GhAsset {
                    name: (*a).into(),
                    browser_download_url: Some(format!("https://github.com/x/y/releases/download/{tag}/{a}")),
                    digest: None,
                })
                .collect(),
        }
    }

    fn routine_releases() -> Vec<GhRelease> {
        vec![
            rel("dv12.0.13", &["RoutineData-dv12.0.13.zip"]),
            rel("v12.0.376", &["MidnightRoutine-v12.0.376.zip"]),
            rel("v12.0.351", &["MidnightRoutine-v12.0.351.zip"]),
            rel("dv12.0.12", &["RoutineData-dv12.0.12.zip"]),
        ]
    }

    #[test]
    fn picks_family_by_asset_name() {
        let v = Version::parse("v12.0.13");
        let h = Hint {
            folder: "RoutineData",
            installed: Some(&v),
            ..Hint::default()
        };
        assert_eq!(
            pick(routine_releases(), None, h).unwrap().version.as_str(),
            "12.0.13"
        );
        let v = Version::parse("v12.0.376");
        let h = Hint {
            folder: "MidnightRoutine",
            installed: Some(&v),
            ..Hint::default()
        };
        assert_eq!(
            pick(routine_releases(), None, h).unwrap().version.as_str(),
            "12.0.376"
        );
    }

    #[test]
    fn picks_the_asset_for_the_flavor_and_folder() {
        let names = [
            "WeakAuras-5.22.1.zip",
            "WeakAuras-5.22.1-bcc.zip",
            "WeakAuras-5.22.1-cata.zip",
            "WeakAuras-5.22.1-classic.zip",
            "WeakAuras-5.22.1-mists.zip",
            "WeakAuras-5.22.1-nolib.zip",
        ];
        let r = rel("5.22.1", &names);
        let pick = |fl| pick_asset(&r.assets, "WeakAuras", fl).map(|a| a.name.as_str());
        assert_eq!(pick(Flavor::Retail), Some("WeakAuras-5.22.1.zip"));
        assert_eq!(pick(Flavor::Vanilla), Some("WeakAuras-5.22.1-classic.zip"));
        assert_eq!(pick(Flavor::Classic), Some("WeakAuras-5.22.1-bcc.zip"));
        // Plain zip bundling every flavor: fine for Classic when nothing better exists.
        let r = rel("v427.7", &["BigWigs-v427.7.zip"]);
        assert_eq!(
            pick_asset(&r.assets, "BigWigs", Flavor::Classic).map(|a| a.name.as_str()),
            Some("BigWigs-v427.7.zip")
        );
        // Two addons in one release: the folder name picks.
        let r = rel(
            "v12.0.13",
            &["MidnightRoutine-v12.0.376.zip", "RoutineData-dv12.0.13.zip"],
        );
        assert_eq!(
            pick_asset(&r.assets, "RoutineData", Flavor::Retail).map(|a| a.name.as_str()),
            Some("RoutineData-dv12.0.13.zip")
        );
    }

    #[test]
    fn picks_family_by_version_affinity_without_assets() {
        let releases = vec![
            rel("dv12.0.13", &[]),
            rel("v12.0.376", &[]),
            rel("v12.0.351", &[]),
        ];
        let v = Version::parse("v12.0.13");
        let h = Hint {
            folder: "Whatever",
            installed: Some(&v),
            ..Hint::default()
        };
        assert_eq!(pick(releases, None, h).unwrap().version.as_str(), "12.0.13");
    }

    #[test]
    fn explicit_prefix_and_plain_fallback() {
        let h = Hint::default();
        assert_eq!(
            pick(routine_releases(), Some("d"), h).unwrap().version.as_str(),
            "12.0.13"
        );
        assert_eq!(
            pick(routine_releases(), Some("dv"), h).unwrap().version.as_str(),
            "12.0.13"
        );
        assert_eq!(
            pick(routine_releases(), Some("v"), h).unwrap().version.as_str(),
            "12.0.376"
        );
        assert_eq!(
            pick(routine_releases(), None, h).unwrap().version.as_str(),
            "12.0.376"
        );
        assert!(pick(vec![rel("nightly", &[])], None, h).is_none());
    }
}
