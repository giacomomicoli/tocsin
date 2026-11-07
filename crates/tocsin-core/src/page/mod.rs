//! An addon's page as its site tells it: name, authors, summary, gallery
//! and description, for the GUI's addon view.
//!
//! Each provider answers in its own format, through documented API
//! endpoints only (no web page is ever scraped):
//!
//! * CurseForge: `GET /v1/mods/{id}` (screenshots, summary) and
//!   `GET /v1/mods/{id}/description`, HTML; needs the key.
//! * GitHub: `GET /repos/{owner}/{repo}` (description) and
//!   `GET /repos/{owner}/{repo}/readme`, which GitHub renders to HTML.
//! * WoWInterface: `GET /v3/game/WOW/filedetails/{id}.json`, the request an
//!   update check makes anyway; its description is BBCode.
//! * Tukui: `GET /v1/addon/{slug}`, the same as its update check; Markdown.
//!
//! The description becomes a [`Doc`] (see [`doc`]); pictures are fetched
//! later, by the GUI, through [`media`].

use crate::http::Http;
use crate::sources::{curseforge, github, tukui, wowinterface, Credentials, SourceError, SourceRef};

mod bbcode;
pub mod doc;
pub(crate) mod html;
mod markdown;
pub mod media;

pub use doc::{Block, Doc, Image, Kind, Marker, Span};

/// What a site says about one addon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Name as the site shows it.
    pub name: String,
    /// One-line description; empty when the site has none apart from the
    /// description itself (WoWInterface).
    pub summary: String,
    /// Author names (GitHub: the repository owner).
    pub authors: Vec<String>,
    /// The page on the site, `https://`, for "Open".
    pub url: String,
    /// Screenshots, in the site's order.
    pub gallery: Vec<Shot>,
    /// The description.
    pub body: Doc,
}

/// One screenshot of a [`Page::gallery`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    /// The full picture. Untrusted: only [`media::fetch`] may act on it.
    pub url: String,
    /// A smaller rendition, when the site has one (else `url` again).
    pub thumb: String,
    /// Its caption, maybe empty.
    pub title: String,
}

impl Shot {
    /// A screenshot at `url`, with `thumb` when it is usable; `None` when
    /// `url` is not an `https://` address.
    pub(crate) fn new(url: &str, thumb: Option<&str>, title: &str) -> Option<Self> {
        let url = url.trim();
        if !url.starts_with("https://") {
            return None;
        }
        let thumb = thumb
            .map(str::trim)
            .filter(|t| t.starts_with("https://"))
            .unwrap_or(url);
        Some(Self {
            url: url.to_string(),
            thumb: thumb.to_string(),
            title: title.trim().to_string(),
        })
    }
}

/// Most screenshots kept per page.
pub const MAX_SHOTS: usize = 24;

/// Why a page could not be shown. Written for the user.
#[derive(Debug, thiserror::Error)]
pub enum PageError {
    /// A CurseForge page without a key.
    #[error("CurseForge needs an API key to show its pages.")]
    NeedsKey,
    /// A manual link: there is no API to ask.
    #[error("This source has no page Tocsin can show; open it in the browser.")]
    NoPage,
    /// The site does not know the project any more.
    #[error("The site no longer has this addon.")]
    NotFound,
    /// Transport or decode failure.
    #[error("The page could not be loaded: {0}")]
    Source(SourceError),
}

impl From<SourceError> for PageError {
    fn from(e: SourceError) -> Self {
        match e {
            SourceError::NotFound | SourceError::Http(crate::http::HttpError::Status { status: 404, .. }) => {
                Self::NotFound
            }
            SourceError::MissingCredentials(_) => Self::NeedsKey,
            other => Self::Source(other),
        }
    }
}

/// Whether `source` has a page to show: every provider with an API.
pub fn has_page(source: &SourceRef) -> bool {
    !matches!(source, SourceRef::Manual { .. })
}

/// The sources of `sources` that have a page, those usable with `creds`
/// first (a CurseForge page needs the key), each once.
pub fn page_sources<'a>(
    sources: impl IntoIterator<Item = &'a SourceRef>,
    creds: &Credentials,
) -> Vec<SourceRef> {
    let mut out: Vec<SourceRef> = Vec::new();
    for s in sources {
        if has_page(s) && !out.iter().any(|o| o.project_key() == s.project_key()) {
            out.push(s.clone());
        }
    }
    out.sort_by_key(|s| !s.is_usable(creds));
    out
}

/// Ask the site behind `source` for its page.
pub async fn fetch(http: &Http, creds: &Credentials, source: &SourceRef) -> Result<Page, PageError> {
    let key = || creds.curseforge.as_deref().ok_or(PageError::NeedsKey);
    let mut page = match source {
        SourceRef::CurseForge { id } => curseforge::page(http, key()?, *id).await?,
        SourceRef::CurseForgeSlug { slug } => curseforge::page_by_slug(http, key()?, slug).await?,
        SourceRef::Github { owner, repo, .. } => github::page(http, creds, owner, repo).await?,
        SourceRef::WowInterface { id } => wowinterface::page(http, *id).await?,
        SourceRef::Tukui { slug } => tukui::page(http, slug).await?,
        SourceRef::Manual { .. } => return Err(PageError::NoPage),
    };
    page.gallery.truncate(MAX_SHOTS);
    if crate::http::browser_url(&page.url).is_none() {
        page.url = source.web_url();
    }
    Ok(page)
}

/// The first paragraph of a WoWInterface description as plain text: the
/// summary the library shows, which WoWInterface has no field for.
pub fn bbcode_summary(description: &str) -> Option<String> {
    doc::first_paragraph(&bbcode::to_doc(description))
}

/// Convert a CurseForge or GitHub HTML description.
pub(crate) fn from_html(html: &str, link_base: &str, image_base: &str) -> Doc {
    html::to_doc(
        html,
        doc::Bases {
            link: url::Url::parse(link_base).ok(),
            image: url::Url::parse(image_base).ok(),
        },
    )
}

/// Convert a WoWInterface BBCode description.
pub(crate) fn from_bbcode(text: &str) -> Doc {
    bbcode::to_doc(text)
}

/// Convert a Tukui Markdown description.
pub(crate) fn from_markdown(md: &str, base: &str) -> Doc {
    markdown::to_doc(md, doc::Bases::same(base))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shots_need_https_and_fall_back_to_the_full_picture() {
        let s = Shot::new(
            "https://media.forgecdn.net/a.png",
            Some("http://x/t.png"),
            " Raid ",
        )
        .unwrap();
        assert_eq!(s.thumb, "https://media.forgecdn.net/a.png");
        assert_eq!(s.title, "Raid");
        assert!(Shot::new("http://media.forgecdn.net/a.png", None, "").is_none());
    }

    #[test]
    fn page_sources_skip_links_and_put_usable_first() {
        let creds = Credentials::default();
        let sources = [
            SourceRef::CurseForge { id: 7 },
            SourceRef::Manual {
                url: "https://example.com".into(),
            },
            SourceRef::WowInterface { id: 3 },
            SourceRef::WowInterface { id: 3 },
        ];
        assert_eq!(
            page_sources(&sources, &creds),
            [SourceRef::WowInterface { id: 3 }, SourceRef::CurseForge { id: 7 }]
        );
    }

    #[test]
    fn errors_read_for_the_user() {
        let e: PageError = SourceError::Http(crate::http::HttpError::Status {
            host: "api.github.com".into(),
            status: 404,
        })
        .into();
        assert_eq!(e.to_string(), "The site no longer has this addon.");
        assert!(matches!(
            PageError::from(SourceError::MissingCredentials("CurseForge")),
            PageError::NeedsKey
        ));
    }
}
