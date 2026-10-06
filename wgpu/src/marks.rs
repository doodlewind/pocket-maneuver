//! The marks on the world, in the screen's pixels: where each wire would
//! bite, the streaks of speed and the nearest target with its distance
//! (`vita/src/hud.rs` and `draw_marks` of `vita/src/main.rs`). Solid quads and
//! text from the pack's glyph atlas, batched into one draw. Everything else on
//! the screen is the interface's.
//!
//! The PS Vita draws them for 960 × 544. On another screen the places follow
//! the screen and the sizes follow its width.

use maneuver_pack::{self as pack, FontHeader, Glyph, Pack};
use maneuver_sim::math::*;
use maneuver_sim::Sim;

use crate::mat::{self, Mat4};

/// Three reticles, eighteen streaks, the target marker and its distance fit several times over.
pub const MAX_QUADS: usize = 128;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [u8; 4],
}

pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
    [r, g, b, a]
}

pub struct Marks {
    glyphs: Vec<Glyph>,
    sizes: Vec<u16>,
    /// Texture coordinates of the atlas's solid block.
    solid: [f32; 2],
    w: f32,
    h: f32,
    /// The font's coverage, one byte a texel, with a solid block in its last corner.
    pub cover: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// This frame's quads, four vertices each.
    pub verts: Vec<Vertex>,
}

impl Marks {
    pub fn load(p: &Pack) -> Result<Marks, String> {
        let b = p.section(pack::FONT)?;
        let head: FontHeader = pack::read(b, 0).ok_or("font header")?;
        let gsize = core::mem::size_of::<Glyph>();
        let table = core::mem::size_of::<FontHeader>();
        let glyphs: Vec<Glyph> = (0..head.glyphs as usize).filter_map(|i| pack::read(b, table + i * gsize)).collect();
        let (w, h) = (head.width as usize, head.height as usize);
        let mut cover = b.get(table + head.glyphs as usize * gsize..table + head.glyphs as usize * gsize + w * h).ok_or("the font's picture is cut short")?.to_vec();
        for (x, y) in [(w - 1, h - 1), (w - 2, h - 1), (w - 1, h - 2), (w - 2, h - 2)] {
            cover[y * w + x] = 255;
        }
        let mut sizes: Vec<u16> = glyphs.iter().map(|g| g.size).collect();
        sizes.dedup();
        Ok(Marks { glyphs, sizes, solid: [(w as f32 - 1.0) / w as f32, (h as f32 - 1.0) / h as f32], w: w as f32, h: h as f32, cover, width: head.width, height: head.height, verts: Vec::with_capacity(MAX_QUADS * 4) })
    }

    #[allow(clippy::too_many_arguments)]
    fn quad(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, u0: f32, v0: f32, u1: f32, v1: f32, color: [u8; 4]) {
        if self.verts.len() >= MAX_QUADS * 4 {
            return;
        }
        self.verts.extend_from_slice(&[Vertex { pos: [x0, y0], uv: [u0, v0], color }, Vertex { pos: [x1, y0], uv: [u1, v0], color }, Vertex { pos: [x1, y1], uv: [u1, v1], color }, Vertex { pos: [x0, y1], uv: [u0, v1], color }]);
    }

    /// A solid quad through four corners, in order around it.
    fn poly(&mut self, p: [(f32, f32); 4], color: [u8; 4]) {
        if self.verts.len() >= MAX_QUADS * 4 {
            return;
        }
        let uv = self.solid;
        self.verts.extend(p.into_iter().map(|(x, y)| Vertex { pos: [x, y], uv, color }));
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [u8; 4]) {
        let [u, v] = self.solid;
        self.quad(x, y, x + w, y + h, u, v, u, v, color)
    }

    /// A rectangle outline `t` pixels thick.
    fn frame(&mut self, x: f32, y: f32, w: f32, h: f32, t: f32, color: [u8; 4]) {
        self.rect(x, y, w, t, color);
        self.rect(x, y + h - t, w, t, color);
        self.rect(x, y + t, t, h - t * 2.0, color);
        self.rect(x + w - t, y + t, t, h - t * 2.0, color);
    }

    fn glyph(&self, size: u16, code: u8) -> Option<&Glyph> {
        let s = self.sizes.iter().position(|&x| x == size)?;
        self.glyphs.get(s * 95 + (code.clamp(32, 126) - 32) as usize)
    }

    /// Text of the pack's `size`, drawn `k` times as large, centred on `x` with its baseline at `y`.
    fn text(&mut self, size: u16, k: f32, x: f32, y: f32, color: [u8; 4], text: &str) {
        let width: f32 = text.bytes().filter_map(|c| self.glyph(size, c)).map(|g| g.advance * k).sum();
        let pen = (x - width * 0.5).round();
        let shadow = [0, 0, 0, (color[3] as u32 * 150 / 255) as u8];
        for (o, col) in [(1.5 * k, shadow), (0.0, color)] {
            let mut px = pen;
            for c in text.bytes() {
                let Some(gl) = self.glyph(size, c).copied() else { continue };
                if gl.w > 0 {
                    let x0 = px + gl.left as f32 * k + o;
                    let y0 = y - gl.top as f32 * k + o;
                    let (u0, v0) = (gl.x as f32 / self.w, gl.y as f32 / self.h);
                    let (u1, v1) = ((gl.x + gl.w) as f32 / self.w, (gl.y + gl.h) as f32 / self.h);
                    self.quad(x0, y0, x0 + gl.w as f32 * k, y0 + gl.h as f32 * k, u0, v0, u1, v1, col);
                }
                px += gl.advance * k;
            }
        }
    }

    /// The marks of a frame on a screen `w` by `h` pixels.
    pub fn draw(&mut self, sim: &Sim, vp: &Mat4, w: f32, h: f32) {
        let k = w / 960.0;
        let (cx, cy) = (w * 0.5, h * 0.5);
        // Where each wire would bite, and where an aimed pair would.
        for (i, r) in sim.reticle.iter().enumerate() {
            if !r.valid {
                continue;
            }
            if let Some((x, y)) = mat::project(vp, r.point, w, h) {
                if (0.0..w).contains(&x) && (0.0..h).contains(&y) {
                    if i < 2 {
                        self.frame(x - 6.0 * k, y - 6.0 * k, 12.0 * k, 12.0 * k, (2.0 * k).max(1.0), rgba(255, 255, 255, 220));
                    } else {
                        self.frame(x - 4.0 * k, y - 4.0 * k, 8.0 * k, 8.0 * k, (2.0 * k).max(1.0), rgba(255, 179, 71, 230));
                    }
                }
            }
        }
        // Streaks from the rim toward the centre at speed.
        let fast = smoothstep(24.0, 60.0, sim.speed());
        if fast > 0.0 {
            for i in 0..18u32 {
                let seed = i.wrapping_mul(2654435761).wrapping_add((sim.tick / 3).wrapping_mul(40503));
                let a = (seed % 6283) as f32 / 1000.0;
                let r0 = (300.0 + ((seed >> 8) % 160) as f32) * k;
                let len = (40.0 + ((seed >> 16) % 90) as f32) * (0.5 + fast) * k;
                let (c, s) = (cos(a), sin(a) * 0.62 * (h / w) / (544.0 / 960.0));
                let (x0, y0) = (cx + c * r0, cy + s * r0);
                let (x1, y1) = (cx + c * (r0 + len), cy + s * (r0 + len));
                let (nx, ny) = (-s * 1.2 * k, c * 1.2 * k);
                self.poly([(x0, y0), (x1 - nx, y1 - ny), (x1 + nx, y1 + ny), (x0, y0)], rgba(255, 255, 255, (fast * 70.0) as u8));
            }
        }
        // The nearest standing target: a marker on it, or at the rim of the screen toward it.
        let mut best: Option<(f32, V3)> = None;
        for d in sim.dummies.iter().filter(|d| d.alive) {
            let dist = (d.nape - sim.p.pos).len();
            if best.is_none_or(|b| dist < b.0) {
                best = Some((dist, d.nape));
            }
        }
        if let Some((dist, nape)) = best {
            let red = rgba(255, 96, 72, 235);
            let edge = 24.0 * k;
            let on = mat::project(vp, nape, w, h).filter(|(x, y)| (edge..w - edge).contains(x) && (edge..h - edge).contains(y));
            let (x, y) = match on {
                Some(p) => p,
                None => {
                    // Off screen: toward it, from the centre, clamped to an ellipse inside the frame.
                    let to = nape - sim.cam.pos;
                    let right = sim.cam.look.cross(V3::UP).norm_or(v3(1.0, 0.0, 0.0));
                    let up = right.cross(sim.cam.look);
                    let (dx, dy) = (to.dot(right), to.dot(up));
                    let l = sqrt(dx * dx + dy * dy).max(1e-3);
                    (cx + dx / l * (cx - 60.0 * k), cy - dy / l * (cy - 42.0 * k))
                }
            };
            let r = 9.0 * k;
            self.poly([(x, y - r), (x + r, y), (x, y + r), (x - r, y)], red);
            self.text(18, k, x, y + 28.0 * k, red, &format!("{:.0} m", dist));
        }
    }
}
