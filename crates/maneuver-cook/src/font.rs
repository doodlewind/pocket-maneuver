//! Interface glyphs: printable ASCII at a few pixel sizes, packed in rows into
//! one 8-bit coverage image.

use maneuver_pack::{FontHeader, Glyph};

pub fn bake(ttf: &[u8], sizes: &[u32], width: usize, height: usize) -> Result<Vec<u8>, String> {
    let font = fontdue::Font::from_bytes(ttf, fontdue::FontSettings::default()).map_err(|e| e.to_string())?;
    let mut pixels = vec![0u8; width * height];
    let mut glyphs = Vec::new();
    let (mut x, mut y, mut row) = (1usize, 1usize, 0usize);
    for &size in sizes {
        for code in 32u8..127 {
            let (m, bitmap) = font.rasterize(code as char, size as f32);
            if x + m.width + 1 > width {
                x = 1;
                y += row + 1;
                row = 0;
            }
            if y + m.height + 1 > height {
                return Err(format!("the font atlas {width}×{height} is too small for sizes {sizes:?}"));
            }
            for j in 0..m.height {
                pixels[(y + j) * width + x..(y + j) * width + x + m.width].copy_from_slice(&bitmap[j * m.width..(j + 1) * m.width]);
            }
            glyphs.push(Glyph {
                code: code as u16,
                size: size as u16,
                x: x as u16,
                y: y as u16,
                w: m.width as u16,
                h: m.height as u16,
                left: m.xmin as i16,
                top: (m.ymin + m.height as i32) as i16,
                advance: m.advance_width,
            });
            x += m.width + 1;
            row = row.max(m.height);
        }
    }
    let head = FontHeader { width: width as u32, height: height as u32, glyphs: glyphs.len() as u32, pad: 0 };
    let mut out = maneuver_pack::bytes_of(&head).to_vec();
    out.extend_from_slice(maneuver_pack::slice_bytes(&glyphs));
    out.extend_from_slice(&pixels);
    Ok(out)
}
