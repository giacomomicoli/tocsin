//! Tukui source: ElvUI and Tukui are distributed from tukui.org, not from
//! CurseForge. Their `.toc` carries `## X-Tukui-ProjectID: 2` and
//! `## X-Website: https://tukui.org/elvui`.
//!
//! `GET https://api.tukui.org/v1/addon/{slug}` — public, no key. Fields of
//! interest: `version`, `url` (zip), `web_url`, `last_update`.

use serde::Deserialize;

use super::{Release, SourceError};
use crate::http::Http;
use crate::search::{downloads, repo_of, simplify, year_of_iso, Hit};
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
    #[serde(default)]
    directories: Vec<String>,
}

/// Search the catalog. `GET https://api.tukui.org/v1/addons` lists every
/// project (two today: ElvUI and Tukui, a few KiB), so the filtering is ours.
pub async fn search(http: &Http, query: &str) -> Result<Vec<Hit>, SourceError> {
    let all: Vec<Listed> = http.get_json("https://api.tukui.org/v1/addons", &[]).await?;
    Ok(matching(all, query))
}

fn matching(all: Vec<Listed>, query: &str) -> Vec<Hit> {
    let q = simplify(query);
    all.into_iter()
        .filter(|a| valid_slug(&a.slug))
        .filter(|a| !q.is_empty() && (simplify(&a.name).contains(&q) || simplify(&a.slug).contains(&q)))
        .map(|a| Hit {
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
        version: Version::parse(&a.version),
        url: a.web_url.unwrap_or_else(|| format!("https://tukui.org/{slug}")),
        published_at: a.last_update,
        download: a.url,
        digest: None,
        provider: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let hits = matching(all(), "elv");
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].source,
            super::super::SourceRef::Tukui { slug: "elvui".into() }
        );
        assert_eq!(hits[0].repo.as_deref(), Some("tukui-org/elvui"));
        assert_eq!(hits[0].authors, ["Elv"]);
        assert_eq!(matching(all(), "TUKUI").len(), 1);
        assert!(matching(all(), "weakauras").is_empty());
    }

    #[test]
    fn decodes() {
        let json = r#"{"name":"ElvUI","slug":"elvui","version":"13.95","url":"https://api.tukui.org/v1/download/elvui-13.95.zip","web_url":"https://tukui.org/elvui","last_update":"2026-10-15","patch":["12.0.1"],"directories":["ElvUI","ElvUI_Libraries","ElvUI_Options"]}"#;
        let a: TukuiAddon = serde_json::from_str(json).unwrap();
        assert_eq!(a.version, "13.95");
    }
}
