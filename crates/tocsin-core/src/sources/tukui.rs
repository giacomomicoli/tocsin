//! Tukui source: ElvUI and Tukui are distributed from tukui.org, not from
//! CurseForge. Their `.toc` carries `## X-Tukui-ProjectID: 2` and
//! `## X-Website: https://tukui.org/elvui`.
//!
//! `GET https://api.tukui.org/v1/addon/{slug}` — public, no key. Fields of
//! interest: `version`, `url` (zip), `web_url`, `last_update`.

use serde::Deserialize;

use super::{Release, SourceError};
use crate::addon::{Compat, Flavor};
use crate::http::Http;
use crate::page::{Page, Shot};
use crate::search::{downloads, interface_codes, repo_of, simplify, year_of_iso, Hit};
use crate::version::Version;

#[derive(Debug, Deserialize)]
struct TukuiAddon {
    version: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    web_url: Option<String>,
    #[serde(default)]
    last_update: Option<String>,
    /// One line about the addon, for the library.
    #[serde(default)]
    small_desc: Option<String>,
    // The addon page reads these from the same answer.
    #[serde(default)]
    name: String,
    #[serde(default)]
    author: Option<String>,
    /// Markdown.
    #[serde(default)]
    desc: Option<String>,
    /// The first screenshot, on `storage.tukui.org`.
    #[serde(default)]
    screenshot_url: Option<String>,
    /// The others.
    #[serde(default, deserialize_with = "strings")]
    gallery_url: Vec<String>,
}

/// Map the legacy numeric `X-Tukui-ProjectID` to an API slug.
pub fn slug_from_project_id(id: &str) -> Option<&'static str> {
    // The old site used negative ids for its own two projects.
    match id.trim() {
        "1" | "-1" => Some("tukui"),
        "2" | "-2" => Some("elvui"),
        _ => None,
    }
}

/// `https://tukui.org/elvui` / `https://www.tukui.org/addons.php?id=2` → slug.
pub fn slug_from_url(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = [
        "https://tukui.org/",
        "https://www.tukui.org/",
        "http://tukui.org/",
        "http://www.tukui.org/",
    ]
    .iter()
    .find_map(|p| url.strip_prefix(p))?;
    if let Some(id) = rest.strip_prefix("addons.php?id=") {
        return slug_from_project_id(id).map(str::to_string);
    }
    let slug = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    valid_slug(&slug).then_some(slug)
}

fn valid_slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[derive(Debug, Deserialize)]
struct Listed {
    slug: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    small_desc: String,
    #[serde(default)]
    web_url: Option<String>,
    #[serde(default)]
    git_url: Option<String>,
    #[serde(default)]
    downloads: Option<f64>,
    #[serde(default)]
    last_update: Option<String>,
    #[serde(default, deserialize_with = "strings")]
    directories: Vec<String>,
    /// The game version of each client the one download serves, unsorted:
    /// ElvUI lists `["12.1.5","1.60.1","5.5.4","1.15.9","2.5.6"]`.
    #[serde(default, deserialize_with = "strings")]
    patch: Vec<String>,
}

/// The strings of a list, leniently: the whole catalog is one answer, so a
/// `null` or a stray number in one project's list must not fail the search.
fn strings<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    // Any JSON value decodes; a shape other than a list reads as empty.
    let items = match serde_json::Value::deserialize(d)? {
        serde_json::Value::Array(items) => items,
        _ => Vec::new(),
    };
    Ok(items
        .into_iter()
        .filter_map(|v| match v {
            serde_json::Value::String(s) => Some(s),
            _ => None,
        })
        .collect())
}

/// Search the catalog. `GET https://api.tukui.org/v1/addons` lists every
/// project (two today: ElvUI and Tukui, a few KiB), so the filtering is ours.
pub async fn search(
    http: &Http,
    query: &str,
    flavor: Flavor,
    client_interface: Option<u32>,
) -> Result<Vec<Hit>, SourceError> {
    let all: Vec<Listed> = http.get_json("https://api.tukui.org/v1/addons", &[]).await?;
    Ok(matching(all, query, flavor, client_interface))
}

fn matching(all: Vec<Listed>, query: &str, flavor: Flavor, client_interface: Option<u32>) -> Vec<Hit> {
    let q = simplify(query);
    all.into_iter()
        .filter(|a| valid_slug(&a.slug))
        .filter(|a| !q.is_empty() && (simplify(&a.name).contains(&q) || simplify(&a.slug).contains(&q)))
        .map(|a| {
            let codes = interface_codes(a.patch.iter().map(String::as_str));
            (Compat::listed(&codes, flavor, client_interface), a)
        })
        .filter(|(compat, _)| *compat != Compat::OtherFlavor)
        .map(|(compat, a)| Hit {
            compat,
            url: a
                .web_url
                .unwrap_or_else(|| format!("https://tukui.org/{}", a.slug)),
            repo: repo_of(a.git_url.as_deref()),
            name: if a.name.is_empty() { a.slug.clone() } else { a.name },
            source: super::SourceRef::Tukui { slug: a.slug },
            summary: a.small_desc,
            authors: a.author.into_iter().collect(),
            popularity: downloads(a.downloads),
            updated: year_of_iso(a.last_update.as_deref()),
            folders: a.directories,
            icon: None,
            site_only: false,
        })
        .collect()
}

/// Fetch the current version of `slug`.
pub async fn latest(http: &Http, slug: &str) -> Result<Release, SourceError> {
    if !valid_slug(slug) {
        return Err(SourceError::NoRelease);
    }
    let url = format!("https://api.tukui.org/v1/addon/{slug}");
    let a: TukuiAddon = http.get_json(&url, &[]).await?;
    Ok(Release {
        summary: a
            .small_desc
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        version: Version::parse(&a.version),
        url: a.web_url.unwrap_or_else(|| format!("https://tukui.org/{slug}")),
        published_at: a.last_update,
        download: a.url,
        digest: None,
        provider: String::new(),
        interfaces: None,
        size: None,
    })
}

/// The page of `slug`: the same `GET /v1/addon/{slug}` as an update check.
pub async fn page(http: &Http, slug: &str) -> Result<Page, SourceError> {
    if !valid_slug(slug) {
        return Err(SourceError::NotFound);
    }
    let url = format!("https://api.tukui.org/v1/addon/{slug}");
    let a: TukuiAddon = http.get_json(&url, &[]).await?;
    Ok(page_from(a, slug))
}

fn page_from(a: TukuiAddon, slug: &str) -> Page {
    let gallery = a
        .screenshot_url
        .iter()
        .chain(&a.gallery_url)
        .filter_map(|u| Shot::new(u, None, ""))
        .collect();
    Page {
        name: if a.name.trim().is_empty() {
            slug.to_string()
        } else {
            a.name.trim().to_string()
        },
        summary: a.small_desc.unwrap_or_default().trim().to_string(),
        authors: a.author.into_iter().filter(|s| !s.trim().is_empty()).collect(),
        url: a
            .web_url
            .filter(|u| u.starts_with("https://"))
            .unwrap_or_else(|| format!("https://tukui.org/{slug}")),
        gallery,
        body: crate::page::from_markdown(a.desc.as_deref().unwrap_or_default(), "https://tukui.org/"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_has_the_gallery_and_the_markdown() {
        // Shaped like `GET /v1/addon/elvui` in November 2026.
        let json = r###"{"id":-2,"slug":"elvui","author":"Elv","name":"ElvUI","version":"16.01",
            "web_url":"https://tukui.org/elvui","small_desc":"A user interface designed around user-friendliness.",
            "desc":"## About\n\n**ElvUI** replaces the default UI.",
            "screenshot_url":"https://storage.tukui.org/ElvUI-1.jpg",
            "gallery_url":["https://storage.tukui.org/ElvUI-2.jpg",null]}"###;
        let a: TukuiAddon = serde_json::from_str(json).unwrap();
        let page = page_from(a, "elvui");
        assert_eq!(
            page.summary,
            "A user interface designed around user-friendliness."
        );
        assert_eq!(page.authors, ["Elv"]);
        assert_eq!(page.gallery.len(), 2);
        assert_eq!(
            crate::page::doc::plain(&page.body),
            ["h2 About", "ElvUI replaces the default UI."]
        );
    }

    #[test]
    fn slugs() {
        assert_eq!(slug_from_url("https://tukui.org/elvui").as_deref(), Some("elvui"));
        assert_eq!(
            slug_from_url("https://www.tukui.org/addons.php?id=2").as_deref(),
            Some("elvui")
        );
        assert_eq!(slug_from_url("https://tukui.org/").as_deref(), None);
        assert_eq!(slug_from_url("https://example.org/elvui"), None);
        assert_eq!(slug_from_project_id("2"), Some("elvui"));
        assert_eq!(slug_from_project_id("-2"), Some("elvui"));
    }

    #[test]
    fn searches_the_catalog() {
        let json = r#"[{"id":-2,"slug":"elvui","author":"Elv","name":"ElvUI","version":"16.00",
            "git_url":"https://github.com/tukui-org/ElvUI","web_url":"https://tukui.org/elvui",
            "small_desc":"A user interface replacement."},
            {"id":-1,"slug":"tukui","author":"Tukz","name":"Tukui","version":"20.0"}]"#;
        let all = || serde_json::from_str::<Vec<Listed>>(json).unwrap();
        let find = |q: &str| matching(all(), q, Flavor::Retail, Some(120105));
        let hits = find("elv");
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].source,
            super::super::SourceRef::Tukui { slug: "elvui".into() }
        );
        assert_eq!(hits[0].repo.as_deref(), Some("tukui-org/elvui"));
        assert_eq!(hits[0].authors, ["Elv"]);
        // No `patch` listed: kept, unverified.
        assert_eq!(hits[0].compat, Compat::Unknown);
        assert_eq!(find("TUKUI").len(), 1);
        assert!(find("weakauras").is_empty());
    }

    #[test]
    fn the_patch_list_says_which_clients_it_serves() {
        // ElvUI and Tukui as `GET /v1/addons` listed them on 2026-11-06.
        let json = r#"[{"id":-2,"slug":"elvui","author":"Elv","name":"ElvUI","version":"16.01",
            "patch":["12.1.5","1.60.1","5.5.4","1.15.9","2.5.6"],"last_update":"2026-11-03",
            "directories":["ElvUI","ElvUI_Libraries","ElvUI_Options"]},
            {"id":-1,"slug":"tukui","author":"Tukz","name":"Tukui","version":"20.0",
            "patch":["11.1.7","5.5.0","1.15.7"],"directories":["Tukui"]}]"#;
        let elvui = |flavor, client_interface| {
            let all = serde_json::from_str::<Vec<Listed>>(json).unwrap();
            matching(all, "elvui", flavor, client_interface)
                .first()
                .map(|h| h.compat)
        };
        assert_eq!(elvui(Flavor::Forever, Some(16001)), Some(Compat::Exact));
        // No Wrath build listed: not offered on a Wrath client.
        assert_eq!(elvui(Flavor::Classic, Some(30405)), None);
        assert_eq!(elvui(Flavor::Classic, Some(50504)), Some(Compat::Exact));
        // Listed for Retail, the game's build unknown: the listing confirms it.
        assert_eq!(elvui(Flavor::Retail, None), Some(Compat::Exact));
        assert_eq!(elvui(Flavor::Retail, Some(120105)), Some(Compat::Exact));
        assert_eq!(
            elvui(Flavor::Retail, Some(120107)),
            Some(Compat::Older { built_for: 120105 })
        );
        // Tukui lists no Forever build.
        let all = serde_json::from_str::<Vec<Listed>>(json).unwrap();
        assert!(matching(all, "tukui", Flavor::Forever, Some(16001)).is_empty());
    }

    #[test]
    fn junk_in_a_list_does_not_fail_the_catalog() {
        let json = r#"[{"slug":"elvui","name":"ElvUI","patch":null,"directories":null},
            {"slug":"tukui","name":"Tukui","patch":["12.1.5",7,null],"directories":["Tukui",{}]},
            {"slug":"other","name":"Other","patch":"12.1.5","directories":3}]"#;
        let all = serde_json::from_str::<Vec<Listed>>(json).unwrap();
        assert!(all[0].patch.is_empty() && all[0].directories.is_empty());
        assert_eq!(all[1].patch, ["12.1.5"]);
        assert_eq!(all[1].directories, ["Tukui"]);
        assert!(all[2].patch.is_empty() && all[2].directories.is_empty());
    }

    #[test]
    fn decodes() {
        let json = r#"{"name":"ElvUI","slug":"elvui","version":"13.95","url":"https://api.tukui.org/v1/download/elvui-13.95.zip","web_url":"https://tukui.org/elvui","last_update":"2026-10-15","patch":["12.0.1"],"directories":["ElvUI","ElvUI_Libraries","ElvUI_Options"]}"#;
        let a: TukuiAddon = serde_json::from_str(json).unwrap();
        assert_eq!(a.version, "13.95");
    }
}
