//! Programs. The six Cg sources are part of the binary; each compiles once on
//! the device through SceShaccCg and is cached as a GXP keyed by its source.
//! A packaged build ships the GXPs and never loads the compiler.

use pocket_vita_gxm::patcher::Patcher;
use pocket_vita_gxm::program::{self, Attr, Blend, Gxp, Output, Registered};
use pocket_vita_gxm::shacccg::{Compiler, Stage};
use vita2d_sys as g;

use crate::paths;

pub struct Program {
    pub vs: Registered,
    pub fs: Registered,
    pub vp: *mut g::SceGxmVertexProgram,
    pub opaque: *mut g::SceGxmFragmentProgram,
    pub alpha: *mut g::SceGxmFragmentProgram,
    u_mvp: *const g::SceGxmProgramParameter,
    u_fog: *const g::SceGxmProgramParameter,
    u_bones: *const g::SceGxmProgramParameter,
    u_light: *const g::SceGxmProgramParameter,
}

impl Program {
    /// Binds the program; `blend` picks the alpha-blended fragment program.
    pub unsafe fn bind(&self, ctx: *mut g::SceGxmContext, blend: bool) {
        g::sceGxmSetVertexProgram(ctx, self.vp);
        g::sceGxmSetFragmentProgram(ctx, if blend { self.alpha } else { self.opaque });
    }

    /// Reserves this draw's vertex uniforms and writes the matrix (and the haze density, where the program has one).
    #[inline]
    pub unsafe fn uniforms(&self, ctx: *mut g::SceGxmContext, mvp: &[f32; 16], fog: f32) {
        let mut buf = core::ptr::null_mut();
        g::sceGxmReserveVertexDefaultUniformBuffer(ctx, &mut buf);
        if buf.is_null() {
            return;
        }
        if !self.u_mvp.is_null() {
            g::sceGxmSetUniformDataF(buf, self.u_mvp, 0, 16, mvp.as_ptr());
        }
        if !self.u_fog.is_null() {
            let v = [fog, 0.0, 0.0, 0.0];
            g::sceGxmSetUniformDataF(buf, self.u_fog, 0, 4, v.as_ptr());
        }
    }
}

impl Program {
    /// Uniforms of a skinned draw: the view-projection, three rows per bone, the light table and the haze density.
    /// `bones` and `light` are uploaded at the lengths the program declares.
    pub unsafe fn skin_uniforms(&self, ctx: *mut g::SceGxmContext, vp: &[f32; 16], bones: &[f32], light: &[f32; 16], fog: f32) {
        let mut buf = core::ptr::null_mut();
        g::sceGxmReserveVertexDefaultUniformBuffer(ctx, &mut buf);
        if buf.is_null() || self.u_bones.is_null() || self.u_light.is_null() {
            return;
        }
        g::sceGxmSetUniformDataF(buf, self.u_mvp, 0, 16, vp.as_ptr());
        g::sceGxmSetUniformDataF(buf, self.u_bones, 0, bones.len() as u32, bones.as_ptr());
        g::sceGxmSetUniformDataF(buf, self.u_light, 0, 16, light.as_ptr());
        let v = [fog, 0.0, 0.0, 0.0];
        g::sceGxmSetUniformDataF(buf, self.u_fog, 0, 4, v.as_ptr());
    }
}

pub struct Gpu {
    pub patcher: Patcher,
    compiler: Option<Compiler>,
    /// Programs compiled this run (the rest came from the cache).
    pub compiled: u32,
    pub cached: u32,
    /// Hashes of every program in use, for packaging.
    pub manifest: Vec<String>,
    live: bool,
}

fn fnv64(parts: &[&[u8]]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for p in parts {
        for &b in *p {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub struct Layout<'a> {
    pub attrs: &'a [(&'a str, u16, u32, u8)],
    pub stride: u16,
}

impl Gpu {
    pub unsafe fn new(live: bool) -> Result<Gpu, String> {
        let _ = std::fs::create_dir_all(paths::GXP_CACHE);
        Ok(Gpu { patcher: Patcher::new(512 * 1024, 256 * 1024, 512 * 1024)?, compiler: None, compiled: 0, cached: 0, manifest: Vec::new(), live })
    }

    /// The GXP of a source: cached on the memory card, shipped in the package, or compiled now.
    unsafe fn gxp(&mut self, name: &str, source: &str, stage: Stage) -> Result<Vec<u8>, String> {
        let tag: &[u8] = match stage {
            Stage::Vertex => b"v",
            Stage::Fragment => b"f",
        };
        let hash = format!("{:016x}", fnv64(&[source.as_bytes(), tag]));
        self.manifest.push(hash.clone());
        for dir in [paths::GXP_CACHE, paths::GXP_PACKAGED] {
            if let Ok(bytes) = std::fs::read(format!("{dir}/{hash}.gxp")) {
                if bytes.len() > 16 {
                    self.cached += 1;
                    return Ok(bytes);
                }
            }
        }
        if self.compiler.is_none() {
            self.compiler = Some(Compiler::load().map_err(|e| format!("{name}: no cached program and no shader compiler ({e})"))?);
        }
        let out = self.compiler.as_ref().unwrap().compile(name, source, stage, &[]).map_err(|d| {
            let lines: Vec<String> = d.iter().map(|x| format!("{}:{}: {} {}", x.line, x.column, x.level, x.message)).collect();
            format!("{name}: {}", lines.join("; "))
        })?;
        self.compiled += 1;
        let _ = std::fs::write(format!("{}/{hash}.gxp", paths::GXP_CACHE), &out.program);
        if self.live {
            // A copy on the computer, for packaging.
            let _ = crate::hostfs::write(&format!("{}/{hash}.gxp", paths::GXP_HOST), &out.program);
        }
        Ok(out.program)
    }

    /// Compiles (or loads) a vertex and a fragment source and patches them for `layout`.
    pub unsafe fn program(&mut self, name: &str, defines: &str, vs: &str, fs: &str, layout: &Layout, msaa: u32) -> Result<Program, String> {
        let vsrc = format!("{defines}{vs}");
        let fsrc = format!("{defines}{fs}");
        let vbytes = self.gxp(&format!("{name}_v"), &vsrc, Stage::Vertex)?;
        let fbytes = self.gxp(&format!("{name}_f"), &fsrc, Stage::Fragment)?;
        let vs = Registered::new(self.patcher.raw, Gxp::new(&vbytes)?)?;
        let fs = Registered::new(self.patcher.raw, Gxp::new(&fbytes)?)?;
        let mut attrs = Vec::new();
        for (attr, offset, format, count) in layout.attrs {
            let reg = vs.attribute_index(attr).ok_or(format!("{name}: no attribute {attr}"))?;
            attrs.push(Attr { reg, offset: *offset, format: *format, count: *count, stream: 0 });
        }
        let vp = program::vertex_program(self.patcher.raw, &vs, &attrs, &[layout.stride])?;
        let opaque = program::fragment_program(self.patcher.raw, &fs, Output::Uchar4, msaa, Blend::Opaque, vs.program())?;
        let alpha = program::fragment_program(self.patcher.raw, &fs, Output::Uchar4, msaa, Blend::Alpha, vs.program())?;
        let u_mvp = vs.param("uMvp");
        let u_fog = vs.param("uFog");
        let u_bones = vs.param("uBones");
        let u_light = vs.param("uLight");
        Ok(Program { vs, fs, vp, opaque, alpha, u_mvp, u_fog, u_bones, u_light })
    }

    /// The compiler is only needed while programs build.
    pub fn finish(&mut self) {
        if let Some(c) = self.compiler.take() {
            c.unload();
        }
        if self.live {
            let _ = crate::hostfs::write(&format!("{}/manifest.txt", paths::GXP_HOST), self.manifest.join("\n").as_bytes());
        }
    }
}

/// Depth and culling state for opaque world geometry.
pub unsafe fn state_opaque(ctx: *mut g::SceGxmContext, cull_cw: bool) {
    g::sceGxmSetFrontDepthFunc(ctx, g::SceGxmDepthFunc_SCE_GXM_DEPTH_FUNC_LESS_EQUAL);
    g::sceGxmSetFrontDepthWriteEnable(ctx, g::SceGxmDepthWriteMode_SCE_GXM_DEPTH_WRITE_ENABLED);
    g::sceGxmSetCullMode(ctx, if cull_cw { g::SceGxmCullMode_SCE_GXM_CULL_CW } else { g::SceGxmCullMode_SCE_GXM_CULL_CCW });
}

/// Depth-tested, not written, two-sided: wires, gas, the sky.
pub unsafe fn state_overlay(ctx: *mut g::SceGxmContext, depth_test: bool) {
    g::sceGxmSetFrontDepthFunc(ctx, if depth_test { g::SceGxmDepthFunc_SCE_GXM_DEPTH_FUNC_LESS_EQUAL } else { g::SceGxmDepthFunc_SCE_GXM_DEPTH_FUNC_ALWAYS });
    g::sceGxmSetFrontDepthWriteEnable(ctx, g::SceGxmDepthWriteMode_SCE_GXM_DEPTH_WRITE_DISABLED);
    g::sceGxmSetCullMode(ctx, g::SceGxmCullMode_SCE_GXM_CULL_NONE);
}

#[inline]
pub unsafe fn draw(ctx: *mut g::SceGxmContext, vb: *const u8, ib: *const u16, count: u32) {
    g::sceGxmSetVertexStream(ctx, 0, vb.cast());
    g::sceGxmDraw(ctx, g::SceGxmPrimitiveType_SCE_GXM_PRIMITIVE_TRIANGLES, g::SceGxmIndexFormat_SCE_GXM_INDEX_FORMAT_U16, ib.cast(), count);
}
