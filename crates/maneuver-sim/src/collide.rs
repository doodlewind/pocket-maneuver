//! Static collision world: a triangle soup indexed by a uniform XZ grid.
//!
//! Triangles are one-sided for contacts (the front is the side the normal
//! points to) and two-sided for rays. The world compiler emits only exterior
//! faces, so the front of every triangle faces open air.

use alloc::vec;
use alloc::vec::Vec;

use crate::math::*;

/// Surface kinds, shared with the world generator (`web/src/world/kinds.ts`).
pub mod kind {
    pub const GROUND: u8 = 0;
    pub const WALL: u8 = 1;
    pub const ROOF: u8 = 2;
    pub const STONE: u8 = 3;
    pub const WOOD: u8 = 4;
    pub const WATER: u8 = 5;
    pub const NOHOOK: u8 = 6;
    pub const COUNT: u8 = 7;
}

/// Kind masks for ray queries.
pub mod mask {
    pub const ALL: u32 = 0xffff_ffff;
    /// Surfaces an automatic anchor search accepts.
    pub const ANCHOR: u32 = (1 << super::kind::WALL) | (1 << super::kind::ROOF) | (1 << super::kind::STONE) | (1 << super::kind::WOOD);
    /// Surfaces an aimed hook accepts.
    pub const AIMED: u32 = ANCHOR | (1 << super::kind::GROUND);
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Tri {
    pub v0: V3,
    pub e1: V3,
    pub e2: V3,
    pub n: V3,
    pub kind: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub t: f32,
    pub tri: u32,
    /// The triangle's normal (its front side), not flipped toward the ray.
    pub n: V3,
    pub kind: u8,
    /// The ray came from the front side.
    pub front: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub tri: u32,
    /// Push-out direction.
    pub n: V3,
    pub depth: f32,
    pub kind: u8,
}

pub struct World {
    pub tris: Vec<Tri>,
    cell: f32,
    inv_cell: f32,
    min_x: f32,
    min_z: f32,
    nx: i32,
    nz: i32,
    start: Vec<u32>,
    items: Vec<u32>,
}

const CELL: f32 = 16.0;
/// A contact still counts when the centre is this far behind the plane.
const BACK: f32 = 0.3;

impl World {
    pub fn empty() -> World {
        World { tris: Vec::new(), cell: CELL, inv_cell: 1.0 / CELL, min_x: 0.0, min_z: 0.0, nx: 1, nz: 1, start: vec![0, 0], items: Vec::new() }
    }

    /// The grid as built: triangles, cell starts, cell items, then `min_x`, `min_z`, `nx`, `nz`.
    pub fn raw(&self) -> (&[Tri], &[u32], &[u32], f32, f32, i32, i32) {
        (&self.tris, &self.start, &self.items, self.min_x, self.min_z, self.nx, self.nz)
    }

    /// A world from the parts `raw` returns, checked for consistency. A device with
    /// little memory loads the grid a compiler built instead of building it.
    pub fn from_raw(tris: Vec<Tri>, start: Vec<u32>, items: Vec<u32>, min_x: f32, min_z: f32, nx: i32, nz: i32) -> Result<World, &'static str> {
        if nx < 1 || nz < 1 || start.len() != (nx as usize) * (nz as usize) + 1 {
            return Err("collision grid has the wrong number of cells");
        }
        if start[0] != 0 || start.windows(2).any(|w| w[0] > w[1]) || start[start.len() - 1] as usize != items.len() {
            return Err("collision grid cell ranges are inconsistent");
        }
        if items.iter().any(|&i| i as usize >= tris.len()) {
            return Err("collision grid item out of range");
        }
        Ok(World { tris, cell: CELL, inv_cell: 1.0 / CELL, min_x, min_z, nx, nz, start, items })
    }

    /// Builds the grid over `verts` (xyz) and `idx` (three per triangle).
    pub fn build(verts: &[f32], idx: &[u32], kinds: &[u8]) -> World {
        let n = idx.len() / 3;
        let mut tris = Vec::with_capacity(n);
        let (mut min_x, mut min_z, mut max_x, mut max_z) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let at = |i: u32| {
            let o = i as usize * 3;
            v3(verts[o], verts[o + 1], verts[o + 2])
        };
        for t in 0..n {
            let (a, b, c) = (at(idx[t * 3]), at(idx[t * 3 + 1]), at(idx[t * 3 + 2]));
            let e1 = b - a;
            let e2 = c - a;
            let nn = e1.cross(e2);
            if nn.len2() < 1e-12 {
                continue;
            }
            for p in [a, b, c] {
                min_x = min(min_x, p.x);
                max_x = max(max_x, p.x);
                min_z = min(min_z, p.z);
                max_z = max(max_z, p.z);
            }
            tris.push(Tri { v0: a, e1, e2, n: nn.norm(), kind: kinds[t] });
        }
        if tris.is_empty() {
            return World::empty();
        }
        min_x -= 1.0;
        min_z -= 1.0;
        let nx = (floor((max_x + 1.0 - min_x) / CELL) as i32 + 1).max(1);
        let nz = (floor((max_z + 1.0 - min_z) / CELL) as i32 + 1).max(1);
        let inv = 1.0 / CELL;
        let range = |t: &Tri| {
            let (b, c) = (t.v0 + t.e1, t.v0 + t.e2);
            let x0 = min(t.v0.x, min(b.x, c.x));
            let x1 = max(t.v0.x, max(b.x, c.x));
            let z0 = min(t.v0.z, min(b.z, c.z));
            let z1 = max(t.v0.z, max(b.z, c.z));
            let ix0 = (floor((x0 - min_x) * inv) as i32).clamp(0, nx - 1);
            let ix1 = (floor((x1 - min_x) * inv) as i32).clamp(0, nx - 1);
            let iz0 = (floor((z0 - min_z) * inv) as i32).clamp(0, nz - 1);
            let iz1 = (floor((z1 - min_z) * inv) as i32).clamp(0, nz - 1);
            (ix0, ix1, iz0, iz1)
        };
        let cells = (nx * nz) as usize;
        let mut start = vec![0u32; cells + 1];
        for t in &tris {
            let (ix0, ix1, iz0, iz1) = range(t);
            for iz in iz0..=iz1 {
                for ix in ix0..=ix1 {
                    start[(iz * nx + ix) as usize + 1] += 1;
                }
            }
        }
        for i in 0..cells {
            start[i + 1] += start[i];
        }
        let mut fill = start.clone();
        let mut items = vec![0u32; start[cells] as usize];
        for (ti, t) in tris.iter().enumerate() {
            let (ix0, ix1, iz0, iz1) = range(t);
            for iz in iz0..=iz1 {
                for ix in ix0..=ix1 {
                    let c = (iz * nx + ix) as usize;
                    items[fill[c] as usize] = ti as u32;
                    fill[c] += 1;
                }
            }
        }
        World { tris, cell: CELL, inv_cell: inv, min_x, min_z, nx, nz, start, items }
    }

    #[inline]
    fn cell_items(&self, ix: i32, iz: i32) -> &[u32] {
        let c = (iz * self.nx + ix) as usize;
        &self.items[self.start[c] as usize..self.start[c + 1] as usize]
    }

    /// Nearest hit along `o + d·t`, `t` in (0, tmax]. `d` must be unit length.
    pub fn raycast(&self, o: V3, d: V3, tmax: f32, kinds: u32) -> Option<Hit> {
        if self.tris.is_empty() {
            return None;
        }
        // Clip the XZ segment to the grid.
        let (gx0, gz0) = (self.min_x, self.min_z);
        let (gx1, gz1) = (gx0 + self.nx as f32 * self.cell, gz0 + self.nz as f32 * self.cell);
        let mut t0 = 0.0f32;
        let mut t1 = tmax;
        for (p, dir, lo, hi) in [(o.x, d.x, gx0, gx1), (o.z, d.z, gz0, gz1)] {
            if abs(dir) < 1e-9 {
                if p < lo || p >= hi {
                    return None;
                }
            } else {
                let inv = 1.0 / dir;
                let (mut a, mut b) = ((lo - p) * inv, (hi - p) * inv);
                if a > b {
                    core::mem::swap(&mut a, &mut b);
                }
                t0 = max(t0, a);
                t1 = min(t1, b);
                if t0 > t1 {
                    return None;
                }
            }
        }
        let start = o + d * (t0 + 1e-4);
        let mut ix = (floor((start.x - gx0) * self.inv_cell) as i32).clamp(0, self.nx - 1);
        let mut iz = (floor((start.z - gz0) * self.inv_cell) as i32).clamp(0, self.nz - 1);
        let step_x = if d.x > 0.0 { 1 } else { -1 };
        let step_z = if d.z > 0.0 { 1 } else { -1 };
        let next = |i: i32, step: i32, lo: f32| lo + (i + if step > 0 { 1 } else { 0 }) as f32 * self.cell;
        let (mut tx, dx) = if abs(d.x) < 1e-9 { (f32::MAX, f32::MAX) } else { ((next(ix, step_x, gx0) - o.x) / d.x, self.cell / abs(d.x)) };
        let (mut tz, dz) = if abs(d.z) < 1e-9 { (f32::MAX, f32::MAX) } else { ((next(iz, step_z, gz0) - o.z) / d.z, self.cell / abs(d.z)) };

        let mut best: Option<Hit> = None;
        let mut best_t = tmax;
        loop {
            for &ti in self.cell_items(ix, iz) {
                let tri = &self.tris[ti as usize];
                if kinds & (1 << tri.kind) == 0 {
                    continue;
                }
                if let Some((t, front)) = ray_tri(o, d, tri) {
                    if t < best_t {
                        best_t = t;
                        best = Some(Hit { t, tri: ti, n: tri.n, kind: tri.kind, front });
                    }
                }
            }
            let exit = min(tx, tz);
            if best_t <= exit || exit > t1 {
                break;
            }
            if tx < tz {
                ix += step_x;
                tx += dx;
                if ix < 0 || ix >= self.nx {
                    break;
                }
            } else {
                iz += step_z;
                tz += dz;
                if iz < 0 || iz >= self.nz {
                    break;
                }
            }
        }
        best
    }

    /// Whether anything of `kinds` blocks the segment from `a` to `b`.
    pub fn blocked(&self, a: V3, b: V3, kinds: u32) -> Option<Hit> {
        let d = b - a;
        let l = d.len();
        if l < 1e-4 {
            return None;
        }
        self.raycast(a, d * (1.0 / l), l, kinds)
    }

    /// The deepest front-side contact of a sphere, if it touches anything.
    pub fn deepest_contact(&self, c: V3, r: f32) -> Option<Contact> {
        let mut best: Option<Contact> = None;
        self.contacts(c, r, |k| {
            if best.map_or(true, |b| k.depth > b.depth) {
                best = Some(k);
            }
        });
        best
    }

    /// Calls `f` for each triangle whose front side the sphere touches.
    /// A triangle spanning several cells can be reported more than once.
    pub fn contacts<F: FnMut(Contact)>(&self, c: V3, r: f32, mut f: F) {
        if self.tris.is_empty() {
            return;
        }
        let ix0 = (floor((c.x - r - self.min_x) * self.inv_cell) as i32).clamp(0, self.nx - 1);
        let ix1 = (floor((c.x + r - self.min_x) * self.inv_cell) as i32).clamp(0, self.nx - 1);
        let iz0 = (floor((c.z - r - self.min_z) * self.inv_cell) as i32).clamp(0, self.nz - 1);
        let iz1 = (floor((c.z + r - self.min_z) * self.inv_cell) as i32).clamp(0, self.nz - 1);
        for iz in iz0..=iz1 {
            for ix in ix0..=ix1 {
                for &ti in self.cell_items(ix, iz) {
                    let tri = &self.tris[ti as usize];
                    let s = (c - tri.v0).dot(tri.n);
                    if s > r || s < -BACK {
                        continue;
                    }
                    let q = closest_on_tri(c, tri);
                    let dq = c - q;
                    let d2 = dq.len2();
                    if d2 >= r * r {
                        continue;
                    }
                    let d = sqrt(d2);
                    // Behind the plane, or on it: push along the face normal.
                    let (n, depth) = if s <= 1e-4 || d < 1e-5 {
                        if (q - (c - tri.n * s)).len2() > 1e-6 {
                            // The closest point is on an edge and the centre is behind the plane: not inside the face.
                            continue;
                        }
                        (tri.n, r - s)
                    } else {
                        (dq * (1.0 / d), r - d)
                    };
                    f(Contact { tri: ti, n, depth, kind: tri.kind });
                }
            }
        }
    }
}

/// Möller–Trumbore, two-sided. Returns `t` and whether the front was hit.
#[inline]
fn ray_tri(o: V3, d: V3, tri: &Tri) -> Option<(f32, bool)> {
    const EDGE: f32 = -1e-5;
    let p = d.cross(tri.e2);
    let det = tri.e1.dot(p);
    if abs(det) < 1e-10 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - tri.v0;
    let u = s.dot(p) * inv;
    if u < EDGE || u > 1.0 - EDGE {
        return None;
    }
    let q = s.cross(tri.e1);
    let v = d.dot(q) * inv;
    if v < EDGE || u + v > 1.0 - EDGE {
        return None;
    }
    let t = tri.e2.dot(q) * inv;
    if t > 1e-4 {
        Some((t, det > 0.0))
    } else {
        None
    }
}

/// Closest point on a triangle to `p` (Ericson, Real-Time Collision Detection 5.1.5).
fn closest_on_tri(p: V3, tri: &Tri) -> V3 {
    let a = tri.v0;
    let ab = tri.e1;
    let ac = tri.e2;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let b = a + ab;
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let c = a + ac;
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad_world() -> World {
        // A 40 m ground square at y = 0 (facing up) and a wall at x = 10 facing -x.
        let verts = [
            -20.0, 0.0, -20.0, 20.0, 0.0, -20.0, 20.0, 0.0, 20.0, -20.0, 0.0, 20.0, //
            10.0, 0.0, -5.0, 10.0, 8.0, -5.0, 10.0, 8.0, 5.0, 10.0, 0.0, 5.0,
        ];
        let idx = [0, 2, 1, 0, 3, 2, 4, 6, 5, 4, 7, 6];
        World::build(&verts, &idx, &[kind::GROUND, kind::GROUND, kind::WALL, kind::WALL])
    }

    #[test]
    fn ray_hits_ground_and_wall() {
        let w = quad_world();
        let h = w.raycast(v3(0.0, 5.0, 0.0), v3(0.0, -1.0, 0.0), 100.0, mask::ALL).unwrap();
        assert!((h.t - 5.0).abs() < 1e-4 && h.kind == kind::GROUND && h.front);
        assert!(h.n.y > 0.99);
        let h = w.raycast(v3(0.0, 2.0, 0.0), v3(1.0, 0.0, 0.0), 100.0, mask::ALL).unwrap();
        assert!((h.t - 10.0).abs() < 1e-4 && h.kind == kind::WALL && h.front);
        assert!(h.n.x < -0.99);
        assert!(w.raycast(v3(0.0, 2.0, 0.0), v3(1.0, 0.0, 0.0), 100.0, 1 << kind::ROOF).is_none());
        assert!(w.raycast(v3(0.0, 2.0, 0.0), v3(-1.0, 0.0, 0.0), 100.0, mask::ALL).is_none());
    }

    #[test]
    fn sphere_contacts_ground() {
        let w = quad_world();
        let c = w.deepest_contact(v3(0.0, 0.4, 0.0), 0.5).unwrap();
        assert!((c.depth - 0.1).abs() < 1e-4 && c.n.y > 0.99);
        assert!(w.deepest_contact(v3(0.0, 0.6, 0.0), 0.5).is_none());
        let c = w.deepest_contact(v3(9.7, 4.0, 0.0), 0.5).unwrap();
        assert!((c.depth - 0.2).abs() < 1e-4 && c.n.x < -0.99);
    }
}
