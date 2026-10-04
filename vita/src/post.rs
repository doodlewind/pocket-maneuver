//! Post-processing. The scene renders into an off-screen target; a
//! quarter-size chain extracts what is bright, blurs it and smears it toward
//! the sun; one full-screen pass composes and grades the frame onto the
//! display. Every tap's texture coordinate comes from the vertex stage, so no
//! fragment program computes one.

use maneuver_sim::math::V3;
use pocket_vita_gxm::mem::{Arena, Block, Kind};
use pocket_vita_gxm::program::F32;
use pocket_vita_gxm::target::{ColorFormat, Depth, Msaa, Target};
use vita2d_sys as g;

use crate::gpu::{self, Gpu, Layout, Program};
use crate::mat::Mat4;

const POST_V: &str = include_str!("../shaders/post_v.cg");
const BRIGHT_F: &str = include_str!("../shaders/bright_f.cg");
const BLUR_F: &str = include_str!("../shaders/blur_f.cg");
const RAYS_F: &str = include_str!("../shaders/rays_f.cg");
const COMPOSITE_F: &str = include_str!("../shaders/composite_f.cg");

pub const W: u32 = 960;
pub const H: u32 = 544;
/// The bright chain runs at a quarter of the frame.
const QW: u32 = 240;
const QH: u32 = 136;

#[derive(Clone, Copy)]
pub struct Look {
    pub bloom: bool,
    pub rays: bool,
    pub speed: bool,
    pub threshold: f32,
    pub bloom_gain: f32,
    pub rays_gain: f32,
    pub vignette: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub warm: f32,
    pub cool: f32,
}

impl Look {
    pub const DEFAULT: Look = Look { bloom: true, rays: true, speed: true, threshold: 0.86, bloom_gain: 0.7, rays_gain: 0.9, vignette: 0.24, contrast: 1.1, saturation: 1.16, warm: 0.05, cool: 0.06 };
}

struct Pass {
    prog: Program,
    tap: *const g::SceGxmProgramParameter,
    params: [*const g::SceGxmProgramParameter; 2],
    samplers: [Option<u32>; 3],
}

impl Pass {
    unsafe fn new(gpu: &mut Gpu, name: &str, defines: &str, fs: &str, uniforms: [&str; 2], samplers: [&str; 3]) -> Result<Pass, String> {
        let prog = gpu.program(name, defines, POST_V, fs, &Layout { attrs: &[("aPosition", 0, F32, 2)], stride: 8 }, 0)?;
        let tap = prog.vs.param("uTap");
        let params = uniforms.map(|u| if u.is_empty() { core::ptr::null() } else { prog.fs.param(u) });
        let samplers = samplers.map(|s| if s.is_empty() { None } else { prog.fs.sampler_index(s) });
        Ok(Pass { prog, tap, params, samplers })
    }

    /// Draws the full-screen triangle with this pass's taps, fragment uniforms and textures.
    unsafe fn draw(&self, ctx: *mut g::SceGxmContext, tri: (*const u8, *const u16), taps: &[f32; 32], uniforms: [&[f32]; 2], textures: [Option<&g::SceGxmTexture>; 3]) {
        self.prog.bind(ctx, false);
        gpu::state_overlay(ctx, false);
        let mut vbuf = core::ptr::null_mut();
        g::sceGxmReserveVertexDefaultUniformBuffer(ctx, &mut vbuf);
        if !vbuf.is_null() && !self.tap.is_null() {
            g::sceGxmSetUniformDataF(vbuf, self.tap, 0, 32, taps.as_ptr());
        }
        if self.params.iter().any(|p| !p.is_null()) {
            let mut fbuf = core::ptr::null_mut();
            g::sceGxmReserveFragmentDefaultUniformBuffer(ctx, &mut fbuf);
            for (p, v) in self.params.iter().zip(uniforms) {
                if !p.is_null() && !fbuf.is_null() && !v.is_empty() {
                    g::sceGxmSetUniformDataF(fbuf, *p, 0, v.len() as u32, v.as_ptr());
                }
            }
        }
        for (unit, tex) in self.samplers.iter().zip(textures) {
            if let (Some(unit), Some(tex)) = (unit, tex) {
                g::sceGxmSetFragmentTexture(ctx, *unit, tex);
            }
        }
        gpu::draw(ctx, tri.0, tri.1, 3);
    }
}

/// Eight identity taps.
fn taps() -> [f32; 32] {
    let mut t = [0.0; 32];
    for k in 0..8 {
        t[k * 4] = 1.0;
        t[k * 4 + 1] = 1.0;
    }
    t
}

pub struct Post {
    _block: Block,
    tri: (*const u8, *const u16),
    pub scene: Target,
    a: Target,
    b: Target,
    rays: Target,
    bright: Pass,
    blur: Pass,
    shafts: Pass,
    plain: Pass,
    glow: Pass,
    speed: Pass,
    /// Whether the shafts target holds this frame's shafts.
    lit: bool,
}

unsafe fn viewport(ctx: *mut g::SceGxmContext, w: u32, h: u32) {
    let (hw, hh) = (w as f32 * 0.5, h as f32 * 0.5);
    g::sceGxmSetViewport(ctx, hw, hw, hh, -hh, 0.0, 1.0);
    g::sceGxmSetRegionClip(ctx, g::SceGxmRegionClipMode_SCE_GXM_REGION_CLIP_OUTSIDE, 0, 0, w - 1, h - 1);
}

impl Post {
    /// # Safety
    /// GXM is initialized; the arenas outlive the targets.
    pub unsafe fn new(gpu: &mut Gpu, vram: &mut Arena, main: &mut Arena, defines: &str, msaa: Msaa) -> Result<Post, String> {
        let mut block = Block::with_access(Kind::Main, 256, false)?;
        let vb = block.alloc(24, 16).ok_or("post triangle")?.cast::<f32>();
        let ib = block.alloc(8, 16).ok_or("post triangle")?.cast::<u16>();
        for (i, v) in [-1.0f32, -1.0, 3.0, -1.0, -1.0, 3.0].into_iter().enumerate() {
            *vb.add(i) = v;
        }
        for i in 0..3 {
            *ib.add(i) = i as u16;
        }
        let scene = Target::new(vram, main, W, H, ColorFormat::Rgba8, msaa, Depth::Transient)?;
        let a = Target::new(vram, main, QW, QH, ColorFormat::Rgba8, Msaa::None, Depth::None)?;
        let b = Target::new(vram, main, QW, QH, ColorFormat::Rgba8, Msaa::None, Depth::None)?;
        let rays = Target::new(vram, main, QW, QH, ColorFormat::Rgba8, Msaa::None, Depth::None)?;
        Ok(Post {
            tri: (vb.cast(), ib),
            _block: block,
            scene,
            a,
            b,
            rays,
            bright: Pass::new(gpu, "bright", defines, BRIGHT_F, ["uParams", ""], ["uSource", "", ""])?,
            blur: Pass::new(gpu, "blur", defines, BLUR_F, ["", ""], ["uSource", "", ""])?,
            shafts: Pass::new(gpu, "rays", defines, RAYS_F, ["", ""], ["uSource", "", ""])?,
            plain: Pass::new(gpu, "composite", defines, COMPOSITE_F, ["uGlow", "uGrade"], ["uScene", "", ""])?,
            glow: Pass::new(gpu, "composite_glow", &format!("{defines}#define GLOW\n"), COMPOSITE_F, ["uGlow", "uGrade"], ["uScene", "uBloom", "uRays"])?,
            speed: Pass::new(gpu, "composite_speed", &format!("{defines}#define GLOW\n#define SPEED\n"), COMPOSITE_F, ["uGlow", "uGrade"], ["uScene", "uBloom", "uRays"])?,
            lit: false,
        })
    }

    /// Opens the scene pass. Depth starts at the far plane.
    pub unsafe fn begin_scene(&mut self, ctx: *mut g::SceGxmContext) -> Result<(), String> {
        self.scene.begin(ctx, 1.0)?;
        viewport(ctx, W, H);
        Ok(())
    }

    /// Closes the scene pass and runs the quarter-size chain: bright, blur across, blur down, shafts.
    pub unsafe fn finish_scene(&mut self, ctx: *mut g::SceGxmContext, look: &Look, vp: &Mat4, eye: V3, sun: V3) -> Result<(), String> {
        self.scene.end(ctx, None);
        self.lit = false;
        if !look.bloom {
            return Ok(());
        }
        // Bright: four taps one source texel off the centre, each a bilinear 2 x 2.
        let mut t = taps();
        let (tx, ty) = (1.0 / W as f32, 1.0 / H as f32);
        for (k, (sx, sy)) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)].into_iter().enumerate() {
            t[k * 4 + 2] = sx * tx;
            t[k * 4 + 3] = sy * ty;
        }
        self.a.begin(ctx, 1.0)?;
        viewport(ctx, QW, QH);
        self.bright.draw(ctx, self.tri, &t, [&[look.threshold, 1.0 / (1.0 - look.threshold).max(0.05), 0.0, 0.0], &[]], [Some(&self.scene.texture), None, None]);
        self.a.end(ctx, None);
        // Blur: across into b, down back into a.
        for (from_a, (dx, dy)) in [(true, (1.0 / QW as f32, 0.0)), (false, (0.0, 1.0 / QH as f32))] {
            let mut t = taps();
            for (k, o) in [0.0f32, 1.3846, -1.3846, 3.2308, -3.2308].into_iter().enumerate() {
                t[k * 4 + 2] = o * dx;
                t[k * 4 + 3] = o * dy;
            }
            let (src, dst) = if from_a { (&self.a.texture as *const g::SceGxmTexture, &mut self.b) } else { (&self.b.texture as *const g::SceGxmTexture, &mut self.a) };
            dst.begin(ctx, 1.0)?;
            viewport(ctx, QW, QH);
            self.blur.draw(ctx, self.tri, &t, [&[], &[]], [Some(&*src), None, None]);
            dst.end(ctx, None);
        }
        // Shafts, when the sun is in front of the eye and near the frame.
        if look.rays {
            let p = eye + sun * 1000.0;
            let w = vp[12] * p.x + vp[13] * p.y + vp[14] * p.z + vp[15];
            if w > 1.0 {
                let u = (vp[0] * p.x + vp[1] * p.y + vp[2] * p.z + vp[3]) / w * 0.5 + 0.5;
                let v = 0.5 - (vp[4] * p.x + vp[5] * p.y + vp[6] * p.z + vp[7]) / w * 0.5;
                if (-0.4..1.4).contains(&u) && (-0.4..1.4).contains(&v) {
                    // Each tap is the pixel moved a step toward the sun: uv' = sun + (uv - sun) * s.
                    let mut t = taps();
                    for k in 0..8 {
                        let s = 1.0 - k as f32 * 0.062;
                        t[k * 4] = s;
                        t[k * 4 + 1] = s;
                        t[k * 4 + 2] = u * (1.0 - s);
                        t[k * 4 + 3] = v * (1.0 - s);
                    }
                    self.rays.begin(ctx, 1.0)?;
                    viewport(ctx, QW, QH);
                    self.shafts.draw(ctx, self.tri, &t, [&[], &[]], [Some(&self.a.texture), None, None]);
                    self.rays.end(ctx, None);
                    self.lit = true;
                }
            }
        }
        Ok(())
    }

    /// Composes the frame onto the open display scene. `fast` is 0 at rest and 1 at full speed.
    pub unsafe fn composite(&self, ctx: *mut g::SceGxmContext, look: &Look, fast: f32) {
        viewport(ctx, W, H);
        let mut t = taps();
        // Tap 1: the screen as -1..1.
        t[4] = 2.0;
        t[5] = 2.0;
        t[6] = -1.0;
        t[7] = -1.0;
        // Taps 2..4: the scene pulled toward the centre.
        for (k, s) in [(2usize, 0.985f32), (3, 0.97), (4, 0.955)] {
            let s = 1.0 - (1.0 - s) * fast;
            t[k * 4] = s;
            t[k * 4 + 1] = s;
            t[k * 4 + 2] = 0.5 * (1.0 - s);
            t[k * 4 + 3] = 0.5 * (1.0 - s);
        }
        let glow = [if look.bloom { look.bloom_gain } else { 0.0 }, if self.lit { look.rays_gain } else { 0.0 }, look.vignette, fast * 1.6];
        let grade = [look.contrast, look.saturation, look.warm, look.cool];
        let pass = if !look.bloom { &self.plain } else if look.speed && fast > 0.02 { &self.speed } else { &self.glow };
        pass.draw(ctx, self.tri, &t, [&glow, &grade], [Some(&self.scene.texture), Some(&self.a.texture), Some(&self.rays.texture)]);
    }
}
