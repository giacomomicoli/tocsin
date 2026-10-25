//! Make sense of a folder the user picked in the system file dialog.
//!
//! Tocsin never goes looking for the game on its own (no registry, no drive
//! scan): the user points it at a game installation, like most classic addon
//! managers. Any level works — the WoW root, `_retail_`, `Interface` or
//! `AddOns` itself — but only inside a real game install: another game's
//! `AddOns` folder (Elder Scrolls Online keeps one in `Documents`) or a
//! loose copy would be accepted by its name alone, and Tocsin would install
//! WoW addons into it.

use std::path::{Path, PathBuf};

use crate::addon::Flavor;

/// Why a picked folder cannot be used. The messages are shown as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FolderError {
    /// Nothing there belongs to World of Warcraft.
    #[error("That is not a World of Warcraft folder. Pick the game folder, or _retail_ (or another version) inside it.")]
    NotWow,
    /// A WoW install that has never had addons: no `Interface\AddOns` yet.
    #[error(
        "This game has no Interface\\AddOns folder yet. Start the game once (or create the folder), then add it again."
    )]
    NoAddOns,
}

/// Accept a WoW root, a flavor folder (`_retail_`, `_ptr_`, `_anniversary_`,
/// …), its `Interface` folder or the `AddOns` folder itself, and return the
/// `AddOns` folder. A root with several flavors installed resolves to Retail,
/// then Classic Era, then Classic, then the others by name.
pub fn normalize(path: &Path) -> Result<PathBuf, FolderError> {
    let games = game_dirs(path);
    if games.is_empty() {
        return Err(FolderError::NotWow);
    }
    games
        .iter()
        .map(|d| d.join("Interface").join("AddOns"))
        .find(|p| p.is_dir())
        .ok_or(FolderError::NoAddOns)
}

/// The flavor folders `path` is, lies inside, or (for the install root)
/// contains one level down, best first. Only folders that pass
/// [`is_game_dir`] count.
fn game_dirs(path: &Path) -> Vec<PathBuf> {
    // Picked from inside: AddOns → Interface → flavor folder.
    let inside = match name_of(path).as_deref() {
        Some("addons") => path
            .parent()
            .filter(|p| name_of(p).as_deref() == Some("interface"))
            .and_then(Path::parent),
        Some("interface") => path.parent(),
        _ => None,
    };
    if let Some(dir) = inside.or_else(|| is_game_dir(path).then_some(path)) {
        return if is_game_dir(dir) {
            vec![dir.to_path_buf()]
        } else {
            Vec::new()
        };
    }
    // The install root. `DirEntry::file_type` does not follow links, so a
    // linked flavor folder is skipped, like a linked addon folder.
    let mut dirs: Vec<(u8, String, PathBuf)> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter_map(|p| {
            let n = name_of(&p)?;
            (n.len() > 2 && n.starts_with('_') && n.ends_with('_') && is_game_dir(&p))
                .then(|| (preference(&n), n, p))
        })
        .collect();
    dirs.sort();
    dirs.into_iter().map(|(_, _, p)| p).collect()
}

/// A flavor folder has Blizzard's `.flavor.info` from the moment it is
/// installed, the game executable (what CurseForge and WowUp check), and a
/// `WTF` folder once the game has run (also the marker of the pre-8.0
/// layout, where `WTF` and `Interface` sat in the root). None of them is
/// read here, and none may be a link.
fn is_game_dir(dir: &Path) -> bool {
    let is_file = |name: &str| std::fs::symlink_metadata(dir.join(name)).is_ok_and(|m| m.is_file());
    is_file(".flavor.info")
        || GAME_EXES.iter().any(|exe| is_file(exe))
        || crate::install::is_real_dir(&dir.join("WTF"))
}

/// Windows executables of every flavor, as Battle.net writes them (WoW
/// Forever ships `Wow.exe`, the Classic beta `WowB.exe`), and their ARM builds.
const GAME_EXES: &[&str] = &[
    "Wow.exe",
    "WowT.exe",
    "WowB.exe",
    "WowClassic.exe",
    "WowClassicT.exe",
    "WowClassicB.exe",
    "Wow-ARM64.exe",
    "WowT-ARM64.exe",
    "WowB-ARM64.exe",
    "WowClassic-ARM64.exe",
    "WowClassicT-ARM64.exe",
    "WowClassicB-ARM64.exe",
];

/// Order of flavor folders in a root with several installed.
fn preference(name: &str) -> u8 {
    match name {
        "_retail_" => 0,
        "_classic_era_" => 1,
        "_classic_" => 2,
        _ => 3,
    }
}

/// Lower-case last component, if it is valid Unicode.
fn name_of(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_ascii_lowercase)
}

/// Game flavor implied by Blizzard's folder naming (`_retail_`, `_classic_era_`,
/// `_classic_`, `_ptr_`, …). Unknown layouts default to Retail.
pub fn flavor_of(addons_dir: &Path) -> Flavor {
    addons_dir
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .map(str::to_ascii_lowercase)
        .find_map(|seg| match seg.as_str() {
            "_retail_" | "_ptr_" | "_xptr_" | "_beta_" => Some(Flavor::Retail),
            "_classic_era_" => Some(Flavor::Vanilla),
            // What each folder carries as of October 2026, the same snapshot
            // as `client::flavor_from_product`: the Classic Era PTR tests TBC
            // Anniversary, the Classic beta is WoW Forever. The build, read by
            // `client::GameClient` when it can, wins over both.
            "_classic_" | "_classic_ptr_" | "_classic_era_ptr_" | "_classic_titan_" | "_anniversary_" => {
                Some(Flavor::Classic)
            }
            "_classic_forever_" | "_forever_" | "_classic_beta_" => Some(Flavor::Forever),
            _ => None,
        })
        .unwrap_or_default()
}

/// Short label for a game folder, e.g. `Retail`, `Classic Era`, `Anniversary`.
pub fn label_of(addons_dir: &Path) -> &'static str {
    let anniversary = addons_dir
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .any(|seg| seg.to_ascii_lowercase().starts_with("_anniversary"));
    if anniversary {
        return "Anniversary";
    }
    match flavor_of(addons_dir) {
        Flavor::Retail => "Retail",
        Flavor::Vanilla => "Classic Era",
        Flavor::Classic => "Classic",
        Flavor::Forever => "Forever",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_any_level() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("World of Warcraft");
        let addons = root.join("_retail_").join("Interface").join("AddOns");
        std::fs::create_dir_all(&addons).unwrap();
        flavor_info(&root.join("_retail_"), "wow");

        assert_eq!(normalize(&root).unwrap(), addons);
        assert_eq!(normalize(&root.join("_retail_")).unwrap(), addons);
        assert_eq!(
            normalize(&root.join("_retail_").join("Interface")).unwrap(),
            addons
        );
        assert_eq!(normalize(&addons).unwrap(), addons);
        assert_eq!(normalize(&tmp.path().join("nope")), Err(FolderError::NotWow));
    }

    fn flavor_info(dir: &Path, product: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(".flavor.info"),
            format!("Product Flavor!STRING:0\n{product}\n"),
        )
        .unwrap();
    }

    #[test]
    fn refuses_addons_folders_outside_a_game() {
        let tmp = tempfile::tempdir().unwrap();
        // Elder Scrolls Online: Documents\Elder Scrolls Online\live\AddOns.
        let eso = tmp.path().join("Elder Scrolls Online").join("live");
        std::fs::create_dir_all(eso.join("AddOns").join("LibAddonMenu-2.0")).unwrap();
        assert_eq!(normalize(&eso), Err(FolderError::NotWow));
        assert_eq!(normalize(&eso.join("AddOns")), Err(FolderError::NotWow));
        // A hand-copied Interface\AddOns with no game around it.
        let copy = tmp.path().join("backup").join("Interface").join("AddOns");
        std::fs::create_dir_all(&copy).unwrap();
        assert_eq!(normalize(&copy), Err(FolderError::NotWow));
        assert_eq!(normalize(&tmp.path().join("backup")), Err(FolderError::NotWow));
    }

    #[test]
    fn wtf_marks_a_game_folder_too() {
        let tmp = tempfile::tempdir().unwrap();
        let retail = tmp.path().join("_retail_");
        std::fs::create_dir_all(retail.join("WTF")).unwrap();
        std::fs::create_dir_all(retail.join("Interface").join("AddOns")).unwrap();
        assert_eq!(
            normalize(&retail).unwrap(),
            retail.join("Interface").join("AddOns")
        );
    }

    #[test]
    fn executable_marks_a_game_folder_too() {
        let tmp = tempfile::tempdir().unwrap();
        let anniversary = tmp.path().join("_anniversary_");
        std::fs::create_dir_all(anniversary.join("Interface").join("AddOns")).unwrap();
        assert_eq!(normalize(tmp.path()), Err(FolderError::NotWow));
        std::fs::write(anniversary.join("WowClassic.exe"), b"").unwrap();
        assert_eq!(
            normalize(tmp.path()).unwrap(),
            anniversary.join("Interface").join("AddOns")
        );
    }

    #[test]
    fn current_folders_carry_current_games() {
        let f = |dir: &str| flavor_of(&Path::new("/wow").join(dir).join("Interface").join("AddOns"));
        assert_eq!(f("_classic_beta_"), Flavor::Forever);
        assert_eq!(f("_classic_era_ptr_"), Flavor::Classic);
        assert_eq!(f("_classic_titan_"), Flavor::Classic);
        assert_eq!(f("_xptr_"), Flavor::Retail);
    }

    #[test]
    fn fresh_install_has_no_addons_folder() {
        let tmp = tempfile::tempdir().unwrap();
        flavor_info(&tmp.path().join("_retail_"), "wow");
        assert_eq!(normalize(tmp.path()), Err(FolderError::NoAddOns));
        assert_eq!(
            normalize(&tmp.path().join("_retail_")),
            Err(FolderError::NoAddOns)
        );
    }

    #[test]
    fn root_finds_any_flavor_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Only a PTR and Anniversary installed; Retail fresh, without addons.
        flavor_info(&root.join("_retail_"), "wow");
        for (dir, product) in [("_ptr_", "wowt"), ("_anniversary_", "wow_anniversary")] {
            flavor_info(&root.join(dir), product);
            std::fs::create_dir_all(root.join(dir).join("Interface").join("AddOns")).unwrap();
        }
        // A folder shaped like a flavor but not one is ignored.
        std::fs::create_dir_all(root.join("_junk_").join("Interface").join("AddOns")).unwrap();
        assert_eq!(
            normalize(root).unwrap(),
            root.join("_anniversary_").join("Interface").join("AddOns")
        );
        std::fs::create_dir_all(root.join("_retail_").join("Interface").join("AddOns")).unwrap();
        assert_eq!(
            normalize(root).unwrap(),
            root.join("_retail_").join("Interface").join("AddOns")
        );
    }

    #[test]
    fn anniversary_label() {
        let p = Path::new("/g/wow/_anniversary_/Interface/AddOns");
        assert_eq!(label_of(p), "Anniversary");
        assert_eq!(flavor_of(p), Flavor::Classic);
    }

    #[test]
    fn flavor_from_path() {
        assert_eq!(
            flavor_of(Path::new(r"C:\WoW\_retail_\Interface\AddOns")),
            Flavor::Retail
        );
        assert_eq!(
            flavor_of(Path::new("/g/wow/_classic_era_/Interface/AddOns")),
            Flavor::Vanilla
        );
        assert_eq!(
            flavor_of(Path::new("/g/wow/_classic_/Interface/AddOns")),
            Flavor::Classic
        );
        assert_eq!(flavor_of(Path::new("/somewhere/AddOns")), Flavor::Retail);
    }
}
