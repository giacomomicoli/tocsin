//! Which game client lives in a game folder, read from Blizzard's own files.
//!
//! Every flavor folder (`_retail_`, `_classic_era_`, …) has a `.flavor.info`
//! naming its product (`wow`, `wow_classic`, `wow_classic_era`, `wowf`), and
//! the install root has a `.build.info` table with one row per installed
//! product, including its `Version` (`12.1.5.70448`). Together they answer
//! "what game is this and which build?" without guessing from folder names.
//!
//! Both are small text files; when they are missing (a hand-copied AddOns
//! folder, a fixture) we fall back to the folder name and know no version.

use std::path::Path;

use crate::addon::Flavor;

const MAX_INFO_BYTES: u64 = 256 * 1024;

/// A game client: flavor plus, when known, the exact build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameClient {
    /// Retail, Classic Era, …
    pub flavor: Flavor,
    /// Blizzard product code (`wow`, `wow_classic_era`), if `.flavor.info` was found.
    pub product: Option<String>,
    /// `(major, minor, patch)` from `.build.info`, if found.
    pub version: Option<(u32, u32, u32)>,
}

impl GameClient {
    /// Detect the client owning `addons_dir` (`…/<flavor>/Interface/AddOns`).
    pub fn detect(addons_dir: &Path) -> Self {
        let flavor_dir = addons_dir.parent().and_then(Path::parent);
        let product = flavor_dir.and_then(|d| read_flavor_info(&d.join(".flavor.info")));
        let version = match (&product, flavor_dir.and_then(Path::parent)) {
            (Some(p), Some(root)) => read_build_info(&root.join(".build.info"), p),
            _ => None,
        };
        // The build is the one answer that cannot go stale: Blizzard reuses
        // product codes for other games (`wow_classic_beta` carried Cataclysm
        // and Mists betas, then WoW Forever 1.60). The product and the folder
        // name are fallbacks for a `.build.info` that lacks the row, as it did
        // for an installed `_classic_beta_`.
        let flavor = version
            .map(|(ma, mi, pa)| Flavor::of_interface(ma * 10000 + mi * 100 + pa))
            .or_else(|| product.as_deref().and_then(flavor_from_product))
            .unwrap_or_else(|| crate::detect::flavor_of(addons_dir));
        Self {
            flavor,
            product,
            version,
        }
    }

    /// Client with a known flavor and no version (tests, fixtures).
    pub fn flavor_only(flavor: Flavor) -> Self {
        Self {
            flavor,
            product: None,
            version: None,
        }
    }

    /// The `## Interface` code of this build (`12.1.5` → `120105`).
    pub fn interface(&self) -> Option<u32> {
        self.version.map(|(ma, mi, pa)| ma * 10000 + mi * 100 + pa)
    }

    /// `12.1.5`, or the flavor label when the build is unknown.
    pub fn describe(&self) -> String {
        match self.version {
            Some((ma, mi, pa)) => format!("{} {ma}.{mi}.{pa}", self.flavor.label()),
            None => self.flavor.label().to_string(),
        }
    }
}

/// `.flavor.info`: a header line, then the product code.
fn read_flavor_info(path: &Path) -> Option<String> {
    let text = read_small(path)?;
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .nth(1)
        .map(str::to_string)
}

/// Blizzard product code → flavor, for when the build is unknown. Betas and
/// PTRs count as the game they currently carry, as of October 2026 (wago.tools
/// build list): the Classic Era PTR tests TBC Anniversary (2.5) since November
/// 2025, the Classic beta is WoW Forever (1.60) since September 2026.
pub fn flavor_from_product(product: &str) -> Option<Flavor> {
    let p = product.trim().to_ascii_lowercase();
    Some(match p.as_str() {
        "wow" | "wowt" | "wowxptr" | "wow_beta" | "wowb" => Flavor::Retail,
        "wow_classic_era" | "wow_classic_era_beta" => Flavor::Vanilla,
        "wow_classic"
        | "wow_classic_ptr"
        | "wow_classic_era_ptr"
        | "wow_anniversary"
        | "wow_classic_titan" => Flavor::Classic,
        "wowf" | "wow_forever" | "wow_classic_beta" => Flavor::Forever,
        _ => return None,
    })
}

/// `.build.info`: pipe-separated table, header cells like `Version!STRING:0`.
/// Returns the version of the row whose `Product` is `product`.
fn read_build_info(path: &Path, product: &str) -> Option<(u32, u32, u32)> {
    let text = read_small(path)?;
    let mut lines = text.lines();
    let header: Vec<&str> = lines
        .next()?
        .split('|')
        .map(|h| h.split('!').next().unwrap_or("").trim())
        .collect();
    let version_col = header.iter().position(|h| *h == "Version")?;
    let product_col = header.iter().position(|h| *h == "Product")?;
    for line in lines {
        let cells: Vec<&str> = line.split('|').collect();
        if cells.get(product_col).map(|c| c.trim()) == Some(product) {
            return cells.get(version_col).and_then(|v| parse_version(v));
        }
    }
    None
}

/// `12.1.5.70448` → `(12, 1, 5)`.
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.trim().split('.').map(|p| p.parse::<u32>().ok());
    Some((it.next()??, it.next()??, it.next()??))
}

/// Size-capped, and never through a link: the same rule as every addon file.
fn read_small(path: &Path) -> Option<String> {
    match crate::discover::read_regular_file(path, MAX_INFO_BYTES).ok()? {
        crate::discover::Bounded::Read(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUILD_INFO: &str = "Branch!STRING:0|Active!DEC:1|Version!STRING:0|Product!STRING:0\n\
us|1|1.60.1.70450|wow_classic_beta\n\
eu|1|12.1.5.70448|wow\n\
us|1|1.60.1.70532|wowf\n";

    #[test]
    fn detects_from_blizzard_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let addons = root.join("_retail_").join("Interface").join("AddOns");
        std::fs::create_dir_all(&addons).unwrap();
        std::fs::write(
            root.join("_retail_").join(".flavor.info"),
            "Product Flavor!STRING:0\nwow\n",
        )
        .unwrap();
        std::fs::write(root.join(".build.info"), BUILD_INFO).unwrap();

        let c = GameClient::detect(&addons);
        assert_eq!(c.flavor, Flavor::Retail);
        assert_eq!(c.product.as_deref(), Some("wow"));
        assert_eq!(c.version, Some((12, 1, 5)));
        assert_eq!(c.interface(), Some(120105));
        assert_eq!(c.describe(), "Retail 12.1.5");
    }

    #[test]
    fn falls_back_to_folder_name() {
        let tmp = tempfile::tempdir().unwrap();
        let addons = tmp.path().join("_classic_era_").join("Interface").join("AddOns");
        std::fs::create_dir_all(&addons).unwrap();
        let c = GameClient::detect(&addons);
        assert_eq!(c.flavor, Flavor::Vanilla);
        assert_eq!(c.version, None);
        assert_eq!(c.describe(), "Classic Era");
    }

    /// A `_classic_beta_` folder, with `.build.info` as given (or none).
    fn classic_beta(root: &Path, build_info: Option<&str>) -> std::path::PathBuf {
        let addons = root.join("_classic_beta_").join("Interface").join("AddOns");
        std::fs::create_dir_all(&addons).unwrap();
        std::fs::write(
            root.join("_classic_beta_").join(".flavor.info"),
            "Product Flavor!STRING:0\nwow_classic_beta\n",
        )
        .unwrap();
        if let Some(info) = build_info {
            std::fs::write(root.join(".build.info"), info).unwrap();
        }
        addons
    }

    #[test]
    fn build_decides_over_product() {
        // The Classic beta carried Mists, then WoW Forever: the build says which.
        let tmp = tempfile::tempdir().unwrap();
        let addons = classic_beta(tmp.path(), Some(BUILD_INFO));
        assert_eq!(GameClient::detect(&addons).describe(), "Forever 1.60.1");
        let mists = "Branch!STRING:0|Version!STRING:0|Product!STRING:0\nus|5.5.4.70000|wow_classic_beta\n";
        std::fs::write(tmp.path().join(".build.info"), mists).unwrap();
        assert_eq!(GameClient::detect(&addons).flavor, Flavor::Classic);
    }

    #[test]
    fn product_decides_when_build_info_lacks_the_row() {
        // Seen on a real install: `.build.info` listed `wow` and `wowf` only.
        let tmp = tempfile::tempdir().unwrap();
        let info = "Branch!STRING:0|Version!STRING:0|Product!STRING:0\neu|12.1.5.70575|wow\n";
        let c = GameClient::detect(&classic_beta(tmp.path(), Some(info)));
        assert_eq!((c.flavor, c.version), (Flavor::Forever, None));
    }

    #[test]
    fn anniversary_is_tbc() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let addons = root.join("_anniversary_").join("Interface").join("AddOns");
        std::fs::create_dir_all(&addons).unwrap();
        std::fs::write(
            root.join("_anniversary_").join(".flavor.info"),
            "Product Flavor!STRING:0\nwow_anniversary\n",
        )
        .unwrap();
        let info = "Branch!STRING:0|Version!STRING:0|Product!STRING:0\neu|2.5.6.70006|wow_anniversary\n";
        std::fs::write(root.join(".build.info"), info).unwrap();
        assert_eq!(GameClient::detect(&addons).describe(), "Classic 2.5.6");
    }

    #[test]
    fn products() {
        assert_eq!(flavor_from_product("wow"), Some(Flavor::Retail));
        assert_eq!(flavor_from_product("wow_classic_era"), Some(Flavor::Vanilla));
        assert_eq!(flavor_from_product("wow_classic"), Some(Flavor::Classic));
        assert_eq!(flavor_from_product("wow_classic_era_ptr"), Some(Flavor::Classic));
        assert_eq!(flavor_from_product("wow_anniversary"), Some(Flavor::Classic));
        assert_eq!(flavor_from_product("wow_classic_titan"), Some(Flavor::Classic));
        assert_eq!(flavor_from_product("wowf"), Some(Flavor::Forever));
        assert_eq!(flavor_from_product("wow_classic_beta"), Some(Flavor::Forever));
        assert_eq!(flavor_from_product("hero"), None);
        assert_eq!(parse_version("12.1.5.70448"), Some((12, 1, 5)));
        assert_eq!(parse_version("garbage"), None);
    }
}
