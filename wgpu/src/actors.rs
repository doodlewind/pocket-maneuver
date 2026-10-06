//! Everything that is not the baked world, as vertices (`vita/src/actors.rs`):
//! the sky's dome, the player and the giants (skinned on the GPU from the
//! simulation's bone matrices), the cloak, the wires and the gas. This module
//! writes what a frame draws; `render` holds the buffers and draws it.

use maneuver_sim::collide::mask;
use maneuver_sim::math::*;
use maneuver_sim::pose::{BONES, CLOAK_H, CLOAK_N, CLOAK_W, ROPE_N};
use maneuver_sim::sim::hook;
use maneuver_sim::Sim;

use crate::mat::{self, Mat4};
use crate::scene::Scene;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ColorVertex {
    pub pos: [f32; 3],
    pub color: [u8; 4],
}

pub const QUADS: usize = 2048;
const PUFFS: usize = 96;
/// A soft disc: a centre and `FAN` rim vertices, opaque in the middle and clear at the rim.
pub const FAN: usize = 12;
/// Discs per frame: the gas and steam puffs and the character's ground shadow.
pub const DISCS: usize = PUFFS + 1;
const SKY_SEGS: usize = 24;
const SKY_RINGS: usize = 13;
/// A giant nearer than this draws its detailed mesh; beyond `TITAN_FAR` it is not drawn.
const TITAN_NEAR: f32 = 140.0;
const TITAN_FAR: f32 = 520.0;
/// The cloak's colour as the display shows it (`CLOAK` in web/src/world/scout.ts).
const CLOAK: [f32; 3] = [0.2, 0.42, 0.31];
/// Floats of a skinned draw's record (`SkinDraw` in scene.wgsl): the matrix, three rows a bone, the light, the haze.
pub const SKIN_FLOATS: usize = 16 + BONES * 12 + 16 + 4;

/// A skinned model of a frame: which of the pack's models, and its draw record.
pub struct Skinned {
    /// 0: the player; 1 + build × 2 + (0 detailed | 1 coarse): a giant.
    pub model: usize,
    pub record: [f32; SKIN_FLOATS],
}

/// Three rows per bone, as the skinning program reads them.
fn bone_rows(skin: &[M34; BONES], sink: f32, out: &mut [f32]) {
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

/// The vertices `update` wrote for a frame, as ranges of `Actors::moving`.
#[derive(Clone, Copy, Default)]
pub struct Frame {
    pub rope_first: u32,
    pub rope_quads: u32,
    pub disc_first: u32,
    pub discs: u32,
}

pub struct Actors {
    /// The dome: rings from just under the horizon to the zenith, and its indices.
    pub sky: Vec<ColorVertex>,
    pub sky_idx: Vec<u16>,
    /// Two soft discs toward the sun: its halo and its disc.
    pub sun: Vec<ColorVertex>,
    pub cloak_idx: Vec<u16>,
    /// Shared indices for quads: 0 1 2, 0 2 3, per four vertices.
    pub quad_idx: Vec<u16>,
    /// Shared indices for discs of `FAN + 1` vertices.
    pub fan_idx: Vec<u16>,
    /// Sun visibility where each giant stands.
    titan_vis: Vec<f32>,
    /// This frame's vertices: the cloak first, then the wires, then the discs.
    pub moving: Vec<ColorVertex>,
    puffs: [Puff; PUFFS],
    next_puff: usize,
    vis: f32,
}

impl Actors {
    pub fn new(scene: &Scene, sim: &Sim) -> Actors {
        let mut sky = Vec::with_capacity(SKY_RINGS * SKY_SEGS);
        for r in 0..SKY_RINGS {
            let el = (-0.14 + (r as f32 / (SKY_RINGS - 1) as f32).powf(1.5) * (PI * 0.5 + 0.14)).min(PI * 0.5);
            for s in 0..SKY_SEGS {
                let az = s as f32 / SKY_SEGS as f32 * TAU;
                let d = v3(cos(el) * cos(az), sin(el), cos(el) * sin(az));
                let c = scene.sky_color(d);
                sky.push(ColorVertex { pos: [d.x * 2000.0, d.y * 2000.0, d.z * 2000.0], color: [scene.encode(c[0]), scene.encode(c[1]), scene.encode(c[2]), 255] });
            }
        }
        let mut sky_idx = Vec::with_capacity((SKY_RINGS - 1) * SKY_SEGS * 6);
        for r in 0..SKY_RINGS - 1 {
            for s in 0..SKY_SEGS {
                let (a, b) = ((r * SKY_SEGS + s) as u16, (r * SKY_SEGS + (s + 1) % SKY_SEGS) as u16);
                let (c, d) = (a + SKY_SEGS as u16, b + SKY_SEGS as u16);
                sky_idx.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        let mut sun = Vec::with_capacity(2 * (FAN + 1));
        let ax = scene.sun_dir.cross(V3::UP).norm_or(v3(1.0, 0.0, 0.0));
        let ay = scene.sun_dir.cross(ax);
        for (radius, alpha) in [(260.0f32, 70u8), (70.0, 255)] {
            let c = scene.sun_dir * 1900.0;
            let warm = [scene.encode(1.0), scene.encode(0.9), scene.encode(0.72)];
            sun.push(ColorVertex { pos: [c.x, c.y, c.z], color: [warm[0], warm[1], warm[2], alpha] });
            for t in 0..FAN {
                let a = t as f32 / FAN as f32 * TAU;
                let q = c + ax * (cos(a) * radius) + ay * (sin(a) * radius);
                sun.push(ColorVertex { pos: [q.x, q.y, q.z], color: [warm[0], warm[1], warm[2], 0] });
            }
        }
        let mut cloak_idx = Vec::with_capacity((CLOAK_W - 1) * (CLOAK_H - 1) * 6);
        for r in 0..CLOAK_H - 1 {
            for c in 0..CLOAK_W - 1 {
                let a = (r * CLOAK_W + c) as u16;
                let w = CLOAK_W as u16;
                cloak_idx.extend_from_slice(&[a, a + w, a + 1, a + 1, a + w, a + w + 1]);
            }
        }
        let mut quad_idx = Vec::with_capacity(QUADS * 6);
        for q in 0..QUADS {
            let b = (q * 4) as u16;
            quad_idx.extend([0u16, 1, 2, 0, 2, 3].iter().map(|o| b + o));
        }
        let mut fan_idx = Vec::with_capacity(DISCS * FAN * 3);
        for d in 0..DISCS {
            let b = (d * (FAN + 1)) as u16;
            for t in 0..FAN {
                fan_idx.extend_from_slice(&[b, b + 1 + t as u16, b + 1 + ((t + 1) % FAN) as u16]);
            }
        }
        let titan_vis = sim.dummies.iter().map(|d| if sim.world.raycast(d.pos + v3(0.0, d.height * 0.7, 0.0) + scene.sun_dir * (d.height * 0.3), scene.sun_dir, 500.0, mask::ALL).is_none() { 1.0 } else { 0.25 }).collect();
        Actors {
            sky,
            sky_idx,
            sun,
            cloak_idx,
            quad_idx,
            fan_idx,
            titan_vis,
            moving: Vec::with_capacity(CLOAK_N + 2 * (ROPE_N - 1) * 4 + DISCS * (FAN + 1)),
            puffs: [Puff { pos: V3::ZERO, vel: V3::ZERO, age: 9.0, life: 0.7, size: 1.0 }; PUFFS],
            next_puff: 0,
            vis: 1.0,
        }
    }

    /// Vertices `moving` holds at most.
    pub fn moving_capacity() -> usize {
        CLOAK_N + 2 * (ROPE_N - 1) * 4 + DISCS * (FAN + 1)
    }

    fn puff(&mut self, pos: V3, vel: V3, life: f32, size: f32) {
        let j = self.next_puff % PUFFS;
        self.next_puff += 1;
        self.puffs[j] = Puff { pos, vel, age: 0.0, life, size };
    }

    /// Writes this frame's vertices into `moving`: the cloak, the wires and the soft discs. `ticks` is the
    /// simulation's ticks since the last frame (none while the game is paused: the gas stands still).
    pub fn update(&mut self, sim: &Sim, scene: &Scene, eye: V3, ticks: u32) -> Frame {
        let dt = ticks as f32 * maneuver_sim::sim::DT;
        // The character darkens in shade: one ray toward the sun.
        let lit = sim.world.raycast(sim.p.pos + v3(0.0, 0.6, 0.0), scene.sun_dir, 400.0, mask::ALL).is_none();
        self.vis = ease(self.vis, if lit { 1.0 } else { 0.0 }, 10.0, max(dt, 1.0 / 60.0));
        self.moving.clear();

        // Cloak: the simulation's cloth, lit on whichever side faces the sun.
        let tint = CLOAK.map(|c| libm::powf(c, 2.2));
        for i in 0..CLOAK_N {
            let p = sim.pose.cloak.p[i];
            let n = sim.pose.cloak.n[i];
            let front = n.dot(scene.sun_dir) >= 0.0;
            let color = scene.shade(if front { n } else { -n }, tint, if front { self.vis } else { self.vis * 0.6 });
            self.moving.push(ColorVertex { pos: [p.x, p.y, p.z], color });
        }

        // Wires: a ribbon along each rope, turned to the eye, at least a pixel wide.
        let rope_first = self.moving.len() as u32;
        let mut quads = 0u32;
        for i in 0..2 {
            if sim.p.hooks[i].state == hook::IDLE {
                continue;
            }
            let pts = &sim.pose.ropes[i].p;
            let side = |k: usize| {
                let t = pts[(k + 1).min(ROPE_N - 1)] - pts[k.saturating_sub(1)];
                let to = pts[k] - eye;
                t.cross(to).norm_or(V3::UP) * max(0.014, to.len() * 0.0011)
            };
            let color = [30, 30, 34, 255];
            for k in 0..ROPE_N - 1 {
                let (a, b) = (pts[k], pts[k + 1]);
                let (sa, sb) = (side(k), side(k + 1));
                for q in [a - sa, b - sb, b + sb, a + sa] {
                    self.moving.push(ColorVertex { pos: [q.x, q.y, q.z], color });
                }
                quads += 1;
            }
        }

        // Gas leaves the hips while it burns; a fallen giant steams.
        if ticks > 0 && (sim.p.thrusting || sim.p.reeling) {
            for i in 0..2 {
                let j = self.next_puff;
                let jitter = v3(sin(j as f32 * 12.99) * 1.4, cos(j as f32 * 7.31) * 1.4, sin(j as f32 * 3.7) * 1.4);
                self.puff(sim.pose.hip[i] + v3(0.0, -0.1, 0.0), sim.p.vel * 0.35 + jitter, 0.7, 1.0);
            }
        }
        for (i, d) in sim.dummies.iter().enumerate() {
            let since = sim.tick.wrapping_sub(d.cut_tick) as f32 / 60.0;
            if ticks == 0 || d.alive || since > 8.0 || (sim.tick + i as u32) % 3 != 0 || (d.pos - eye).len2() > 400.0 * 400.0 {
                continue;
            }
            let j = self.next_puff as f32;
            let fwd = heading(d.yaw);
            let along = d.height * (0.15 + 0.8 * (sin(j * 2.3) * 0.5 + 0.5)) * saturate(since / 1.5);
            let pos = d.pos + fwd * along + v3(sin(j * 5.1) * d.height * 0.15, d.height * 0.12, cos(j * 3.3) * d.height * 0.15);
            self.puff(pos, v3(sin(j) * 1.5, 4.0 + d.height * 0.2, cos(j * 1.7) * 1.5), 1.6, d.height * 0.22);
        }

        let disc_first = self.moving.len() as u32;
        let mut discs = 0u32;
        let moving = &mut self.moving;
        let mut disc = |c: V3, ax: V3, ay: V3, radius: f32, color: [u8; 4]| {
            moving.push(ColorVertex { pos: [c.x, c.y, c.z], color });
            for t in 0..FAN {
                let a = t as f32 / FAN as f32 * TAU;
                let q = c + ax * (cos(a) * radius) + ay * (sin(a) * radius);
                moving.push(ColorVertex { pos: [q.x, q.y, q.z], color: [color[0], color[1], color[2], 0] });
            }
            discs += 1;
        };
        // The character's shadow on whatever is below, fainter with height.
        if let Some(h) = sim.world.raycast(sim.p.pos, v3(0.0, -1.0, 0.0), 30.0, mask::ALL) {
            if h.front {
                let at = sim.p.pos - v3(0.0, h.t - 0.04, 0.0);
                let ax = h.n.cross(v3(0.0, 0.0, 1.0)).norm_or(v3(1.0, 0.0, 0.0));
                let k = 1.0 - h.t / 30.0;
                disc(at, ax, h.n.cross(ax), 0.55 + h.t * 0.06, [0, 0, 0, (150.0 * k * k) as u8]);
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
            disc(p.pos, right, up, (0.22 + k * 1.1) * p.size, [240, 243, 246, ((1.0 - k) * 150.0) as u8]);
        }
        Frame { rope_first, rope_quads: quads, disc_first, discs }
    }

    /// The player (when `character`) and the giants in range, with their draw records.
    pub fn skinned(&self, vp: &Mat4, sim: &Sim, scene: &Scene, eye: V3, character: bool, out: &mut Vec<Skinned>) {
        let planes = mat::planes(vp);
        let mut record = |model: usize, skin: &[M34; BONES], sink: f32, vis: f32| {
            let mut r = [0.0f32; SKIN_FLOATS];
            r[..16].copy_from_slice(vp);
            bone_rows(skin, sink, &mut r[16..16 + BONES * 12]);
            r[16 + BONES * 12..32 + BONES * 12].copy_from_slice(&scene.light(vis));
            r[32 + BONES * 12] = scene.fog_density;
            out.push(Skinned { model, record: r });
        };
        if character {
            record(0, &sim.pose.skin, 0.0, self.vis);
        }
        for (i, d) in sim.dummies.iter().enumerate() {
            let since = sim.tick.wrapping_sub(d.cut_tick) as f32 / 60.0;
            if !d.alive && since > 9.0 {
                continue;
            }
            let dist = (d.pos + v3(0.0, d.height * 0.5, 0.0) - eye).len();
            // Standing or lying, the body fits in a box one height to each side.
            let (lo, hi) = ([d.pos.x - d.height, d.pos.y - 1.0, d.pos.z - d.height], [d.pos.x + d.height, d.pos.y + d.height * 1.1, d.pos.z + d.height]);
            if dist > TITAN_FAR || !mat::visible(&planes, &lo, &hi) {
                continue;
            }
            // A fallen giant sinks away.
            let sink = if d.alive { 0.0 } else { max(since - 5.0, 0.0) * d.height * 0.08 };
            record(1 + (d.variant % 3) as usize * 2 + (dist >= TITAN_NEAR) as usize, &sim.titan_skin(i), sink, self.titan_vis[i]);
        }
    }
}
