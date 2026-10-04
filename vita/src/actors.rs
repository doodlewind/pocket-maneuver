//! Everything that is not the baked world: the sky dome, the character's
//! thirteen parts, the targets, the wires and the gas. Vertices are lit and
//! placed on the CPU into 16-byte records (position, colour); the character
//! is about 700 of them.

use maneuver_pack::{self as pack, ModelHeader, ModelVertex, Pack};
use maneuver_sim::collide::mask;
use maneuver_sim::math::*;
use maneuver_sim::pose::PARTS;
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
const PUFFS: usize = 72;
/// A soft disc: a centre and `FAN` rim vertices, opaque in the middle and clear at the rim.
const FAN: usize = 12;
/// Discs per frame: the gas puffs and the character's ground shadow.
const DISCS: usize = PUFFS + 1;
const SKY_SEGS: usize = 24;
const SKY_RINGS: usize = 13;

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

struct Model {
    /// Position, normal, linear tint.
    verts: Vec<(V3, V3, [f32; 3])>,
    idx: Vec<u16>,
}

fn models(p: &Pack) -> Result<Vec<(u32, Model)>, String> {
    let b = p.section(pack::MODL)?;
    let n: u32 = pack::read(b, 0).ok_or("model count")?;
    let mut at = 4;
    let mut out = Vec::new();
    for _ in 0..n {
        let h: ModelHeader = pack::read(b, at).ok_or("model header")?;
        at += core::mem::size_of::<ModelHeader>();
        let mut verts = Vec::with_capacity(h.vtx_count as usize);
        for _ in 0..h.vtx_count {
            let v: ModelVertex = pack::read(b, at).ok_or("model vertex")?;
            at += core::mem::size_of::<ModelVertex>();
            let lin = |c: u8| libm::powf(c as f32 / 255.0, 2.2);
            verts.push((v3(v.pos[0], v.pos[1], v.pos[2]), v3(v.normal[0], v.normal[1], v.normal[2]), [lin(v.color[0]), lin(v.color[1]), lin(v.color[2])]));
        }
        let mut idx = Vec::with_capacity(h.idx_count as usize);
        for _ in 0..h.idx_count {
            idx.push(pack::read::<u16>(b, at).ok_or("model index")?);
            at += 2;
        }
        at = (at + 3) & !3;
        out.push((h.id, Model { verts, idx }));
    }
    Ok(out)
}

#[derive(Clone, Copy)]
struct Puff {
    pos: V3,
    vel: V3,
    age: f32,
}

/// What `update` wrote for this frame.
pub struct Frame {
    char_vb: *const u8,
    nape_vb: *const u8,
    nape_ib: *const u16,
    nape_idx: u32,
    wire_vb: *const u8,
    wire_quads: u32,
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
    body_vb: *const u8,
    body_ib: *const u16,
    body_idx: u32,
    char_ib: *const u16,
    char_idx: u32,
    char_verts: usize,
    /// Shared indices for quads: 0 1 2, 0 2 3, per four vertices.
    pub quad_ib: *const u16,
    /// Shared indices for discs of `FAN + 1` vertices.
    fan_ib: *const u16,
    parts: Vec<Model>,
    nape: Model,
    puffs: [Puff; PUFFS],
    next_puff: usize,
    vis: f32,
}

impl Actors {
    /// # Safety
    /// GXM is initialized.
    pub unsafe fn load(p: &Pack, scene: &Scene, sim: &Sim) -> Result<Actors, String> {
        let mut all = models(p)?;
        let take = |all: &mut Vec<(u32, Model)>, id: u32| all.iter().position(|m| m.0 == id).map(|i| all.swap_remove(i).1).ok_or(format!("pack has no model {id}"));
        let body = take(&mut all, 100)?;
        let nape = take(&mut all, 101)?;
        let mut parts = Vec::new();
        for i in 0..PARTS as u32 {
            parts.push(take(&mut all, i)?);
        }

        let sky_verts = SKY_RINGS * SKY_SEGS;
        let sky_idx = (SKY_RINGS - 1) * SKY_SEGS * 6;
        let body_verts = body.verts.len() * sim.dummies.len();
        let body_idx = body.idx.len() * sim.dummies.len();
        let char_verts: usize = parts.iter().map(|m| m.verts.len()).sum();
        let char_idx: usize = parts.iter().map(|m| m.idx.len()).sum();
        if body_verts > 65535 || char_verts > 65535 {
            return Err("too many model vertices for 16-bit indices".into());
        }
        let size = (sky_verts + body_verts + 2 * (FAN + 1)) * 16 + (sky_idx + body_idx + char_idx + QUADS * 6 + DISCS * FAN * 3) * 2 + 256;
        let mut block = Block::with_access(Kind::Main, size, false)?;
        let mut alloc = |bytes: usize| block.alloc(bytes, 16).ok_or("actor geometry block".to_string());

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

        // Targets: every body in world space, lit once.
        let body_vb = alloc(body_verts.max(1) * 16)?.cast::<ColorVertex>();
        let body_ib = alloc(body_idx.max(1) * 2)?.cast::<u16>();
        for (di, d) in sim.dummies.iter().enumerate() {
            let rot = M3::rot_y(d.yaw);
            let vis = if sim.world.raycast(d.pos + v3(0.0, d.height * 0.7, 0.0) + scene.sun_dir * 1.5, scene.sun_dir, 500.0, mask::ALL).is_none() { 1.0 } else { 0.0 };
            for (vi, (pos, n, tint)) in body.verts.iter().enumerate() {
                let w = rot.apply(*pos * d.height) + d.pos;
                *body_vb.add(di * body.verts.len() + vi) = ColorVertex { pos: [w.x, w.y, w.z], color: scene.shade(rot.apply(*n), *tint, vis) };
            }
            for (ii, i) in body.idx.iter().enumerate() {
                *body_ib.add(di * body.idx.len() + ii) = (di * body.verts.len()) as u16 + i;
            }
        }

        let char_ib = alloc(char_idx * 2)?.cast::<u16>();
        let (mut vi, mut ii) = (0usize, 0usize);
        for m in &parts {
            for i in &m.idx {
                *char_ib.add(ii) = vi as u16 + i;
                ii += 1;
            }
            vi += m.verts.len();
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

        Ok(Actors {
            _block: block,
            sky_vb: sky_vb.cast(),
            sky_ib,
            sky_idx: sky_idx as u32,
            sun_vb: sun_vb.cast(),
            body_vb: body_vb.cast(),
            body_ib,
            body_idx: body_idx as u32,
            char_ib,
            char_idx: char_idx as u32,
            char_verts,
            quad_ib,
            fan_ib,
            parts,
            nape,
            puffs: [Puff { pos: V3::ZERO, vel: V3::ZERO, age: 9.0 }; PUFFS],
            next_puff: 0,
            vis: 1.0,
        })
    }

    /// Bytes `update` takes from the ring.
    pub fn frame_bytes(&self, dummies: usize) -> usize {
        (self.char_verts + dummies * self.nape.verts.len() + 8 + DISCS * (FAN + 1)) * 16 + dummies * self.nape.idx.len() * 2 + 64
    }

    /// Writes this frame's moving geometry into the ring.
    ///
    /// # Safety
    /// The ring segment is not in use by the GPU.
    pub unsafe fn update(&mut self, sim: &Sim, scene: &Scene, ring: &mut Ring, eye: V3, ticks: u32) -> Option<Frame> {
        let dt = ticks as f32 * maneuver_sim::sim::DT;
        // The character darkens in shade: one ray toward the sun.
        let lit = sim.world.raycast(sim.p.pos + v3(0.0, 0.6, 0.0), scene.sun_dir, 400.0, mask::ALL).is_none();
        self.vis = ease(self.vis, if lit { 1.0 } else { 0.0 }, 10.0, max(dt, 1.0 / 60.0));

        let char_vb = ring.alloc(self.char_verts * 16, 16)?.cast::<ColorVertex>();
        let mut k = 0;
        for (part, m) in self.parts.iter().enumerate() {
            let t = &sim.pose.parts[part];
            for (pos, n, tint) in &m.verts {
                let w = t.apply(*pos);
                *char_vb.add(k) = ColorVertex { pos: [w.x, w.y, w.z], color: scene.shade(t.r.apply(*n), *tint, self.vis) };
                k += 1;
            }
        }

        // Napes: on the target while it stands, falling for a moment once cut.
        let nv = self.nape.verts.len();
        let ni = self.nape.idx.len();
        let nape_vb = ring.alloc(sim.dummies.len().max(1) * nv * 16, 16)?.cast::<ColorVertex>();
        let nape_ib = ring.alloc(sim.dummies.len().max(1) * ni * 2, 16)?.cast::<u16>();
        let mut drawn = 0usize;
        for d in &sim.dummies {
            let since = sim.tick.wrapping_sub(d.cut_tick) as f32 / 60.0;
            if !d.alive && since > 2.5 {
                continue;
            }
            if (d.pos - eye).len2() > 600.0 * 600.0 {
                continue;
            }
            let (drop, spin) = if d.alive { (0.0, 0.0) } else { (4.9 * since * since, since * 4.0) };
            let rot = M3::rot_y(d.yaw).mul(&M3::rot_x(spin));
            let base = d.pos - v3(0.0, drop, 0.0);
            for (vi, (pos, n, tint)) in self.nape.verts.iter().enumerate() {
                let w = rot.apply(*pos * d.height) + base;
                *nape_vb.add(drawn * nv + vi) = ColorVertex { pos: [w.x, w.y, w.z], color: scene.shade(rot.apply(*n), *tint, 1.0) };
            }
            for (ii, i) in self.nape.idx.iter().enumerate() {
                *nape_ib.add(drawn * ni + ii) = (drawn * nv) as u16 + i;
            }
            drawn += 1;
        }

        // Wires: a ribbon from each hip to its hook, facing the eye, at least a pixel wide.
        let wire_vb = ring.alloc(8 * 16, 16)?.cast::<ColorVertex>();
        let mut wires = 0u32;
        for i in 0..2 {
            let h = &sim.p.hooks[i];
            if h.state == hook::IDLE {
                continue;
            }
            let (a, b) = (sim.hip(i), h.tip);
            let mid = (a + b) * 0.5;
            let side = (b - a).cross(mid - eye).norm_or(V3::UP);
            let color = [34, 34, 38, 255];
            for (j, (p, s)) in [(a, -1.0f32), (b, -1.0), (b, 1.0), (a, 1.0)].into_iter().enumerate() {
                let w = max(0.022, (p - eye).len() * 0.0011);
                let q = p + side * (s * w);
                *wire_vb.add(wires as usize * 4 + j) = ColorVertex { pos: [q.x, q.y, q.z], color };
            }
            wires += 1;
        }

        // Gas: puffs leave the hips while it is burning.
        if sim.p.thrusting || sim.p.reeling {
            for i in 0..2 {
                let j = self.next_puff % PUFFS;
                self.next_puff += 1;
                let jitter = v3(sin(j as f32 * 12.99) * 1.4, cos(j as f32 * 7.31) * 1.4, sin(j as f32 * 3.7) * 1.4);
                self.puffs[j] = Puff { pos: sim.hip(i) - v3(0.0, 0.25, 0.0), vel: sim.p.vel * 0.35 + jitter, age: 0.0 };
            }
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
            if p.age > 0.7 {
                continue;
            }
            p.age += dt;
            p.pos += p.vel * dt;
            let k = p.age / 0.7;
            disc(p.pos, right, up, 0.22 + k * 1.1, [240, 243, 246, ((1.0 - k) * 150.0) as u8]);
        }

        Some(Frame { char_vb: char_vb.cast(), nape_vb: nape_vb.cast(), nape_ib, nape_idx: (drawn * ni) as u32, wire_vb: wire_vb.cast(), wire_quads: wires, disc_vb: disc_vb.cast(), discs: discs as u32 })
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

    /// Targets, the character and the wires. Returns draws and triangles.
    pub unsafe fn draw_opaque(&self, ctx: *mut g::SceGxmContext, prog: &Program, vp: &Mat4, f: &Frame, fog: f32, cull_cw: bool, character: bool) -> (u32, u32) {
        prog.bind(ctx, false);
        gpu::state_opaque(ctx, cull_cw);
        let mut tris = self.char_idx / 3;
        let mut draws = 1;
        if self.body_idx > 0 {
            prog.uniforms(ctx, vp, fog);
            gpu::draw(ctx, self.body_vb, self.body_ib, self.body_idx);
            tris += self.body_idx / 3;
            draws += 1;
        }
        if f.nape_idx > 0 {
            prog.uniforms(ctx, vp, fog);
            gpu::draw(ctx, f.nape_vb, f.nape_ib, f.nape_idx);
            tris += f.nape_idx / 3;
            draws += 1;
        }
        if character {
            prog.uniforms(ctx, vp, fog);
            gpu::draw(ctx, f.char_vb, self.char_ib, self.char_idx);
        }
        if f.wire_quads > 0 {
            g::sceGxmSetCullMode(ctx, g::SceGxmCullMode_SCE_GXM_CULL_NONE);
            prog.uniforms(ctx, vp, fog);
            gpu::draw(ctx, f.wire_vb, self.quad_ib, f.wire_quads * 6);
            draws += 1;
        }
        (draws, tris)
    }

    /// The ground shadow and the gas, blended over the scene.
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
