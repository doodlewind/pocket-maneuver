//! Interface drawing in screen pixels: solid quads and text from the pack's
//! glyph atlas, batched as four vertices per quad.

use alloc::vec::Vec;
use maneuver_pack::{self as pack, FontHeader, Glyph};

/// Texture coordinates, colour, position: the order the PSP's GE reads a vertex in.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct HudVertex {
    pub uv: [f32; 2],
    pub color: [u8; 4],
    pub pos: [f32; 3],
}

pub struct Font {
    glyphs: Vec<Glyph>,
    /// The pixel sizes in the atlas, smallest first.
    pub sizes: Vec<u16>,
    pub width: u32,
    pub height: u32,
    /// `tex_format` of the atlas texture.
    pub format: u32,
}

impl Font {
    /// The glyph table of a `FONT` section and the offset of its texture bytes.
    pub fn parse(b: &[u8]) -> Result<(Font, usize), &'static str> {
        let head: FontHeader = pack::read(b, 0).ok_or("font header")?;
        let gsize = core::mem::size_of::<Glyph>();
        let table = core::mem::size_of::<FontHeader>();
        let glyphs: Vec<Glyph> = (0..head.glyphs as usize).filter_map(|i| pack::read(b, table + i * gsize)).collect();
        if glyphs.len() != head.glyphs as usize || glyphs.is_empty() {
            return Err("font glyph table");
        }
        let mut sizes: Vec<u16> = glyphs.iter().map(|g| g.size).collect();
        sizes.dedup();
        let texture = (table + glyphs.len() * gsize + 15) & !15;
        Ok((Font { glyphs, sizes, width: head.width, height: head.height, format: head.pad }, texture))
    }

    fn glyph(&self, size: u16, code: u8) -> Option<&Glyph> {
        let s = self.sizes.iter().position(|&x| x == size)?;
        self.glyphs.get(s * 95 + (code.max(32).min(126) - 32) as usize)
    }

    pub fn width(&self, size: u16, text: &str) -> f32 {
        text.bytes().filter_map(|c| self.glyph(size, c)).map(|g| g.advance).sum()
    }
}

pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
    [r, g, b, a]
}

/// One frame's batch. Texture coordinates leave as `texel × uv_scale + uv_offset`,
/// so a device takes texels or normalized coordinates, either way up.
pub struct Hud<'a> {
    pub font: &'a Font,
    verts: &'a mut [HudVertex],
    pub quads: usize,
    pub uv_scale: [f32; 2],
    pub uv_offset: [f32; 2],
}

impl<'a> Hud<'a> {
    pub fn new(font: &'a Font, verts: &'a mut [HudVertex], uv_scale: [f32; 2], uv_offset: [f32; 2]) -> Hud<'a> {
        Hud { font, verts, quads: 0, uv_scale, uv_offset }
    }

    fn uv(&self, x: f32, y: f32) -> [f32; 2] {
        [x * self.uv_scale[0] + self.uv_offset[0], y * self.uv_scale[1] + self.uv_offset[1]]
    }

    /// The centre of the atlas's solid block.
    fn solid(&self) -> [f32; 2] {
        self.uv(self.font.width as f32 - 1.0, self.font.height as f32 - 1.0)
    }

    #[allow(clippy::too_many_arguments)]
    fn quad(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, a: [f32; 2], b: [f32; 2], color: [u8; 4]) {
        if (self.quads + 1) * 4 > self.verts.len() {
            return;
        }
        let v = &mut self.verts[self.quads * 4..self.quads * 4 + 4];
        v[0] = HudVertex { pos: [x0, y0, 0.0], uv: [a[0], a[1]], color };
        v[1] = HudVertex { pos: [x1, y0, 0.0], uv: [b[0], a[1]], color };
        v[2] = HudVertex { pos: [x1, y1, 0.0], uv: [b[0], b[1]], color };
        v[3] = HudVertex { pos: [x0, y1, 0.0], uv: [a[0], b[1]], color };
        self.quads += 1;
    }

    /// A solid quad through four corners, in order around it.
    pub fn poly(&mut self, p: [(f32, f32); 4], color: [u8; 4]) {
        if (self.quads + 1) * 4 > self.verts.len() {
            return;
        }
        let uv = self.solid();
        for (i, (x, y)) in p.into_iter().enumerate() {
            self.verts[self.quads * 4 + i] = HudVertex { pos: [x, y, 0.0], uv, color };
        }
        self.quads += 1;
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [u8; 4]) {
        let uv = self.solid();
        self.quad(x, y, x + w, y + h, uv, uv, color);
    }

    /// A rectangle outline `t` pixels thick.
    pub fn frame(&mut self, x: f32, y: f32, w: f32, h: f32, t: f32, color: [u8; 4]) {
        self.rect(x, y, w, t, color);
        self.rect(x, y + h - t, w, t, color);
        self.rect(x, y + t, t, h - t * 2.0, color);
        self.rect(x + w - t, y + t, t, h - t * 2.0, color);
    }

    /// Text with its baseline at `y`. `align`: 0 left, 0.5 centre, 1 right of `x`.
    pub fn text(&mut self, size: u16, x: f32, y: f32, align: f32, color: [u8; 4], text: &str) {
        let pen = libm::roundf(x - self.font.width(size, text) * align);
        let shadow = [0, 0, 0, (color[3] as u32 * 150 / 255) as u8];
        for pass in 0..2 {
            let mut px = pen;
            for c in text.bytes() {
                let Some(gl) = self.font.glyph(size, c).copied() else { continue };
                if gl.w > 0 {
                    let (o, col) = if pass == 0 { (1.0, shadow) } else { (0.0, color) };
                    let x0 = px + gl.left as f32 + o;
                    let y0 = y - gl.top as f32 + o;
                    let a = self.uv(gl.x as f32, gl.y as f32);
                    let b = self.uv((gl.x + gl.w) as f32, (gl.y + gl.h) as f32);
                    self.quad(x0, y0, x0 + gl.w as f32, y0 + gl.h as f32, a, b, col);
                }
                px += gl.advance;
            }
        }
    }
}
