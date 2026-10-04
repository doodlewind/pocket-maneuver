//! Clipping for a rasterizer that has none (the PSP's GE).
//!
//! The GE clips a triangle against the near plane and nothing else. A vertex
//! that lands outside its 4096 × 4096 pixel coordinate space (the guard band,
//! centred on the screen) makes it drop the whole triangle. A large triangle
//! near the eye reaches that far with most of it in view, and the ground
//! disappears under the camera.
//!
//! The world compiler sorts each mesh's large triangles to the end of its
//! index list, largest first, with a distance for each: farther than that
//! from the eye, a triangle with that longest edge cannot span from inside the
//! view to outside the guard band. Closer, `classify` decides from the
//! triangle's clip coordinates whether the GE may draw it; if not, `clip`
//! cuts it against a frustum twice the view's size, well inside the guard
//! band, and the device draws the pieces as float vertices.

use maneuver_sim::math::V3;

use crate::mat::Mat4;

/// A float vertex in world space, in the GE's component order.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ClipVertex {
    pub uv: [f32; 2],
    pub color: [u8; 4],
    pub pos: [f32; 3],
}

/// The view a frame clips against.
pub struct Guard {
    /// The `x`, `y` and `w` rows of projection × view: inside the view, `|x| <= w` and `|y| <= w`.
    rows: [[f32; 4]; 3],
    near: f32,
    /// How many screen half-widths and half-heights the guard band reaches, with a margin.
    gx: f32,
    gy: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    /// Every vertex is in front of the near plane and inside the guard band.
    Safe,
    /// Outside the view.
    Culled,
    /// In view with a vertex the rasterizer cannot take.
    Clip,
}

/// The frustum the pieces are cut to, in units of the view's.
const CUT: f32 = 2.0;
/// Most vertices a triangle has after five planes.
pub const MAX_POLY: usize = 8;
/// Vertices `clip` writes at most: a fan of `MAX_POLY` corners.
pub const MAX_OUT: usize = (MAX_POLY - 2) * 3;

#[derive(Clone, Copy)]
struct P {
    v: ClipVertex,
    /// Clip x, y, w.
    c: [f32; 3],
}

impl Guard {
    /// `vp` is projection × view; `width` and `height` are the screen's pixels and
    /// `band` the guard band's half-size in pixels (2048 on the PSP).
    pub fn new(vp: &Mat4, near: f32, width: f32, height: f32, band: f32) -> Guard {
        let row = |i: usize| [vp[i * 4], vp[i * 4 + 1], vp[i * 4 + 2], vp[i * 4 + 3]];
        Guard { rows: [row(0), row(1), row(3)], near: near * 1.02, gx: band / (width * 0.5) * 0.9, gy: band / (height * 0.5) * 0.9 }
    }

    #[inline]
    pub fn to_clip(&self, p: V3) -> [f32; 3] {
        let r = &self.rows;
        [r[0][0] * p.x + r[0][1] * p.y + r[0][2] * p.z + r[0][3], r[1][0] * p.x + r[1][1] * p.y + r[1][2] * p.z + r[1][3], r[2][0] * p.x + r[2][1] * p.y + r[2][2] * p.z + r[2][3]]
    }

    pub fn classify(&self, c: &[[f32; 3]; 3]) -> Verdict {
        let mut safe = true;
        // Bits: left, right, bottom, top, near.
        let mut all = 0b11111u32;
        for [x, y, w] in c {
            let mut out = 0;
            if *x < -*w {
                out |= 1;
            }
            if *x > *w {
                out |= 2;
            }
            if *y < -*w {
                out |= 4;
            }
            if *y > *w {
                out |= 8;
            }
            if *w < self.near {
                out |= 16;
            }
            all &= out;
            if *w < self.near || *x < -*w * self.gx || *x > *w * self.gx || *y < -*w * self.gy || *y > *w * self.gy {
                safe = false;
            }
        }
        if all != 0 {
            Verdict::Culled
        } else if safe {
            Verdict::Safe
        } else {
            Verdict::Clip
        }
    }

    /// Cuts a triangle to the near plane and the enlarged frustum and writes the
    /// pieces as a triangle list. Returns the number of vertices written.
    pub fn clip(&self, tri: &[ClipVertex; 3], c: &[[f32; 3]; 3], out: &mut [ClipVertex]) -> usize {
        let mut a = [P { v: tri[0], c: c[0] }; MAX_POLY];
        let mut b = a;
        let mut n = 3;
        for i in 0..3 {
            a[i] = P { v: tri[i], c: c[i] };
        }
        let near = self.near;
        let planes: [&dyn Fn(&[f32; 3]) -> f32; 5] = [&|c| c[2] - near, &|c| c[0] + c[2] * CUT, &|c| c[2] * CUT - c[0], &|c| c[1] + c[2] * CUT, &|c| c[2] * CUT - c[1]];
        for plane in planes {
            let mut m = 0;
            for i in 0..n {
                let (p, q) = (&a[i], &a[(i + 1) % n]);
                let (dp, dq) = (plane(&p.c), plane(&q.c));
                if dp >= 0.0 {
                    if m < MAX_POLY {
                        b[m] = *p;
                        m += 1;
                    }
                }
                if (dp >= 0.0) != (dq >= 0.0) && m < MAX_POLY {
                    let t = dp / (dp - dq);
                    let l = |x: f32, y: f32| x + (y - x) * t;
                    let mut v = ClipVertex { uv: [l(p.v.uv[0], q.v.uv[0]), l(p.v.uv[1], q.v.uv[1])], color: [0; 4], pos: [l(p.v.pos[0], q.v.pos[0]), l(p.v.pos[1], q.v.pos[1]), l(p.v.pos[2], q.v.pos[2])] };
                    for k in 0..4 {
                        v.color[k] = (l(p.v.color[k] as f32, q.v.color[k] as f32) + 0.5) as u8;
                    }
                    b[m] = P { v, c: [l(p.c[0], q.c[0]), l(p.c[1], q.c[1]), l(p.c[2], q.c[2])] };
                    m += 1;
                }
            }
            core::mem::swap(&mut a, &mut b);
            n = m;
            if n < 3 {
                return 0;
            }
        }
        let mut k = 0;
        for i in 1..n - 1 {
            if k + 3 > out.len() {
                break;
            }
            out[k] = a[0].v;
            out[k + 1] = a[i].v;
            out[k + 2] = a[i + 1].v;
            k += 3;
        }
        k
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mat;
    use maneuver_sim::math::v3;

    fn guard() -> Guard {
        let vp = mat::mul(&mat::perspective(62.0, 480.0 / 272.0, 0.4, 2000.0), &mat::view(v3(0.0, 1.6, 0.0), v3(0.0, 0.0, -1.0), 0.0));
        Guard::new(&vp, 0.4, 480.0, 272.0, 2048.0)
    }

    fn tri(g: &Guard, p: [V3; 3]) -> ([ClipVertex; 3], [[f32; 3]; 3]) {
        (p.map(|p| ClipVertex { uv: [p.x, p.z], color: [255; 4], pos: [p.x, p.y, p.z] }), p.map(|p| g.to_clip(p)))
    }

    #[test]
    fn a_small_triangle_ahead_is_safe() {
        let g = guard();
        let (_, c) = tri(&g, [v3(-1.0, 0.0, -10.0), v3(1.0, 0.0, -10.0), v3(0.0, 2.0, -10.0)]);
        assert_eq!(g.classify(&c), Verdict::Safe);
    }

    #[test]
    fn a_triangle_behind_is_culled() {
        let g = guard();
        let (_, c) = tri(&g, [v3(-1.0, 0.0, 10.0), v3(1.0, 0.0, 10.0), v3(0.0, 2.0, 10.0)]);
        assert_eq!(g.classify(&c), Verdict::Culled);
    }

    #[test]
    fn the_ground_under_the_eye_is_clipped_into_the_guard_band() {
        let g = guard();
        // A 64 m ground quad's triangle with the eye standing on it.
        let (v, c) = tri(&g, [v3(-32.0, 0.0, 32.0), v3(32.0, 0.0, 32.0), v3(-32.0, 0.0, -32.0)]);
        assert_eq!(g.classify(&c), Verdict::Clip);
        let mut out = [ClipVertex::default(); MAX_OUT];
        let n = g.clip(&v, &c, &mut out);
        assert!(n >= 3 && n % 3 == 0);
        for p in &out[..n] {
            let [x, y, w] = g.to_clip(v3(p.pos[0], p.pos[1], p.pos[2]));
            assert!(w >= 0.4, "in front of the near plane: {w}");
            assert!(x.abs() <= w * 2.01 && y.abs() <= w * 2.01, "inside the cut frustum: {x} {y} {w}");
            // The attributes follow the position: uv was set to (x, z).
            assert!((p.uv[0] - p.pos[0]).abs() < 1e-3 && (p.uv[1] - p.pos[2]).abs() < 1e-3);
        }
    }
}
