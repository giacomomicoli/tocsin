//! Free space before a large write, and "the disk is full" told apart from
//! other I/O errors.
//!
//! A voice-over pack is a 450 MB zip that unpacks to as much again: on a
//! nearly full disk the download, or the unpack after it, would stop half
//! way with an OS error the user cannot read. The space is checked first,
//! on the volume the bytes go to; a full disk met anyway (another program
//! filled it meanwhile) still gets a plain message.

use std::path::Path;

/// Left free on top of what a write needs: the game, the OS and the other
/// programs on that disk need room too. A small write keeps as much again
/// as it needs instead, so a 1 MB addon still installs on a nearly full disk
/// as it always did.
pub const HEADROOM: u64 = 64 * 1024 * 1024;

/// Not enough free space for a write.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not enough free disk space: {} needed, {} free", human_size(*.needed), human_size(*.free))]
pub struct NoSpace {
    /// Bytes the write needs.
    pub needed: u64,
    /// Bytes free on that volume.
    pub free: u64,
}

/// Whether `needed` bytes, and their headroom (see [`HEADROOM`]), fit on the
/// volume holding `dir`. When the free space cannot be read, the write goes
/// ahead: a full disk is still reported when it happens (see [`is_full`]).
/// Blocking: call it off the UI thread.
pub fn ensure_space(dir: &Path, needed: u64) -> Result<(), NoSpace> {
    match fs4::available_space(dir) {
        Ok(free) => check(free, needed),
        Err(e) => {
            tracing::debug!("free space of {} unknown: {e}", dir.display());
            Ok(())
        }
    }
}

/// The decision of [`ensure_space`], given the free bytes.
pub fn check(free: u64, needed: u64) -> Result<(), NoSpace> {
    if needed.saturating_add(needed.min(HEADROOM)) > free {
        Err(NoSpace { needed, free })
    } else {
        Ok(())
    }
}

/// Whether `e` says the disk (or the user's quota on it) is full. Matched on
/// the OS codes, as `ErrorKind::StorageFull` is newer than the crate's
/// minimum Rust.
pub fn is_full(e: &std::io::Error) -> bool {
    // Windows: ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL. Unix: ENOSPC, and
    // EDQUOT (122 on Linux, 69 on macOS).
    #[cfg(windows)]
    const FULL: &[i32] = &[39, 112];
    #[cfg(target_os = "linux")]
    const FULL: &[i32] = &[28, 122];
    #[cfg(target_os = "macos")]
    const FULL: &[i32] = &[28, 69];
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    const FULL: &[i32] = &[28];
    e.raw_os_error().is_some_and(|code| FULL.contains(&code))
}

/// A size as people read it: `820 KB`, `453 MB`, `1.2 GB` (1024-based, the
/// way Windows and CurseForge's pages show sizes).
pub fn human_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if bytes >= GIB {
        format!("{:.1} GB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{} MB", (bytes + MIB / 2) / MIB)
    } else {
        format!("{} KB", bytes.div_ceil(KIB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_like_the_sites_show_them() {
        // Spoken Audio: Readables 2.3.1, as CurseForge lists it.
        assert_eq!(human_size(475_482_457), "453 MB");
        assert_eq!(human_size(256 * 1024 * 1024), "256 MB");
        assert_eq!(human_size(1_288_490_189), "1.2 GB");
        assert_eq!(human_size(829_940), "811 KB");
        assert_eq!(human_size(1), "1 KB");
        assert_eq!(human_size(0), "0 KB");
    }

    #[test]
    fn space_keeps_headroom() {
        let needed = 475_482_457;
        assert!(check(needed + HEADROOM, needed).is_ok());
        let err = check(needed + HEADROOM - 1, needed).unwrap_err();
        assert_eq!(err.needed, needed);
        assert_eq!(
            err.to_string(),
            format!(
                "not enough free disk space: 453 MB needed, {} free",
                human_size(needed + HEADROOM - 1)
            )
        );
        // A 1 MB addon keeps 1 MB spare, not 64.
        assert!(check(2 * 1024 * 1024, 1024 * 1024).is_ok());
        assert!(check(2 * 1024 * 1024 - 1, 1024 * 1024).is_err());
        // A huge `needed` (a lying zip directory) never wraps around.
        assert!(check(u64::MAX - 1, u64::MAX).is_err());
    }

    #[test]
    fn full_disk_is_told_from_other_errors() {
        #[cfg(windows)]
        assert!(is_full(&std::io::Error::from_raw_os_error(112)));
        #[cfg(unix)]
        assert!(is_full(&std::io::Error::from_raw_os_error(28)));
        assert!(!is_full(&std::io::Error::from_raw_os_error(5)));
        assert!(!is_full(&std::io::Error::other("no code")));
    }
}
