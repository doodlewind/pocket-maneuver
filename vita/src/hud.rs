//! Interface drawing in display pixels: solid rectangles and text from the
//! pack's glyph atlas, batched into one draw.

use maneuver_pack::{self as pack, FontHeader, Glyph, Pack};
use pocket_vita_gxm::mem::Arena;
use pocket_vita_gxm::texture::{Format, Texture, Uploader, Wrap};
use vita2d_sys as g;

use crate::gpu::{self, Program};

pub const MAX_QUADS: usize = 1024;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [u8; 4],
}

pub struct Hud {
    pub font: Texture,
    glyphs: Vec<Glyph>,
    sizes: Vec<u16>,
    /// Texture coordinates of the atlas's solid block.
    solid: [f32; 2],
    w: f32,
    h: f32,
    verts: *mut Vertex,
    quads: usize,
}

pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
    [r, g, b, a]
}

impl Hud {
    /// # Safety
    /// GXM is initialized; `vram` outlives the texture.
    pub unsafe fn load(p: &Pack, vram: &mut Arena) -> Result<Hud, String> {
        let b = p.section(pack::FONT)?;
        let head: FontHeader = pack::read(b, 0).ok_or("font header")?;
        let gsize = core::mem::size_of::<Glyph>();
        let table = core::mem::size_of::<FontHeader>();
        let glyphs: Vec<Glyph> = (0..head.glyphs as usize).filter_map(|i| pack::read(b, table + i * gsize)).collect();
        let cover = &b[table + head.glyphs as usize * gsize..];
        let (w, h) = (head.width as usize, head.height as usize);
        // White with coverage in alpha; the last 2 × 2 texels are solid.
        let mut px = vec![255u8; w * h * 4];
        for i in 0..w * h {
            px[i * 4 + 3] = cover[i];
        }
        for (x, y) in [(w - 1, h - 1), (w - 2, h - 1), (w - 1, h - 2), (w - 2, h - 2)] {
            px[(y * w + x) * 4 + 3] = 255;
        }
        let mut up = Uploader::new(2 * 1024 * 1024)?;
        let mut font = up.texture(vram, Format::Rgba8, head.width, head.height, 1, &px)?;
        up.flush();
        up.free();
        font.set_wrap(Wrap::Clamp, Wrap::Clamp);
        font.set_filter(true, false);
        let mut sizes: Vec<u16> = glyphs.iter().map(|g| g.size).collect();
        sizes.dedup();
        Ok(Hud { font, glyphs, sizes, solid: [(w as f32 - 1.0) / w as f32, (h as f32 - 1.0) / h as f32], w: w as f32, h: h as f32, verts: core::ptr::null_mut(), quads: 0 })
    }

    /// Starts a frame's batch in `verts`, room for `MAX_QUADS` quads.
    pub fn begin(&mut self, verts: *mut u8) {
        self.verts = verts.cast();
        self.quads = 0;
    }

    pub const VERTEX_BYTES: usize = MAX_QUADS * 4 * core::mem::size_of::<Vertex>();

    unsafe fn quad(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, u0: f32, v0: f32, u1: f32, v1: f32, color: [u8; 4]) {
        if self.quads >= MAX_QUADS || self.verts.is_null() {
            return;
        }
        let v = self.verts.add(self.quads * 4);
        *v = Vertex { pos: [x0, y0], uv: [u0, v0], color };
        *v.add(1) = Vertex { pos: [x1, y0], uv: [u1, v0], color };
        *v.add(2) = Vertex { pos: [x1, y1], uv: [u1, v1], color };
        *v.add(3) = Vertex { pos: [x0, y1], uv: [u0, v1], color };
        self.quads += 1;
    }

    /// A solid quad through four corners, in order around it.
    pub fn poly(&mut self, p: [(f32, f32); 4], color: [u8; 4]) {
        if self.quads >= MAX_QUADS || self.verts.is_null() {
            return;
        }
        unsafe {
            let v = self.verts.add(self.quads * 4);
            for (i, (x, y)) in p.into_iter().enumerate() {
                *v.add(i) = Vertex { pos: [x, y], uv: self.solid, color };
            }
        }
        self.quads += 1;
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [u8; 4]) {
        let [u, v] = self.solid;
        unsafe { self.quad(x, y, x + w, y + h, u, v, u, v, color) }
    }

    /// A rectangle outline `t` pixels thick.
    pub fn frame(&mut self, x: f32, y: f32, w: f32, h: f32, t: f32, color: [u8; 4]) {
        self.rect(x, y, w, t, color);
        self.rect(x, y + h - t, w, t, color);
        self.rect(x, y + t, t, h - t * 2.0, color);
        self.rect(x + w - t, y + t, t, h - t * 2.0, color);
    }

    fn glyph(&self, size: u16, code: u8) -> Option<&Glyph> {
        let s = self.sizes.iter().position(|&x| x == size)?;
        self.glyphs.get(s * 95 + (code.max(32).min(126) - 32) as usize)
    }

    pub fn width(&self, size: u16, text: &str) -> f32 {
        text.bytes().filter_map(|c| self.glyph(size, c)).map(|g| g.advance).sum()
    }

    /// Text with its baseline at `y`. `align`: 0 left, 0.5 centre, 1 right of `x`.
    pub fn text(&mut self, size: u16, x: f32, y: f32, align: f32, color: [u8; 4], text: &str) {
        let mut pen = (x - self.width(size, text) * align).round();
        let shadow = [0, 0, 0, (color[3] as u32 * 150 / 255) as u8];
        for pass in 0..2 {
            let mut px = pen;
            for c in text.bytes() {
                let Some(gl) = self.glyph(size, c).copied() else { continue };
                if gl.w > 0 {
                    let (o, col) = if pass == 0 { (1.5, shadow) } else { (0.0, color) };
                    let x0 = px + gl.left as f32 + o;
                    let y0 = y - gl.top as f32 + o;
                    let (u0, v0) = (gl.x as f32 / self.w, gl.y as f32 / self.h);
                    let (u1, v1) = ((gl.x + gl.w) as f32 / self.w, (gl.y + gl.h) as f32 / self.h);
                    unsafe { self.quad(x0, y0, x0 + gl.w as f32, y0 + gl.h as f32, u0, v0, u1, v1, col) }
                }
                px += gl.advance;
            }
            if pass == 1 {
                pen = px;
            }
        }
        let _ = pen;
    }

    /// Draws the batch. `ib` is the shared quad index buffer.
    ///
    /// # Safety
    /// Inside a scene on `ctx`; the buffers stay untouched until the frame's GPU work completes.
    pub unsafe fn flush(&mut self, ctx: *mut g::SceGxmContext, prog: &Program, ib: *const u16) {
        if self.quads == 0 {
            return;
        }
        prog.bind(ctx, true);
        gpu::state_overlay(ctx, false);
        g::sceGxmSetFragmentTexture(ctx, 0, &self.font.gxm);
        gpu::draw(ctx, self.verts.cast(), ib, (self.quads * 6) as u32);
        self.quads = 0;
    }
}
