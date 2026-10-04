//! Everything that is not the baked world, as vertices a device copies or
//! draws in place: the sky dome, the cloak, the wires, the soft discs of gas,
//! steam and the ground shadow; and the list of giants a frame draws.

use alloc::vec::Vec;
use maneuver_sim::collide::mask;
use maneuver_sim::math::*;
use maneuver_sim::pose::{BONES, CLOAK_H, CLOAK_N, CLOAK_W, ROPE_N};
use maneuver_sim::sim::hook;
use maneuver_sim::Sim;

use crate::mat;
use crate::scene::{powf, Scene};

/// Colour, then position: the order the PSP's GE reads a vertex in.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ColorVertex {
    pub color: [u8; 4],
    pub pos: [f32; 3],
}

pub const PUFFS: usize = 48;
/// A soft disc: a centre and `FAN` rim vertices, opaque in the middle and clear at the rim.
pub const FAN: usize = 8;
/// Discs per frame: the puffs and the character's ground shadow.
pub const DISCS: usize = PUFFS + 1;
pub const SKY_SEGS: usize = 20;
pub const SKY_RINGS: usize = 9;
pub const SKY_VERTS: usize = SKY_RINGS * SKY_SEGS;
pub const SKY_INDICES: usize = (SKY_RINGS - 1) * SKY_SEGS * 6;
pub const SUN_VERTS: usize = 2 * (FAN + 1);
pub const CLOAK_INDICES: usize = (CLOAK_W - 1) * (CLOAK_H - 1) * 6;
pub const ROPE_VERTS: usize = 2 * (ROPE_N - 1) * 4;
pub const DISC_VERTS: usize = DISCS * (FAN + 1);
pub const FAN_INDICES: usize = DISCS * FAN * 3;
/// The cloak's colour (`CLOAK` in web/src/world/scout.ts, sRGB 0.2, 0.42, 0.31), linear.
const CLOAK: [f32; 3] = [0.029, 0.148, 0.076];

/// The sky: rings from just under the horizon to the zenith, at `radius` around the origin.
pub fn sky(scene: &Scene, radius: f32, verts: &mut [ColorVertex], idx: &mut [u16]) {
    for r in 0..SKY_RINGS {
        let el = min(-0.14 + powf(r as f32 / (SKY_RINGS - 1) as f32, 1.5) * (PI * 0.5 + 0.14), PI * 0.5);
        for s in 0..SKY_SEGS {
            let az = s as f32 / SKY_SEGS as f32 * TAU;
            let d = v3(cos(el) * cos(az), sin(el), cos(el) * sin(az));
            let c = scene.sky_color(d);
            verts[r * SKY_SEGS + s] = ColorVertex { pos: [d.x * radius, d.y * radius, d.z * radius], color: [scene.encode(c[0]), scene.encode(c[1]), scene.encode(c[2]), 255] };
        }
    }
    let mut k = 0;
    for r in 0..SKY_RINGS - 1 {
        for s in 0..SKY_SEGS {
            let (a, b) = ((r * SKY_SEGS + s) as u16, (r * SKY_SEGS + (s + 1) % SKY_SEGS) as u16);
            let (c, d) = (a + SKY_SEGS as u16, b + SKY_SEGS as u16);
            for i in [a, b, d, a, d, c] {
                idx[k] = i;
                k += 1;
            }
        }
    }
}

/// Two soft discs toward the sun, the halo then the disc, drawn with the fan indices.
pub fn sun(scene: &Scene, radius: f32, verts: &mut [ColorVertex]) {
    let ax = scene.sun_dir.cross(V3::UP).norm_or(v3(1.0, 0.0, 0.0));
    let ay = scene.sun_dir.cross(ax);
    let warm = [scene.encode(1.0), scene.encode(0.9), scene.encode(0.72)];
    for (d, (size, alpha)) in [(0.13f32, 70u8), (0.035, 255)].into_iter().enumerate() {
        let c = scene.sun_dir * (radius * 0.95);
        verts[d * (FAN + 1)] = ColorVertex { pos: [c.x, c.y, c.z], color: [warm[0], warm[1], warm[2], alpha] };
        for t in 0..FAN {
            let a = t as f32 / FAN as f32 * TAU;
            let q = c + ax * (cos(a) * size * radius) + ay * (sin(a) * size * radius);
            verts[d * (FAN + 1) + 1 + t] = ColorVertex { pos: [q.x, q.y, q.z], color: [warm[0], warm[1], warm[2], 0] };
        }
    }
}

pub fn cloak_indices(idx: &mut [u16]) {
    let mut k = 0;
    for r in 0..CLOAK_H - 1 {
        for c in 0..CLOAK_W - 1 {
            let a = (r * CLOAK_W + c) as u16;
            let w = CLOAK_W as u16;
            for i in [a, a + w, a + 1, a + 1, a + w, a + w + 1] {
                idx[k] = i;
                k += 1;
            }
        }
    }
}

/// Indices for `quads` quads of four vertices: 0 1 2, 0 2 3.
pub fn quad_indices(idx: &mut [u16], quads: usize) {
    for q in 0..quads {
        let b = (q * 4) as u16;
        for (j, o) in [0u16, 1, 2, 0, 2, 3].iter().enumerate() {
            idx[q * 6 + j] = b + o;
        }
    }
}

pub fn fan_indices(idx: &mut [u16]) {
    for d in 0..DISCS {
        let b = (d * (FAN + 1)) as u16;
        for t in 0..FAN {
            idx[(d * FAN + t) * 3] = b;
            idx[(d * FAN + t) * 3 + 1] = b + 1 + t as u16;
            idx[(d * FAN + t) * 3 + 2] = b + 1 + ((t + 1) % FAN) as u16;
        }
    }
}

/// Three rows per bone, as a skinning program reads them.
pub fn bone_rows(skin: &[M34; BONES], sink: f32, out: &mut [f32; BONES * 12]) {
    for (b, m) in skin.iter().enumerate() {
        let o = b * 12;
        out[o..o + 4].copy_from_slice(&[m.r.x.x, m.r.y.x, m.r.z.x, m.t.x]);
        out[o + 4..o + 8].copy_from_slice(&[m.r.x.y, m.r.y.y, m.r.z.y, m.t.y - sink]);
        out[o + 8..o + 12].copy_from_slice(&[m.r.x.z, m.r.y.z, m.r.z.z, m.t.z]);
    }
}

#[derive(Clone, Copy)]
struct Puff {
    pos: V3,
    vel: V3,
    age: f32,
    life: f32,
    size: f32,
}

/// What `update` wrote for this frame.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct Frame {
    pub rope_quads: u32,
    pub discs: u32,
}

/// A giant this frame draws.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Giant {
    pub index: u32,
    pub variant: u32,
    /// 0: the detailed model; 1: the coarse one.
    pub level: u32,
    /// How far a fallen giant has sunk into the ground.
    pub sink: f32,
    /// Sun visibility where it stands.
    pub vis: f32,
    pub dist: f32,
}

pub struct Actors {
    titan_vis: Vec<f32>,
    puffs: [Puff; PUFFS],
    next_puff: usize,
    /// Sun visibility at the character, eased.
    pub vis: f32,
    /// Whether the last ray toward the sun was clear.
    lit: bool,
    /// Per giant: whether buildings hide it from the eye, as of its last test.
    hidden: Vec<bool>,
    /// The giant the next frame tests.
    probe: usize,
}

impl Actors {
    pub fn new(sim: &Sim, scene: &Scene) -> Actors {
        let titan_vis = sim.dummies.iter().map(|d| if sim.world.raycast(d.pos + v3(0.0, d.height * 0.7, 0.0) + scene.sun_dir * (d.height * 0.3), scene.sun_dir, 500.0, mask::ALL).is_none() { 1.0 } else { 0.25 }).collect();
        Actors { titan_vis, puffs: [Puff { pos: V3::ZERO, vel: V3::ZERO, age: 9.0, life: 0.7, size: 1.0 }; PUFFS], next_puff: 0, vis: 1.0, lit: true, hidden: alloc::vec![false; sim.dummies.len()], probe: 0 }
    }

    fn puff(&mut self, pos: V3, vel: V3, life: f32, size: f32) {
        let j = self.next_puff % PUFFS;
        self.next_puff += 1;
        self.puffs[j] = Puff { pos, vel, age: 0.0, life, size };
    }

    /// Writes this frame's CPU geometry: `cloak` takes `CLOAK_N` vertices,
    /// `rope` `ROPE_VERTS`, `disc` `DISC_VERTS`.
    pub fn update(&mut self, sim: &Sim, scene: &Scene, eye: V3, ticks: u32, cloak: &mut [ColorVertex], rope: &mut [ColorVertex], disc: &mut [ColorVertex]) -> Frame {
        let dt = ticks as f32 * maneuver_sim::sim::DT;
        // The character darkens in shade: one ray toward the sun, every fourth tick; the easing covers the gap.
        if sim.tick % 4 < ticks {
            self.lit = sim.world.raycast(sim.p.pos + v3(0.0, 0.6, 0.0), scene.sun_dir, 400.0, mask::ALL).is_none();
        }
        self.vis = ease(self.vis, if self.lit { 1.0 } else { 0.0 }, 10.0, max(dt, 1.0 / 60.0));

        // Cloak: the simulation's cloth, lit on whichever side faces the sun.
        for i in 0..CLOAK_N {
            let p = sim.pose.cloak.p[i];
            let n = sim.pose.cloak.n[i];
            let front = n.dot(scene.sun_dir) >= 0.0;
            let color = scene.shade(if front { n } else { -n }, CLOAK, if front { self.vis } else { self.vis * 0.6 });
            cloak[i] = ColorVertex { pos: [p.x, p.y, p.z], color };
        }

        // Wires: a ribbon along each rope, turned to the eye, at least a pixel wide.
        let width = 1.3 / scene.h.screen[1];
        let mut quads = 0usize;
        for i in 0..2 {
            if sim.p.hooks[i].state == hook::IDLE {
                continue;
            }
            let pts = &sim.pose.ropes[i].p;
            let side = |k: usize| {
                let t = pts[(k + 1).min(ROPE_N - 1)] - pts[k.saturating_sub(1)];
                let to = pts[k] - eye;
                t.cross(to).norm_or(V3::UP) * max(0.014, to.len() * width)
            };
            let color = [30, 30, 34, 255];
            for k in 0..ROPE_N - 1 {
                let (a, b) = (pts[k], pts[k + 1]);
                let (sa, sb) = (side(k), side(k + 1));
                for (j, q) in [a - sa, b - sb, b + sb, a + sa].into_iter().enumerate() {
                    rope[quads * 4 + j] = ColorVertex { pos: [q.x, q.y, q.z], color };
                }
                quads += 1;
            }
        }

        // Gas leaves the hips while it burns; a fallen giant steams.
        if sim.p.thrusting || sim.p.reeling {
            let i = self.next_puff % 2;
            let j = self.next_puff;
            let jitter = v3(sin(j as f32 * 12.99) * 1.4, cos(j as f32 * 7.31) * 1.4, sin(j as f32 * 3.7) * 1.4);
            self.puff(sim.pose.hip[i] + v3(0.0, -0.1, 0.0), sim.p.vel * 0.35 + jitter, 0.6, 1.1);
        }
        for (i, d) in sim.dummies.iter().enumerate() {
            let since = sim.tick.wrapping_sub(d.cut_tick) as f32 / 60.0;
            if d.alive || since > 8.0 || (sim.tick + i as u32) % 4 != 0 || (d.pos - eye).len2() > 300.0 * 300.0 {
                continue;
            }
            let j = self.next_puff as f32;
            let fwd = heading(d.yaw);
            let along = d.height * (0.15 + 0.8 * (sin(j * 2.3) * 0.5 + 0.5)) * saturate(since / 1.5);
            let pos = d.pos + fwd * along + v3(sin(j * 5.1) * d.height * 0.15, d.height * 0.12, cos(j * 3.3) * d.height * 0.15);
            self.puff(pos, v3(sin(j) * 1.5, 4.0 + d.height * 0.2, cos(j * 1.7) * 1.5), 1.6, d.height * 0.22);
        }

        let mut discs = 0usize;
        let mut put = |c: V3, ax: V3, ay: V3, radius: f32, color: [u8; 4]| {
            let v = &mut disc[discs * (FAN + 1)..(discs + 1) * (FAN + 1)];
            v[0] = ColorVertex { pos: [c.x, c.y, c.z], color };
            for t in 0..FAN {
                let a = t as f32 / FAN as f32 * TAU;
                let q = c + ax * (cos(a) * radius) + ay * (sin(a) * radius);
                v[1 + t] = ColorVertex { pos: [q.x, q.y, q.z], color: [color[0], color[1], color[2], 0] };
            }
            discs += 1;
        };
        // The character's shadow on whatever is below, fainter with height.
        if let Some(h) = sim.world.raycast(sim.p.pos, v3(0.0, -1.0, 0.0), 30.0, mask::ALL) {
            if h.front {
                let at = sim.p.pos - v3(0.0, h.t - 0.05, 0.0);
                let ax = h.n.cross(v3(0.0, 0.0, 1.0)).norm_or(v3(1.0, 0.0, 0.0));
                let k = 1.0 - h.t / 30.0;
                put(at, ax, h.n.cross(ax), 0.55 + h.t * 0.06, [0, 0, 0, (150.0 * k * k) as u8]);
            }
        }
        let fwd = sim.cam.look;
        let right = fwd.cross(V3::UP).norm_or(v3(1.0, 0.0, 0.0));
        let up = right.cross(fwd);
        for p in &mut self.puffs {
            if p.age > p.life {
                continue;
            }
            p.age += dt;
            p.pos += p.vel * dt;
            let k = p.age / p.life;
            put(p.pos, right, up, (0.22 + k * 1.1) * p.size, [240, 243, 246, ((1.0 - k) * 150.0) as u8]);
        }
        Frame { rope_quads: quads as u32, discs: discs as u32 }
    }

    /// Whether the town hides giant `i` from `eye`: rays to its head, chest and hip are all blocked.
    fn occluded(sim: &Sim, i: usize, eye: V3) -> bool {
        let d = &sim.dummies[i];
        [0.95, 0.6, 0.25].iter().all(|k| {
            let to = d.pos + v3(0.0, d.height * k, 0.0) - eye;
            let len = to.len();
            len > 2.0 && sim.world.raycast(eye, to * (1.0 / len), len - 1.5, mask::ALL).is_some()
        })
    }

    /// The giants in view and in range, nearest first. One giant beyond the near distance is tested
    /// against the town each frame, in turn; one that is hidden is not drawn until a later test sees it.
    pub fn giants(&mut self, sim: &Sim, scene: &Scene, planes: &[[f32; 4]; 6], eye: V3, out: &mut Vec<Giant>) {
        let n = sim.dummies.len();
        for step in 1..=n {
            let i = (self.probe + step) % n;
            let d = &sim.dummies[i];
            let dist = (d.pos + v3(0.0, d.height * 0.5, 0.0) - eye).len();
            if dist > scene.h.titan_far || dist < scene.h.titan_near {
                self.hidden[i] = false;
                continue;
            }
            self.hidden[i] = Self::occluded(sim, i, eye);
            self.probe = i;
            break;
        }
        for (i, d) in sim.dummies.iter().enumerate() {
            let since = sim.tick.wrapping_sub(d.cut_tick) as f32 / 60.0;
            if !d.alive && since > 9.0 {
                continue;
            }
            let dist = (d.pos + v3(0.0, d.height * 0.5, 0.0) - eye).len();
            // Standing or lying, the body fits in a box one height to each side.
            let (lo, hi) = ([d.pos.x - d.height, d.pos.y - 1.0, d.pos.z - d.height], [d.pos.x + d.height, d.pos.y + d.height * 1.1, d.pos.z + d.height]);
            if dist > scene.h.titan_far || !mat::visible(planes, &lo, &hi) || (self.hidden[i] && dist > scene.h.titan_near) {
                continue;
            }
            // A fallen giant sinks away.
            let sink = if d.alive { 0.0 } else { max(since - 5.0, 0.0) * d.height * 0.08 };
            out.push(Giant { index: i as u32, variant: d.variant % 3, level: if dist < scene.h.titan_near { 0 } else { 1 }, sink, vis: self.titan_vis[i], dist });
        }
        out.sort_unstable_by(|a, b| a.dist.partial_cmp(&b.dist).unwrap_or(core::cmp::Ordering::Equal));
        out.truncate(scene.h.max_giants as usize);
    }
}
