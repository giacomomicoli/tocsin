//! User configuration: `tocsin.toml`.
//!
//! The file lives in the per-user config directory (never inside the repo or
//! the game folder) and holds two things:
//!
//! * settings — the `AddOns` path and optional API keys;
//! * overrides — per-addon source declarations for addons whose `.toc` does
//!   not say where they come from (e.g. Zygor).
//!
//! API keys may also be supplied through environment variables, which always
//! take precedence: `TOCSIN_CURSEFORGE_KEY`.
//!
//! ```toml
//! [settings]
//! addons_dir = 'C:\Program Files (x86)\World of Warcraft\_retail_\Interface\AddOns'
//!
//! [addons.ZygorGuidesViewer]
//! name = "Zygor Guides"
//! source = { type = "manual", url = "https://zygorguides.com/" }
//!
//! [addons.MidnightRoutine]
//! source = { type = "github", owner = "LoyalFTW", repo = "Midnight-Routine" }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::sources::SourceRef;

/// Hard cap for the config file; anything bigger is almost certainly not ours.
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

/// Per-addon override. Keyed by the addon **folder** name.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceOverride {
    /// Friendlier display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Where to check for updates. A provider Tocsin dropped reads as none.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::sources::deserialize_lenient"
    )]
    pub source: Option<SourceRef>,
    /// Installed version when the `.toc` lies or omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Skip this addon when checking for updates.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignore: bool,
}

/// Global settings.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Path of `Interface/AddOns`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addons_dir: Option<PathBuf>,
    /// Every game installation the user added (one `Interface/AddOns` each:
    /// Retail, Classic Era, …). The first one is shown by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub game_folders: Vec<PathBuf>,
    /// CurseForge API key (optional). Prefer the env var `TOCSIN_CURSEFORGE_KEY`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curseforge_api_key: Option<String>,
}

/// Written by hand so a `{:?}` of the manifest never prints the legacy key.
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("addons_dir", &self.addons_dir)
            .field("game_folders", &self.game_folders)
            .field(
                "curseforge_api_key",
                &if self.curseforge_api_key.is_some() {
                    "set"
                } else {
                    "unset"
                },
            )
            .finish()
    }
}

/// The whole `tocsin.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Global settings.
    #[serde(default)]
    pub settings: Settings,
    /// Per-addon overrides, keyed by folder name.
    #[serde(default, rename = "addons")]
    pub overrides: BTreeMap<String, SourceOverride>,
}

impl Manifest {
    /// Default location: `%APPDATA%\tocsin\config\tocsin.toml` on Windows,
    /// `~/.config/tocsin/tocsin.toml` on Linux (see [`crate::paths`]).
    pub fn default_path() -> Option<PathBuf> {
        crate::paths::project_dirs().map(|d| d.config_dir().join("tocsin.toml"))
    }

    /// Load from `path`; a missing file yields an empty manifest.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::metadata(path) {
            Ok(meta) if meta.len() > MAX_MANIFEST_BYTES => {
                anyhow::bail!("manifest {} is too large", path.display())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        }
        let text = std::fs::read_to_string(path)?;
        let mut m: Self = toml::from_str(&text)?;
        // Migrate the single `addons_dir` of early versions into the list.
        if let Some(dir) = m.settings.addons_dir.take() {
            if !m.settings.game_folders.contains(&dir) {
                m.settings.game_folders.insert(0, dir);
            }
        }
        Ok(m)
    }

    /// Serialize to `path`, creating parent directories. Written atomically
    /// (temp file + rename) so a crash never leaves a half-written config.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// CurseForge key: environment first, then the file.
    pub fn curseforge_api_key(&self) -> Option<String> {
        std::env::var("TOCSIN_CURSEFORGE_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| self.settings.curseforge_api_key.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let text = r#"
[settings]
addons_dir = "/games/wow/Interface/AddOns"

[addons.ZygorGuidesViewer]
name = "Zygor Guides"
source = { type = "manual", url = "https://zygorguides.com/" }
version = "9.6.37349"

[addons.MidnightRoutine]
source = { type = "github", owner = "LoyalFTW", repo = "Midnight-Routine" }
ignore = true
"#;
        let m: Manifest = toml::from_str(text).unwrap();
        assert_eq!(m.overrides.len(), 2);
        let z = &m.overrides["ZygorGuidesViewer"];
        assert_eq!(z.name.as_deref(), Some("Zygor Guides"));
        assert!(matches!(z.source, Some(SourceRef::Manual { .. })));
        assert!(m.overrides["MidnightRoutine"].ignore);

        let out = toml::to_string(&m).unwrap();
        let back: Manifest = toml::from_str(&out).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn debug_never_prints_the_key() {
        let mut m = Manifest::default();
        m.settings.curseforge_api_key = Some("sk-secret-123".into());
        let shown = format!("{m:?} {m:#?}");
        assert!(!shown.contains("sk-secret-123"));
        assert!(shown.contains("\"set\""));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let text = "[settings]\nbogus = 1\n";
        assert!(toml::from_str::<Manifest>(text).is_err());
    }

    #[test]
    fn save_and_load() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("sub").join("tocsin.toml");
        let mut m = Manifest::default();
        m.overrides.insert(
            "X".into(),
            SourceOverride {
                ignore: true,
                ..Default::default()
            },
        );
        m.save(&p).unwrap();
        assert_eq!(Manifest::load(&p).unwrap(), m);
        assert_eq!(
            Manifest::load(&tmp.path().join("nope.toml")).unwrap(),
            Manifest::default()
        );
    }
}
