//! Update sources: where an addon comes from and how to ask for its latest version.
//!
//! A [`SourceRef`] is *data* (serializable into `tocsin.toml`); a [`Source`] is
//! the behaviour that turns a `SourceRef` into a [`Release`].

pub mod curseforge;
pub mod github;
pub mod tukui;
pub mod wowinterface;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::http::Http;
use crate::toc::Toc;
use crate::version::Version;

/// Where an addon is published. Resolved from the `.toc` or declared by the user.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SourceRef {
    /// GitHub Releases of `owner/repo`.
    Github {
        /// Repository owner.
        owner: String,
        /// Repository name.
        repo: String,
        /// Force a release tag family (the non-numeric prefix before the
        /// version, e.g. `d` for `dv12.0.13`). Normally auto-detected.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag_prefix: Option<String>,
    },
    /// CurseForge project id (`## X-Curse-Project-ID`).
    CurseForge {
        /// Numeric project id.
        id: u64,
    },
    /// tukui.org project (`## X-Tukui-ProjectID` or a tukui.org website). No key.
    Tukui {
        /// API slug, e.g. `elvui`.
        slug: String,
    },
    /// WoWInterface file id (`## X-WoWI-ID`). No key.
    WowInterface {
        /// Numeric file id.
        id: u64,
    },
    /// No API available: the user checks `url` by hand, Tocsin only shows it.
    Manual {
        /// Homepage or download page.
        url: String,
    },
}

/// Read an optional source without failing the whole file on one Tocsin no
/// longer knows. Wago was supported once and dropped (its API key is a paid
/// Patreon perk), so a `tocsin.toml` override or an install record may still
/// say `type = "wago"`; that entry reads as "no source" and the addon falls
/// back to what its `.toc` advertises.
pub fn deserialize_lenient<'de, D>(d: D) -> Result<Option<SourceRef>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Maybe {
        Known(SourceRef),
        Unknown(serde::de::IgnoredAny),
    }
    Ok(match Option::<Maybe>::deserialize(d)? {
        Some(Maybe::Known(s)) => Some(s),
        Some(Maybe::Unknown(_)) | None => None,
    })
}

impl SourceRef {
    /// Every source the `.toc` advertises, best first: GitHub link in
    /// `X-Website`, the other key-free providers (Tukui, WoWInterface), the
    /// CurseForge id, then any other website as a manual fallback. The
    /// checker tries them in this order and skips providers it has no
    /// credentials for. `## X-Wago-ID` is ignored: Wago is not supported.
    pub fn candidates_from_toc(toc: &Toc) -> Vec<Self> {
        let mut out = Vec::with_capacity(3);
        let site = toc.website();
        if let Some(gh) = site.and_then(github::parse_repo_url) {
            out.push(gh);
        }
        // Key-free providers next.
        if let Some(slug) = toc
            .get("x-tukui-projectid")
            .and_then(tukui::slug_from_project_id)
            .map(str::to_string)
            .or_else(|| site.and_then(tukui::slug_from_url))
        {
            out.push(Self::Tukui { slug });
        }
        if let Some(id) = toc.wowinterface_id() {
            out.push(Self::WowInterface { id });
        }
        if let Some(id) = toc.curse_project_id() {
            out.push(Self::CurseForge { id });
        }
        if let Some(u) = site {
            if !out
                .iter()
                .any(|s| matches!(s, Self::Github { .. } | Self::Tukui { .. }))
            {
                out.push(Self::Manual { url: u.to_string() });
            }
        }
        out
    }

    /// The preferred source from the `.toc`, if any.
    pub fn from_toc(toc: &Toc) -> Option<Self> {
        Self::candidates_from_toc(toc).into_iter().next()
    }

    /// Whether we hold the credentials this provider needs.
    pub fn is_usable(&self, creds: &Credentials) -> bool {
        match self {
            Self::Github { .. } | Self::Tukui { .. } | Self::WowInterface { .. } | Self::Manual { .. } => {
                true
            }
            Self::CurseForge { .. } => creds.curseforge.is_some(),
        }
    }

    /// The project behind this source, whatever way it is checked:
    /// `curseforge:65387`, `github:weakauras/weakauras2` (GitHub names are
    /// case-insensitive; `tag_prefix` only picks among its releases). `None`
    /// for a manual link, which names a page, not a project. Search uses it
    /// to tell which results are installed already.
    pub fn project_key(&self) -> Option<String> {
        match self {
            Self::Github { owner, repo, .. } => Some(format!("github:{owner}/{repo}").to_ascii_lowercase()),
            Self::CurseForge { id } => Some(format!("curseforge:{id}")),
            Self::Tukui { slug } => Some(format!("tukui:{}", slug.to_ascii_lowercase())),
            Self::WowInterface { id } => Some(format!("wowinterface:{id}")),
            Self::Manual { .. } => None,
        }
    }

    /// Human-readable name of the provider.
    pub fn provider(&self) -> &'static str {
        match self {
            Self::Github { .. } => "GitHub",
            Self::CurseForge { .. } => "CurseForge",
            Self::Tukui { .. } => "Tukui",
            Self::WowInterface { .. } => "WoWInterface",
            Self::Manual { .. } => "Manual",
        }
    }

    /// A page the user can open in a browser.
    pub fn web_url(&self) -> String {
        match self {
            Self::Github { owner, repo, .. } => format!("https://github.com/{owner}/{repo}/releases"),
            Self::CurseForge { id } => format!("https://www.curseforge.com/projects/{id}"),
            Self::Tukui { slug } => format!("https://tukui.org/{slug}"),
            Self::WowInterface { id } => format!("https://www.wowinterface.com/downloads/info{id}"),
            // Upgrade plain http:// links from old .toc files: we only ever
            // open https:// in the browser.
            Self::Manual { url } => match url.strip_prefix("http://") {
                Some(rest) => format!("https://{rest}"),
                None => url.clone(),
            },
        }
    }
}

/// The newest release a source knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Version string, parsed leniently.
    pub version: Version,
    /// Page to read about it (release notes, project page).
    pub url: String,
    /// ISO-8601 timestamp if the provider gives one.
    pub published_at: Option<String>,
    /// Direct link to the zip for the requested flavor, when the provider
    /// gives one. `None` means "open the page and download by hand".
    pub download: Option<String>,
    /// Provider-supplied digest of the zip, when any.
    pub digest: Option<Digest>,
    /// Which provider answered (`fetch_latest` fills it in). The addon's
    /// first source may have failed and a fallback answered instead; the GUI
    /// says so, because the fallback may have no direct download.
    pub provider: String,
}

/// A checksum published by the provider for a release file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Digest {
    /// Hex SHA-256.
    Sha256(String),
    /// Hex SHA-1 (CurseForge).
    Sha1(String),
    /// Hex MD5 (CurseForge).
    Md5(String),
}

/// Credentials for providers that need them. All optional.
///
/// `Debug` is written by hand and shows only whether each key is set, so a
/// stray `{:?}` or `tracing` field never prints a key.
#[derive(Clone, Default)]
pub struct Credentials {
    /// `x-api-key` for api.curseforge.com.
    pub curseforge: Option<String>,
    /// Personal token for api.github.com (raises the rate limit).
    pub github: Option<String>,
    /// Set once GitHub answers 401 to the token. Shared by every clone, so a
    /// whole check run (and the GUI, which holds the same credentials) learns
    /// it from the first refusal and stops sending the token.
    pub(crate) github_refused: Arc<AtomicBool>,
}

impl Credentials {
    /// The GitHub token to send: none once GitHub has refused it this session.
    pub fn github_token(&self) -> Option<&str> {
        if self.github_token_refused() {
            None
        } else {
            self.github.as_deref()
        }
    }

    /// Whether GitHub refused the token (expired, revoked, mistyped). Requests
    /// then go out without it, at the anonymous rate limit; the GUI tells the
    /// user to replace or remove it.
    pub fn github_token_refused(&self) -> bool {
        self.github.is_some() && self.github_refused.load(Ordering::Relaxed)
    }

    pub(crate) fn refuse_github_token(&self) {
        self.github_refused.store(true, Ordering::Relaxed);
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let set = |k: &Option<String>| if k.is_some() { "set" } else { "unset" };
        let github = if self.github_token_refused() {
            "refused"
        } else {
            set(&self.github)
        };
        f.debug_struct("Credentials")
            .field("curseforge", &set(&self.curseforge))
            .field("github", &github)
            .finish()
    }
}

/// Errors while resolving a release.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// Provider requires a key we don't have.
    #[error("{0} requires an API key (see README)")]
    MissingCredentials(&'static str),
    /// Provider answered but had no usable release.
    #[error("no release found")]
    NoRelease,
    /// The source can't be queried automatically.
    #[error("manual source, open {0} to check")]
    Manual(String),
    /// Transport / decode failure.
    #[error(transparent)]
    Http(#[from] crate::http::HttpError),
}

/// Ask the provider behind `source` for its latest release.
pub async fn fetch_latest(
    http: &Http,
    creds: &Credentials,
    source: &SourceRef,
    hint: github::Hint<'_>,
) -> Result<Release, SourceError> {
    let release = match source {
        SourceRef::Github {
            owner,
            repo,
            tag_prefix,
        } => github::latest(http, creds, owner, repo, tag_prefix.as_deref(), hint).await,
        SourceRef::CurseForge { id } => {
            let key = creds
                .curseforge
                .as_deref()
                .ok_or(SourceError::MissingCredentials("CurseForge"))?;
            curseforge::latest(http, key, *id, hint.flavor).await
        }
        SourceRef::Tukui { slug } => tukui::latest(http, slug).await,
        SourceRef::WowInterface { id } => wowinterface::latest(http, *id).await,
        SourceRef::Manual { url } => Err(SourceError::Manual(url.clone())),
    }?;
    Ok(Release {
        provider: source.provider().to_string(),
        ..release
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_from_toc_in_order() {
        // Wago ids are ignored.
        let t = Toc::parse("## X-Wago-ID: abc\n## X-Curse-Project-ID: 7\n");
        assert_eq!(
            SourceRef::candidates_from_toc(&t),
            vec![SourceRef::CurseForge { id: 7 }]
        );

        let t = Toc::parse("## X-Website: https://github.com/Foo/Bar\n## X-Curse-Project-ID: 7\n");
        assert_eq!(
            SourceRef::from_toc(&t),
            Some(SourceRef::Github {
                owner: "Foo".into(),
                repo: "Bar".into(),
                tag_prefix: None,
            })
        );

        let t = Toc::parse("## X-Website: http://zygorguides.com/\n");
        assert_eq!(
            SourceRef::from_toc(&t),
            Some(SourceRef::Manual {
                url: "http://zygorguides.com/".into()
            })
        );

        assert_eq!(SourceRef::from_toc(&Toc::parse("## Title: x")), None);

        // ElvUI: tukui.org project, no key, beats a manual link to the same site.
        let t = Toc::parse("## X-Tukui-ProjectID: 2\n## X-Website: https://tukui.org/elvui\n");
        assert_eq!(
            SourceRef::candidates_from_toc(&t),
            vec![SourceRef::Tukui { slug: "elvui".into() }]
        );

        // WeakAuras-like: everything at once, key-free ones first.
        let t = Toc::parse("## X-Curse-Project-ID: 65387\n## X-WoWI-ID: 17019\n## X-Wago-ID: VBNBxKx5\n");
        let c = SourceRef::candidates_from_toc(&t);
        assert_eq!(c[0], SourceRef::WowInterface { id: 17019 });
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn a_dropped_provider_reads_as_no_source() {
        #[derive(Deserialize)]
        struct Entry {
            #[serde(default, deserialize_with = "deserialize_lenient")]
            source: Option<SourceRef>,
        }
        let old: Entry = toml::from_str(r#"source = { type = "wago", id = "QN53j5KB" }"#).unwrap();
        assert_eq!(old.source, None);
        let known: Entry = toml::from_str(r#"source = { type = "curseforge", id = 7 }"#).unwrap();
        assert_eq!(known.source, Some(SourceRef::CurseForge { id: 7 }));
        let none: Entry = toml::from_str("").unwrap();
        assert_eq!(none.source, None);
    }

    #[test]
    fn project_keys_ignore_case_and_tag_prefix() {
        let a = SourceRef::Github {
            owner: "WeakAuras".into(),
            repo: "WeakAuras2".into(),
            tag_prefix: Some("v".into()),
        };
        let b = github::parse_repo_url("https://github.com/weakauras/weakauras2").unwrap();
        assert_eq!(a.project_key(), b.project_key());
        assert_eq!(
            SourceRef::CurseForge { id: 7 }.project_key().as_deref(),
            Some("curseforge:7")
        );
        assert_eq!(
            SourceRef::Manual {
                url: "https://x".into()
            }
            .project_key(),
            None
        );
    }

    #[test]
    fn credentials_debug_hides_the_keys() {
        let c = Credentials {
            curseforge: Some("$2a$10$secretcurseforgekey".into()),
            github: None,
            ..Default::default()
        };
        let shown = format!("{c:?} {c:#?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("curseforge: \"set\""));
        assert!(shown.contains("github: \"unset\""));
    }

    #[test]
    fn serde_shape() {
        let s: SourceRef = toml::from_str(
            r#"type = "github"
owner = "a"
repo = "b""#,
        )
        .unwrap();
        assert_eq!(
            s,
            SourceRef::Github {
                owner: "a".into(),
                repo: "b".into(),
                tag_prefix: None,
            }
        );
    }
}
