//! PNG encoding helpers (output side; COG *input* uses codecs in `crate::cog::decode`).

use png::{BitDepth, ColorType, Encoder};

/// Encode an RGBA canvas (`width * height * 4`) as PNG bytes.
pub fn encode_rgba(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = Encoder::new(&mut out, width, height);
        enc.set_color(ColorType::Rgba);
        enc.set_depth(BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| format!("png header: {e}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| format!("png data: {e}"))?;
    }
    Ok(out)
}

/// Composite RGBA over a background color (for WMS TRANSPARENT=FALSE).
pub fn composite_over_bg(rgba: &mut [u8], bg: [u8; 3]) {
    for p in rgba.chunks_exact_mut(4) {
        let a = p[3] as f32 / 255.0;
        if a >= 1.0 {
            continue;
        }
        for ch in 0..3 {
            p[ch] = (p[ch] as f32 * a + bg[ch] as f32 * (1.0 - a)).round() as u8;
        }
        p[3] = 255;
    }
}
