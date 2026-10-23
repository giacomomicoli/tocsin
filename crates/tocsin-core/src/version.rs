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
            .filter_map(|(m, rank)| lower.find(m).map(|at| (at, *rank, m.len())))
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
    fn huge_numbers_do_not_panic() {
        let big = "9".repeat(100);
        let _ = v(&big);
        let many = (0..100).map(|i| i.to_string()).collect::<Vec<_>>().join(".");
        assert!(v(&many).numbers().len() <= 8);
    }
}
