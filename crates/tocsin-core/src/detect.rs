//! Make sense of a folder the user picked in the system file dialog.
//!
//! Tocsin never goes looking for the game on its own (no registry, no drive
//! scan): the user points it at a game installation, like most classic addon
//! managers. Any level works — the WoW root, `_retail_`, `Interface` or
//! `AddOns` itself — and the flavor is read off the path.

use std::path::{Path, PathBuf};

use crate::addon::Flavor;

/// Accept a WoW root, a `_retail_`/`_classic_era_` folder, an `Interface`
/// folder or the `AddOns` folder itself and return the `AddOns` folder if it
/// exists. A plain root with several flavors installed resolves to Retail.
pub fn normalize(path: &Path) -> Option<PathBuf> {
    let candidates = [
        path.to_path_buf(),
        path.join("Interface").join("AddOns"),
        path.join("AddOns"),
        path.join("_retail_").join("Interface").join("AddOns"),
        path.join("_classic_era_").join("Interface").join("AddOns"),
        path.join("_classic_").join("Interface").join("AddOns"),
    ];
    candidates.into_iter().find(|p| {
        p.is_dir()
            && p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("AddOns"))
    })
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
            "_classic_era_" | "_classic_era_ptr_" => Some(Flavor::Vanilla),
            "_classic_" | "_classic_ptr_" | "_classic_beta_" => Some(Flavor::Classic),
            "_classic_forever_" | "_forever_" => Some(Flavor::Forever),
            _ => None,
        })
        .unwrap_or_default()
}

/// Short label for a game folder, e.g. `Retail`, `Classic Era`.
pub fn label_of(addons_dir: &Path) -> &'static str {
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

        assert_eq!(normalize(&root).unwrap(), addons);
        assert_eq!(normalize(&root.join("_retail_")).unwrap(), addons);
        assert_eq!(
            normalize(&root.join("_retail_").join("Interface")).unwrap(),
            addons
        );
        assert_eq!(normalize(&addons).unwrap(), addons);
        assert!(normalize(&tmp.path().join("nope")).is_none());
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
