//! Shared alpha-blending primitive for the rasterizers (vector + font + legend).

/// Blend `color` onto the RGBA canvas pixel at (x, y); out-of-canvas writes
/// are silently dropped. Fully opaque colors overwrite; translucent colors
/// composite over the existing pixel (source-over).
pub fn blend_px(canvas: &mut [u8], x: i64, y: i64, w: usize, h: usize, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
        return;
    }
    let i = (y as usize * w + x as usize) * 4;
    let a = color[3] as f32 / 255.0;
    if a >= 1.0 {
        canvas[i..i + 4].copy_from_slice(&color);
    } else if a > 0.0 {
        for ch in 0..3 {
            canvas[i + ch] =
                (color[ch] as f32 * a + canvas[i + ch] as f32 * (1.0 - a)).round() as u8;
        }
        let na = canvas[i + 3] as f32 + color[3] as f32 * (1.0 - canvas[i + 3] as f32 / 255.0);
        canvas[i + 3] = na.round().clamp(0.0, 255.0) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_overwrites() {
        let mut cv = vec![10u8, 20, 30, 255];
        blend_px(&mut cv, 0, 0, 1, 1, [1, 2, 3, 255]);
        assert_eq!(cv, vec![1, 2, 3, 255]);
    }

    #[test]
    fn translucent_composites() {
        let mut cv = vec![0u8, 0, 0, 255];
        blend_px(&mut cv, 0, 0, 1, 1, [255, 255, 255, 128]);
        // ~50% white over black.
        assert!((cv[0] as i32 - 128).abs() <= 1, "r={}", cv[0]);
    }

    #[test]
    fn transparent_noop_and_out_of_canvas() {
        let mut cv = vec![1u8, 2, 3, 4];
        blend_px(&mut cv, 0, 0, 1, 1, [9, 9, 9, 0]);
        assert_eq!(cv, vec![1, 2, 3, 4]);
        blend_px(&mut cv, -1, 0, 1, 1, [9, 9, 9, 255]);
        blend_px(&mut cv, 0, 5, 1, 1, [9, 9, 9, 255]);
        assert_eq!(cv, vec![1, 2, 3, 4]);
    }
}
