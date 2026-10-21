//! Unmodified DejaVu Sans, embedded so a PDF does not depend on Courier.
//! WinAnsi covers the French repertoire already accepted. U+1F389 is drawn
//! as an image because this font has no emoji glyph.
use std::sync::OnceLock;

pub const PARTY: char = '🎉';
pub const PARTY_PX: u32 = 48;

pub struct Face {
    pub bytes: &'static [u8],
    pub ascent: i32,
    pub descent: i32,
    pub cap_height: i32,
    pub bbox: [i32; 4],
    /// PDF widths in 1/1000 em, indexed by WinAnsi byte.
    pub widths: [u16; 256],
    /// Unicode scalar, or 0 when the byte is not in the repertoire.
    pub unicode: [u32; 256],
    pub glyphs: [u16; 256],
}

static FACE: OnceLock<Face> = OnceLock::new();

pub fn face() -> &'static Face {
    FACE.get_or_init(load)
}

pub fn encode(c: char) -> Option<u8> {
    Some(match c {
        '\u{20}'..='\u{7e}' | '\u{a0}'..='\u{ff}' => c as u8,
        '\u{202f}' => 160,
        '€' => 128,
        '‚' => 130,
        'ƒ' => 131,
        '„' => 132,
        '…' => 133,
        '†' => 134,
        '‡' => 135,
        'ˆ' => 136,
        '‰' => 137,
        'Š' => 138,
        '‹' => 139,
        'Œ' => 140,
        'Ž' => 142,
        '‘' => 145,
        '’' => 146,
        '“' => 147,
        '”' => 148,
        '•' => 149,
        '–' => 150,
        '—' => 151,
        '˜' => 152,
        '™' => 153,
        'š' => 154,
        '›' => 155,
        'œ' => 156,
        'ž' => 158,
        'Ÿ' => 159,
        _ => return None,
    })
}

pub fn party_popper() -> &'static [u8] {
    static PIXELS: OnceLock<Vec<u8>> = OnceLock::new();
    PIXELS.get_or_init(draw_party)
}

fn draw_party() -> Vec<u8> {
    let n = PARTY_PX as i32;
    let mut px = vec![255u8; (n * n * 3) as usize];
    let mut put = |x: i32, y: i32, rgb: [u8; 3]| {
        if (0..n).contains(&x) && (0..n).contains(&y) {
            let i = ((y * n + x) * 3) as usize;
            px[i..i + 3].copy_from_slice(&rgb);
        }
    };
    for y in 30..44 {
        for x in 6..16 {
            put(x, y, [217, 119, 6]);
        }
    }
    for y in 8..34 {
        let t = (y - 8) as f32 / 26.0;
        let cx = 18.0 + (1.0 - t) * 6.0;
        let half = 2.0 + t * 9.0;
        for x in (cx - half) as i32..=(cx + half) as i32 {
            put(x, y, [249, 115, 22]);
        }
    }
    for (cx, cy, color) in [
        (34, 8, [220, 38, 38]),
        (40, 14, [37, 99, 235]),
        (36, 20, [22, 163, 74]),
        (42, 8, [147, 51, 234]),
        (30, 16, [234, 179, 8]),
        (28, 6, [14, 165, 233]),
    ] {
        for dy in -2..=2 {
            for dx in -2..=2 {
                if dx * dx + dy * dy <= 5 {
                    put(cx + dx, cy + dy, color);
                }
            }
        }
    }
    px
}

fn load() -> Face {
    let bytes = include_bytes!("../fonts/DejaVuSans.ttf");
    let upem = table(bytes, b"head")
        .and_then(|t| be_u16(t, 18))
        .filter(|n| *n > 0)
        .unwrap_or(2048);
    let head = table(bytes, b"head").unwrap_or(&[]);
    let hhea = table(bytes, b"hhea").unwrap_or(&[]);
    let scale = |v: i16| v as i32 * 1000 / upem as i32;
    let bbox = [
        scale(be_i16(head, 36).unwrap_or(0)),
        scale(be_i16(head, 38).unwrap_or(0)),
        scale(be_i16(head, 40).unwrap_or(0)),
        scale(be_i16(head, 42).unwrap_or(0)),
    ];
    let ascent = scale(be_i16(hhea, 4).unwrap_or(1600));
    let descent = scale(be_i16(hhea, 6).unwrap_or(-400));
    let cap_height = table(bytes, b"OS/2")
        .filter(|os2| be_u16(os2, 0).unwrap_or(0) >= 2)
        .and_then(|os2| be_i16(os2, 88))
        .map(scale)
        .filter(|h| *h > 0)
        .unwrap_or(ascent * 7 / 10);
    let mut unicode = [0u32; 256];
    let mut glyphs = [0u16; 256];
    let mut widths = [0u16; 256];
    for c in [
        '€', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', 'Ž', '‘', '’', '“', '”', '•',
        '–', '—', '˜', '™', 'š', '›', 'œ', 'ž', 'Ÿ', '\u{202f}',
    ] {
        if let Some(b) = encode(c) {
            unicode[b as usize] = c as u32;
        }
    }
    for u in (0x20u32..=0x7e).chain(0xa0..=0xff) {
        unicode[u as usize] = u;
    }
    for b in 0..256 {
        let Some(c) = char::from_u32(unicode[b]) else {
            continue;
        };
        if c == '\0' {
            continue;
        }
        let glyph = glyph_of(bytes, c as u32);
        glyphs[b] = glyph;
        if glyph != 0 {
            let advance = advance_of(bytes, glyph);
            widths[b] = (advance as u32 * 1000 / upem as u32).min(u16::MAX as u32) as u16;
        }
    }
    Face {
        bytes,
        ascent,
        descent,
        cap_height,
        bbox,
        widths,
        unicode,
        glyphs,
    }
}

fn table<'a>(font: &'a [u8], tag: &[u8; 4]) -> Option<&'a [u8]> {
    let n = be_u16(font, 4)? as usize;
    for i in 0..n {
        let rec = 12 + i * 16;
        if font.get(rec..rec + 4)? != tag {
            continue;
        }
        let off = be_u32(font, rec + 8)? as usize;
        let len = be_u32(font, rec + 12)? as usize;
        return font.get(off..off + len);
    }
    None
}

fn glyph_of(font: &[u8], code: u32) -> u16 {
    let Some(cmap) = table(font, b"cmap") else {
        return 0;
    };
    let n = be_u16(cmap, 2).unwrap_or(0) as usize;
    let mut chosen: Option<(u8, usize)> = None;
    for i in 0..n {
        let base = 4 + i * 8;
        let Some(plat) = be_u16(cmap, base) else {
            break;
        };
        let Some(enc) = be_u16(cmap, base + 2) else {
            break;
        };
        let Some(off) = be_u32(cmap, base + 4) else {
            break;
        };
        let off = off as usize;
        let Some(format) = be_u16(cmap, off) else {
            continue;
        };
        let score = match (plat, enc, format) {
            (3, 10, 12) => 5,
            (0, 4, 12) => 4,
            (3, 1, 4) => 3,
            (0, 3, 4) => 2,
            (_, _, 12) => 1,
            (_, _, 4) => 1,
            _ => 0,
        };
        if score > 0 && chosen.map(|(s, _)| score > s).unwrap_or(true) {
            chosen = Some((score, off));
        }
    }
    let Some((_, off)) = chosen else {
        return 0;
    };
    let sub = &cmap[off..];
    match be_u16(sub, 0).unwrap_or(0) {
        4 => glyph_format4(sub, code),
        12 => glyph_format12(sub, code),
        _ => 0,
    }
}

fn glyph_format4(sub: &[u8], code: u32) -> u16 {
    if code > 0xffff {
        return 0;
    }
    let code = code as u16;
    let seg_count = be_u16(sub, 6).unwrap_or(0) as usize / 2;
    let end_pos = 14;
    let start_pos = end_pos + seg_count * 2 + 2;
    let delta_pos = start_pos + seg_count * 2;
    let offset_pos = delta_pos + seg_count * 2;
    for i in 0..seg_count {
        let Some(end) = be_u16(sub, end_pos + i * 2) else {
            return 0;
        };
        if code > end {
            continue;
        }
        let Some(start) = be_u16(sub, start_pos + i * 2) else {
            return 0;
        };
        if code < start {
            return 0;
        }
        let delta = be_i16(sub, delta_pos + i * 2).unwrap_or(0) as u16;
        let range_off = be_u16(sub, offset_pos + i * 2).unwrap_or(0);
        if range_off == 0 {
            return code.wrapping_add(delta);
        }
        let glyph_pos = offset_pos + i * 2 + range_off as usize + (code - start) as usize * 2;
        let Some(glyph) = be_u16(sub, glyph_pos) else {
            return 0;
        };
        if glyph == 0 {
            return 0;
        }
        return glyph.wrapping_add(delta);
    }
    0
}

fn glyph_format12(sub: &[u8], code: u32) -> u16 {
    let n = be_u32(sub, 12).unwrap_or(0) as usize;
    let mut pos = 16;
    for _ in 0..n {
        let Some(start) = be_u32(sub, pos) else {
            return 0;
        };
        let Some(end) = be_u32(sub, pos + 4) else {
            return 0;
        };
        let Some(glyph) = be_u32(sub, pos + 8) else {
            return 0;
        };
        if code >= start && code <= end {
            let id = glyph + (code - start);
            return if id == 0 || id > 0xffff { 0 } else { id as u16 };
        }
        pos += 12;
    }
    0
}

fn advance_of(font: &[u8], glyph: u16) -> u16 {
    let Some(hhea) = table(font, b"hhea") else {
        return 0;
    };
    let Some(hmtx) = table(font, b"hmtx") else {
        return 0;
    };
    let n = be_u16(hhea, 34).unwrap_or(0);
    if n == 0 {
        return 0;
    }
    let index = if glyph < n { glyph } else { n - 1 };
    be_u16(hmtx, index as usize * 4).unwrap_or(0)
}

fn be_u16(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn be_i16(b: &[u8], i: usize) -> Option<i16> {
    Some(i16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn be_u32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repertoire_glyphs_are_present() {
        let face = face();
        for b in 0..256 {
            let Some(c) = char::from_u32(face.unicode[b]) else {
                continue;
            };
            if c == '\0' {
                continue;
            }
            assert_ne!(face.glyphs[b], 0, "{c} U+{:04X}", c as u32);
            assert!(face.widths[b] > 0, "{c}");
        }
        assert_eq!(encode('é'), Some(0xe9));
        assert_eq!(encode('ç'), Some(0xe7));
        assert!(encode(PARTY).is_none());
        assert!(encode('→').is_none());
        assert_eq!(party_popper().len(), (PARTY_PX * PARTY_PX * 3) as usize);
    }
}
