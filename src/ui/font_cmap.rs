//! In-memory cmap filtering, not outline subsetting. The returned font retains its
//! face index, glyph IDs, outlines, metrics, and shaping tables. Only the selected
//! face's cmap directory entry changes; a replacement cmap is appended to the
//! owned buffer. No installed font is ever written.
//!
//! `Chinese` deliberately includes the compatible Japanese/Korean ranges and
//! CJK/fullwidth punctuation. `English` means everything outside those ranges,
//! not just ASCII. Variation-sequence cmap format 14 records are not retained
//! when rebuilding: egui/ab_glyph resolves base characters, not shaped IVS text.

use ab_glyph::{Font as _, FontRef};

#[derive(Debug, Clone, Copy)]
pub enum Script {
    English,
    Chinese,
}

/// Script partition used by both font choices, including supplementary Han.
/// Generic punctuation, combining marks, variation selectors, and ordinary emoji
/// stay on the non-CJK side; CJK-specific enclosed ideographs stay on the CJK side.
pub fn is_chinese(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11ff | // Hangul Jamo
        0x2e80..=0x33ff | // radicals, ideographic punctuation, kana, bopomofo, compatibility
        0x3400..=0x4dbf | 0x4e00..=0x9fff |
        0xa960..=0xa97f | 0xac00..=0xd7ff |
        0xf900..=0xfaff | 0xfe10..=0xfe1f | 0xfe30..=0xfe4f |
        0xff00..=0xffef | // fullwidth / halfwidth forms
        0x16fe0..=0x16fff | // ideographic symbols and punctuation
        0x1aff0..=0x1afff | 0x1b000..=0x1b16f | // supplementary kana
        0x1f200..=0x1f2ff | // enclosed ideographic supplement
        0x20000..=0x2ffff | 0x30000..=0x3ffff // Han extensions and compatibility supplement
    )
}

const MAX_FONT_BYTES: usize = 64 * 1024 * 1024;
const UNICODE_LIMIT: usize = 0x110000;
const MAX_TABLES: usize = 4096;
const MAX_CMAP_RECORDS: usize = 64;
// Windows fonts commonly repeat the same coverage in Unicode and Windows records.
const MAX_ENUMERATED_CODEPOINTS: usize = UNICODE_LIMIT * 8;
const CHECKSUM_MAGIC: u32 = 0xb1b0_afba;

/// Keep only one side of the script partition, excluding glyph zero (.notdef).
/// Supports standalone SFNT (TTF/OTF) and indexed TTC/OTC faces. Keep passing the
/// same `index` to ab_glyph/egui after this call. If nothing needs removing, the
/// original allocation is returned unchanged. Empty Unicode coverage is valid:
/// callers can retain their normal fallback fonts. Input is limited to 64 MiB.
pub fn subset(bytes: Vec<u8>, index: u32, script: Script) -> Result<Vec<u8>, String> {
    // ttf-parser is checked, but keep this public malformed-input boundary robust
    // against dependency panics as well (including debug arithmetic assertions).
    std::panic::catch_unwind(|| subset_inner(bytes, index, script))
        .unwrap_or_else(|_| Err("Font parser panicked while reading malformed font data".into()))
}

#[derive(Clone, Copy, Debug)]
struct Table {
    entry: usize,
    offset: usize,
    length: usize,
}

struct Directory {
    collection: bool,
    cmap: Table,
    head: Option<Table>,
}

fn invalid() -> String {
    "Malformed or unsupported font table".into()
}

fn add(a: usize, b: usize) -> Result<usize, String> {
    a.checked_add(b).ok_or_else(invalid)
}

fn mul(a: usize, b: usize) -> Result<usize, String> {
    a.checked_mul(b).ok_or_else(invalid)
}

fn slice(data: &[u8], offset: usize, length: usize) -> Result<&[u8], String> {
    data.get(offset..add(offset, length)?).ok_or_else(invalid)
}

fn u16_at(data: &[u8], offset: usize) -> Result<u16, String> {
    let s = slice(data, offset, 2)?;
    Ok(u16::from_be_bytes([s[0], s[1]]))
}

fn u32_at(data: &[u8], offset: usize) -> Result<u32, String> {
    let s = slice(data, offset, 4)?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn usize_at(data: &[u8], offset: usize) -> Result<usize, String> {
    usize::try_from(u32_at(data, offset)?).map_err(|_| invalid())
}

fn put_u32(data: &mut [u8], offset: usize, value: u32) -> Result<(), String> {
    data.get_mut(offset..add(offset, 4)?)
        .ok_or_else(invalid)?
        .copy_from_slice(&value.to_be_bytes());
    Ok(())
}

fn directory(data: &[u8], index: u32) -> Result<Directory, String> {
    let collection = slice(data, 0, 4)? == b"ttcf";
    let (face, collection_header) = if collection {
        let version = u32_at(data, 4)?;
        if !matches!(version, 0x0001_0000 | 0x0002_0000) {
            return Err(invalid());
        }
        let count = usize_at(data, 8)?;
        let offsets_end = add(12, mul(count, 4)?)?;
        let header_end = add(offsets_end, if version == 0x0002_0000 { 12 } else { 0 })?;
        slice(data, 0, header_end)?;
        let index = usize::try_from(index).map_err(|_| invalid())?;
        if index >= count {
            return Err("Font collection face index is out of range".into());
        }
        (usize_at(data, add(12, mul(index, 4)?)?)?, header_end)
    } else {
        if index != 0 {
            return Err("Standalone font face index must be zero".into());
        }
        (0, 0)
    };
    if face < collection_header {
        return Err(invalid());
    }
    let signature = slice(data, face, 4)?;
    if signature != b"\0\x01\0\0" && signature != b"OTTO" && signature != b"true" {
        return Err("Unsupported SFNT signature".into());
    }
    let count = usize::from(u16_at(data, add(face, 4)?)?);
    if count == 0 || count > MAX_TABLES {
        return Err(invalid());
    }
    let entries = add(face, 12)?;
    let end = add(entries, mul(count, 16)?)?;
    slice(data, face, end - face)?;
    let mut cmap = None;
    let mut head = None;
    let mut previous_tag = None;
    let mut ranges = Vec::with_capacity(count);
    for i in 0..count {
        let entry = add(entries, mul(i, 16)?)?;
        let tag = u32_at(data, entry)?;
        if previous_tag.is_some_and(|previous| previous >= tag) {
            return Err("Font table directory is unsorted or contains duplicate tags".into());
        }
        previous_tag = Some(tag);
        let offset = usize_at(data, add(entry, 8)?)?;
        let length = usize_at(data, add(entry, 12)?)?;
        slice(data, offset, length)?;
        let table_end = add(offset, length)?;
        if length != 0 {
            if offset < collection_header || (offset < end && table_end > face) {
                return Err("Font table overlaps its directory or collection header".into());
            }
            ranges.push((offset, table_end));
        }
        let table = Table {
            entry,
            offset,
            length,
        };
        if tag == u32::from_be_bytes(*b"cmap") {
            cmap = Some(table);
        } else if tag == u32::from_be_bytes(*b"head") {
            if length < 54 {
                return Err(invalid());
            }
            head = Some(table);
        }
    }
    // Sharing tables between collection faces is fine, overlapping distinct
    // tables within the selected face is not (especially before adjusting head).
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err("Overlapping font tables".into());
    }
    Ok(Directory {
        collection,
        cmap: cmap.ok_or_else(|| "Font has no cmap table".to_owned())?,
        head,
    })
}

struct Coverage {
    candidates: Vec<bool>,
    enumerated: usize,
}

impl Coverage {
    fn range(&mut self, start: u32, end: u32) -> Result<(), String> {
        if start > end || end >= UNICODE_LIMIT as u32 {
            return Err("cmap range is outside Unicode".into());
        }
        self.enumerated = add(self.enumerated, (end - start + 1) as usize)?;
        if self.enumerated > MAX_ENUMERATED_CODEPOINTS {
            return Err("cmap exceeds the codepoint enumeration limit".into());
        }
        self.candidates[start as usize..=end as usize].fill(true);
        Ok(())
    }

    fn counted_range(&mut self, start: u32, count: u32) -> Result<(), String> {
        if count == 0 {
            return Ok(());
        }
        self.range(start, start.checked_add(count - 1).ok_or_else(invalid)?)
    }
}

/// Validate before calling codepoint_ids: that iterator eagerly enumerates entire
/// subtables, so merely putting .take() on it does not bound hostile range data.
fn coverage(cmap: &[u8]) -> Result<Coverage, String> {
    if u16_at(cmap, 0)? != 0 {
        return Err(invalid());
    }
    let count = usize::from(u16_at(cmap, 2)?);
    if count > MAX_CMAP_RECORDS {
        return Err("Unsupported number of cmap encoding records".into());
    }
    let header_end = add(4, mul(count, 8)?)?;
    slice(cmap, 0, header_end)?;
    let mut result = Coverage {
        candidates: vec![false; UNICODE_LIMIT],
        enumerated: 0,
    };
    for i in 0..count {
        let entry = add(4, mul(i, 8)?)?;
        let platform = u16_at(cmap, entry)?;
        let encoding = u16_at(cmap, add(entry, 2)?)?;
        let offset = usize_at(cmap, add(entry, 4)?)?;
        if offset < header_end {
            return Err(invalid());
        }
        let format = u16_at(cmap, offset)?;
        let length = match format {
            0 | 2 | 4 | 6 => usize::from(u16_at(cmap, add(offset, 2)?)?),
            8 | 10 | 12 | 13 => usize_at(cmap, add(offset, 4)?)?,
            14 => usize_at(cmap, add(offset, 2)?)?,
            _ => return Err("Unsupported cmap subtable format".into()),
        };
        let sub = slice(cmap, offset, length)?;
        // Same Unicode predicate as ab_glyph's ttf-parser implementation.
        let unicode = platform == 0
            || (platform == 3 && (encoding == 1 || (encoding == 10 && matches!(format, 12 | 13))));
        if !unicode {
            continue;
        }
        match format {
            0 => {
                slice(sub, 0, 262)?;
                result.range(0, 255)?;
            }
            2 => {
                slice(sub, 0, 518)?;
                let mut max_key = 0;
                for key in 0..256 {
                    let value = usize::from(u16_at(sub, 6 + key * 2)?);
                    if value % 8 != 0 {
                        return Err(invalid());
                    }
                    max_key = max_key.max(value);
                }
                slice(sub, 518, add(max_key, 8)?)?;
                for key in 0..256 {
                    let offset = add(518, usize::from(u16_at(sub, 6 + key * 2)?))?;
                    let first = usize::from(u16_at(sub, offset)?);
                    let count = usize::from(u16_at(sub, add(offset, 2)?)?);
                    if add(first, count)? > 256 {
                        return Err(invalid());
                    }
                    let glyphs = add(add(offset, 6)?, usize::from(u16_at(sub, add(offset, 6)?)?))?;
                    slice(sub, glyphs, mul(count, 2)?)?;
                }
                // Conservative BMP superset includes aliases and bounds all of
                // format 2's at-most-256 low bytes for each of 256 high bytes.
                result.range(0, 0xffff)?;
            }
            4 => {
                let count_x2 = usize::from(u16_at(sub, 6)?);
                if count_x2 == 0 || count_x2 % 2 != 0 {
                    return Err(invalid());
                }
                let count = count_x2 / 2;
                slice(sub, 0, add(16, mul(count, 8)?)?)?;
                let mut previous_end = None;
                for group in 0..count {
                    let end = u32::from(u16_at(sub, 14 + group * 2)?);
                    let start = u32::from(u16_at(sub, 16 + count * 2 + group * 2)?);
                    if previous_end.is_some_and(|previous| previous >= start) {
                        return Err(invalid());
                    }
                    previous_end = Some(end);
                    let range_pos = 16 + count * 6 + group * 2;
                    let range = usize::from(u16_at(sub, range_pos)?);
                    if start > end {
                        return Err(invalid());
                    }
                    if range != 0 {
                        slice(
                            sub,
                            add(range_pos, range)?,
                            mul((end - start + 1) as usize, 2)?,
                        )?;
                    }
                    result.range(start, end)?;
                }
            }
            6 => {
                let first = u32::from(u16_at(sub, 6)?);
                let count = u32::from(u16_at(sub, 8)?);
                slice(sub, 10, mul(count as usize, 2)?)?;
                if first + count > 0x10000 {
                    return Err(invalid());
                }
                result.counted_range(first, count)?;
            }
            10 => {
                let first = u32_at(sub, 12)?;
                let count = u32_at(sub, 16)?;
                slice(sub, 20, mul(count as usize, 2)?)?;
                result.counted_range(first, count)?;
            }
            12 | 13 => {
                let count = usize_at(sub, 12)?;
                if count > UNICODE_LIMIT {
                    return Err(invalid());
                }
                slice(sub, 16, mul(count, 12)?)?;
                let mut previous_end = None;
                for group in 0..count {
                    let pos = 16 + group * 12;
                    let start = u32_at(sub, pos)?;
                    let end = u32_at(sub, pos + 4)?;
                    let glyph = u32_at(sub, pos + 8)?;
                    if previous_end.is_some_and(|previous| previous >= start) {
                        return Err(invalid());
                    }
                    previous_end = Some(end);
                    result.range(start, end)?;
                    let last_glyph = if format == 12 {
                        glyph.checked_add(end - start).ok_or_else(invalid)?
                    } else {
                        glyph
                    };
                    if last_glyph > u16::MAX as u32 {
                        return Err(invalid());
                    }
                }
            }
            // ab_glyph does not enumerate these formats or use them for base
            // character lookup. Require their fixed headers to be present.
            8 => {
                slice(sub, 0, 8208)?;
            }
            14 => {
                slice(sub, 0, 10)?;
            }
            _ => return Err(invalid()),
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy)]
struct Group {
    start: u32,
    end: u32,
    glyph: u32,
}

fn make_cmap(groups: &[Group]) -> Result<Vec<u8>, String> {
    if groups.len() > UNICODE_LIMIT {
        return Err(invalid());
    }
    let sub_length = add(16, mul(groups.len(), 12)?)?;
    let length = add(20, sub_length)?;
    let mut data = Vec::new();
    data.try_reserve_exact(length).map_err(|e| e.to_string())?;
    data.extend_from_slice(&[0, 0, 0, 2]);
    // Unicode platform, full repertoire (0/4), then Windows UCS-4 (3/10).
    for (platform, encoding) in [(0u16, 4u16), (3, 10)] {
        data.extend_from_slice(&platform.to_be_bytes());
        data.extend_from_slice(&encoding.to_be_bytes());
        data.extend_from_slice(&20u32.to_be_bytes());
    }
    data.extend_from_slice(&12u16.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes());
    data.extend_from_slice(&(sub_length as u32).to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&(groups.len() as u32).to_be_bytes());
    for group in groups {
        data.extend_from_slice(&group.start.to_be_bytes());
        data.extend_from_slice(&group.end.to_be_bytes());
        data.extend_from_slice(&group.glyph.to_be_bytes());
    }
    Ok(data)
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, bytes| {
        let mut word = [0; 4];
        word[..bytes.len()].copy_from_slice(bytes);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn subset_inner(mut bytes: Vec<u8>, index: u32, script: Script) -> Result<Vec<u8>, String> {
    if bytes.is_empty() || bytes.len() > MAX_FONT_BYTES {
        return Err("Font input must be between 1 byte and 64 MiB".into());
    }
    let dir = directory(&bytes, index)?;
    let coverage = coverage(slice(&bytes, dir.cmap.offset, dir.cmap.length)?)?;
    let font = FontRef::try_from_slice_and_index(&bytes, index)
        .map_err(|_| "ab_glyph cannot parse this font face".to_owned())?;
    let mut mappings = vec![0u16; UNICODE_LIMIT];
    // Seed from ab_glyph, using its actual face-level lookup to resolve differing
    // mappings in legacy and full-Unicode subtables consistently with rendering.
    for (_, c) in font.codepoint_ids() {
        mappings[c as usize] = font.glyph_id(c).0;
    }
    // codepoint_ids deduplicates by GLYPH, not codepoint. Recover every alias
    // from validated coverage, e.g. ideographic space sharing an ASCII glyph.
    for (code, candidate) in coverage.candidates.into_iter().enumerate() {
        if candidate && mappings[code] == 0 {
            if let Some(c) = char::from_u32(code as u32) {
                mappings[code] = font.glyph_id(c).0;
            }
        }
    }
    let mut removed = false;
    let mut groups: Vec<Group> = Vec::new();
    for (code, glyph) in mappings.into_iter().enumerate() {
        if glyph == 0 {
            continue;
        }
        if usize::from(glyph) >= font.glyph_count() {
            return Err("cmap references a glyph outside maxp".into());
        }
        let c = char::from_u32(code as u32).ok_or_else(invalid)?;
        let keep = match script {
            Script::English => !is_chinese(c),
            Script::Chinese => is_chinese(c),
        };
        if !keep {
            removed = true;
            continue;
        }
        let code = code as u32;
        let glyph = u32::from(glyph);
        if let Some(last) = groups.last_mut() {
            if last.end + 1 == code && last.glyph + (code - last.start) == glyph {
                last.end = code;
                continue;
            }
        }
        if groups.len() >= UNICODE_LIMIT {
            return Err(invalid());
        }
        groups.push(Group {
            start: code,
            end: code,
            glyph,
        });
    }
    if !removed {
        return Ok(bytes);
    }
    let cmap = make_cmap(&groups)?;
    let aligned = add(bytes.len(), 3)? & !3;
    let final_size = add(aligned, add(cmap.len(), 3)? & !3)?;
    if final_size > add(MAX_FONT_BYTES, add(36, mul(UNICODE_LIMIT, 12)?)?)? {
        return Err("Subset output exceeds bounded cmap allocation".into());
    }
    bytes
        .try_reserve_exact(final_size - bytes.len())
        .map_err(|e| e.to_string())?;
    bytes.resize(aligned, 0);
    bytes.extend_from_slice(&cmap);
    bytes.resize(final_size, 0);
    put_u32(&mut bytes, add(dir.cmap.entry, 4)?, checksum(&cmap))?;
    put_u32(
        &mut bytes,
        add(dir.cmap.entry, 8)?,
        u32::try_from(aligned).map_err(|_| invalid())?,
    )?;
    put_u32(
        &mut bytes,
        add(dir.cmap.entry, 12)?,
        u32::try_from(cmap.len()).map_err(|_| invalid())?,
    )?;
    if !dir.collection {
        if let Some(head) = dir.head {
            let adjustment = add(head.offset, 8)?;
            put_u32(&mut bytes, adjustment, 0)?;
            let head_sum = checksum(slice(&bytes, head.offset, head.length)?);
            put_u32(&mut bytes, add(head.entry, 4)?, head_sum)?;
            let adjustment_value = CHECKSUM_MAGIC.wrapping_sub(checksum(&bytes));
            put_u32(&mut bytes, adjustment, adjustment_value)?;
        }
    }
    // TTC head tables can be shared by several faces; their adjustment must not
    // be changed. Collection files do not use the standalone whole-file checksum.
    FontRef::try_from_slice_and_index(&bytes, index)
        .map_err(|_| "ab_glyph cannot parse the filtered cmap".to_owned())?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn installed(name: &str) -> Option<Vec<u8>> {
        let root = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
        let path = PathBuf::from(root).join("Fonts").join(name);
        match std::fs::read(&path) {
            Ok(data) => Some(data),
            Err(_) => {
                eprintln!(
                    "Skipping installed-font test: {} not present",
                    path.display()
                );
                None
            }
        }
    }

    fn assert_raster(font: &FontRef<'_>, c: char) {
        let glyph = font
            .glyph_id(c)
            .with_scale_and_position(32.0, ab_glyph::point(0.0, 0.0));
        let outline = font
            .outline_glyph(glyph)
            .expect("retained glyph must still have an outline");
        let mut ink = 0;
        outline.draw(|_, _, coverage| {
            if coverage > 0.1 {
                ink += 1;
            }
        });
        assert!(ink > 10, "retained {c} rasterized blank");
    }

    fn with_test_cmap(mut bytes: Vec<u8>, cmap: &[u8]) -> Vec<u8> {
        let dir = directory(&bytes, 0).unwrap();
        let offset = (bytes.len() + 3) & !3;
        bytes.resize(offset, 0);
        bytes.extend_from_slice(cmap);
        put_u32(&mut bytes, dir.cmap.entry + 4, checksum(cmap)).unwrap();
        put_u32(&mut bytes, dir.cmap.entry + 8, offset as u32).unwrap();
        put_u32(&mut bytes, dir.cmap.entry + 12, cmap.len() as u32).unwrap();
        bytes
    }

    fn assert_format12(bytes: &[u8], index: u32) {
        let dir = directory(bytes, index).unwrap();
        let table = slice(bytes, dir.cmap.offset, dir.cmap.length).unwrap();
        assert_eq!(dir.cmap.offset % 4, 0);
        assert_eq!(u16_at(table, 0).unwrap(), 0);
        assert_eq!(u16_at(table, 2).unwrap(), 2);
        for (entry, platform, encoding) in [(4, 0, 4), (12, 3, 10)] {
            assert_eq!(u16_at(table, entry).unwrap(), platform);
            assert_eq!(u16_at(table, entry + 2).unwrap(), encoding);
            assert_eq!(u32_at(table, entry + 4).unwrap(), 20);
        }
        let sub = &table[20..];
        assert_eq!(u16_at(sub, 0).unwrap(), 12);
        assert_eq!(u16_at(sub, 2).unwrap(), 0);
        assert_eq!(usize_at(sub, 4).unwrap(), sub.len());
        assert_eq!(u32_at(sub, 8).unwrap(), 0);
        let count = usize_at(sub, 12).unwrap();
        assert_eq!(sub.len(), 16 + count * 12);
        let mut previous_end = None;
        for i in 0..count {
            let start = u32_at(sub, 16 + i * 12).unwrap();
            let end = u32_at(sub, 20 + i * 12).unwrap();
            let glyph = u32_at(sub, 24 + i * 12).unwrap();
            assert!(start <= end && end < UNICODE_LIMIT as u32);
            assert!(previous_end.is_none_or(|previous| previous < start));
            assert!(glyph > 0 && glyph + end - start <= u16::MAX as u32);
            previous_end = Some(end);
        }
        assert_eq!(u32_at(bytes, dir.cmap.entry + 4).unwrap(), checksum(table));
        coverage(table).unwrap();
    }

    #[test]
    fn partition_covers_supplementary_han_and_compatible_cjk() {
        for c in [
            '中',
            '，',
            '。',
            '！',
            'Ａ',
            '\u{3000}',
            'ㄅ',
            'あ',
            '한',
            '\u{20000}',
            '\u{2b740}',
            '\u{2f800}',
            '\u{30000}',
            '\u{3347f}',
        ] {
            assert!(is_chinese(c), "{c} must be CJK");
        }
        for c in [
            'A',
            ' ',
            '!',
            'é',
            'Ж',
            '\u{301}',
            '😀',
            '\u{fe0f}',
            '\u{e0100}',
        ] {
            assert!(!is_chinese(c), "{c} must be non-CJK");
        }
    }

    #[test]
    fn msjh_every_collection_face_has_independent_cmaps_and_intact_outlines() {
        let Some(original) = installed("msjh.ttc") else {
            return;
        };
        let count = u32_at(&original, 8).unwrap();
        assert!(count > 1, "exercise later TTC faces too");
        for index in 0..count {
            let before = FontRef::try_from_slice_and_index(&original, index).unwrap();
            for (script, retained, removed) in
                [(Script::English, 'A', '中'), (Script::Chinese, '中', 'A')]
            {
                let filtered = subset(original.clone(), index, script).unwrap();
                let after = FontRef::try_from_slice_and_index(&filtered, index).unwrap();
                assert_eq!(after.glyph_id(retained), before.glyph_id(retained));
                assert_ne!(after.glyph_id(retained).0, 0);
                assert_eq!(after.glyph_id(removed).0, 0);
                assert!(!after.codepoint_ids().any(|(_, c)| match script {
                    Script::English => is_chinese(c),
                    Script::Chinese => !is_chinese(c),
                }));
                assert_raster(&after, retained);
                assert_format12(&filtered, index);
                // Every other face continues to point to the original shared cmap.
                for other in 0..count {
                    if other == index {
                        continue;
                    }
                    let old = FontRef::try_from_slice_and_index(&original, other).unwrap();
                    let new = FontRef::try_from_slice_and_index(&filtered, other).unwrap();
                    assert_eq!(new.glyph_id('A'), old.glyph_id('A'));
                    assert_eq!(new.glyph_id('中'), old.glyph_id('中'));
                }
                let old_dir = directory(&original, index).unwrap();
                for i in 0..original.len() {
                    if !(old_dir.cmap.entry + 4..old_dir.cmap.entry + 16).contains(&i) {
                        assert_eq!(filtered[i], original[i], "modified existing table at {i}");
                    }
                }
            }
        }
        assert!(subset(original, count, Script::English).is_err());
    }

    #[test]
    fn arial_remains_valid_and_standalone_checksum_is_adjusted() {
        let Some(original) = installed("arial.ttf") else {
            return;
        };
        let before = FontRef::try_from_slice(&original).unwrap();
        let english = subset(original.clone(), 0, Script::English).unwrap();
        let after = FontRef::try_from_slice(&english).unwrap();
        for c in ' '..='~' {
            assert_eq!(before.glyph_id(c), after.glyph_id(c));
        }
        assert_raster(&after, 'A');
        if english != original {
            assert_format12(&english, 0);
            assert_eq!(checksum(&english), CHECKSUM_MAGIC);
        }
        let chinese = subset(original, 0, Script::Chinese).unwrap();
        assert_eq!(
            FontRef::try_from_slice(&chinese).unwrap().glyph_id('A').0,
            0
        );
        assert_format12(&chinese, 0);
        assert_eq!(checksum(&chinese), CHECKSUM_MAGIC);
    }

    #[test]
    fn cff_otf_remains_rasterizable_when_available() {
        let Some(original) =
            installed("MyriadPro-Regular.otf").or_else(|| installed("ROGFonts-Regular.otf"))
        else {
            return;
        };
        assert_eq!(&original[..4], b"OTTO");
        let english = subset(original.clone(), 0, Script::English).unwrap();
        assert_raster(&FontRef::try_from_slice(&english).unwrap(), 'A');
        let chinese = subset(original, 0, Script::Chinese).unwrap();
        assert_format12(&chinese, 0);
        assert_eq!(
            FontRef::try_from_slice(&chinese).unwrap().glyph_id('A').0,
            0
        );
        assert_eq!(checksum(&chinese), CHECKSUM_MAGIC);
    }

    #[test]
    fn aliases_survive_and_repeated_filtering_is_a_noop() {
        let Some(original) = installed("msjh.ttc") else {
            return;
        };
        let before = FontRef::try_from_slice(&original).unwrap();
        let chinese = subset(original.clone(), 0, Script::Chinese).unwrap();
        let after = FontRef::try_from_slice(&chinese).unwrap();
        for code in 0..UNICODE_LIMIT as u32 {
            if let Some(c) = char::from_u32(code) {
                if is_chinese(c) {
                    assert_eq!(
                        after.glyph_id(c),
                        before.glyph_id(c),
                        "lost alias U+{code:04X}"
                    );
                }
            }
        }
        let filtered_again = subset(chinese.clone(), 0, Script::Chinese).unwrap();
        assert_eq!(filtered_again, chinese);
        let english = subset(original, 0, Script::English).unwrap();
        assert_eq!(
            subset(english.clone(), 0, Script::English).unwrap(),
            english
        );
    }

    #[test]
    fn shared_latin_han_glyphs_and_supplementary_aliases_are_partitioned() {
        let Some(original) = installed("arial.ttf") else {
            return;
        };
        let font = FontRef::try_from_slice(&original).unwrap();
        let glyph = u32::from(font.glyph_id('A').0);
        // One outline, four Unicode aliases on both sides of the partition.
        // codepoint_ids alone reports only the first alias, ASCII A.
        let cmap = make_cmap(&[65, 0x3000, 0x4e2d, 0x20000].map(|code| Group {
            start: code,
            end: code,
            glyph,
        }))
        .unwrap();
        let original = with_test_cmap(original, &cmap);
        let font = FontRef::try_from_slice(&original).unwrap();
        assert_eq!(font.codepoint_ids().count(), 1);
        for script in [Script::English, Script::Chinese] {
            let filtered = subset(original.clone(), 0, script).unwrap();
            let font = FontRef::try_from_slice(&filtered).unwrap();
            for c in ['A', '\u{3000}', '中', '\u{20000}'] {
                let keep = match script {
                    Script::English => !is_chinese(c),
                    Script::Chinese => is_chinese(c),
                };
                assert_eq!(u32::from(font.glyph_id(c).0), if keep { glyph } else { 0 });
            }
            assert_format12(&filtered, 0);
            assert_raster(
                &font,
                match script {
                    Script::English => 'A',
                    Script::Chinese => '\u{20000}',
                },
            );
            assert_eq!(checksum(&filtered), CHECKSUM_MAGIC);
            assert_eq!(subset(filtered.clone(), 0, script).unwrap(), filtered);
        }
    }

    #[test]
    fn empty_script_subsets_and_symbol_only_fonts_remain_valid() {
        let Some(original) = installed("arial.ttf") else {
            return;
        };
        let font = FontRef::try_from_slice(&original).unwrap();
        let glyph = u32::from(font.glyph_id('A').0);
        for (code, script) in [(65, Script::Chinese), (0x4e2d, Script::English)] {
            let cmap = make_cmap(&[Group {
                start: code,
                end: code,
                glyph,
            }])
            .unwrap();
            let fixture = with_test_cmap(original.clone(), &cmap);
            let filtered = subset(fixture, 0, script).unwrap();
            let font = FontRef::try_from_slice(&filtered).unwrap();
            assert_eq!(font.codepoint_ids().count(), 0);
            assert_eq!(font.glyph_id('A').0, 0);
            assert_eq!(font.glyph_id('中').0, 0);
            assert_format12(&filtered, 0);
            let dir = directory(&filtered, 0).unwrap();
            assert_eq!(u32_at(&filtered, dir.cmap.offset + 32).unwrap(), 0);
            assert_eq!(checksum(&filtered), CHECKSUM_MAGIC);
        }
        // A Windows Symbol record (3/0) has no Unicode mappings to steal, so
        // filtering is allowed to leave it unchanged, with empty Unicode coverage.
        let mut symbol_cmap = make_cmap(&[Group {
            start: 0xf041,
            end: 0xf041,
            glyph,
        }])
        .unwrap();
        symbol_cmap[2..4].copy_from_slice(&1u16.to_be_bytes());
        symbol_cmap[4..6].copy_from_slice(&3u16.to_be_bytes());
        symbol_cmap[6..8].copy_from_slice(&0u16.to_be_bytes());
        for cmap in [symbol_cmap, make_cmap(&[]).unwrap(), vec![0, 0, 0, 0]] {
            let fixture = with_test_cmap(original.clone(), &cmap);
            for script in [Script::English, Script::Chinese] {
                let filtered = subset(fixture.clone(), 0, script).unwrap();
                assert_eq!(filtered, fixture);
                assert_eq!(
                    FontRef::try_from_slice(&filtered)
                        .unwrap()
                        .codepoint_ids()
                        .count(),
                    0
                );
            }
        }
    }

    #[test]
    fn generated_format12_has_sorted_compact_groups_and_no_notdef() {
        let cmap = make_cmap(&[
            Group {
                start: 65,
                end: 67,
                glyph: 1,
            },
            Group {
                start: 0x20000,
                end: 0x20002,
                glyph: 4,
            },
        ])
        .unwrap();
        let candidates = coverage(&cmap).unwrap();
        assert!(candidates.candidates[65]);
        assert!(candidates.candidates[0x20002]);
        assert!(!candidates.candidates[64]);
        assert!(!candidates.candidates[0x20003]);
        assert!(coverage(&make_cmap(&[]).unwrap()).is_ok());
    }

    #[test]
    fn malformed_fonts_and_unbounded_cmap_ranges_return_errors_not_panics() {
        for data in [vec![], vec![0; 128], b"ttcf".to_vec(), b"OTTO".to_vec()] {
            assert!(subset(data, u32::MAX, Script::English).is_err());
        }
        let mut collection = vec![0u8; 16];
        collection[..4].copy_from_slice(b"ttcf");
        put_u32(&mut collection, 4, 0x0001_0000).unwrap();
        put_u32(&mut collection, 8, u32::MAX).unwrap();
        assert!(subset(collection, 0, Script::English).is_err());
        let valid = make_cmap(&[Group {
            start: 65,
            end: 90,
            glyph: 1,
        }])
        .unwrap();
        for (offset, value) in [
            (8, u32::MAX),
            (24, u32::MAX),
            (32, u32::MAX),
            (36, 0x110000),
            (40, u32::MAX),
            (44, u32::MAX),
        ] {
            let mut malformed = valid.clone();
            put_u32(&mut malformed, offset, value).unwrap();
            assert!(
                coverage(&malformed).is_err(),
                "accepted malformed cmap at {offset}"
            );
        }
        for end in 0..valid.len() {
            assert!(coverage(&valid[..end]).is_err());
        }
        let Some(original) = installed("arial.ttf") else {
            return;
        };
        let dir = directory(&original, 0).unwrap();
        for (offset, value) in [
            (dir.cmap.entry + 8, u32::MAX),
            (dir.cmap.entry + 12, u32::MAX),
            (dir.cmap.offset + 8, u32::MAX),
        ] {
            let mut malformed = original.clone();
            put_u32(&mut malformed, offset, value).unwrap();
            assert!(
                subset(malformed, 0, Script::English).is_err(),
                "accepted malformed font at {offset}"
            );
        }
        for length in [0, 1, 4, 11, 12, 100, original.len() - 1] {
            assert!(subset(original[..length].to_vec(), 0, Script::English).is_err());
        }
        assert!(subset(vec![0; MAX_FONT_BYTES + 1], 0, Script::English).is_err());
    }
}
