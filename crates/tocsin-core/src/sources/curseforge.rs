//! CurseForge source (`## X-Curse-Project-ID`).
//!
//! `GET https://api.curseforge.com/v1/mods/{id}` with header `x-api-key`.
//! Keys are free for personal use at <https://console.curseforge.com/>.
//! Set `TOCSIN_CURSEFORGE_KEY`; the key is never written into the repo.

use reqwest::Url;
use serde::Deserialize;

use super::{Release, SourceError};
use crate::addon::Flavor;
use crate::http::Http;
use crate::search::{author_names, downloads, repo_of, year_of_iso, Hit, PER_PROVIDER};
use crate::version::Version;

/// World of Warcraft game id on CurseForge.
const WOW_GAME_ID: u64 = 1;

#[derive(Debug, Deserialize)]
struct Envelope {
    data: Mod,
}

#[derive(Debug, Deserialize)]
struct Mod {
    #[serde(default, rename = "gameId")]
    game_id: u64,
    #[serde(default)]
    links: Links,
    #[serde(default, rename = "latestFiles")]
    latest_files: Vec<File>,
}

#[derive(Debug, Deserialize, Default)]
struct Links {
    #[serde(default, rename = "websiteUrl")]
    website_url: Option<String>,
    /// Repository link, when the author filled it in. Search uses it to
    /// recognise the same addon on GitHub.
    #[serde(default, rename = "sourceUrl")]
    source_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchEnvelope {
    #[serde(default)]
    data: Vec<SearchMod>,
}

#[derive(Debug, Deserialize)]
struct SearchMod {
    id: u64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default, rename = "downloadCount")]
    download_count: Option<f64>,
    #[serde(default)]
    authors: Vec<serde_json::Value>,
    #[serde(default)]
    links: Links,
    /// One entry per game version with a file: enough to tell the flavors
    /// a project serves without its full file list.
    #[serde(default, rename = "latestFilesIndexes")]
    indexes: Vec<FileIndex>,
    /// Only their folder lists (`modules`) are read here.
    #[serde(default, rename = "latestFiles")]
    latest_files: Vec<SearchFile>,
    #[serde(default, rename = "dateModified")]
    date_modified: Option<String>,
    /// The project logo (`ModAsset`); `null` for projects without one.
    #[serde(default)]
    logo: Option<Logo>,
}

#[derive(Debug, Deserialize)]
struct Logo {
    /// A small square rendition of the logo, enough for a list icon.
    #[serde(default, rename = "thumbnailUrl")]
    thumbnail_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchFile {
    #[serde(default)]
    modules: Vec<Module>,
}

#[derive(Debug, Deserialize)]
struct Module {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct FileIndex {
    #[serde(default, rename = "gameVersion")]
    game_version: String,
}

#[derive(Debug, Clone, Deserialize)]
struct File {
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(default, rename = "fileDate")]
    file_date: Option<String>,
    /// 1 = release, 2 = beta, 3 = alpha
    #[serde(default = "default_release_type", rename = "releaseType")]
    release_type: u8,
    #[serde(default, rename = "downloadUrl")]
    download_url: Option<String>,
    /// Game versions the file was built for, e.g. `["12.1.5"]`; CurseForge
    /// also mixes in labels like `"Retail"`, which are ignored.
    #[serde(default, rename = "gameVersions")]
    game_versions: Vec<String>,
    /// `[{ "value": "…", "algo": 1 }]`, algo 1 = sha1, 2 = md5.
    #[serde(default)]
    hashes: Vec<Hash>,
}

#[derive(Debug, Clone, Deserialize)]
struct Hash {
    value: String,
    #[serde(default)]
    algo: u8,
}

/// Whether a file targets `flavor`. Files with no parsable version are
/// accepted: better a possibly wrong answer than none.
fn targets(file: &File, flavor: Flavor) -> bool {
    serves(file.game_versions.iter().map(String::as_str), flavor)
}

/// Whether any of `versions` (`"12.1.5"`, `"Retail"`, …) belongs to
/// `flavor`; true when none parses.
fn serves<'a>(versions: impl Iterator<Item = &'a str>, flavor: Flavor) -> bool {
    let codes: Vec<u32> = versions
        .filter_map(crate::client::parse_version)
        .map(|(ma, mi, pa)| ma * 10000 + mi * 100 + pa)
        .collect();
    codes.is_empty() || codes.iter().any(|&c| Flavor::of_interface(c) == flavor)
}

/// Search WoW projects by name, most popular first, keeping those with a
/// file for `flavor`.
///
/// `GET /v1/mods/search?gameId=1&searchFilter=…&sortField=2&sortOrder=desc`
/// (sort field 2 is popularity), documented at
/// <https://docs.curseforge.com/rest-api/#search-mods>.
pub async fn search(http: &Http, key: &str, query: &str, flavor: Flavor) -> Result<Vec<Hit>, SourceError> {
    let mut url = Url::parse("https://api.curseforge.com/v1/mods/search").expect("static url");
    url.query_pairs_mut()
        .append_pair("gameId", &WOW_GAME_ID.to_string())
        .append_pair("searchFilter", query)
        .append_pair("sortField", "2")
        .append_pair("sortOrder", "desc")
        .append_pair("pageSize", &PER_PROVIDER.to_string());
    // Uncached: one URL per query, the per-run cache would only grow.
    let env: SearchEnvelope = http
        .get_json_uncached(url.as_str(), &[("x-api-key", key)])
        .await?;
    Ok(env
        .data
        .into_iter()
        .filter(|m| serves(m.indexes.iter().map(|i| i.game_version.as_str()), flavor))
        .map(|m| Hit {
            source: super::SourceRef::CurseForge { id: m.id },
            url: m
                .links
                .website_url
                .unwrap_or_else(|| format!("https://www.curseforge.com/projects/{}", m.id)),
            repo: repo_of(m.links.source_url.as_deref()),
            folders: m
                .latest_files
                .iter()
                .map(|f| &f.modules)
                .find(|mods| !mods.is_empty())
                .map(|mods| {
                    mods.iter()
                        .map(|m| m.name.clone())
                        .filter(|n| !n.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            updated: year_of_iso(m.date_modified.as_deref()),
            icon: m.logo.and_then(|l| l.thumbnail_url).filter(|u| !u.is_empty()),
            name: m.name,
            summary: m.summary,
            authors: author_names(&m.authors),
            popularity: downloads(m.download_count),
        })
        .collect())
}

/// Projects asked per logo request: the 20 mods of a search page fit the
/// 1 MiB answer cap, and `POST /v1/mods` answers the same full objects.
pub const LOGOS_PER_REQUEST: usize = 20;

#[derive(Debug, serde::Serialize)]
struct GetModsBody<'a> {
    #[serde(rename = "modIds")]
    mod_ids: &'a [u64],
    #[serde(rename = "filterPcOnly")]
    filter_pc_only: bool,
}

#[derive(Debug, Deserialize)]
struct LogosEnvelope {
    #[serde(default)]
    data: Vec<LogoMod>,
}

/// Only what a logo lookup needs of a mod.
#[derive(Debug, Deserialize)]
struct LogoMod {
    id: u64,
    #[serde(default, rename = "gameId")]
    game_id: u64,
    #[serde(default)]
    logo: Option<Logo>,
}

/// The logo thumbnail of each project in `ids` that has one, as
/// `(id, url)`, asked [`LOGOS_PER_REQUEST`] at a time:
/// `POST /v1/mods` with `{"modIds": [...]}`, documented at
/// <https://docs.curseforge.com/rest-api/#get-mods>. A request that fails
/// is skipped (its projects keep their monogram); the others still answer.
pub async fn logos(http: &Http, key: &str, ids: &[u64]) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    for chunk in ids.chunks(LOGOS_PER_REQUEST) {
        let body = GetModsBody {
            mod_ids: chunk,
            filter_pc_only: true,
        };
        match http
            .post_json_uncached::<_, LogosEnvelope>(
                "https://api.curseforge.com/v1/mods",
                &[("x-api-key", key)],
                &body,
            )
            .await
        {
            Ok(env) => out.extend(logos_of(env)),
            Err(e) => tracing::warn!(error = %e, "CurseForge logo lookup failed"),
        }
    }
    out
}

/// `(id, thumbnail)` of each WoW project in the answer that has a logo.
fn logos_of(env: LogosEnvelope) -> impl Iterator<Item = (u64, String)> {
    env.data
        .into_iter()
        .filter(|m| m.game_id == 0 || m.game_id == WOW_GAME_ID)
        .filter_map(|m| {
            let url = m.logo?.thumbnail_url.filter(|u| !u.is_empty())?;
            Some((m.id, url))
        })
}

fn default_release_type() -> u8 {
    1
}

/// Fetch the newest *release*-type file of mod `id`.
pub async fn latest(http: &Http, key: &str, id: u64, flavor: Flavor) -> Result<Release, SourceError> {
    let url = format!("https://api.curseforge.com/v1/mods/{id}");
    let env: Envelope = http.get_json(&url, &[("x-api-key", key)]).await?;
    if env.data.game_id != 0 && env.data.game_id != WOW_GAME_ID {
        return Err(SourceError::NoRelease);
    }
    let page = env
        .data
        .links
        .website_url
        .unwrap_or_else(|| format!("https://www.curseforge.com/projects/{id}"));
    // Newest release-type file by date (ISO-8601 strings sort lexicographically).
    let file = env
        .data
        .latest_files
        .into_iter()
        .filter(|f| f.release_type == 1 && targets(f, flavor))
        .max_by(|a, b| a.file_date.cmp(&b.file_date))
        .ok_or(SourceError::NoRelease)?;
    let digest = file.hashes.iter().find_map(|h| match h.algo {
        1 => Some(super::Digest::Sha1(h.value.clone())),
        2 => Some(super::Digest::Md5(h.value.clone())),
        _ => None,
    });
    Ok(Release {
        version: Version::parse(&file.display_name),
        url: page,
        published_at: file.file_date,
        download: file.download_url,
        digest,
        provider: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_envelope() {
        let json = r#"{"data":{"id":1472162,"gameId":1,"links":{"websiteUrl":"https://www.curseforge.com/wow/addons/routine"},
          "latestFiles":[{"displayName":"v12.0.376","fileDate":"2026-10-19T00:00:00Z","releaseType":1},
                         {"displayName":"v12.0.380-beta","fileDate":"2026-10-20T00:00:00Z","releaseType":2}]}}"#;
        let e: Envelope = serde_json::from_str(json).unwrap();
        let stable = e.data.latest_files.iter().filter(|f| f.release_type == 1).count();
        assert_eq!(stable, 1);
    }

    #[test]
    fn decodes_search_and_filters_by_flavor() {
        let json = r#"{"data":[
          {"id":61284,"name":"Details! Damage Meter","summary":"Computes damage","downloadCount":3.1E8,
           "authors":[{"id":1,"name":"Terciob","url":"x"}],
           "links":{"websiteUrl":"https://www.curseforge.com/wow/addons/details","sourceUrl":"https://github.com/Tercioo/Details-Damage-Meter"},
           "latestFilesIndexes":[{"gameVersion":"12.0.1","fileId":1},{"gameVersion":"1.15.8","fileId":2}],
           "latestFiles":[{"displayName":"x","modules":[{"name":"Details","fingerprint":1},{"name":"Details_DataStorage"}]}],
           "dateModified":"2026-10-19T08:00:00.000Z",
           "logo":{"id":9,"modId":61284,"title":"logo.png","description":"",
             "thumbnailUrl":"https://media.forgecdn.net/avatars/thumbnails/9/9/256/256/1.png",
             "url":"https://media.forgecdn.net/avatars/9/9/1.png"}},
          {"id":7,"name":"Era only","latestFilesIndexes":[{"gameVersion":"1.15.8"}],"logo":null},
          {"id":8}],
          "pagination":{"index":0,"pageSize":20,"resultCount":3,"totalCount":3}}"#;
        let env: SearchEnvelope = serde_json::from_str(json).unwrap();
        let retail: Vec<&SearchMod> = env
            .data
            .iter()
            .filter(|m| serves(m.indexes.iter().map(|i| i.game_version.as_str()), Flavor::Retail))
            .collect();
        // The era-only project is dropped; one with no index at all is kept.
        assert_eq!(retail.iter().map(|m| m.id).collect::<Vec<_>>(), [61284, 8]);
        let d = &env.data[0];
        assert_eq!(
            repo_of(d.links.source_url.as_deref()).as_deref(),
            Some("tercioo/details-damage-meter")
        );
        assert_eq!(author_names(&d.authors), ["Terciob"]);
        assert_eq!(d.latest_files[0].modules[1].name, "Details_DataStorage");
        assert_eq!(year_of_iso(d.date_modified.as_deref()), Some(2026));
        // The thumbnail, not the full-size logo; `null` and absent are none.
        let thumb = |m: &SearchMod| m.logo.as_ref().and_then(|l| l.thumbnail_url.clone());
        assert_eq!(
            thumb(d).as_deref(),
            Some("https://media.forgecdn.net/avatars/thumbnails/9/9/256/256/1.png")
        );
        assert_eq!(thumb(&env.data[1]), None);
        assert_eq!(thumb(&env.data[2]), None);
    }

    #[test]
    fn logo_lookup_keeps_wow_projects_with_a_thumbnail() {
        // Shaped like `POST /v1/mods`: full mod objects, only id and logo read.
        let json = r#"{"data":[
          {"id":1472162,"gameId":1,"name":"Routine","links":{},"latestFiles":[{"displayName":"v1"}],
           "logo":{"id":1,"modId":1472162,"thumbnailUrl":"https://media.forgecdn.net/avatars/thumbnails/1/2/256/256/3.png",
                   "url":"https://media.forgecdn.net/avatars/1/2/3.png"}},
          {"id":1733527,"gameId":1,"logo":null},
          {"id":5,"gameId":1,"logo":{"id":2,"thumbnailUrl":""}},
          {"id":6,"gameId":432,"logo":{"thumbnailUrl":"https://media.forgecdn.net/minecraft.png"}}]}"#;
        let env: LogosEnvelope = serde_json::from_str(json).unwrap();
        let got: Vec<(u64, String)> = logos_of(env).collect();
        assert_eq!(
            got,
            [(
                1472162,
                "https://media.forgecdn.net/avatars/thumbnails/1/2/256/256/3.png".to_string()
            )]
        );
        let body = serde_json::to_string(&GetModsBody {
            mod_ids: &[1, 2],
            filter_pc_only: true,
        })
        .unwrap();
        assert_eq!(body, r#"{"modIds":[1,2],"filterPcOnly":true}"#);
    }

    #[test]
    fn picks_the_file_for_the_flavor() {
        let retail = File {
            display_name: "v1".into(),
            file_date: None,
            release_type: 1,
            download_url: None,
            game_versions: vec!["12.1.5".into(), "Retail".into()],
            hashes: vec![],
        };
        let era = File {
            game_versions: vec!["1.15.9".into()],
            ..retail.clone()
        };
        let unknown = File {
            game_versions: vec![],
            ..retail.clone()
        };
        assert!(targets(&retail, Flavor::Retail));
        assert!(!targets(&retail, Flavor::Vanilla));
        assert!(targets(&era, Flavor::Vanilla));
        assert!(targets(&unknown, Flavor::Classic));
    }
}
