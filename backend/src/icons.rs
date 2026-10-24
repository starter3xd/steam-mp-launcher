//! Icons stored in Windows PE resources (`RT_GROUP_ICON` / `RT_ICON`).
//!
//! The UI only needs a file it can hand to `QPixmap`, so the embedded icon is
//! written once to `$XDG_CACHE_HOME/steam-mp-launcher/icons/<key>.ico` (or
//! `.png` for Vista-style PNG entries) and reused while the .exe is unchanged.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::paths::{atomic_write_bytes, home};

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".cache"));
    base.join("steam-mp-launcher/icons")
}

/// Cache path for the icon embedded in `exe`, or an empty string.
pub fn icon_for(exe: &str) -> String {
    extract_to_cache(Path::new(exe))
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn extract_to_cache(exe: &Path) -> Option<PathBuf> {
    let meta = fs::metadata(exe).ok()?;
    if !meta.is_file() {
        return None;
    }
    let stamp = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|delta| delta.as_secs())
        .unwrap_or(0);
    let key = fnv1a(format!("{}|{}|{stamp}", exe.display(), meta.len()).as_bytes());
    let dir = cache_dir();
    for ext in ["ico", "png"] {
        let candidate = dir.join(format!("{key:016x}.{ext}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let data = fs::read(exe).ok()?;
    let (bytes, ext) = extract(&data)?;
    let target = dir.join(format!("{key:016x}.{ext}"));
    atomic_write_bytes(&target, &bytes).ok()?;
    Some(target)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    let slice = data.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    let slice = data.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_size: u32,
    raw_pointer: u32,
}

struct Pe<'a> {
    data: &'a [u8],
    sections: Vec<Section>,
    resource_rva: u32,
}

impl<'a> Pe<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        if data.get(0..2)? != b"MZ" {
            return None;
        }
        let pe = u32_at(data, 0x3c)? as usize;
        if data.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let count = u16_at(data, pe + 6)? as usize;
        let optional_size = u16_at(data, pe + 20)? as usize;
        let optional = pe + 24;
        let directories = match u16_at(data, optional)? {
            0x10b => optional + 96,
            0x20b => optional + 112,
            _ => return None,
        };
        let resource_rva = u32_at(data, directories + 2 * 8)?;
        let resource_size = u32_at(data, directories + 2 * 8 + 4)?;
        if resource_rva == 0 || resource_size == 0 {
            return None;
        }

        let table = optional + optional_size;
        let mut sections = Vec::with_capacity(count);
        for index in 0..count {
            let at = table + index * 40;
            sections.push(Section {
                virtual_size: u32_at(data, at + 8)?,
                virtual_address: u32_at(data, at + 12)?,
                raw_size: u32_at(data, at + 16)?,
                raw_pointer: u32_at(data, at + 20)?,
            });
        }
        Some(Self {
            data,
            sections,
            resource_rva,
        })
    }

    fn offset(&self, rva: u32) -> Option<usize> {
        for section in &self.sections {
            let span = section.virtual_size.max(section.raw_size);
            if rva >= section.virtual_address && rva < section.virtual_address.checked_add(span)? {
                let delta = rva - section.virtual_address;
                if delta >= section.raw_size {
                    return None;
                }
                return Some((section.raw_pointer + delta) as usize);
            }
        }
        None
    }

    fn resource_base(&self) -> Option<usize> {
        self.offset(self.resource_rva)
    }

    /// Directory entries as `(id, relative offset, is directory)`.
    fn directory(&self, base: usize, relative: usize) -> Option<Vec<(u32, u32, bool)>> {
        let named = u16_at(self.data, base + relative + 12)? as usize;
        let ids = u16_at(self.data, base + relative + 14)? as usize;
        let mut entries = Vec::with_capacity(named + ids);
        for index in 0..named + ids {
            let at = base + relative + 16 + index * 8;
            let name = u32_at(self.data, at)?;
            let offset = u32_at(self.data, at + 4)?;
            entries.push((name, offset & 0x7fff_ffff, offset & 0x8000_0000 != 0));
        }
        Some(entries)
    }

    /// First leaf below `directory` as `(rva, size)`.
    fn leaf(&self, base: usize, directory: u32) -> Option<(u32, u32)> {
        let mut cursor = directory;
        for _ in 0..8 {
            let entries = self.directory(base, cursor as usize)?;
            let (_, offset, is_dir) = *entries.first()?;
            if is_dir {
                cursor = offset;
                continue;
            }
            let at = base + offset as usize;
            let rva = u32_at(self.data, at)?;
            let size = u32_at(self.data, at + 4)?;
            if size == 0 || size as usize > self.data.len() {
                return None;
            }
            return Some((rva, size));
        }
        None
    }

    fn bytes(&self, rva: u32, size: u32) -> Option<&'a [u8]> {
        let offset = self.offset(rva)?;
        self.data.get(offset..offset + size as usize)
    }
}

/// One `GRPICONDIRENTRY`: the small header describing a single image.
struct GroupEntry {
    width: u16,
    height: u16,
    colors: u8,
    planes: u16,
    bits: u16,
    id: u16,
}

impl GroupEntry {
    fn pixels(&self) -> u32 {
        let width = if self.width == 0 {
            256
        } else {
            u32::from(self.width)
        };
        let height = if self.height == 0 {
            256
        } else {
            u32::from(self.height)
        };
        width * height * u32::from(self.bits.max(1))
    }
}

fn group_entries(data: &[u8]) -> Vec<GroupEntry> {
    let count = u16_at(data, 4).unwrap_or(0);
    let mut entries = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        // GRPICONDIRENTRY: width, height, colors, reserved (u8 each), then
        // planes, bits, byte size and the RT_ICON id.
        let at = 6 + index * 14;
        let (Some(width), Some(height), Some(colors)) = (
            data.get(at).copied(),
            data.get(at + 1).copied(),
            data.get(at + 2).copied(),
        ) else {
            break;
        };
        let (Some(planes), Some(bits), Some(id)) = (
            u16_at(data, at + 4),
            u16_at(data, at + 6),
            u16_at(data, at + 12),
        ) else {
            break;
        };
        entries.push(GroupEntry {
            width: u16::from(width),
            height: u16::from(height),
            colors,
            planes,
            bits,
            id,
        });
    }
    entries
}

/// Extract the icon of a PE file as `(file bytes, "ico" | "png")`.
fn extract(data: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    let pe = Pe::parse(data)?;
    let base = pe.resource_base()?;
    let types = pe.directory(base, 0)?;

    let group_dir = types
        .iter()
        .find(|(name, _, is_dir)| *is_dir && *name == RT_GROUP_ICON)?
        .1;
    let icon_dir = types
        .iter()
        .find(|(name, _, is_dir)| *is_dir && *name == RT_ICON)?
        .1;

    // The group with the lowest id is the application icon (Explorer's choice).
    let groups = pe.directory(base, group_dir as usize)?;
    let (_, first_group, _) = *groups
        .iter()
        .filter(|(_, _, is_dir)| *is_dir)
        .min_by_key(|(name, _, _)| *name)?;
    let (group_rva, group_size) = pe.leaf(base, first_group)?;
    let group = pe.bytes(group_rva, group_size)?;
    let entries = group_entries(group);
    if entries.is_empty() {
        return None;
    }

    let icons = pe.directory(base, icon_dir as usize)?;
    let mut images = Vec::new();
    for entry in &entries {
        let Some((_, offset, is_dir)) = icons
            .iter()
            .find(|(name, _, _)| *name == u32::from(entry.id))
        else {
            continue;
        };
        if !*is_dir {
            continue;
        }
        let Some((rva, size)) = pe.leaf(base, *offset) else {
            continue;
        };
        let Some(bytes) = pe.bytes(rva, size) else {
            continue;
        };
        images.push((entry, bytes));
    }
    let (_, best_bytes) = images.iter().max_by_key(|(entry, _)| entry.pixels())?;
    if best_bytes.starts_with(PNG_SIGNATURE) {
        return Some((best_bytes.to_vec(), "png"));
    }

    // A single ICO holding every size: QPixmap picks the best one.
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (entry, bytes) in &images {
        out.push(if entry.width == 256 {
            0
        } else {
            entry.width as u8
        });
        out.push(if entry.height == 256 {
            0
        } else {
            entry.height as u8
        });
        out.push(entry.colors);
        out.push(0);
        out.extend_from_slice(&entry.planes.to_le_bytes());
        out.extend_from_slice(&entry.bits.to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += bytes.len() as u32;
    }
    for (_, bytes) in &images {
        out.extend_from_slice(bytes);
    }
    Some((out, "ico"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put16(data: &mut [u8], at: usize, value: u16) {
        data[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put32(data: &mut [u8], at: usize, value: u32) {
        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// A 16x16/32bpp DIB the way Windows stores it inside `RT_ICON`.
    fn dib() -> Vec<u8> {
        let mut out = vec![0u8; 40 + 16 * 16 * 4 + 16 * 32 / 8];
        put32(&mut out, 0, 40);
        put32(&mut out, 4, 16);
        put32(&mut out, 8, 32);
        put16(&mut out, 12, 1);
        put16(&mut out, 14, 32);
        for pixel in out[40..40 + 16 * 16 * 4].chunks_mut(4) {
            pixel.copy_from_slice(&[0x20, 0xa0, 0x40, 0xff]);
        }
        out
    }

    /// Minimal PE32 with one .rsrc section holding one 16x16 icon.
    fn pe_with_icon(icon: &[u8]) -> Vec<u8> {
        const RAW: usize = 0x200;
        const RSRC: usize = 0x1000;
        let group_offset = 0xc0usize;
        let icon_offset = 0x200usize;
        let mut rsrc = vec![0u8; icon_offset + icon.len()];

        // Root: RT_ICON -> id 1 -> lang 1033, RT_GROUP_ICON -> id 1 -> lang.
        put16(&mut rsrc, 12, 0);
        put16(&mut rsrc, 14, 2);
        put32(&mut rsrc, 16, RT_ICON);
        put32(&mut rsrc, 20, 0x8000_0000 | 0x20);
        put32(&mut rsrc, 24, RT_GROUP_ICON);
        put32(&mut rsrc, 28, 0x8000_0000 | 0x40);

        put16(&mut rsrc, 0x20 + 14, 1);
        put32(&mut rsrc, 0x20 + 16, 1);
        put32(&mut rsrc, 0x20 + 20, 0x8000_0000 | 0x60);
        put16(&mut rsrc, 0x40 + 14, 1);
        put32(&mut rsrc, 0x40 + 16, 1);
        put32(&mut rsrc, 0x40 + 20, 0x8000_0000 | 0x80);

        put16(&mut rsrc, 0x60 + 14, 1);
        put32(&mut rsrc, 0x60 + 16, 1033);
        put32(&mut rsrc, 0x60 + 20, 0xb0);
        put16(&mut rsrc, 0x80 + 14, 1);
        put32(&mut rsrc, 0x80 + 16, 1033);
        put32(&mut rsrc, 0x80 + 20, 0xa0);

        put32(&mut rsrc, 0xa0, (RSRC + group_offset) as u32);
        put32(&mut rsrc, 0xa4, 20);
        put32(&mut rsrc, 0xb0, (RSRC + icon_offset) as u32);
        put32(&mut rsrc, 0xb4, icon.len() as u32);

        put16(&mut rsrc, group_offset + 2, 1);
        put16(&mut rsrc, group_offset + 4, 1);
        rsrc[group_offset + 6] = 16;
        rsrc[group_offset + 7] = 16;
        put16(&mut rsrc, group_offset + 10, 1);
        put16(&mut rsrc, group_offset + 12, 32);
        put32(&mut rsrc, group_offset + 14, icon.len() as u32);
        put16(&mut rsrc, group_offset + 18, 1);
        rsrc[icon_offset..].copy_from_slice(icon);

        let mut file = vec![0u8; RAW + rsrc.len()];
        file[0..2].copy_from_slice(b"MZ");
        put32(&mut file, 0x3c, 0x40);
        file[0x40..0x44].copy_from_slice(b"PE\0\0");
        put16(&mut file, 0x44, 0x14c);
        put16(&mut file, 0x46, 1);
        put16(&mut file, 0x54, 0xe0);
        let optional = 0x58;
        put16(&mut file, optional, 0x10b);
        put32(&mut file, optional + 8, RSRC as u32);
        put32(&mut file, optional + 96 + 2 * 8, RSRC as u32);
        put32(&mut file, optional + 96 + 2 * 8 + 4, rsrc.len() as u32);
        let section = optional + 0xe0;
        file[section..section + 5].copy_from_slice(b".rsrc");
        put32(&mut file, section + 8, rsrc.len() as u32);
        put32(&mut file, section + 12, RSRC as u32);
        put32(&mut file, section + 16, rsrc.len() as u32);
        put32(&mut file, section + 20, RAW as u32);
        file[RAW..].copy_from_slice(&rsrc);
        file
    }

    #[test]
    fn ignores_files_that_are_not_pe() {
        assert!(extract(b"not an executable").is_none());
        assert!(extract(&[]).is_none());
        let mut dos = vec![0u8; 128];
        dos[0..2].copy_from_slice(b"MZ");
        assert!(extract(&dos).is_none());
    }

    #[test]
    fn builds_an_ico_from_the_resource_section() {
        let icon = dib();
        let (bytes, ext) = extract(&pe_with_icon(&icon)).expect("icon");
        assert_eq!(ext, "ico");
        assert_eq!(&bytes[0..4], &[0, 0, 1, 0]);
        assert_eq!(u16_at(&bytes, 4), Some(1));
        assert_eq!(u32_at(&bytes, 14), Some(icon.len() as u32));
        assert_eq!(&bytes[22..22 + icon.len()], icon.as_slice());
    }

    #[test]
    fn leaves_png_entries_alone() {
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend_from_slice(&[1, 2, 3, 4]);
        let (bytes, ext) = extract(&pe_with_icon(&png)).expect("icon");
        assert_eq!(ext, "png");
        assert_eq!(bytes, png);
    }
}
