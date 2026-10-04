//! Everything that is not the baked world: the sky dome, the player and the
//! giants (skinned on the GPU from the simulation's bone matrices), the cloak,
//! the wires and the gas.

use maneuver_pack::{self as pack, ModelHeader, Pack, SkinVertex};
use maneuver_sim::collide::mask;
use maneuver_sim::math::*;
use maneuver_sim::pose::{BONES, CLOAK_H, CLOAK_N, CLOAK_W, ROPE_N};
use maneuver_sim::sim::hook;
use maneuver_sim::Sim;
use pocket_vita_gxm::mem::{Block, Kind, Ring};
use serde_json::Value;
use vita2d_sys as g;

use crate::gpu::{self, Program};
use crate::mat::{self, Mat4};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ColorVertex {
    pub pos: [f32; 3],
    pub color: [u8; 4],
}

pub const QUADS: usize = 2048;
const PUFFS: usize = 96;
/// A soft disc: a centre and `FAN` rim vertices, opaque in the middle and clear at the rim.
const FAN: usize = 12;
/// Discs per frame: the gas and steam puffs and the character's ground shadow.
const DISCS: usize = PUFFS + 1;
const SKY_SEGS: usize = 24;
const SKY_RINGS: usize = 13;
/// A giant nearer than this draws its detailed mesh; beyond `TITAN_FAR` it is not drawn.
const TITAN_NEAR: f32 = 140.0;
const TITAN_FAR: f32 = 520.0;
/// The cloak's sRGB colour (`CLOAK` in web/src/world/scout.ts).
const CLOAK: [f32; 3] = [0.2, 0.42, 0.31];

/// Scene constants from the pack's META.
pub struct Scene {
    pub sun_dir: V3,
    pub sun: [f32; 3],
    pub sky: [f32; 3],
    pub bounce: [f32; 3],
    pub fog: [f32; 3],
    pub fog_density: f32,
    pub horizon: [f32; 3],
    pub zenith: [f32; 3],
    pub glow: [f32; 3],
    pub lod_near: f32,
    pub lod_mid: f32,
    pub clip_near: f32,
    pub clip_far: f32,
    pub cell: f32,
    pub super_cell: f32,
    /// Linear 0..2 to sRGB bytes.
    lut: Vec<u8>,
}

fn arr3(v: &Value) -> [f32; 3] {
    let f = |i: usize| v.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    [f(0), f(1), f(2)]
}

impl Scene {
    pub fn from_meta(meta: &Value) -> Scene {
        let s = &meta["scene"];
        let d = arr3(&s["sunDir"]);
        let num = |v: &Value, d: f32| v.as_f64().map(|x| x as f32).unwrap_or(d);
        let lut = (0..1024).map(|i| (libm::powf(i as f32 / 511.5, 1.0 / 2.2).min(1.0) * 255.0 + 0.5) as u8).collect();
        Scene {
            sun_dir: v3(d[0], d[1], d[2]).norm(),
            sun: arr3(&s["sun"]),
            sky: arr3(&s["sky"]),
            bounce: arr3(&s["bounce"]),
            fog: arr3(&s["fog"]),
            fog_density: num(&s["fogDensity"], 0.0008),
            horizon: arr3(&s["horizon"]),
            zenith: arr3(&s["zenith"]),
            glow: arr3(&s["glow"]),
            lod_near: num(&s["lod"]["near"], 110.0),
            lod_mid: num(&s["lod"]["mid"], 380.0),
            clip_near: num(&s["clip"]["near"], 0.35),
            clip_far: num(&s["clip"]["far"], 4200.0),
            cell: num(&s["cell"], 64.0),
            super_cell: num(&s["superCell"], 256.0),
            lut,
        }
    }

    #[inline]
    pub fn encode(&self, lin: f32) -> u8 {
        self.lut[((lin * 511.5) as usize).min(1023)]
    }

    /// sRGB haze colour, as the shaders take it.
    pub fn fog_srgb(&self) -> [f32; 3] {
        [0, 1, 2].map(|i| libm::powf(self.fog[i], 1.0 / 2.2))
    }

    /// Lit colour of a surface with normal `n` and linear tint `tint`; `vis` is how much of the sun reaches it.
    #[inline]
    pub fn shade(&self, n: V3, tint: [f32; 3], vis: f32) -> [u8; 4] {
        let ndl = max(n.dot(self.sun_dir), 0.0) * vis;
        let up = 0.5 + 0.5 * n.y;
        let mut out = [0u8, 0, 0, 255];
        for c in 0..3 {
            let hemi = self.bounce[c] + (self.sky[c] - self.bounce[c]) * up;
            out[c] = self.encode(tint[c] * (self.sun[c] * ndl + hemi * 0.92));
        }
        out
    }

    /// The light table of the skinning program: sun direction and visibility, sun, sky, bounce.
    pub fn light(&self, vis: f32) -> [f32; 16] {
        [self.sun_dir.x, self.sun_dir.y, self.sun_dir.z, vis, self.sun[0], self.sun[1], self.sun[2], 0.0, self.sky[0], self.sky[1], self.sky[2], 0.0, self.bounce[0], self.bounce[1], self.bounce[2], 0.0]
    }

    /// Sky radiance toward `d` (`web/src/render/sky.ts`).
    pub fn sky_color(&self, d: V3) -> [f32; 3] {
        let h = max(d.y, 0.0);
        let k = 1.0 - libm::powf(1.0 - h, 3.2);
        let s = max(d.dot(self.sun_dir), 0.0);
        let glow = 0.42 * libm::powf(s, 6.0) + 0.9 * libm::powf(s, 90.0);
        let below = saturate(-d.y * 6.0);
        [0, 1, 2].map(|i| {
            let sky = self.horizon[i] + (self.zenith[i] - self.horizon[i]) * k + self.glow[i] * glow;
            sky + (self.fog[i] - sky) * below
        })
    }
}

/// A skinned model in GPU memory.
#[derive(Clone, Copy)]
struct SkinMesh {
    vb: *const u8,
    ib: *const u16,
    idx: u32,
}

/// Three rows per bone, as the skinning program reads them.
fn bone_rows(skin: &[M34; BONES], sink: f32, out: &mut [f32; BONES * 12]) {
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
pub struct Frame {
    cloak_vb: *const u8,
    rope_vb: *const u8,
    rope_quads: u32,
    disc_vb: *const u8,
    discs: u32,
}

pub struct Actors {
    _block: Block,
    sky_vb: *const u8,
    sky_ib: *const u16,
    sky_idx: u32,
    /// Two soft discs toward the sun: the disc and its halo.
    sun_vb: *const u8,
    scout: SkinMesh,
    /// Per build: detailed and coarse.
    titans: [[SkinMesh; 2]; 3],
    /// Sun visibility where each giant stands.
    titan_vis: Vec<f32>,
    cloak_ib: *const u16,
    cloak_idx: u32,
    /// Shared indices for quads: 0 1 2, 0 2 3, per four vertices.
    pub quad_ib: *const u16,
    /// Shared indices for discs of `FAN + 1` vertices.
    fan_ib: *const u16,
    puffs: [Puff; PUFFS],
    next_puff: usize,
    vis: f32,
}

impl Actors {
    /// # Safety
    /// GXM is initialized.
    pub unsafe fn load(p: &Pack, scene: &Scene, sim: &Sim) -> Result<Actors, String> {
        let modl = p.section(pack::MODL)?;
        let count: u32 = pack::read(modl, 0).ok_or("model count")?;
        let sky_verts = SKY_RINGS * SKY_SEGS;
        let sky_idx = (SKY_RINGS - 1) * SKY_SEGS * 6;
        let cloak_idx = (CLOAK_W - 1) * (CLOAK_H - 1) * 6;
        let size = modl.len() + count as usize * 64 + (sky_verts + 2 * (FAN + 1)) * 16 + (sky_idx + cloak_idx + QUADS * 6 + DISCS * FAN * 3) * 2 + 1024;
        let mut block = Block::with_access(Kind::Main, size, false)?;
        let mut alloc = |bytes: usize| block.alloc(bytes, 16).ok_or("actor geometry block".to_string());

        // Skinned models, copied as they are in the pack.
        let none = SkinMesh { vb: core::ptr::null(), ib: core::ptr::null(), idx: 0 };
        let mut scout = none;
        let mut titans = [[none; 2]; 3];
        let mut at = 4;
        for _ in 0..count {
            let h: ModelHeader = pack::read(modl, at).ok_or("model header")?;
            at += core::mem::size_of::<ModelHeader>();
            let vbytes = h.vtx_count as usize * core::mem::size_of::<SkinVertex>();
            let ibytes = h.idx_count as usize * 2;
            if at + vbytes + ibytes > modl.len() {
                return Err("model section is truncated".into());
            }
            let vb = alloc(vbytes)?;
            core::ptr::copy_nonoverlapping(modl.as_ptr().add(at), vb, vbytes);
            at += vbytes;
            let ib = alloc(ibytes)?;
            core::ptr::copy_nonoverlapping(modl.as_ptr().add(at), ib, ibytes);
            at = (at + ibytes + 3) & !3;
            let mesh = SkinMesh { vb, ib: ib.cast(), idx: h.idx_count };
            match h.id {
                0 => scout = mesh,
                10..=12 => titans[(h.id - 10) as usize][0] = mesh,
                20..=22 => titans[(h.id - 20) as usize][1] = mesh,
                _ => {}
            }
        }
        if scout.idx == 0 || titans.iter().flatten().any(|m| m.idx == 0) {
            return Err("the pack lacks a skinned model".into());
        }

        // Sky: rings from just under the horizon to the zenith.
        let sky_vb = alloc(sky_verts * 16)?.cast::<ColorVertex>();
        let sky_ib = alloc(sky_idx * 2)?.cast::<u16>();
        for r in 0..SKY_RINGS {
            let el = (-0.14 + (r as f32 / (SKY_RINGS - 1) as f32).powf(1.5) * (PI * 0.5 + 0.14)).min(PI * 0.5);
            for s in 0..SKY_SEGS {
                let az = s as f32 / SKY_SEGS as f32 * TAU;
                let d = v3(cos(el) * cos(az), sin(el), cos(el) * sin(az));
                let c = scene.sky_color(d);
                *sky_vb.add(r * SKY_SEGS + s) = ColorVertex { pos: [d.x * 2000.0, d.y * 2000.0, d.z * 2000.0], color: [scene.encode(c[0]), scene.encode(c[1]), scene.encode(c[2]), 255] };
            }
        }
        let mut k = 0;
        for r in 0..SKY_RINGS - 1 {
            for s in 0..SKY_SEGS {
                let (a, b) = ((r * SKY_SEGS + s) as u16, (r * SKY_SEGS + (s + 1) % SKY_SEGS) as u16);
                let (c, d) = (a + SKY_SEGS as u16, b + SKY_SEGS as u16);
                for i in [a, b, d, a, d, c] {
                    *sky_ib.add(k) = i;
                    k += 1;
                }
            }
        }
        let sun_vb = alloc(2 * (FAN + 1) * 16)?.cast::<ColorVertex>();
        let ax = scene.sun_dir.cross(V3::UP).norm_or(v3(1.0, 0.0, 0.0));
        let ay = scene.sun_dir.cross(ax);
        for (d, (radius, alpha)) in [(260.0f32, 70u8), (70.0, 255)].into_iter().enumerate() {
            let c = scene.sun_dir * 1900.0;
            let warm = [scene.encode(1.0), scene.encode(0.9), scene.encode(0.72)];
            *sun_vb.add(d * (FAN + 1)) = ColorVertex { pos: [c.x, c.y, c.z], color: [warm[0], warm[1], warm[2], alpha] };
            for t in 0..FAN {
                let a = t as f32 / FAN as f32 * TAU;
                let q = c + ax * (cos(a) * radius) + ay * (sin(a) * radius);
                *sun_vb.add(d * (FAN + 1) + 1 + t) = ColorVertex { pos: [q.x, q.y, q.z], color: [warm[0], warm[1], warm[2], 0] };
            }
        }

        let cloak_ib = alloc(cloak_idx * 2)?.cast::<u16>();
        let mut k = 0;
        for r in 0..CLOAK_H - 1 {
            for c in 0..CLOAK_W - 1 {
                let a = (r * CLOAK_W + c) as u16;
                let w = CLOAK_W as u16;
                for i in [a, a + w, a + 1, a + 1, a + w, a + w + 1] {
                    *cloak_ib.add(k) = i;
                    k += 1;
                }
            }
        }
        let quad_ib = alloc(QUADS * 12)?.cast::<u16>();
        for q in 0..QUADS {
            let b = (q * 4) as u16;
            for (j, o) in [0u16, 1, 2, 0, 2, 3].iter().enumerate() {
                *quad_ib.add(q * 6 + j) = b + o;
            }
        }
        let fan_ib = alloc(DISCS * FAN * 6)?.cast::<u16>();
        for d in 0..DISCS {
            let b = (d * (FAN + 1)) as u16;
            for t in 0..FAN {
                *fan_ib.add((d * FAN + t) * 3) = b;
                *fan_ib.add((d * FAN + t) * 3 + 1) = b + 1 + t as u16;
                *fan_ib.add((d * FAN + t) * 3 + 2) = b + 1 + ((t + 1) % FAN) as u16;
            }
        }
        let titan_vis = sim.dummies.iter().map(|d| if sim.world.raycast(d.pos + v3(0.0, d.height * 0.7, 0.0) + scene.sun_dir * (d.height * 0.3), scene.sun_dir, 500.0, mask::ALL).is_none() { 1.0 } else { 0.25 }).collect();

        Ok(Actors {
            _block: block,
            sky_vb: sky_vb.cast(),
            sky_ib,
            sky_idx: sky_idx as u32,
            sun_vb: sun_vb.cast(),
            scout,
            titans,
            titan_vis,
            cloak_ib,
            cloak_idx: cloak_idx as u32,
            quad_ib,
            fan_ib,
            puffs: [Puff { pos: V3::ZERO, vel: V3::ZERO, age: 9.0, life: 0.7, size: 1.0 }; PUFFS],
            next_puff: 0,
            vis: 1.0,
        })
    }

    /// Bytes `update` takes from the ring.
    pub fn frame_bytes(&self) -> usize {
        (CLOAK_N + 2 * (ROPE_N - 1) * 4 + DISCS * (FAN + 1)) * 16 + 256
    }

    fn puff(&mut self, pos: V3, vel: V3, life: f32, size: f32) {
        let j = self.next_puff % PUFFS;
        self.next_puff += 1;
        self.puffs[j] = Puff { pos, vel, age: 0.0, life, size };
    }

    /// Writes this frame's CPU geometry into the ring: the cloak, the wires and the soft discs.
    ///
    /// # Safety
    /// The ring segment is not in use by the GPU.
    pub unsafe fn update(&mut self, sim: &Sim, scene: &Scene, ring: &mut Ring, eye: V3, ticks: u32) -> Option<Frame> {
        let dt = ticks as f32 * maneuver_sim::sim::DT;
        // The character darkens in shade: one ray toward the sun.
        let lit = sim.world.raycast(sim.p.pos + v3(0.0, 0.6, 0.0), scene.sun_dir, 400.0, mask::ALL).is_none();
        self.vis = ease(self.vis, if lit { 1.0 } else { 0.0 }, 10.0, max(dt, 1.0 / 60.0));

        // Cloak: the simulation's cloth, lit on whichever side faces the sun.
        let tint = CLOAK.map(|c| libm::powf(c, 2.2));
        let cloak_vb = ring.alloc(CLOAK_N * 16, 16)?.cast::<ColorVertex>();
        for i in 0..CLOAK_N {
            let p = sim.pose.cloak.p[i];
            let n = sim.pose.cloak.n[i];
            let front = n.dot(scene.sun_dir) >= 0.0;
            let color = scene.shade(if front { n } else { -n }, tint, if front { self.vis } else { self.vis * 0.6 });
            *cloak_vb.add(i) = ColorVertex { pos: [p.x, p.y, p.z], color };
        }

        // Wires: a ribbon along each rope, turned to the eye, at least a pixel wide.
        let rope_vb = ring.alloc(2 * (ROPE_N - 1) * 4 * 16, 16)?.cast::<ColorVertex>();
        let mut quads = 0usize;
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
                for (j, q) in [a - sa, b - sb, b + sb, a + sa].into_iter().enumerate() {
                    *rope_vb.add(quads * 4 + j) = ColorVertex { pos: [q.x, q.y, q.z], color };
                }
                quads += 1;
            }
        }

        // Gas leaves the hips while it burns; a fallen giant steams.
        if sim.p.thrusting || sim.p.reeling {
            for i in 0..2 {
                let j = self.next_puff;
                let jitter = v3(sin(j as f32 * 12.99) * 1.4, cos(j as f32 * 7.31) * 1.4, sin(j as f32 * 3.7) * 1.4);
                self.puff(sim.pose.hip[i] + v3(0.0, -0.1, 0.0), sim.p.vel * 0.35 + jitter, 0.7, 1.0);
            }
        }
        for (i, d) in sim.dummies.iter().enumerate() {
            let since = sim.tick.wrapping_sub(d.cut_tick) as f32 / 60.0;
            if d.alive || since > 8.0 || (sim.tick + i as u32) % 3 != 0 || (d.pos - eye).len2() > 400.0 * 400.0 {
                continue;
            }
            let j = self.next_puff as f32;
            let fwd = heading(d.yaw);
            let along = d.height * (0.15 + 0.8 * (sin(j * 2.3) * 0.5 + 0.5)) * saturate(since / 1.5);
            let pos = d.pos + fwd * along + v3(sin(j * 5.1) * d.height * 0.15, d.height * 0.12, cos(j * 3.3) * d.height * 0.15);
            self.puff(pos, v3(sin(j) * 1.5, 4.0 + d.height * 0.2, cos(j * 1.7) * 1.5), 1.6, d.height * 0.22);
        }

        let disc_vb = ring.alloc(DISCS * (FAN + 1) * 16, 16)?.cast::<ColorVertex>();
        let mut discs = 0usize;
        let mut disc = |c: V3, ax: V3, ay: V3, radius: f32, color: [u8; 4]| {
            let v = disc_vb.add(discs * (FAN + 1));
            *v = ColorVertex { pos: [c.x, c.y, c.z], color };
            for t in 0..FAN {
                let a = t as f32 / FAN as f32 * TAU;
                let q = c + ax * (cos(a) * radius) + ay * (sin(a) * radius);
                *v.add(1 + t) = ColorVertex { pos: [q.x, q.y, q.z], color: [color[0], color[1], color[2], 0] };
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

        Some(Frame { cloak_vb: cloak_vb.cast(), rope_vb: rope_vb.cast(), rope_quads: quads as u32, disc_vb: disc_vb.cast(), discs: discs as u32 })
    }

    /// The sky, first in the frame: centred on the eye, no depth, no haze.
    pub unsafe fn draw_sky(&self, ctx: *mut g::SceGxmContext, prog: &Program, vp: &Mat4, eye: V3) {
        prog.bind(ctx, false);
        gpu::state_overlay(ctx, false);
        let m = mat::translated(vp, eye);
        prog.uniforms(ctx, &m, 0.0);
        gpu::draw(ctx, self.sky_vb, self.sky_ib, self.sky_idx);
        prog.bind(ctx, true);
        prog.uniforms(ctx, &m, 0.0);
        gpu::draw(ctx, self.sun_vb, self.fan_ib, 2 * (FAN * 3) as u32);
    }

    /// The player and the giants in range. Returns draws and triangles.
    pub unsafe fn draw_skinned(&self, ctx: *mut g::SceGxmContext, prog: &Program, vp: &Mat4, sim: &Sim, scene: &Scene, eye: V3, cull_cw: bool, character: bool) -> (u32, u32) {
        prog.bind(ctx, false);
        gpu::state_opaque(ctx, cull_cw);
        let planes = mat::planes(vp);
        let mut rows = [0.0f32; BONES * 12];
        let (mut draws, mut tris) = (0, 0);
        if character {
            bone_rows(&sim.pose.skin, 0.0, &mut rows);
            prog.skin_uniforms(ctx, vp, &rows, &scene.light(self.vis), scene.fog_density);
            gpu::draw(ctx, self.scout.vb, self.scout.ib, self.scout.idx);
            draws += 1;
            tris += self.scout.idx / 3;
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
            let mesh = self.titans[(d.variant % 3) as usize][if dist < TITAN_NEAR { 0 } else { 1 }];
            // A fallen giant sinks away.
            let sink = if d.alive { 0.0 } else { max(since - 5.0, 0.0) * d.height * 0.08 };
            bone_rows(&sim.titan_skin(i), sink, &mut rows);
            prog.skin_uniforms(ctx, vp, &rows, &scene.light(self.titan_vis[i]), scene.fog_density);
            gpu::draw(ctx, mesh.vb, mesh.ib, mesh.idx);
            draws += 1;
            tris += mesh.idx / 3;
        }
        (draws, tris)
    }

    /// The cloak and the wires: lit on the CPU, drawn from both sides.
    pub unsafe fn draw_cloth(&self, ctx: *mut g::SceGxmContext, prog: &Program, vp: &Mat4, f: &Frame, fog: f32, character: bool) {
        prog.bind(ctx, false);
        gpu::state_opaque(ctx, true);
        g::sceGxmSetCullMode(ctx, g::SceGxmCullMode_SCE_GXM_CULL_NONE);
        if character {
            prog.uniforms(ctx, vp, fog);
            gpu::draw(ctx, f.cloak_vb, self.cloak_ib, self.cloak_idx);
        }
        if f.rope_quads > 0 {
            prog.uniforms(ctx, vp, fog);
            gpu::draw(ctx, f.rope_vb, self.quad_ib, f.rope_quads * 6);
        }
    }

    /// The ground shadow, the gas and the steam, blended over the scene.
    pub unsafe fn draw_blend(&self, ctx: *mut g::SceGxmContext, prog: &Program, vp: &Mat4, f: &Frame, fog: f32) {
        if f.discs == 0 {
            return;
        }
        prog.bind(ctx, true);
        gpu::state_overlay(ctx, true);
        prog.uniforms(ctx, vp, fog);
        gpu::draw(ctx, f.disc_vb, self.fan_ib, f.discs * (FAN * 3) as u32);
    }
}
