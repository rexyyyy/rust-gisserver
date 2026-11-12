//! Minimal 5×7 bitmap font (ASCII 32..=126) for `TextSymbolizer` labels and
//! legend swatches — zero-dependency, no fontconfig/FreeType.
//!
//! Each glyph is 5 columns × 7 rows; a glyph byte is one column with bit 0 =
//! top row (classic `font5x7` layout, public domain). Characters outside the
//! printable ASCII range render as blanks (Latin-1 accents and CJK are a
//! documented boundary — labels fall back to `?`-free blanks).
//!
//! Rendering is nearest-neighbour at an integer scale: `scale 1` = 5×7 px,
//! `scale 2` = 10×14 px. Advance is `6 * scale` px per character (1 px gap).

use crate::renderer::blend::blend_px;

/// Glyph cell width in pixels (at scale 1).
pub const GLYPH_W: usize = 5;
/// Glyph cell height in pixels (at scale 1).
pub const GLYPH_H: usize = 7;
/// Horizontal advance per character (glyph + 1 px gap) at scale 1.
pub const ADVANCE: usize = GLYPH_W + 1;

/// Column bitmap for a printable ASCII char (`None` → render as blank).
pub fn glyph(c: char) -> Option<[u8; 5]> {
    let idx = c as u32;
    if !(0x20..=0x7E).contains(&idx) {
        return None;
    }
    Some(FONT5X7[(idx - 0x20) as usize])
}

/// Text advance width in pixels at scale 1 (trailing gap excluded).
pub fn text_width(s: &str, scale: usize) -> usize {
    if s.is_empty() {
        return 0;
    }
    s.chars().count() * ADVANCE * scale.max(1) - scale.max(1)
}

/// Text block height in pixels.
pub fn text_height(scale: usize) -> usize {
    GLYPH_H * scale.max(1)
}

/// Draw a single glyph with its top-left corner at (x, y). `scale` is the
/// integer pixel multiplier (≥1). Unknown chars draw nothing.
pub fn draw_glyph(
    canvas: &mut [u8],
    cw: usize,
    ch: usize,
    x: i64,
    y: i64,
    c: char,
    color: [u8; 4],
    scale: usize,
) {
    let Some(cols) = glyph(c) else { return };
    let s = scale.max(1) as i64;
    for (ci, col) in cols.iter().enumerate() {
        for row in 0..GLYPH_H as u8 {
            if col & (1 << row) == 0 {
                continue;
            }
            for dy in 0..s {
                for dx in 0..s {
                    blend_px(
                        canvas,
                        x + ci as i64 * s + dx,
                        y + row as i64 * s + dy,
                        cw,
                        ch,
                        color,
                    );
                }
            }
        }
    }
}

/// Draw one line of text with its top-left corner at (x, y). No wrapping.
pub fn draw_text(
    canvas: &mut [u8],
    cw: usize,
    ch: usize,
    x: i64,
    y: i64,
    text: &str,
    color: [u8; 4],
    scale: usize,
) {
    let s = scale.max(1) as i64;
    let mut cx = x;
    for c in text.chars() {
        draw_glyph(canvas, cw, ch, cx, y, c, color, scale);
        cx += ADVANCE as i64 * s;
    }
}

/// Draw a text line centered horizontally around `cx` (canvas px), with the
/// glyph box top at `top`.
pub fn draw_text_centered(
    canvas: &mut [u8],
    cw: usize,
    ch: usize,
    cx: f64,
    top: i64,
    text: &str,
    color: [u8; 4],
    scale: usize,
) {
    let w = text_width(text, scale) as f64;
    let x = (cx - w / 2.0).round() as i64;
    draw_text(canvas, cw, ch, x, top, text, color, scale);
}

/// `font5x7` table for ASCII 0x20..=0x7E (95 glyphs × 5 columns).
#[rustfmt::skip]
static FONT5X7: [[u8; 5]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00], // ' '
    [0x00, 0x00, 0x5F, 0x00, 0x00], // !
    [0x00, 0x07, 0x00, 0x07, 0x00], // "
    [0x14, 0x7F, 0x14, 0x7F, 0x14], // #
    [0x24, 0x2A, 0x7F, 0x2A, 0x12], // $
    [0x23, 0x13, 0x08, 0x64, 0x62], // %
    [0x36, 0x49, 0x55, 0x22, 0x50], // &
    [0x00, 0x05, 0x03, 0x00, 0x00], // '
    [0x00, 0x1C, 0x22, 0x41, 0x00], // (
    [0x00, 0x41, 0x22, 0x1C, 0x00], // )
    [0x14, 0x08, 0x3E, 0x08, 0x14], // *
    [0x08, 0x08, 0x3E, 0x08, 0x08], // +
    [0x00, 0x50, 0x30, 0x00, 0x00], // ,
    [0x08, 0x08, 0x08, 0x08, 0x08], // -
    [0x00, 0x60, 0x60, 0x00, 0x00], // .
    [0x20, 0x10, 0x08, 0x04, 0x02], // /
    [0x3E, 0x51, 0x49, 0x45, 0x3E], // 0
    [0x00, 0x42, 0x7F, 0x40, 0x00], // 1
    [0x42, 0x61, 0x51, 0x49, 0x46], // 2
    [0x21, 0x41, 0x45, 0x4B, 0x31], // 3
    [0x18, 0x14, 0x12, 0x7F, 0x10], // 4
    [0x27, 0x45, 0x45, 0x45, 0x39], // 5
    [0x3C, 0x4A, 0x49, 0x49, 0x30], // 6
    [0x01, 0x71, 0x09, 0x05, 0x03], // 7
    [0x36, 0x49, 0x49, 0x49, 0x36], // 8
    [0x06, 0x49, 0x49, 0x29, 0x1E], // 9
    [0x00, 0x36, 0x36, 0x00, 0x00], // :
    [0x00, 0x56, 0x36, 0x00, 0x00], // ;
    [0x08, 0x14, 0x22, 0x41, 0x00], // <
    [0x14, 0x14, 0x14, 0x14, 0x14], // =
    [0x00, 0x41, 0x22, 0x14, 0x08], // >
    [0x02, 0x01, 0x51, 0x09, 0x06], // ?
    [0x32, 0x49, 0x79, 0x41, 0x3E], // @
    [0x7E, 0x11, 0x11, 0x11, 0x7E], // A
    [0x7F, 0x49, 0x49, 0x49, 0x36], // B
    [0x3E, 0x41, 0x41, 0x41, 0x22], // C
    [0x7F, 0x41, 0x41, 0x22, 0x1C], // D
    [0x7F, 0x49, 0x49, 0x49, 0x41], // E
    [0x7F, 0x09, 0x09, 0x01, 0x01], // F
    [0x3E, 0x41, 0x41, 0x51, 0x32], // G
    [0x7F, 0x08, 0x08, 0x08, 0x7F], // H
    [0x00, 0x41, 0x7F, 0x41, 0x00], // I
    [0x20, 0x40, 0x41, 0x3F, 0x01], // J
    [0x7F, 0x08, 0x14, 0x22, 0x41], // K
    [0x7F, 0x40, 0x40, 0x40, 0x40], // L
    [0x7F, 0x02, 0x04, 0x02, 0x7F], // M
    [0x7F, 0x04, 0x08, 0x10, 0x7F], // N
    [0x3E, 0x41, 0x41, 0x41, 0x3E], // O
    [0x7F, 0x09, 0x09, 0x09, 0x06], // P
    [0x3E, 0x41, 0x51, 0x21, 0x5E], // Q
    [0x7F, 0x09, 0x19, 0x29, 0x46], // R
    [0x46, 0x49, 0x49, 0x49, 0x31], // S
    [0x01, 0x01, 0x7F, 0x01, 0x01], // T
    [0x3F, 0x40, 0x40, 0x40, 0x3F], // U
    [0x1F, 0x20, 0x40, 0x20, 0x1F], // V
    [0x7F, 0x20, 0x18, 0x20, 0x7F], // W
    [0x63, 0x14, 0x08, 0x14, 0x63], // X
    [0x03, 0x04, 0x78, 0x04, 0x03], // Y
    [0x61, 0x51, 0x49, 0x45, 0x43], // Z
    [0x00, 0x7F, 0x41, 0x41, 0x00], // [
    [0x02, 0x04, 0x08, 0x10, 0x20], // backslash
    [0x00, 0x41, 0x41, 0x7F, 0x00], // ]
    [0x04, 0x02, 0x01, 0x02, 0x04], // ^
    [0x40, 0x40, 0x40, 0x40, 0x40], // _
    [0x00, 0x01, 0x02, 0x04, 0x00], // `
    [0x20, 0x54, 0x54, 0x54, 0x78], // a
    [0x7F, 0x48, 0x44, 0x44, 0x38], // b
    [0x38, 0x44, 0x44, 0x44, 0x20], // c
    [0x38, 0x44, 0x44, 0x48, 0x7F], // d
    [0x38, 0x54, 0x54, 0x54, 0x18], // e
    [0x08, 0x7E, 0x09, 0x01, 0x02], // f
    [0x08, 0x14, 0x54, 0x54, 0x3C], // g
    [0x7F, 0x08, 0x04, 0x04, 0x78], // h
    [0x00, 0x44, 0x7D, 0x40, 0x00], // i
    [0x20, 0x40, 0x44, 0x3D, 0x00], // j
    [0x00, 0x7F, 0x10, 0x28, 0x44], // k
    [0x00, 0x41, 0x7F, 0x40, 0x00], // l
    [0x7C, 0x04, 0x18, 0x04, 0x78], // m
    [0x7C, 0x08, 0x04, 0x04, 0x78], // n
    [0x38, 0x44, 0x44, 0x44, 0x38], // o
    [0x7C, 0x14, 0x14, 0x14, 0x08], // p
    [0x08, 0x14, 0x14, 0x18, 0x7C], // q
    [0x7C, 0x08, 0x04, 0x04, 0x08], // r
    [0x48, 0x54, 0x54, 0x54, 0x20], // s
    [0x04, 0x3F, 0x44, 0x40, 0x20], // t
    [0x3C, 0x40, 0x40, 0x20, 0x1C], // u
    [0x1C, 0x20, 0x40, 0x20, 0x1C], // v
    [0x3C, 0x40, 0x30, 0x40, 0x3C], // w
    [0x44, 0x28, 0x10, 0x28, 0x44], // x
    [0x0C, 0x50, 0x50, 0x50, 0x3C], // y
    [0x44, 0x64, 0x54, 0x4C, 0x44], // z
    [0x00, 0x08, 0x36, 0x41, 0x00], // {
    [0x00, 0x00, 0x7F, 0x00, 0x00], // |
    [0x00, 0x41, 0x36, 0x08, 0x00], // }
    [0x02, 0x01, 0x02, 0x04, 0x02], // ~
];

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas_of(w: usize, h: usize) -> Vec<u8> {
        vec![0u8; w * h * 4]
    }

    #[test]
    fn glyph_table_shape() {
        assert_eq!(FONT5X7.len(), 95);
        // 'A' = 0x41: columns 7E 11 11 11 7E.
        assert_eq!(glyph('A'), Some([0x7E, 0x11, 0x11, 0x11, 0x7E]));
        assert_eq!(glyph('0'), Some([0x3E, 0x51, 0x49, 0x45, 0x3E]));
        assert_eq!(glyph(' '), Some([0, 0, 0, 0, 0]));
        assert_eq!(glyph('é'), None);
        assert_eq!(glyph('中'), None);
    }

    #[test]
    fn text_metrics() {
        assert_eq!(text_width("", 1), 0);
        assert_eq!(text_width("A", 1), 5);
        assert_eq!(text_width("AB", 1), 11);
        assert_eq!(text_width("AB", 2), 22);
        assert_eq!(text_height(1), 7);
        assert_eq!(text_height(3), 21);
    }

    #[test]
    fn draw_text_paints_pixels() {
        let mut cv = canvas_of(24, 10);
        draw_text(&mut cv, 24, 10, 0, 0, "A", [255, 0, 0, 255], 1);
        // 'A' column 0 = 0x7E (rows 1..5 set), column 1 = 0x11 (rows 0,4).
        assert_eq!(&cv[0..4], &[0, 0, 0, 0]); // row 0 of col 0 is clear
        let at = |x: usize, y: usize| {
            let i = (y * 24 + x) * 4;
            [cv[i], cv[i + 1], cv[i + 2], cv[i + 3]]
        };
        assert_eq!(at(0, 0), [0, 0, 0, 0]); // row 0 of 'A' col 0 is clear
        assert_eq!(at(0, 1), [255, 0, 0, 255]);
        assert_eq!(at(4, 1), [255, 0, 0, 255]);
        assert_eq!(at(1, 0), [255, 0, 0, 255]); // col 1 row 0 (0x11 bit0)
        assert_eq!(at(1, 4), [255, 0, 0, 255]); // col 1 row 4
        assert_eq!(at(1, 2), [0, 0, 0, 0]);
    }

    #[test]
    fn draw_text_scale_and_clip() {
        let mut cv = canvas_of(12, 16);
        // scale 2: 10x14 px for one char — fits.
        draw_text(&mut cv, 12, 16, 0, 0, "I", [0, 255, 0, 255], 2);
        let at = |x: usize, y: usize| {
            let i = (y * 12 + x) * 4;
            [cv[i], cv[i + 1], cv[i + 2], cv[i + 3]]
        };
        // 'I' = [00 41 7F 41 00]: only column 2 (0x7F, all 7 rows) is solid;
        // at scale 2 that column spans x 4..5, rows span y 0..13.
        assert_eq!(at(4, 12), [0, 255, 0, 255]); // solid column bottom
        assert_eq!(at(0, 0), [0, 0, 0, 0]); // column 0 is empty
        // Out-of-canvas writes must not panic (negative + overflow coords).
        let mut cv2 = canvas_of(8, 8);
        draw_text(&mut cv2, 8, 8, -20, -20, "OVERFLOW", [1, 2, 3, 255], 3);
        draw_text(&mut cv2, 8, 8, 100, 100, "X", [1, 2, 3, 255], 1);
        // Centered draw clamps into the canvas without panic.
        draw_text_centered(&mut cv2, 8, 8, 4.0, 0, "A", [9, 9, 9, 255], 1);
    }

    #[test]
    fn non_ascii_renders_blank() {
        let mut cv = canvas_of(20, 10);
        draw_text(&mut cv, 20, 10, 0, 0, "昆明", [255, 255, 255, 255], 1);
        assert!(cv.iter().all(|&b| b == 0), "CJK must render as blanks");
    }
}
