//! Group installed addons into families: an addon and the companions that
//! ship with it (`Routine` + `RoutineData`, `DBM-Core` + `DBM-*`, …).
//!
//! Two addons belong to the same family when any of these holds:
//!
//! * one names the other in `## Dependencies` / `## RequiredDeps` (either
//!   direction: a module depends on its core, Routine depends on its data),
//!   or in `## OptionalDeps` when both carry the same `## Author`;
//! * one declares `## X-Part-Of: <folder>` pointing at the other;
//! * both resolve to the same GitHub repository;
//! * one lists the other in `## X-Tukui-ProjectFolders` (ElvUI's siblings);
//! * one names the other in `## Group` (Blizzard's addon-list grouping) or
//!   `## Globe-Main` (WeakAuras).
//!
//! Families are connected components of that graph. The **head** of a family
//! is the addon others point at with `X-Part-Of`; failing that, an
//! always-loaded addon over `LoadOnDemand` modules, then the one with the
//! shortest title (`Routine` over `Routine Data`), ties broken by how many
//! members depend on it.

use std::collections::{BTreeMap, HashMap};

use crate::addon::InstalledAddon;
use crate::sources::SourceRef;

/// A family: indices into the slice passed to [`group`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    /// Index of the head addon.
    pub head: usize,
    /// Indices of the other members, sorted by folder name.
    pub members: Vec<usize>,
}

/// Partition `addons` into families, ordered like the heads appear in `addons`.
/// Single addons form a family of one.
pub fn group(addons: &[InstalledAddon]) -> Vec<Family> {
    let n = addons.len();
    let by_folder: HashMap<String, usize> = addons
        .iter()
        .enumerate()
        .map(|(i, a)| (a.folder.to_ascii_lowercase(), i))
        .collect();
    let lookup = |name: &str| by_folder.get(&name.trim().to_ascii_lowercase()).copied();

    let mut uf = UnionFind::new(n);
    let mut depended_on = vec![0usize; n];
    let mut part_of: Vec<Option<usize>> = vec![None; n];

    for (i, a) in addons.iter().enumerate() {
        for dep in a.toc.dependencies() {
            if let Some(j) = lookup(&dep) {
                if j != i {
                    uf.union(i, j);
                    depended_on[j] += 1;
                }
            }
        }
        // `## OptionalDeps` counts only between addons by the same author:
        // Routine v12.0.377 demoted RoutineData from a hard to an optional
        // dependency (so the game shows its "not installed" warning), and the
        // two are still one bundle. Across authors it is just a hint that an
        // addon integrates with another (`OptionalDeps: Ace3, LibStub`).
        let author = a.toc.author().map(str::to_ascii_lowercase);
        if author.is_some() {
            for dep in a.toc.optional_dependencies() {
                if let Some(j) = lookup(&dep) {
                    if j != i && addons[j].toc.author().map(str::to_ascii_lowercase) == author {
                        uf.union(i, j);
                        depended_on[j] += 1;
                    }
                }
            }
        }
        // `X-Part-Of`, WeakAuras' `Globe-Main` and Blizzard's `## Group` (the
        // addon list grouping, 11.x+) all name the head of the bundle.
        for key in ["x-part-of", "globe-main", "group"] {
            if let Some(parent) = a.toc.get(key).and_then(lookup) {
                if parent != i {
                    uf.union(i, parent);
                    part_of[i] = Some(parent);
                }
            }
        }
        // ElvUI lists its sibling folders: `## X-Tukui-ProjectFolders: ElvUI, ElvUI_Libraries, ElvUI_Options`.
        if let Some(list) = a.toc.get("x-tukui-projectfolders") {
            for j in list.split(',').filter_map(lookup) {
                if j != i {
                    uf.union(i, j);
                }
            }
        }
    }

    // Same GitHub repo → same family.
    let mut by_repo: HashMap<(String, String), usize> = HashMap::new();
    for (i, a) in addons.iter().enumerate() {
        for s in &a.sources {
            if let SourceRef::Github { owner, repo, .. } = s {
                let key = (owner.to_ascii_lowercase(), repo.to_ascii_lowercase());
                match by_repo.get(&key) {
                    Some(&j) => uf.union(i, j),
                    None => {
                        by_repo.insert(key, i);
                    }
                }
                break;
            }
        }
    }

    // Collect components, preserving first-appearance order.
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut order: Vec<usize> = Vec::new();
    for i in 0..n {
        let root = uf.find(i);
        let entry = components.entry(root).or_insert_with(|| {
            order.push(root);
            Vec::new()
        });
        entry.push(i);
    }

    let mut families: Vec<Family> = order
        .into_iter()
        .map(|root| {
            let mut members = components.remove(&root).unwrap_or_default();
            let head = pick_head(addons, &members, &part_of, &depended_on);
            members.retain(|&i| i != head);
            members.sort_by_key(|&i| addons[i].folder.to_ascii_lowercase());
            Family { head, members }
        })
        .collect();
    families.sort_by_key(|f| addons[f.head].display_name.to_ascii_lowercase());
    families
}

fn pick_head(
    addons: &[InstalledAddon],
    members: &[usize],
    part_of: &[Option<usize>],
    depended_on: &[usize],
) -> usize {
    if members.len() == 1 {
        return members[0];
    }
    // Explicit X-Part-Of wins: the most-pointed-at parent.
    let mut votes: HashMap<usize, usize> = HashMap::new();
    for &i in members {
        if let Some(p) = part_of[i] {
            *votes.entry(p).or_default() += 1;
        }
    }
    if let Some((&p, _)) = votes.iter().max_by_key(|(_, v)| **v) {
        if members.contains(&p) {
            return p;
        }
    }
    // Otherwise: an always-loaded addon over load-on-demand modules (ElvUI
    // over ElvUI_Options, WeakAuras over its five companions), then the
    // shortest title, then most depended on, then folder name.
    *members
        .iter()
        .min_by(|&&a, &&b| {
            let (la, lb) = (
                addons[a].toc.is_load_on_demand(),
                addons[b].toc.is_load_on_demand(),
            );
            let ta = addons[a].display_name.trim().len();
            let tb = addons[b].display_name.trim().len();
            la.cmp(&lb)
                .then(ta.cmp(&tb))
                .then(depended_on[b].cmp(&depended_on[a]))
                .then(addons[a].folder.cmp(&addons[b].folder))
        })
        .expect("non-empty family")
}

/// Member folder → its family head's folder, for every family in `families`
/// (from [`group`] over the same `addons`). Heads are not keys.
pub fn heads(addons: &[InstalledAddon], families: &[Family]) -> HashMap<String, String> {
    families
        .iter()
        .flat_map(|f| {
            let head = &addons[f.head].folder;
            f.members
                .iter()
                .map(move |&i| (addons[i].folder.clone(), head.clone()))
        })
        .collect()
}

/// Hard dependencies (`## Dependencies` / `## RequiredDeps`) that are not
/// installed, per addon index. Blizzard's own addons are never reported: they
/// ship with the game and are not in `Interface/AddOns`.
pub fn missing_dependencies(addons: &[InstalledAddon]) -> BTreeMap<usize, Vec<String>> {
    let installed: std::collections::HashSet<String> =
        addons.iter().map(|a| a.folder.to_ascii_lowercase()).collect();
    let mut out = BTreeMap::new();
    for (i, a) in addons.iter().enumerate() {
        let mut missing: Vec<String> = a
            .toc
            .dependencies()
            .into_iter()
            .filter(|d| !d.to_ascii_lowercase().starts_with("blizzard_"))
            .filter(|d| !installed.contains(&d.to_ascii_lowercase()))
            .collect();
        missing.sort();
        missing.dedup();
        if !missing.is_empty() {
            out.insert(i, missing);
        }
    }
    out
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }
    fn find(&mut self, i: usize) -> usize {
        let mut r = i;
        while self.parent[r] != r {
            r = self.parent[r];
        }
        let mut c = i;
        while self.parent[c] != r {
            let next = self.parent[c];
            self.parent[c] = r;
            c = next;
        }
        r
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toc::Toc;
    use std::path::PathBuf;

    fn addon(folder: &str, toc: &str) -> InstalledAddon {
        let toc = Toc::parse(toc);
        InstalledAddon {
            folder: folder.into(),
            path: PathBuf::new(),
            toc_path: PathBuf::new(),
            installed: None,
            sources: SourceRef::candidates_from_toc(&toc),
            display_name: toc.title().unwrap_or_else(|| folder.into()),
            issues: Vec::new(),
            disabled: false,
            toc,
        }
    }

    #[test]
    fn routine_owns_routine_data() {
        let addons = vec![
            addon("RoutineData", "## Title: Routine Data\n"),
            addon(
                "MidnightRoutine",
                "## Title: |cff2ae7c6Routine\n## Dependencies: RoutineData\n",
            ),
            addon("ZygorGuidesViewer", "## Title: Zygor\n"),
        ];
        let fams = group(&addons);
        assert_eq!(fams.len(), 2);
        let routine = fams
            .iter()
            .find(|f| addons[f.head].folder == "MidnightRoutine")
            .unwrap();
        assert_eq!(routine.members, vec![0]);
        assert!(fams
            .iter()
            .any(|f| addons[f.head].folder == "ZygorGuidesViewer" && f.members.is_empty()));
    }

    #[test]
    fn heads_map_members_only() {
        let addons = vec![
            addon("RoutineData", "## Title: Routine Data\n"),
            addon(
                "MidnightRoutine",
                "## Title: |cff2ae7c6Routine\n## Dependencies: RoutineData\n",
            ),
            addon("ZygorGuidesViewer", "## Title: Zygor\n"),
        ];
        let heads = heads(&addons, &group(&addons));
        assert_eq!(heads.len(), 1);
        assert_eq!(heads["RoutineData"], "MidnightRoutine");
    }

    #[test]
    fn optional_deps_group_only_within_the_same_author() {
        let addons = vec![
            addon(
                "MidnightRoutine",
                "## Title: |cff2ae7c6Routine\n## Author: Azro\n## OptionalDeps: RoutineData, Ace3\n",
            ),
            addon("RoutineData", "## Title: Routine Data\n## Author: Azro\n"),
            addon("Ace3", "## Title: Ace3\n## Author: Ace3 Development Team\n"),
        ];
        let fams = group(&addons);
        assert_eq!(fams.len(), 2);
        let routine = fams.iter().find(|f| f.head == 0).unwrap();
        assert_eq!(routine.members, vec![1]);
    }

    #[test]
    fn modules_group_under_core_by_dependency_and_part_of() {
        let addons = vec![
            addon("DBM-Raids", "## Title: DBM - Raids\n## Dependencies: DBM-Core\n"),
            addon("DBM-Core", "## Title: Deadly Boss Mods\n"),
            addon(
                "DBM-Party",
                "## Title: DBM - Dungeons\n## RequiredDeps: DBM-Core\n## X-Part-Of: DBM-Core\n",
            ),
        ];
        let fams = group(&addons);
        assert_eq!(fams.len(), 1);
        assert_eq!(addons[fams[0].head].folder, "DBM-Core");
        assert_eq!(fams[0].members, vec![2, 0]);
    }

    #[test]
    fn group_and_globe_main_name_the_head() {
        let addons = vec![
            addon(
                "ElvUI_Options",
                "## Title: ElvUI Options\n## LoadOnDemand: 1\n## RequiredDeps: ElvUI\n",
            ),
            addon(
                "ElvUI",
                "## Title: ElvUI Midnight\n## RequiredDeps: ElvUI_Libraries\n",
            ),
            addon(
                "ElvUI_Libraries",
                "## Title: ElvUI Libraries\n## Group: ElvUI_Libraries\n",
            ),
            addon(
                "WeakAurasTemplates",
                "## Title: WeakAuras Templates\n## Globe-Main: WeakAuras\n",
            ),
            addon("WeakAuras", "## Title: WeakAuras\n"),
        ];
        let fams = group(&addons);
        assert_eq!(fams.len(), 2);
        let elv = fams.iter().find(|f| addons[f.head].folder == "ElvUI").unwrap();
        assert_eq!(elv.members.len(), 2);
        let wa = fams
            .iter()
            .find(|f| addons[f.head].folder == "WeakAuras")
            .unwrap();
        assert_eq!(wa.members, vec![3]);
    }

    #[test]
    fn same_github_repo_groups() {
        let addons = vec![
            addon("A", "## Title: A\n## X-Website: https://github.com/x/y\n"),
            addon(
                "ALong",
                "## Title: A Long Name\n## X-Website: https://github.com/X/Y\n",
            ),
            addon("B", "## Title: B\n## X-Website: https://github.com/x/z\n"),
        ];
        let fams = group(&addons);
        assert_eq!(fams.len(), 2);
        assert_eq!(addons[fams[0].head].folder, "A");
        assert_eq!(fams[0].members, vec![1]);
    }

    #[test]
    fn reports_missing_hard_dependencies() {
        let addons = vec![
            addon(
                "MidnightRoutine",
                "## Title: Routine\n## Dependencies: RoutineData, Blizzard_Professions\n",
            ),
            addon("Fine", "## Title: Fine\n## OptionalDeps: Whatever\n"),
        ];
        let m = missing_dependencies(&addons);
        assert_eq!(m.get(&0), Some(&vec!["RoutineData".to_string()]));
        assert_eq!(m.get(&1), None);
    }

    #[test]
    fn missing_dependencies_are_ignored() {
        let addons = vec![addon(
            "X",
            "## Title: X\n## Dependencies: NotInstalled, AlsoMissing\n",
        )];
        let fams = group(&addons);
        assert_eq!(
            fams,
            vec![Family {
                head: 0,
                members: vec![]
            }]
        );
    }
}
