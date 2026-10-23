//! Parser for World of Warcraft `.toc` files.
//!
//! A `.toc` is a plain-text file: lines starting with `## Key: Value` are
//! metadata, everything else is a file path to load (which we ignore).
//! Non-ASCII titles and color codes like `|cff2ae7c6Routine` are common.

use std::collections::BTreeMap;
use std::path::Path;

use crate::discover::Bounded;

/// Upper bound on the `.toc` size we are willing to read (they are ~1 KB).
const MAX_TOC_BYTES: u64 = 1024 * 1024;

/// Parsed metadata of a `.toc` file. Keys are case-insensitive and stored
/// lower-cased; well-known ones are exposed as typed accessors.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toc {
    fields: BTreeMap<String, String>,
}

/// Errors produced while reading a `.toc`.
#[derive(Debug, thiserror::Error)]
pub enum TocError {
    /// The file is unreasonably large for a `.toc`.
    #[error("toc file too large ({0} bytes)")]
    TooLarge(u64),
    /// A symlink, junction or folder where a `.toc` file was expected. Links
    /// are never followed: they could point anywhere on the disk.
    #[error("toc is not a regular file")]
    NotAFile,
    /// Underlying I/O failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Game types Retail evaluates `[AllowLoadGameType …]` against.
pub const RETAIL_GAME_TYPES: &[&str] = &["standard"];

impl Toc {
    /// Parse `.toc` content as the Retail client would.
    pub fn parse(content: &str) -> Self {
        Self::parse_for(content, RETAIL_GAME_TYPES)
    }

    /// Parse `.toc` content, evaluating per-line conditions against
    /// `game_types` (lower-case, e.g. `["standard"]` or `["vanilla", "classic"]`).
    ///
    /// Since 11.x a line may carry `[AllowLoadGameType a, b]` or
    /// `[ExcludeLoadGameType a]` suffixes; ElvUI uses six `## Title` lines this
    /// way. Lines whose condition fails are ignored, and for a key given
    /// several times the first surviving line wins, as in the game.
    pub fn parse_for(content: &str, game_types: &[&str]) -> Self {
        let mut fields = BTreeMap::new();
        for raw in content.lines() {
            let line = raw.trim_start_matches('\u{feff}').trim();
            let Some(rest) = line.strip_prefix("##") else {
                continue;
            };
            // "## - SavedVariablesPerCharacter: x" is a commented-out directive.
            let rest = rest.trim_start();
            if rest.starts_with('-') || rest.starts_with('#') {
                continue;
            }
            let Some((key, value)) = rest.split_once(':') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let Some(value) = strip_conditions(value.trim(), game_types) else {
                continue;
            };
            if !key.is_empty() {
                fields.entry(key).or_insert(value);
            }
        }
        Self { fields }
    }

    /// Read and parse a `.toc` file from disk, as the Retail client would.
    pub fn from_path(path: &Path) -> Result<Self, TocError> {
        Self::from_path_for(path, RETAIL_GAME_TYPES)
    }

    /// Read and parse a `.toc` file from disk for the given game types.
    ///
    /// A symlinked `.toc` is refused ([`TocError::NotAFile`]), not followed.
    pub fn from_path_for(path: &Path, game_types: &[&str]) -> Result<Self, TocError> {
        let bytes = match crate::discover::read_regular_file(path, MAX_TOC_BYTES)? {
            Bounded::Read(bytes) => bytes,
            Bounded::TooLarge(len) => return Err(TocError::TooLarge(len)),
            Bounded::NotAFile => return Err(TocError::NotAFile),
        };
        // Some authors save as Latin-1; be lenient instead of failing.
        let content = String::from_utf8_lossy(&bytes);
        Ok(Self::parse_for(&content, game_types))
    }

    /// Raw lookup, case-insensitive on the key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .get(&key.to_ascii_lowercase())
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// `## Title`, with WoW color escape codes (`|cAARRGGBB`, `|r`) removed.
    pub fn title(&self) -> Option<String> {
        self.get("title").map(strip_color_codes)
    }

    /// `## Version` as written by the author (e.g. `v12.0.376`, `9.6`).
    ///
    /// Addons installed from a source zip instead of a packaged release still
    /// carry the packager placeholder (`@project-version@`, `#@project-version@`);
    /// that is "no version", not a version.
    pub fn version(&self) -> Option<&str> {
        self.get("version").filter(|v| !v.contains("@project-version@"))
    }

    /// `## Author`.
    pub fn author(&self) -> Option<&str> {
        self.get("author")
    }

    /// `## Notes`, color codes stripped.
    pub fn notes(&self) -> Option<String> {
        self.get("notes").map(strip_color_codes)
    }

    /// Supported client builds from `## Interface: 120007, 120100, …`.
    pub fn interface_versions(&self) -> Vec<u32> {
        self.get("interface")
            .map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect())
            .unwrap_or_default()
    }

    /// `## X-Curse-Project-ID`.
    pub fn curse_project_id(&self) -> Option<u64> {
        self.get("x-curse-project-id").and_then(|v| v.parse().ok())
    }

    /// `## X-WoWI-ID` (WoWInterface).
    pub fn wowinterface_id(&self) -> Option<u64> {
        self.get("x-wowi-id").and_then(|v| v.parse().ok())
    }

    /// `## X-Website`.
    pub fn website(&self) -> Option<&str> {
        self.get("x-website")
    }

    /// Hard dependencies from `## Dependencies` / `## RequiredDeps`.
    pub fn dependencies(&self) -> Vec<String> {
        self.folder_list(&["dependencies", "requireddeps"])
    }

    /// Soft dependencies from `## OptionalDeps`.
    pub fn optional_dependencies(&self) -> Vec<String> {
        self.folder_list(&["optionaldeps"])
    }

    fn folder_list(&self, keys: &[&str]) -> Vec<String> {
        keys.iter()
            .filter_map(|k| self.get(k))
            .flat_map(|v| v.split(','))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Whether this is a library bundled inside another addon (`## X-Embeds` or
    /// a folder-less lib `.toc`, which we never treat as a standalone addon).
    pub fn is_load_on_demand(&self) -> bool {
        self.get("loadondemand").is_some_and(|v| v == "1")
    }

    /// Iterate over all `(key, value)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// Evaluate trailing `[AllowLoadGameType …]` / `[ExcludeLoadGameType …]`
/// groups. Returns the value without them, or `None` when the line does not
/// apply to `game_types`.
fn strip_conditions(value: &str, game_types: &[&str]) -> Option<String> {
    let mut v = value.trim_end();
    let mut allowed: Option<bool> = None;
    while let Some(open) = v.rfind('[') {
        if !v.ends_with(']') {
            break;
        }
        let inner = &v[open + 1..v.len() - 1];
        let Some((directive, args)) = inner.trim().split_once(char::is_whitespace) else {
            break;
        };
        let listed = args
            .split(',')
            .map(|a| a.trim().to_ascii_lowercase())
            .any(|a| game_types.contains(&a.as_str()));
        match directive.to_ascii_lowercase().as_str() {
            "allowloadgametype" => allowed = Some(allowed.unwrap_or(false) || listed),
            "excludeloadgametype" if listed => return None,
            "excludeloadgametype" => {}
            _ => break,
        }
        v = v[..open].trim_end();
    }
    if allowed == Some(false) {
        return None;
    }
    Some(v.to_string())
}

/// Remove `|cAARRGGBB` … `|r` color escapes and `|T…|t` texture escapes.
pub fn strip_color_codes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '|' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('c') => {
                // 8 hex digits follow.
                for _ in 0..8 {
                    chars.next();
                }
            }
            Some('r') => {}
            Some('T') => {
                // skip until the closing "|t"
                while let Some(n) = chars.next() {
                    if n == '|' && chars.peek() == Some(&'t') {
                        chars.next();
                        break;
                    }
                }
            }
            Some('|') => out.push('|'),
            Some(other) => {
                out.push('|');
                out.push(other);
            }
            None => out.push('|'),
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTINE: &str = r#"## Interface: 120007, 120100, 120105, 16001
## Title: |cff2ae7c6Routine
## Notes: Auto-tracking quest log style todo list, with custom tasks.
## Author: Azro
## Version: v12.0.376
## SavedVariables: MidnightRoutineDB
## Dependencies: RoutineData
## X-Curse-Project-ID: 1472162
## X-Wago-ID: QN53j5KB

Libs/LibStub/LibStub.lua
"#;

    const ZYGOR: &str = r#"## Interface: 120100
## Title: |cffffffffZygor Guide Viewer|r |cfffe6100(Retail)|r
## Author: Zygor Guides
## - SavedVariablesPerCharacter: ZygorGuidesViewerSettingsChar
## X-Website: http://zygorguides.com/
## Version: 9.6
"#;

    #[test]
    fn parses_routine() {
        let t = Toc::parse(ROUTINE);
        assert_eq!(t.title().as_deref(), Some("Routine"));
        assert_eq!(t.version(), Some("v12.0.376"));
        assert_eq!(t.curse_project_id(), Some(1472162));
        assert_eq!(t.dependencies(), vec!["RoutineData"]);
        assert_eq!(t.interface_versions(), vec![120007, 120100, 120105, 16001]);
    }

    #[test]
    fn parses_zygor() {
        let t = Toc::parse(ZYGOR);
        assert_eq!(t.title().as_deref(), Some("Zygor Guide Viewer (Retail)"));
        assert_eq!(t.version(), Some("9.6"));
        assert_eq!(t.website(), Some("http://zygorguides.com/"));
        assert!(t.curse_project_id().is_none());
        // commented-out directive must not be picked up
        assert!(t.get("- SavedVariablesPerCharacter").is_none());
        assert!(t.get("SavedVariablesPerCharacter").is_none());
    }

    const ELVUI: &str = "## Version: v16.00\n\
## Title: |cff1784d1ElvUI|r Mists [AllowLoadGameType mists]\n\
## Title: |cff1784d1ElvUI|r Midnight [AllowLoadGameType standard]\n\
## Title: |cff1784d1ElvUI|r Forever [AllowLoadGameType camelot][ExcludeLoadGameType standard, classic]\n\
## Title: |cff1784d1ElvUI|r Classic [AllowLoadGameType vanilla]\n\
## OptionalDeps: LibHealComm-4.0 [AllowLoadGameType classic]\n\
## X-Tukui-ProjectID: -2\n";

    #[test]
    fn conditional_lines_follow_the_game_type() {
        let retail = Toc::parse(ELVUI);
        assert_eq!(retail.title().as_deref(), Some("ElvUI Midnight"));
        assert_eq!(retail.get("optionaldeps"), None);

        let vanilla = Toc::parse_for(ELVUI, &["vanilla", "classic"]);
        assert_eq!(vanilla.title().as_deref(), Some("ElvUI Classic"));
        assert_eq!(vanilla.get("optionaldeps"), Some("LibHealComm-4.0"));

        // Unconditional lines are untouched, including ones with brackets inside.
        assert_eq!(
            Toc::parse("## Title: BigWigs [Core]\n").title().as_deref(),
            Some("BigWigs [Core]")
        );
        assert_eq!(
            strip_conditions("x [ExcludeLoadGameType mists]", &["standard"]).as_deref(),
            Some("x")
        );
        assert_eq!(
            strip_conditions("x [ExcludeLoadGameType standard]", &["standard"]),
            None
        );
    }

    #[test]
    fn unreplaced_packager_token_is_no_version() {
        assert_eq!(Toc::parse("## Version: @project-version@\n").version(), None);
        assert_eq!(Toc::parse("## Version: #@project-version@\n").version(), None);
        assert_eq!(Toc::parse("## Version: 5.19.11\n").version(), Some("5.19.11"));
    }

    #[test]
    fn reads_regular_toc_and_refuses_links_and_giants() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("Routine.toc");
        std::fs::write(&real, ROUTINE).unwrap();
        assert_eq!(Toc::from_path(&real).unwrap().version(), Some("v12.0.376"));

        let huge = tmp.path().join("Huge.toc");
        std::fs::write(&huge, vec![b'#'; MAX_TOC_BYTES as usize + 1]).unwrap();
        assert!(matches!(Toc::from_path(&huge), Err(TocError::TooLarge(_))));
        assert!(matches!(Toc::from_path(tmp.path()), Err(TocError::NotAFile)));

        // A .toc symlinked to a file elsewhere is not read from there.
        let link = tmp.path().join("Linked.toc");
        if crate::discover::try_symlink_file(&real, &link) {
            assert!(matches!(Toc::from_path(&link), Err(TocError::NotAFile)));
        } else {
            eprintln!("symlinks not permitted here, skipping the link case");
        }
    }

    #[test]
    fn strips_escapes() {
        assert_eq!(strip_color_codes("|cff00ff00Hi|r there"), "Hi there");
        assert_eq!(strip_color_codes("|TInterface\\x:0|t Name"), "Name");
        assert_eq!(strip_color_codes("a||b"), "a|b");
    }
}
