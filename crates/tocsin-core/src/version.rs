//! Lenient version comparison.
//!
//! Addon versions are anything but SemVer: `v12.0.376`, `9.6`, `9.6.37349`,
//! `2.1.0-beta3`, `r1234`, `11.0.2.1`. We compare by numeric components first,
//! then fall back to the raw string so that two different strings never
//! compare equal.

use std::cmp::Ordering;
use std::fmt;

/// A parsed addon version. Keeps the original text for display.
///
/// Equality and ordering are *semantic*: `v1.2.3 == 1.2.3`, `9.6 == 9.6.0`.
#[derive(Debug, Clone)]
pub struct Version {
    raw: String,
    numbers: Vec<u64>,
    /// Pre-release part, if any: (rank: alpha < beta < rc, trailing numbers).
    pre: Option<(u8, Vec<u64>)>,
}

/// Pre-release markers with their rank. Lower rank = earlier in the cycle.
const MARKERS: &[(&str, u8)] = &[("alpha", 0), ("dev", 0), ("pre", 1), ("beta", 1), ("rc", 2)];

/// Where pre-release `marker` starts in `lower`, counting only a marker that
/// follows the first digit and does not continue a word. A file title brings
/// the addon's name along, and names hold these letters: "Warcraft" has
/// "rc", "Preydator" has "pre", "DevTool" has "dev". Taken as markers, they
/// made `WarcraftLogs-2.1.0.zip` a pre-release with no numbers at all.
fn marker_at(lower: &str, marker: &str) -> Option<usize> {
    let first_digit = lower.find(|c: char| c.is_ascii_digit())?;
    let bytes = lower.as_bytes();
    lower
        .match_indices(marker)
        .map(|(at, _)| at)
        .find(|&at| at > first_digit && !bytes[at - 1].is_ascii_alphabetic())
}

/// Archive extensions a file title standing in for a version ends with.
const ARCHIVE_EXTS: &[&str] = &[".tar.gz", ".zip", ".7z", ".rar"];

fn ends_with_ignore_case(s: &str, suffix: &str) -> bool {
    let Some(at) = s.len().checked_sub(suffix.len()) else {
        return false;
    };
    s.is_char_boundary(at) && s[at..].eq_ignore_ascii_case(suffix)
}

/// Where the version starts after a leading name, if there is one: right
/// after a `-`, `_`, space or dot that follows a name with letters in it, at
/// a dotted number (`2.9.0`, `v12.0.376`). Plain digit runs (`r1234`) and a
/// text with no name before its number are left alone. The dot is Details':
/// its `.toc` says `#Details.20261104.15400.172-1-gcec582c` and CurseForge
/// `Details.20261107.15402.172`. A dot inside a version (`v12.0.376`) is
/// never cut there: the label would lose a number, which [`Version::label`]
/// refuses.
fn version_start(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    (1..bytes.len()).find(|&at| {
        matches!(bytes[at - 1], b'-' | b'_' | b' ' | b'.')
            && bytes[..at - 1].iter().any(u8::is_ascii_alphabetic)
            && is_dotted_number(&bytes[at..])
    })
}

/// `1.2…` or `v1.2…`: digits, a dot, a digit.
fn is_dotted_number(b: &[u8]) -> bool {
    let b = match b.first() {
        Some(b'v' | b'V') => &b[1..],
        _ => b,
    };
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    digits > 0 && b.get(digits) == Some(&b'.') && b.get(digits + 1).is_some_and(u8::is_ascii_digit)
}

/// Extract every run of digits from `s`, in order, capped at 8 components.
fn digit_runs(s: &str) -> Vec<u64> {
    let mut numbers = Vec::with_capacity(4);
    let mut current: Option<u64> = None;
    for c in s.chars() {
        if let Some(d) = c.to_digit(10) {
            let d = u64::from(d);
            current = Some(current.map_or(d, |n| n.saturating_mul(10).saturating_add(d)));
        } else if let Some(n) = current.take() {
            numbers.push(n);
            if numbers.len() >= 8 {
                return numbers;
            }
        }
    }
    if let Some(n) = current {
        numbers.push(n);
    }
    numbers
}

impl Version {
    /// Parse any version-ish string. Never fails.
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim().to_string();
        let lower = raw.to_ascii_lowercase();
        // Everything from a pre-release marker onwards ("-beta1", "rc2") is
        // excluded from the numeric comparison: 2.0.0-beta1 must sort below 2.0.0.
        let marker = MARKERS
            .iter()
            .filter_map(|(m, rank)| marker_at(&lower, m).map(|at| (at, *rank, m.len())))
            .min_by_key(|(at, _, _)| *at);
        let (numeric_part, pre) = match marker {
            Some((at, rank, len)) => (&lower[..at], Some((rank, digit_runs(&lower[at + len..])))),
            None => (lower.as_str(), None),
        };
        let numbers = digit_runs(numeric_part);
        Self { raw, numbers, pre }
    }

    /// The version text as written by the author.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// The version as shown to the user: the author's text without what a
    /// file name brings along. Providers hand back free text: CurseForge a
    /// file title (`AztarecHelper-2.9.0.zip`), WoWInterface and Tukui what
    /// the author typed, a `.toc` whatever its build wrote. So the archive
    /// extension and a name before the version (`AddonName-`, `AddonName v`)
    /// are dropped, and nothing else.
    ///
    /// The label is kept only when it compares exactly as the original (same
    /// numbers, same pre-release): it never shows a version the comparison
    /// did not see. `WeakAuras2-5.12.0` would lose the `2`, so it stays as is.
    pub fn label(&self) -> &str {
        let mut s = self.raw.as_str();
        if let Some(ext) = ARCHIVE_EXTS.iter().find(|e| ends_with_ignore_case(s, e)) {
            s = &s[..s.len() - ext.len()];
        }
        if let Some(at) = version_start(s) {
            s = &s[at..];
        }
        if s.is_empty() || s.len() == self.raw.len() {
            return &self.raw;
        }
        let cleaned = Version::parse(s);
        if cleaned.numbers == self.numbers && cleaned.pre == self.pre {
            s
        } else {
            &self.raw
        }
    }

    /// Numeric components, e.g. `[12, 0, 376]`.
    pub fn numbers(&self) -> &[u64] {
        &self.numbers
    }

    /// Whether `other` is strictly newer than `self`.
    pub fn is_older_than(&self, other: &Version) -> bool {
        self.cmp(other) == Ordering::Less
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        // Compare component-wise; a missing component counts as 0, so that
        // 9.6 == 9.6.0 numerically.
        let len = self.numbers.len().max(other.numbers.len());
        for i in 0..len {
            let a = self.numbers.get(i).copied().unwrap_or(0);
            let b = other.numbers.get(i).copied().unwrap_or(0);
            match a.cmp(&b) {
                Ordering::Equal => continue,
                ord => return ord,
            }
        }
        // Same numbers: a stable release is newer than any pre-release;
        // between pre-releases, alpha < beta < rc, then by their own numbers.
        match (&self.pre, &other.pre) {
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
            (Some(a), Some(b)) => a.cmp(b),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s)
    }

    #[test]
    fn orders_common_shapes() {
        assert!(v("v12.0.351") < v("v12.0.376"));
        assert!(v("9.6.37349") < v("9.6.37379"));
        assert!(v("9.6") < v("9.6.1"));
        assert!(v("1.10") > v("1.9"));
        assert!(v("2.0.0-beta1") < v("2.0.0"));
        assert!(v("2.0.0-beta1") < v("2.0.0-beta2"));
        assert!(v("2.0.0-rc1") > v("1.9.9"));
        assert!(v("r1234") < v("r1235"));
    }

    #[test]
    fn numeric_components() {
        assert_eq!(v("v1.2.3").numbers(), &[1, 2, 3]);
        assert_eq!(v("9.6").numbers(), &[9, 6]);
        assert_eq!(v("2.0.0-beta1").numbers(), &[2, 0, 0]);
        // Same numbers, different text: semantically equal.
        assert_eq!(v("v1.2.3"), v("1.2.3"));
        assert_eq!(v("9.6"), v("9.6.0"));
        assert!(!v("1.2.3").is_older_than(&v("v1.2.3")));
    }

    #[test]
    fn marker_letters_in_a_name_are_not_a_pre_release() {
        // "Warcraft" holds "rc", "Preydator" "pre", "DevTool" "dev".
        assert_eq!(v("WarcraftLogs-2.1.0.zip").numbers(), &[2, 1, 0]);
        assert_eq!(v("WarcraftLogs-2.1.0.zip"), v("2.1.0"));
        assert!(v("Preydator-1.2.3") < v("Preydator-1.2.4"));
        assert_eq!(v("DevTool-1.0.5"), v("1.0.5"));
        // A word that merely contains a marker, after the number.
        assert_eq!(v("1.0-srcpack"), v("1.0"));
        // Real markers still count, with or without a separator.
        assert!(v("WarcraftLogs-2.1.0-beta2.zip") < v("WarcraftLogs-2.1.0.zip"));
        assert!(v("1.2rc1") < v("1.2"));
        assert!(v("3.0-pre2") < v("3.0"));
        assert!(v("1.0.0.dev5") < v("1.0.0"));
        assert!(v("12.0.380-beta") < v("12.0.380"));
    }

    #[test]
    fn labels_drop_the_file_name_around_a_version() {
        // CurseForge file titles (issue #14).
        assert_eq!(v("AztarecHelper-2.9.0.zip").label(), "2.9.0");
        assert_eq!(v("WeakAuras-5.12.0-beta2.zip").label(), "5.12.0-beta2");
        assert_eq!(v("Details-Retail-v12.0.376").label(), "v12.0.376");
        // Author-typed versions on WoWInterface / Tukui.
        assert_eq!(v("ElvUI 13.80").label(), "13.80");
        assert_eq!(v("Questie_10.2.0.ZIP").label(), "10.2.0");
        assert_eq!(v("1.4.2.zip").label(), "1.4.2");
        // Details: a dot after the name, a `#` before it (issue #27).
        assert_eq!(
            v("#Details.20261104.15400.172-1-gcec582c").label(),
            "20261104.15400.172-1-gcec582c"
        );
        assert_eq!(v("Details.20261107.15402.172").label(), "20261107.15402.172");
    }

    #[test]
    fn labels_leave_plain_versions_alone() {
        // GitHub tags (prefix already split off), .toc versions.
        for raw in [
            "12.0.391",
            "v12.0.376",
            "9.6.37349",
            "2.0.0-beta1",
            "r1234",
            "11.0.2.1",
        ] {
            assert_eq!(v(raw).label(), raw);
        }
        // A git-describe build: long, but nothing in it is noise.
        assert_eq!(v("v12.0.391-12-gabcdef0").label(), "v12.0.391-12-gabcdef0");
        assert_eq!(v("@project-version@").label(), "@project-version@");
        // No letters before the separator: not a name.
        assert_eq!(v("2026-1.2").label(), "2026-1.2");
        assert_eq!(v("").label(), "");
    }

    #[test]
    fn labels_never_change_what_the_comparison_sees() {
        // The name's digit belongs to the comparison: cutting it would show
        // 5.12.0 for what compares as 2.5.12.0.
        assert_eq!(v("WeakAuras2-5.12.0").label(), "WeakAuras2-5.12.0");
        assert_eq!(v("Bartender4_4.15.3.zip").label(), "Bartender4_4.15.3.zip");
        // A dot inside the version is not a name's end: `v12` → `0.376` would
        // lose the 12, `r5.1.2` the 5.
        assert_eq!(v("v12.0.376").label(), "v12.0.376");
        assert_eq!(v("r5.1.2").label(), "r5.1.2");
        assert_eq!(v("WeakAuras2.5.12.0").label(), "WeakAuras2.5.12.0");
        // Marker letters in the name are not a pre-release: cleaned like any other.
        assert_eq!(v("Preydator-1.2.3.zip").label(), "1.2.3");
        for raw in [
            "AztarecHelper-2.9.0.zip",
            "Details-Retail-v12.0.376",
            "ElvUI 13.80",
            "WarcraftLogs-2.1.0-beta2.zip",
            "#Details.20261104.15400.172-1-gcec582c",
        ] {
            assert_eq!(v(v(raw).label()), v(raw));
        }
    }

    #[test]
    fn labels_survive_odd_text() {
        assert_eq!(v("Ünïcödé-1.2.zip").label(), "1.2");
        assert_eq!(v("–1.2").label(), "–1.2");
        assert_eq!(v(".zip").label(), ".zip");
        assert_eq!(v("a-v").label(), "a-v");
    }

    #[test]
    fn huge_numbers_do_not_panic() {
        let big = "9".repeat(100);
        let _ = v(&big);
        let many = (0..100).map(|i| i.to_string()).collect::<Vec<_>>().join(".");
        assert!(v(&many).numbers().len() <= 8);
    }
}
