//! WoWInterface source (`## X-WoWI-ID`). Public, no key.
//!
//! * `GET https://api.mmoui.com/v3/game/WOW/filedetails/{id}.json` answers an
//!   array with one entry: `UIVersion`, `UIDownload`, `UIMD5`,
//!   `UIFileInfoURL`, `UIDate` (epoch millis).
//! * `GET https://api.mmoui.com/v3/game/WOW/filelist.json` is the whole
//!   catalog, the only way to search: there is no search endpoint. See
//!   [`Catalog`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Deserializer};

use super::{Digest, Release, SourceError, SourceRef};
use crate::addon::Flavor;
use crate::http::Http;
use crate::search::{simplify, simplify_name, Hit, Popularity, PER_PROVIDER};
use crate::version::Version;

#[derive(Debug, Deserialize)]
struct FileDetails {
    #[serde(rename = "UIVersion")]
    version: String,
    #[serde(default, rename = "UIDownload")]
    download: Option<String>,
    #[serde(default, rename = "UIMD5")]
    md5: Option<String>,
    #[serde(default, rename = "UIFileInfoURL")]
    info_url: Option<String>,
    #[serde(default, rename = "UIDate")]
    date: Option<u64>,
}

/// Fetch the current version of file `id`.
pub async fn latest(http: &Http, id: u64) -> Result<Release, SourceError> {
    let url = format!("https://api.mmoui.com/v3/game/WOW/filedetails/{id}.json");
    let mut list: Vec<FileDetails> = http.get_json(&url, &[]).await?;
    let f = list.drain(..).next().ok_or(SourceError::NoRelease)?;
    Ok(Release {
        version: Version::parse(&f.version),
        url: f
            .info_url
            .unwrap_or_else(|| format!("https://www.wowinterface.com/downloads/info{id}")),
        published_at: f.date.map(|ms| ms.to_string()),
        download: f.download,
        digest: f
            .md5
            .filter(|h| h.len() == 32 && h.chars().all(|c| c.is_ascii_hexdigit()))
            .map(Digest::Md5),
        provider: String::new(),
    })
}

/// The catalog's address.
const CATALOG_URL: &str = "https://api.mmoui.com/v3/game/WOW/filelist.json";

/// Largest catalog accepted. It was 5.6 MB for 8 226 files in October 2026;
/// three times that leaves years of growth and still refuses anything absurd.
/// Every other answer stays under [`crate::http::MAX_BODY_BYTES`].
pub const MAX_CATALOG_BYTES: usize = 16 * 1024 * 1024;

/// How long a downloaded catalog is reused. New uploads are rare enough that
/// a few hours stale costs nothing, and it keeps a session to one download.
const CATALOG_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// One file as the catalog lists it. Counts come as strings (`"598607"`) and
/// lists may be `null`; everything is optional.
#[derive(Debug, Deserialize)]
struct Listed {
    #[serde(default, rename = "UID", deserialize_with = "lenient_u64")]
    id: Option<u64>,
    #[serde(default, rename = "UIName")]
    name: String,
    #[serde(default, rename = "UIAuthorName")]
    author: String,
    #[serde(default, rename = "UIFileInfoURL")]
    info_url: Option<String>,
    #[serde(default, rename = "UIDownloadTotal", deserialize_with = "lenient_u64")]
    total: Option<u64>,
    #[serde(default, rename = "UIDownloadMonthly", deserialize_with = "lenient_u64")]
    monthly: Option<u64>,
    #[serde(default, rename = "UIDate")]
    date: Option<u64>,
    #[serde(default, rename = "UICompatibility")]
    compatibility: Option<Vec<Compatibility>>,
    #[serde(default, rename = "UIDir")]
    dirs: Option<Vec<String>>,
    /// Screenshot thumbnails, at most 100px wide, on `cdn-wow.mmoui.com`.
    #[serde(default, rename = "UIIMG_Thumbs")]
    thumbs: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct Compatibility {
    #[serde(default)]
    version: String,
}

/// A number that may arrive as a JSON number or a string; junk reads as none.
fn lenient_u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Num {
        N(u64),
        S(String),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Option::<Num>::deserialize(d)? {
        Some(Num::N(n)) => Some(n),
        Some(Num::S(s)) => s.trim().parse().ok(),
        _ => None,
    })
}

/// The part of a catalog entry search needs, kept small: the catalog is
/// thousands of entries and lives for the whole session.
#[derive(Debug, Clone)]
pub struct Entry {
    id: u64,
    name: String,
    /// `name` simplified once, so a search is a scan of short strings.
    key: String,
    author: String,
    url: String,
    total: u64,
    monthly: u64,
    year: Option<u16>,
    /// Interface codes of the game versions it was marked compatible with.
    interfaces: Vec<u32>,
    folders: Vec<String>,
    /// The first screenshot thumbnail only: one short URL per entry for the
    /// whole session, where all of them would be several per entry.
    thumb: Option<String>,
}

impl Entry {
    fn from_listed(l: Listed) -> Option<Self> {
        let id = l.id?;
        let name = l.name.trim().to_string();
        if name.is_empty() {
            return None;
        }
        let interfaces = l
            .compatibility
            .unwrap_or_default()
            .iter()
            .filter_map(|c| crate::client::parse_version(&c.version))
            .map(|(ma, mi, pa)| ma * 10000 + mi * 100 + pa)
            .collect();
        Some(Self {
            id,
            key: simplify_name(&name),
            author: l.author.trim().to_string(),
            // The page goes to the browser: only https, else the canonical one.
            url: l
                .info_url
                .filter(|u| u.starts_with("https://"))
                .unwrap_or_else(|| format!("https://www.wowinterface.com/downloads/info{id}")),
            total: l.total.unwrap_or(0),
            monthly: l.monthly.unwrap_or(0),
            year: l.date.and_then(crate::search::year_of_millis),
            interfaces,
            folders: l.dirs.unwrap_or_default(),
            thumb: l
                .thumbs
                .and_then(|t| t.into_iter().find(|u| !u.trim().is_empty())),
            name,
        })
    }

    /// Unmarked files are kept: better a result than a silent omission.
    fn serves(&self, flavor: Flavor) -> bool {
        self.interfaces.is_empty() || self.interfaces.iter().any(|&c| Flavor::of_interface(c) == flavor)
    }

    fn hit(&self) -> Hit {
        Hit {
            source: SourceRef::WowInterface { id: self.id },
            name: self.name.clone(),
            // The catalog carries no description; a merged entry shows the
            // other providers' summary.
            summary: String::new(),
            authors: (!self.author.is_empty())
                .then(|| self.author.clone())
                .into_iter()
                .collect(),
            popularity: Some(Popularity::Downloads(self.total)),
            url: self.url.clone(),
            repo: None,
            folders: self.folders.clone(),
            updated: self.year,
            icon: self.thumb.clone(),
        }
    }
}

struct Loaded {
    at: Instant,
    entries: Arc<Vec<Entry>>,
}

/// The WoWInterface catalog, downloaded at most once per [`CATALOG_TTL`] and
/// shared by every search. Cloning shares the same cache. Concurrent callers
/// wait for one download instead of starting their own (the lock is held
/// across it), and a failed refresh keeps serving the copy already held.
#[derive(Clone, Default)]
pub struct Catalog {
    slot: Arc<tokio::sync::Mutex<Option<Loaded>>>,
}

impl std::fmt::Debug for Catalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Catalog")
    }
}

impl Catalog {
    /// The catalog, downloading it when missing or stale.
    pub async fn entries(&self, http: &Http) -> Result<Arc<Vec<Entry>>, SourceError> {
        let mut slot = self.slot.lock().await;
        if let Some(l) = slot.as_ref().filter(|l| l.at.elapsed() < CATALOG_TTL) {
            return Ok(Arc::clone(&l.entries));
        }
        match http
            .get_json_large::<Vec<Listed>>(CATALOG_URL, MAX_CATALOG_BYTES)
            .await
        {
            Ok(listed) => {
                let entries = Arc::new(listed.into_iter().filter_map(Entry::from_listed).collect());
                *slot = Some(Loaded {
                    at: Instant::now(),
                    entries: Arc::clone(&entries),
                });
                Ok(entries)
            }
            Err(e) => match slot.as_ref() {
                Some(l) => {
                    tracing::warn!(error = %e, "WoWInterface catalog refresh failed, keeping the old one");
                    Ok(Arc::clone(&l.entries))
                }
                None => Err(e.into()),
            },
        }
    }

    /// Download ahead of the first search; answers how many files it lists.
    pub async fn preload(&self, http: &Http) -> Result<usize, SourceError> {
        self.entries(http).await.map(|e| e.len())
    }

    /// The first screenshot thumbnail of each file in `ids`, from the
    /// catalog in memory only: never downloads it, so the library's icons
    /// do not cost 6 MB of their own. Empty until a search page loaded it.
    pub async fn loaded_thumbs(&self, ids: &[u64]) -> Vec<(u64, String)> {
        let slot = self.slot.lock().await;
        let Some(loaded) = slot.as_ref() else {
            return Vec::new();
        };
        thumbs_of(&loaded.entries, ids)
    }
}

/// `(id, thumbnail)` of each entry in `ids` that has one.
fn thumbs_of(entries: &[Entry], ids: &[u64]) -> Vec<(u64, String)> {
    let wanted: std::collections::HashSet<u64> = ids.iter().copied().collect();
    entries
        .iter()
        .filter(|e| wanted.contains(&e.id))
        .filter_map(|e| Some((e.id, e.thumb.clone()?)))
        .collect()
}

/// Search the catalog for `flavor`.
pub async fn search(
    http: &Http,
    catalog: &Catalog,
    query: &str,
    flavor: Flavor,
) -> Result<Vec<Hit>, SourceError> {
    let entries = catalog.entries(http).await?;
    Ok(matching(&entries, query, flavor))
}

/// The catalog has no relevance of its own, so: names equal to the query,
/// then names starting with it, then containing it, then files that ship a
/// folder named like it (`dbm` finds Deadly Boss Mods through `DBM-Core`);
/// within each, most downloaded *this month* first, which puts maintained
/// addons ahead of abandoned ones with a large lifetime total.
fn matching(entries: &[Entry], query: &str, flavor: Flavor) -> Vec<Hit> {
    let q = simplify(query);
    if q.is_empty() {
        return Vec::new();
    }
    let tier = |e: &Entry| -> Option<u8> {
        if e.key == q {
            Some(0)
        } else if e.key.starts_with(&q) {
            Some(1)
        } else if e.key.contains(&q) {
            Some(2)
        } else if e.folders.iter().any(|f| simplify(f).contains(&q)) {
            Some(3)
        } else {
            None
        }
    };
    let mut found: Vec<(u8, &Entry)> = entries
        .iter()
        .filter(|e| e.serves(flavor))
        .filter_map(|e| tier(e).map(|t| (t, e)))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.monthly.cmp(&a.1.monthly)));
    found
        .into_iter()
        .take(PER_PROVIDER)
        .map(|(_, e)| e.hit())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_details() {
        let json = r#"[{"UID":"8814","UIName":"Deadly Boss Mods","UIVersion":"11.2.15","UIDate":1757932228000,
            "UIMD5":"e81dc2d50729669d0b7ff8a3f4ab4db1","UIFileName":"DBM-Core-11.2.15.zip",
            "UIDownload":"https://cdn.wowinterface.com/downloads/getfile.php?id=8814&d=1757932228&minion",
            "UIFileInfoURL":"https://www.wowinterface.com/downloads/info8814-DeadlyBossMods.html"}]"#;
        let l: Vec<FileDetails> = serde_json::from_str(json).unwrap();
        assert_eq!(l[0].version, "11.2.15");
        assert_eq!(l[0].md5.as_deref(), Some("e81dc2d50729669d0b7ff8a3f4ab4db1"));
    }

    /// Shaped like the real filelist.json: strings for counts, `null` lists.
    const CATALOG: &str = r#"[
      {"UID":"3826","UICATID":"19","UIVersion":"11.0.2.3","UIDate":1724089877000,"UIName":"CTMod",
       "UIAuthorName":"DahkCeles","UIFileInfoURL":"https://www.wowinterface.com/downloads/info3826-CTMod.html",
       "UIDownloadTotal":"598607","UIDownloadMonthly":"33","UIFavoriteTotal":"695",
       "UICompatibility":[{"version":"11.0.2","name":"The War Within"},{"version":"1.15.2","name":"Classic"}],
       "UIDir":["CT_Core","CT_Library"],"UIIMG_Thumbs":[],"UIIMGs":[],"UISiblings":null},
      {"UID":"8814","UIName":"Deadly Boss Mods","UIAuthorName":"MysticalOS","UIDate":1757932228000,
       "UIDownloadTotal":"90000000","UIDownloadMonthly":"50000",
       "UICompatibility":[{"version":"11.2.0","name":"x"}],"UIDir":["DBM-Core","DBM-StatusBarTimers"],
       "UIIMG_Thumbs":["https://cdn-wow.mmoui.com/preview/tiny/pvw20498.png",
                       "https://cdn-wow.mmoui.com/preview/tiny/pvw20499.png"],
       "UIIMGs":["https://cdn-wow.mmoui.com/preview/pvw20498.png"]},
      {"UID":"13475","UIName":"Broker Bagnon","UIAuthorName":"siema_co","UIDownloadTotal":"6916",
       "UIDownloadMonthly":"6","UICompatibility":null,"UIDir":["Broker_Bagnon"]},
      {"UID":"4459","UIName":"Bagnon","UIAuthorName":"jaliborc","UIDate":1553216401000,
       "UIDownloadTotal":"2029407","UIDownloadMonthly":"333",
       "UICompatibility":[{"version":"8.1.0","name":"Tides of Vengeance"}],"UIDir":["Bagnon"]},
      {"UID":"99","UIName":"Era Only","UICompatibility":[{"version":"1.15.2"}]},
      {"UID":"x","UIName":"Bad id"},
      {"UID":"5","UIName":"   "}
    ]"#;

    fn entries() -> Vec<Entry> {
        let listed: Vec<Listed> = serde_json::from_str(CATALOG).unwrap();
        listed.into_iter().filter_map(Entry::from_listed).collect()
    }

    #[test]
    fn decodes_the_catalog_leniently() {
        let e = entries();
        // A non-numeric id and a blank name are dropped, not fatal.
        assert_eq!(e.len(), 5);
        assert_eq!(e[0].total, 598_607);
        assert_eq!(e[0].year, Some(2024));
        assert_eq!(e[0].interfaces, [110002, 11502]);
        assert!(e[2].interfaces.is_empty());
    }

    #[test]
    fn exact_names_first_then_monthly_downloads() {
        let names = |q: &str, f: Flavor| -> Vec<String> {
            matching(&entries(), q, f).into_iter().map(|h| h.name).collect()
        };
        // Bagnon's 2019 upload is marked for 8.1.0 only: no game today runs
        // it, so Retail does not offer it. Unmarked Broker Bagnon stays.
        assert_eq!(names("bagnon", Flavor::Retail), ["Broker Bagnon"]);
        assert_eq!(names("ct", Flavor::Retail), ["CTMod"]);
        // Through a folder name.
        assert_eq!(names("dbm", Flavor::Retail), ["Deadly Boss Mods"]);
        // Flavor filter: compatibility lists decide, unmarked files stay.
        assert_eq!(names("era", Flavor::Retail), Vec::<String>::new());
        assert_eq!(names("era", Flavor::Vanilla), ["Era Only"]);
        assert!(names("", Flavor::Retail).is_empty());
    }

    #[test]
    fn hits_carry_folders_and_year() {
        let h = &matching(&entries(), "deadly boss mods", Flavor::Retail)[0];
        assert_eq!(h.source, SourceRef::WowInterface { id: 8814 });
        assert_eq!(h.folders, ["DBM-Core", "DBM-StatusBarTimers"]);
        assert_eq!(h.updated, Some(2025));
        assert_eq!(h.authors, ["MysticalOS"]);
        // The first thumbnail; an empty list (CTMod) gives none.
        assert_eq!(
            h.icon.as_deref(),
            Some("https://cdn-wow.mmoui.com/preview/tiny/pvw20498.png")
        );
        assert_eq!(matching(&entries(), "ctmod", Flavor::Retail)[0].icon, None);
    }

    #[test]
    fn thumbs_for_installed_ids_only() {
        // DBM has a thumbnail, CTMod none, 77 is not in the catalog.
        let got = thumbs_of(&entries(), &[8814, 3826, 77]);
        assert_eq!(
            got,
            [(
                8814,
                "https://cdn-wow.mmoui.com/preview/tiny/pvw20498.png".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn an_unloaded_catalog_gives_no_thumbs() {
        assert!(Catalog::default().loaded_thumbs(&[8814]).await.is_empty());
    }
}
