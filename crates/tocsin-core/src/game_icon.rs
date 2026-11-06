//! The game's own icon, for the installation in the sidebar, read from the
//! client executable in the folder the user added.
//!
//! Every client ships its own icon (Retail's "W", WoW Forever's "WF"), so
//! it tells installations apart at a glance. Tocsin bundles none of them:
//! the picture is read at runtime from the user's own install, the same
//! `Wow.exe` (or `WowClassic.exe`, …) that [`crate::detect`] already uses
//! as the mark of a game folder.
//!
//! The executable is an 80 MB file and untrusted input, so it is never read
//! whole: only its PE headers and resource directory, each piece with its
//! own cap, then the largest picture of its icon. The decoding is
//! [`icons`]'s, under the limits of a downloaded logo: a 256px picture is a
//! PNG ([`icons::decode`]; WoW Forever has one), a smaller one a 32-bit
//! bitmap ([`icons::decode_icon_bitmap`]; Retail goes up to 48px). Nothing
//! is cached: the executable changes with every patch, and reading it takes
//! a few milliseconds. When no picture decodes, the sidebar keeps the
//! monogram.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::icons::{self, Icon};

/// Largest single piece read from the executable but the picture itself: a
/// header, the section table, a resource directory, an icon group.
const MAX_PIECE: usize = 64 * 1024;

/// Most sections a PE file may declare (the format's own limit).
const MAX_SECTIONS: usize = 96;

/// Most entries read from one resource directory. Wow.exe's root has 6.
const MAX_ENTRIES: usize = 512;

/// Most pictures of an icon group tried before giving up.
const MAX_TRIES: usize = 4;

/// Resource types, from `winuser.h`.
const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;

/// Set on a directory entry that points to another directory.
const SUBDIR: u32 = 0x8000_0000;

/// The icon of the client owning `addons_dir` (`…/<flavor>/Interface/AddOns`),
/// if its executable is there and carries one. Blocking: call it off the UI
/// thread.
pub fn read(addons_dir: &Path) -> Option<Icon> {
    let exe = find_exe(addons_dir.parent()?.parent()?)?;
    let file = open_regular(&exe)?;
    let len = file.metadata().ok()?.len();
    Pe::open(file, len)?.icon()
}

/// The first game executable of a flavor folder, as a plain file (a link is
/// not followed).
fn find_exe(flavor_dir: &Path) -> Option<PathBuf> {
    crate::detect::GAME_EXES
        .iter()
        .map(|name| flavor_dir.join(name))
        .find(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file()))
}

/// Open `path` if it is a regular file, never through a link; the same
/// check as [`crate::discover`]'s capped reads, which this file is too large
/// for.
fn open_regular(path: &Path) -> Option<File> {
    let checked = std::fs::symlink_metadata(path).ok()?;
    if !checked.file_type().is_file() {
        return None;
    }
    let file = File::open(path).ok()?;
    let opened = file.metadata().ok()?;
    (opened.is_file() && crate::discover::same_file(&checked, &opened)).then_some(file)
}

/// Where a section's bytes are, in memory (RVA) and in the file.
#[derive(Debug, Clone, Copy)]
struct Section {
    rva: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
}

/// A PE file, opened far enough to find its resources.
struct Pe<R> {
    file: R,
    len: u64,
    sections: Vec<Section>,
    /// RVA of the resource directory: entries point relative to it.
    resources: u32,
}

impl<R: Read + Seek> Pe<R> {
    /// Read the headers of the PE file `file` of `len` bytes.
    fn open(file: R, len: u64) -> Option<Self> {
        let mut pe = Self {
            file,
            len,
            sections: Vec::new(),
            resources: 0,
        };
        let dos = pe.read_at(0, 64)?;
        if !dos.starts_with(b"MZ") {
            return None;
        }
        let nt = u64::from(u32_at(&dos, 0x3C)?);
        let head = pe.read_at(nt, 24)?;
        if !head.starts_with(b"PE\0\0") {
            return None;
        }
        let sections = usize::from(u16_at(&head, 6)?);
        let optional_len = usize::from(u16_at(&head, 20)?);
        let optional = pe.read_at(nt + 24, optional_len)?;
        // The data directories sit after the PE32 or PE32+ fields.
        let (count_at, dirs_at) = match u16_at(&optional, 0)? {
            0x10B => (92, 96),
            0x20B => (108, 112),
            _ => return None,
        };
        if u32_at(&optional, count_at)? < 3 {
            return None;
        }
        // Data directory 2 is the resource table.
        pe.resources = u32_at(&optional, dirs_at + 2 * 8)?;
        if pe.resources == 0 || sections > MAX_SECTIONS {
            return None;
        }
        let table = pe.read_at(nt + 24 + optional_len as u64, sections * 40)?;
        pe.sections = table
            .chunks_exact(40)
            .filter_map(|s| {
                Some(Section {
                    virtual_size: u32_at(s, 8)?,
                    rva: u32_at(s, 12)?,
                    raw_size: u32_at(s, 16)?,
                    raw_offset: u32_at(s, 20)?,
                })
            })
            .collect();
        Some(pe)
    }

    /// The application icon: the first icon group, as Explorer shows it.
    fn icon(&mut self) -> Option<Icon> {
        let group = self.resource(RT_GROUP_ICON, None, MAX_PIECE)?;
        for id in pictures(&group).into_iter().take(MAX_TRIES) {
            let Some(bytes) = self.resource(RT_ICON, Some(id), icons::MAX_ICON_BYTES) else {
                continue;
            };
            // The 256px picture is a PNG, the others are bitmaps.
            let icon = icons::decode(&bytes).or_else(|_| icons::decode_icon_bitmap(&bytes));
            if let Ok(icon) = icon {
                return Some(icon);
            }
        }
        None
    }

    /// The data of resource `kind` named `name` (the first one if `None`),
    /// in its first language, if it is at most `cap` bytes. The tree is
    /// always three levels deep (type, name, language), so it is walked
    /// exactly that far and no loop in it can be followed.
    fn resource(&mut self, kind: u32, name: Option<u32>, cap: usize) -> Option<Vec<u8>> {
        let names = self.entry(0, Some(kind))?.checked_sub(SUBDIR)?;
        let languages = self.entry(names, name)?.checked_sub(SUBDIR)?;
        let leaf = self.entry(languages, None)?;
        if leaf & SUBDIR != 0 {
            return None;
        }
        let data = self.read_rva(self.resources.checked_add(leaf)?, 16)?;
        let (rva, size) = (u32_at(&data, 0)?, usize::try_from(u32_at(&data, 4)?).ok()?);
        if size > cap {
            return None;
        }
        self.read_rva(rva, size)
    }

    /// The target of the entry with id `id` (the first entry if `None`) in
    /// the resource directory at `dir`, relative to the resource table.
    fn entry(&mut self, dir: u32, id: Option<u32>) -> Option<u32> {
        let at = self.resources.checked_add(dir)?;
        let head = self.read_rva(at, 16)?;
        let count = usize::from(u16_at(&head, 12)?) + usize::from(u16_at(&head, 14)?);
        let entries = self.read_rva(at.checked_add(16)?, count.min(MAX_ENTRIES) * 8)?;
        entries
            .chunks_exact(8)
            // Named entries have the high bit set, so they never equal an id.
            .find(|e| id.map_or(true, |id| u32_at(e, 0) == Some(id)))
            .and_then(|e| u32_at(e, 4))
    }

    /// `len` bytes at `rva`, which must lie within one section's file data.
    fn read_rva(&mut self, rva: u32, len: usize) -> Option<Vec<u8>> {
        let end = u64::from(rva) + len as u64;
        let s = self.sections.iter().find(|s| {
            let size = u64::from(s.virtual_size.max(s.raw_size));
            s.rva <= rva && end <= u64::from(s.rva) + size
        })?;
        let inside = u64::from(rva - s.rva);
        if inside + len as u64 > u64::from(s.raw_size) {
            return None;
        }
        self.read_at(u64::from(s.raw_offset) + inside, len)
    }

    /// `len` bytes at `offset`, within the file and the caps.
    fn read_at(&mut self, offset: u64, len: usize) -> Option<Vec<u8>> {
        if len > MAX_PIECE.max(icons::MAX_ICON_BYTES) || offset.checked_add(len as u64)? > self.len {
            return None;
        }
        self.file.seek(SeekFrom::Start(offset)).ok()?;
        let mut bytes = vec![0; len];
        self.file.read_exact(&mut bytes).ok()?;
        Some(bytes)
    }
}

/// The `RT_ICON` ids of an icon group (`GRPICONDIR`), largest picture
/// first: the 256px one (stored as width 0) is the one kept as PNG.
fn pictures(group: &[u8]) -> Vec<u32> {
    if u16_at(group, 0) != Some(0) || u16_at(group, 2) != Some(1) {
        return Vec::new();
    }
    let count = usize::from(u16_at(group, 4).unwrap_or(0));
    let mut found: Vec<(u32, u16, u32)> = group
        .get(6..)
        .unwrap_or_default()
        .chunks_exact(14)
        .take(count)
        .filter_map(|e| {
            let side = match e[0] {
                0 => 256,
                w => u32::from(w),
            };
            Some((side, u16_at(e, 6)?, u32::from(u16_at(e, 12)?)))
        })
        .collect();
    found.sort_by_key(|&(side, bits, _)| std::cmp::Reverse((side, bits)));
    found.into_iter().map(|(_, _, id)| id).collect()
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A PNG of `side` pixels, all `rgba`.
    fn png(side: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(side, side, image::Rgba(rgba))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    /// A 32-bit icon bitmap of `side` pixels, all `bgra`, mask after.
    fn bitmap(side: u32, bgra: [u8; 4]) -> Vec<u8> {
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&side.to_le_bytes());
        dib[8..12].copy_from_slice(&(side * 2).to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        for _ in 0..side * side {
            dib.extend_from_slice(&bgra);
        }
        dib.extend(vec![0u8; (side.div_ceil(32) * 4 * side) as usize]);
        dib
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    /// Blue, written BGRA as a bitmap stores it.
    const BLUE_BGRA: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    /// A PE32+ file shaped like Wow.exe's resources: one `.rsrc` section
    /// holding an icon group of a 32px bitmap (id 1) and a 256px picture
    /// (id 2), the group listed after other resource types.
    fn exe(large: &[u8], small: &[u8]) -> Vec<u8> {
        const RSRC_RVA: u32 = 0x1000;
        const RSRC_RAW: u32 = 0x400;
        // Resource section, offsets relative to its start.
        let mut r = vec![0u8; 0x180];
        let dir = |r: &mut Vec<u8>, at: usize, entries: &[(u32, u32)]| {
            r[at + 14..at + 16].copy_from_slice(&(entries.len() as u16).to_le_bytes());
            for (i, (id, to)) in entries.iter().enumerate() {
                let e = at + 16 + i * 8;
                r[e..e + 4].copy_from_slice(&id.to_le_bytes());
                r[e + 4..e + 8].copy_from_slice(&to.to_le_bytes());
            }
        };
        let leaf = |r: &mut Vec<u8>, at: usize, rva: usize, size: usize| {
            r[at..at + 4].copy_from_slice(&(RSRC_RVA + rva as u32).to_le_bytes());
            r[at + 4..at + 8].copy_from_slice(&(size as u32).to_le_bytes());
        };
        // Root: RT_VERSION (pointing anywhere), RT_ICON, RT_GROUP_ICON.
        let root = [
            (16, SUBDIR | 0x30),
            (RT_ICON, SUBDIR | 0x30),
            (RT_GROUP_ICON, SUBDIR | 0x50),
        ];
        dir(&mut r, 0x00, &root);
        // RT_ICON names 1 and 2, one language each.
        dir(&mut r, 0x30, &[(1, SUBDIR | 0x68), (2, SUBDIR | 0x80)]);
        dir(&mut r, 0x68, &[(1033, 0x100)]);
        dir(&mut r, 0x80, &[(1033, 0x110)]);
        // RT_GROUP_ICON: one group, one language.
        dir(&mut r, 0x50, &[(128, SUBDIR | 0x98)]);
        dir(&mut r, 0x98, &[(1033, 0x120)]);
        // The group: 32px 32-bit id 1, 256px 32-bit id 2.
        let mut group = vec![0, 0, 1, 0, 2, 0];
        for (w, id) in [(32u8, 1u16), (0, 2)] {
            group.extend([w, w, 0, 0, 1, 0, 32, 0]);
            group.extend(0u32.to_le_bytes());
            group.extend(id.to_le_bytes());
        }
        let group_at = 0x140;
        r[group_at..group_at + group.len()].copy_from_slice(&group);
        leaf(&mut r, 0x120, group_at, group.len());
        for (entry, picture) in [(0x100, small), (0x110, large)] {
            let at = r.len();
            leaf(&mut r, entry, at, picture.len());
            r.extend_from_slice(picture);
        }

        // Headers: DOS stub pointing at 0x80, PE32+ optional header with
        // 16 data directories, one section.
        let mut f = vec![0u8; RSRC_RAW as usize];
        f[0..2].copy_from_slice(b"MZ");
        f[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        f[0x80..0x84].copy_from_slice(b"PE\0\0");
        f[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        f[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
        let opt = 0x98;
        f[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
        f[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        f[opt + 128..opt + 132].copy_from_slice(&RSRC_RVA.to_le_bytes());
        f[opt + 132..opt + 136].copy_from_slice(&(r.len() as u32).to_le_bytes());
        let sec = opt + 240;
        f[sec..sec + 5].copy_from_slice(b".rsrc");
        f[sec + 8..sec + 12].copy_from_slice(&(r.len() as u32).to_le_bytes());
        f[sec + 12..sec + 16].copy_from_slice(&RSRC_RVA.to_le_bytes());
        f[sec + 16..sec + 20].copy_from_slice(&(r.len() as u32).to_le_bytes());
        f[sec + 20..sec + 24].copy_from_slice(&RSRC_RAW.to_le_bytes());
        f.extend(r);
        f
    }

    fn icon_of(bytes: Vec<u8>) -> Option<Icon> {
        let len = bytes.len() as u64;
        Pe::open(Cursor::new(bytes), len)?.icon()
    }

    /// The colour of the icon's first pixel.
    fn colour(icon: &Icon) -> [u8; 4] {
        icon.rgba[..4].try_into().unwrap()
    }

    #[test]
    fn the_largest_picture_wins() {
        // WoW Forever: a 256px PNG beside the bitmaps.
        let icon = icon_of(exe(&png(256, RED), &bitmap(32, BLUE_BGRA))).unwrap();
        assert_eq!(icon.side, icons::SIDE);
        assert_eq!(icon.rgba.len(), (icons::SIDE * icons::SIDE * 4) as usize);
        assert_eq!(colour(&icon), RED);
    }

    #[test]
    fn a_bitmap_does_when_nothing_larger_decodes() {
        // Retail: bitmaps only (here a 256px slot that holds garbage).
        let icon = icon_of(exe(b"garbage", &bitmap(32, BLUE_BGRA))).unwrap();
        assert_eq!(colour(&icon), BLUE);
        // 24-bit bitmaps are not decoded: no icon, the monogram stays.
        let mut old = bitmap(32, BLUE_BGRA);
        old[14] = 24;
        assert!(icon_of(exe(b"garbage", &old)).is_none());
    }

    #[test]
    fn broken_files_give_nothing() {
        let good = exe(&png(256, RED), b"");
        assert!(icon_of(good.clone()).is_some());
        assert!(icon_of(Vec::new()).is_none());
        assert!(icon_of(b"MZ".to_vec()).is_none());
        // Cut inside the picture.
        assert!(icon_of(good[..good.len() - 10].to_vec()).is_none());
        // Every byte of the headers and directories flipped in turn: never a
        // panic, whatever it then reads.
        for at in 0..0x580 {
            let mut bad = good.clone();
            bad[at] ^= 0xFF;
            let _ = icon_of(bad);
        }
    }

    #[test]
    fn groups_list_the_largest_picture_first() {
        let mut group = vec![0, 0, 1, 0, 3, 0];
        for (w, bits, id) in [(16u8, 32u16, 5u16), (0, 32, 7), (48, 8, 6)] {
            group.extend([w, w, 0, 0, 1, 0]);
            group.extend(bits.to_le_bytes());
            group.extend(0u32.to_le_bytes());
            group.extend(id.to_le_bytes());
        }
        assert_eq!(pictures(&group), [7, 6, 5]);
        // Claims more entries than it holds.
        group[4] = 200;
        assert_eq!(pictures(&group), [7, 6, 5]);
        // Not an icon group (type 2 is a cursor).
        assert!(pictures(b"\0\0\x02\0\x01\0").is_empty());
    }

    #[test]
    fn reads_the_executable_beside_addons() {
        let tmp = tempfile::tempdir().unwrap();
        let flavor = tmp.path().join("_retail_");
        let addons = flavor.join("Interface").join("AddOns");
        std::fs::create_dir_all(&addons).unwrap();
        assert!(read(&addons).is_none());
        std::fs::write(flavor.join("Wow.exe"), exe(&png(256, RED), b"")).unwrap();
        assert_eq!(read(&addons).map(|i| colour(&i)), Some(RED));
    }

    #[test]
    fn a_linked_executable_is_not_read() {
        let tmp = tempfile::tempdir().unwrap();
        let flavor = tmp.path().join("_retail_");
        let addons = flavor.join("Interface").join("AddOns");
        std::fs::create_dir_all(&addons).unwrap();
        let elsewhere = tmp.path().join("elsewhere.exe");
        std::fs::write(&elsewhere, exe(&png(256, RED), b"")).unwrap();
        if crate::discover::try_symlink_file(&elsewhere, &flavor.join("Wow.exe")) {
            assert!(read(&addons).is_none());
        } else {
            eprintln!("cannot create a file link here; skipped");
        }
    }
}
