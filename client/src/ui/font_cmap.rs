//! The characters a TrueType/OpenType face maps (its `cmap`), read from the
//! font bytes Bevy already loaded. The kit uses it to pick a fallback face
//! for text a small display face cannot draw (Cinzel is Latin-only, the
//! Serif SC face is a subset of the zh-Hans dictionaries). Reads the Unicode
//! subtables in format 4 (BMP) and 12 (full range); anything malformed reads
//! as an empty set, which makes every text fall back to a complete face.
use std::collections::HashSet;

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// Offset of the `cmap` table in an sfnt (TrueType/CFF) font.
fn cmap_offset(data: &[u8]) -> Option<usize> {
    let tables = usize::from(u16_at(data, 4)?);
    (0..tables)
        .find_map(|index| {
            let record = 12 + index * 16;
            (data.get(record..record + 4)? == b"cmap").then(|| u32_at(data, record + 8))?
        })
        .map(|offset| offset as usize)
}

fn format4(data: &[u8], table: usize, out: &mut HashSet<char>) -> Option<()> {
    let segments = usize::from(u16_at(data, table + 6)? / 2);
    let ends = table + 14;
    let starts = ends + segments * 2 + 2;
    let deltas = starts + segments * 2;
    let ranges = deltas + segments * 2;
    for segment in 0..segments {
        let end = u16_at(data, ends + segment * 2)?;
        let start = u16_at(data, starts + segment * 2)?;
        let delta = u16_at(data, deltas + segment * 2)?;
        let range_offset = u16_at(data, ranges + segment * 2)?;
        if start == 0xFFFF {
            continue;
        }
        for code in start..=end {
            let glyph = if range_offset == 0 {
                code.wrapping_add(delta)
            } else {
                let at = ranges
                    + segment * 2
                    + usize::from(range_offset)
                    + usize::from(code - start) * 2;
                match u16_at(data, at)? {
                    0 => 0,
                    glyph => glyph.wrapping_add(delta),
                }
            };
            if glyph != 0 {
                if let Some(character) = char::from_u32(u32::from(code)) {
                    out.insert(character);
                }
            }
        }
    }
    Some(())
}

fn format12(data: &[u8], table: usize, out: &mut HashSet<char>) -> Option<()> {
    let groups = u32_at(data, table + 12)? as usize;
    for group in 0..groups {
        let record = table + 16 + group * 12;
        let start = u32_at(data, record)?;
        let end = u32_at(data, record + 4)?;
        let first_glyph = u32_at(data, record + 8)?;
        // Guard against a malformed range blowing up the set.
        if end < start || end - start > 0x10_0000 {
            return None;
        }
        for code in start..=end {
            if first_glyph + (code - start) != 0 {
                if let Some(character) = char::from_u32(code) {
                    out.insert(character);
                }
            }
        }
    }
    Some(())
}

/// Every character with a glyph in the face.
pub(crate) fn characters(data: &[u8]) -> HashSet<char> {
    let mut out = HashSet::new();
    let Some(cmap) = cmap_offset(data) else {
        return out;
    };
    let count = u16_at(data, cmap + 2).unwrap_or(0);
    for index in 0..usize::from(count) {
        let record = cmap + 4 + index * 8;
        let (Some(platform), Some(encoding), Some(offset)) = (
            u16_at(data, record),
            u16_at(data, record + 2),
            u32_at(data, record + 4),
        ) else {
            break;
        };
        // Unicode subtables only: platform 0, or Windows (3) BMP/full.
        if !(platform == 0 || (platform == 3 && matches!(encoding, 1 | 10))) {
            continue;
        }
        let table = cmap + offset as usize;
        let _ = match u16_at(data, table) {
            Some(4) => format4(data, table, &mut out),
            Some(12) => format12(data, table, &mut out),
            _ => None,
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(path: &str) -> HashSet<char> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        characters(&std::fs::read(root.join(path)).unwrap())
    }

    #[test]
    fn reads_what_each_packaged_face_can_draw() {
        let cinzel = face("ui/verdant/fonts/Cinzel-SemiBold.ttf");
        assert!("PLAY again 0123".chars().all(|c| cinzel.contains(&c)));
        assert!(!cinzel.contains(&'Д'), "Cinzel is Latin-only");
        let serif = face("ui/verdant/fonts/NotoSerifSC-SemiBold-subset.ttf");
        assert!("开始游戏".chars().all(|c| serif.contains(&c)));
        assert!(!serif.contains(&'龘'), "a subset of the dictionaries");
        let inter = face("ui/verdant/fonts/Inter-SemiBold.ttf");
        assert!("Дмитрий".chars().all(|c| inter.contains(&c)));
        assert!(characters(b"not a font").is_empty());
    }
}
