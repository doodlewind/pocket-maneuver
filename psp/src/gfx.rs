//! The GE renderer.
//!
//! A frame is one display list, built while the GE draws the previous one:
//!
//! 1. the sky, without depth;
//! 2. the far pass: every mesh beyond the near distance and the giants out
//!    there, with a frustum from 0.8 × that distance to the horizon and the far
//!    quarter of the 16-bit depth buffer;
//! 3. the near pass: the cells around the eye, the character and what moves,
//!    with a short frustum and the other three quarters. The GE's depth buffer
//!    cannot resolve 0.4 m to 2.6 km in one range;
//! 4. the marks on the world (where a wire would bite, the nearest target),
//!    then the interface, which PocketJS's GE backend draws from the guest's
//!    list (`interface`).
//!
//! The GE has fixed-function texturing: atlas texel × vertex colour × 2, then
//! linear haze. Skinned models blend up to four bones per draw and take the
//! sun and the sky as two directional lights.

use core::ffi::c_void;
use core::ptr;

use alloc::vec::Vec;
use maneuver_handheld::actors::{self, ColorVertex, Giant};
use maneuver_handheld::clip::{ClipVertex, Guard, Verdict, MAX_OUT};
use maneuver_handheld::game::Game;
use maneuver_handheld::hud::{Font, Hud, HudVertex};
use maneuver_handheld::mat::{self, Mat4};
use maneuver_handheld::world::{Pick, World, NO_CELL};
use maneuver_pack::{self as pack, tex_format, ClipGroup, HandMesh, ModelHeader, PspBatch, PspSkinVertex, PspVertex, TexHeader};
use maneuver_sim::math::*;
use maneuver_sim::pose::{BONES, CLOAK_N};
use psp::sys::*;
use psp::Align16;

use crate::interface::Ui;
use crate::{mem, store};

const LIST_WORDS: usize = 196_608;
static mut LIST: Align16<[u32; LIST_WORDS]> = Align16([0; LIST_WORDS]);

/// One frame buffer: 512 × 272 texels of 16 bits. The colour and depth buffers are all this size.
pub const FB_BYTES: usize = 512 * 272 * 2;
const VRAM_TEXTURES: usize = FB_BYTES * 3;
/// The palettes of an indexed atlas, where the GE reads them: 16-byte aligned main memory.
static mut CLUT: Align16<[[u32; 256]; 4]> = Align16([[0; 256]; 4]);
const VRAM_BYTES: usize = 2 * 1024 * 1024;

/// Quads of one batch: the marks on the world, or the wires.
pub const MAX_QUADS: usize = 224;
/// Which of the two frame buffers the list being built draws into.
pub static mut DRAW_BUFFER: usize = 0;
/// Vertices of CPU-clipped triangles per atlas page and pass.
const CLIP_CAP: usize = 1536;
/// The far pass's share of the depth buffer.
const DEPTH_SPLIT: i32 = 16384;
/// The compiler's reach per metre of a large triangle's longest edge (`CLIP_REACH` in maneuver-cook).
const CLIP_REACH: f32 = 3.3;

fn vtype_world() -> VertexType {
    VertexType::TEXTURE_16BIT | VertexType::COLOR_5650 | VertexType::VERTEX_16BIT | VertexType::INDEX_16BIT | VertexType::TRANSFORM_3D
}
fn vtype_clip() -> VertexType {
    VertexType::TEXTURE_32BITF | VertexType::COLOR_8888 | VertexType::VERTEX_32BITF | VertexType::TRANSFORM_3D
}
fn vtype_color() -> VertexType {
    VertexType::COLOR_8888 | VertexType::VERTEX_32BITF | VertexType::INDEX_16BIT | VertexType::TRANSFORM_3D
}
fn vtype_skin() -> VertexType {
    VertexType::WEIGHTS4 | VertexType::WEIGHT_8BIT | VertexType::COLOR_8888 | VertexType::NORMAL_8BIT | VertexType::VERTEX_32BITF | VertexType::INDEX_16BIT | VertexType::TRANSFORM_3D
}
fn vtype_hud() -> VertexType {
    VertexType::TEXTURE_32BITF | VertexType::COLOR_8888 | VertexType::VERTEX_32BITF | VertexType::INDEX_16BIT | VertexType::TRANSFORM_2D
}

fn abgr(c: [f32; 3]) -> u32 {
    let b = |x: f32| (clamp(x, 0.0, 1.0) * 255.0 + 0.5) as u32;
    0xff00_0000 | (b(c[2]) << 16) | (b(c[1]) << 8) | b(c[0])
}

fn fmatrix(m: &Mat4) -> ScePspFMatrix4 {
    let col = |c: usize| ScePspFVector4 { x: m[c], y: m[4 + c], z: m[8 + c], w: m[12 + c] };
    ScePspFMatrix4 { x: col(0), y: col(1), z: col(2), w: col(3) }
}

struct Batch {
    bones: [u8; pack::PSP_BATCH_BONES],
    bone_count: u32,
    vtx: *const u8,
    idx: *const u16,
    idx_count: u32,
}

#[derive(Default)]
struct Model {
    batches: Vec<Batch>,
}

#[derive(Clone, Copy, Default)]
pub struct Stats {
    pub draws: u32,
    pub tris: u32,
    /// Large triangles tested and cut on the CPU.
    pub tested: u32,
    pub clipped: u32,
    /// Microseconds of the frame's phases: choosing meshes, far meshes, far giants, near models, near meshes, sky and moving geometry, marks, interface.
    pub phase: [u32; 8],
}

pub struct Gfx {
    tex: TexHeader,
    /// VRAM address of each page's levels.
    pages: Vec<Vec<*const u8>>,
    vtx: &'static [u8],
    idx: &'static [u16],
    clip: &'static [u8],
    scout: Model,
    titans: [[Model; 2]; 3],
    font_tex: *const u8,
    pub font: Font,
    sky_vb: Vec<ColorVertex>,
    sky_ib: Vec<u16>,
    sun_vb: Vec<ColorVertex>,
    cloak_ib: Vec<u16>,
    quad_ib: Vec<u16>,
    fan_ib: Vec<u16>,
    far: Vec<Pick>,
    near: Vec<Pick>,
    giants: Vec<Giant>,
    pub stats: Stats,
    /// Meshes chosen by level this frame.
    pub picked: maneuver_handheld::world::Stats,
    /// Leaves every triangle to the GE (`option=4`), to see what its guard band drops.
    no_clip: bool,
    pub resident_bytes: usize,
}

unsafe fn flush<T>(p: *const T, bytes: usize) {
    sceKernelDcacheWritebackRange(p as *const c_void, bytes as u32);
}

/// Starts the GE: two 16-bit frame buffers, the depth buffer, and the state every frame assumes.
pub unsafe fn init() {
    sceGuInit();
    sceGuStart(GuContextType::Direct, ptr::addr_of_mut!(LIST.0) as *mut c_void);
    // 16-bit colour with ordered dither: half the memory traffic of 32-bit per pixel written.
    sceGuDrawBuffer(DisplayPixelFormat::Psm5650, ptr::null_mut(), 512);
    sceGuDispBuffer(480, 272, FB_BYTES as *mut c_void, 512);
    sceGuDepthBuffer((FB_BYTES * 2) as *mut c_void, 512);
    sceGuOffset(2048 - 240, 2048 - 136);
    sceGuViewport(2048, 2048, 480, 272);
    sceGuDepthRange(65535, 0);
    sceGuDepthFunc(DepthFunc::GreaterOrEqual);
    sceGuScissor(0, 0, 480, 272);
    sceGuEnable(GuState::ScissorTest);
    sceGuEnable(GuState::ClipPlanes);
    sceGuFrontFace(FrontFaceDirection::CounterClockwise);
    sceGuShadeModel(ShadingModel::Smooth);
    let row = |x, y, z, w| ScePspIVector4 { x, y, z, w };
    sceGuSetDither(&ScePspIMatrix4 { x: row(-4, 0, -3, 1), y: row(2, -2, 3, -1), z: row(-3, 1, -4, 0), w: row(3, -1, 2, -2) });
    sceGuEnable(GuState::Dither);
    sceGuTexWrap(GuTexWrapMode::Repeat, GuTexWrapMode::Clamp);
    sceGuBlendFunc(BlendOp::Add, BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha, 0, 0);
    sceGuLightMode(LightMode::SingleColor);
    sceGuColorMaterial(LightComponent::AMBIENT | LightComponent::DIFFUSE);
    sceGuClearColor(BACKDROP);
    sceGuClear(ClearBuffer::COLOR_BUFFER_BIT);
    sceGuFinish();
    sceGuSync(GuSyncMode::Finish, GuSyncBehavior::Wait);
    sceDisplayWaitVblankStart();
    sceGuDisplay(true);
}

/// Presents the frame just drawn; the next list draws into the other buffer.
pub unsafe fn swap() {
    sceGuSwapBuffers();
    DRAW_BUFFER ^= 1;
}

/// A frame out of video memory, 480 × 272 texels of 16 bits: host I/O cannot take a video memory
/// address. `buffer` is `DRAW_BUFFER` for the list just drawn and not yet presented.
pub unsafe fn pixels(buffer: usize) -> Vec<u8> {
    let vram = sceGeEdramGetAddr().add(buffer * FB_BYTES);
    let mut out = alloc::vec![0u8; 480 * 272 * 2];
    for y in 0..272 {
        ptr::copy_nonoverlapping(vram.add(y * 512 * 2), out.as_mut_ptr().add(y * 480 * 2), 480 * 2);
    }
    out
}

/// Behind the interface while there is no scene.
const BACKDROP: u32 = 0xff14_100c;

/// A frame of the interface alone, shown at once: the pack is loading, or it could not be.
pub unsafe fn interlude(ui: &Ui) {
    sceGuStart(GuContextType::Direct, ptr::addr_of_mut!(LIST.0) as *mut c_void);
    sceGuClearColor(BACKDROP);
    sceGuClear(ClearBuffer::COLOR_BUFFER_BIT);
    ui.draw();
    sceGuFinish();
    sceGuSync(GuSyncMode::Finish, GuSyncBehavior::Wait);
    pocketjs_psp::ge::reset_pool();
    sceDisplayWaitVblankStart();
    swap();
}

impl Gfx {
    /// Reads the pack's drawing data. The atlas goes to video memory; `init` may run before or after.
    pub unsafe fn load(file: &store::PackFile, scene: &maneuver_handheld::scene::Scene, progress: &mut dyn FnMut(&str)) -> Result<Gfx, &'static str> {
        // The atlas goes to VRAM after the two frame buffers and the depth buffer. It passes through
        // main memory once, in a buffer that is given back.
        progress("Uploading the atlas");
        let tex_size = file.section(pack::TEX0)?.size as usize;
        let pages_wanted = scene.h.pages as usize;
        let (tex, pages) = mem::scratch(tex_size, |tex_bytes| -> Result<(TexHeader, Vec<Vec<*const u8>>), &'static str> {
            file.read_into(pack::TEX0, 0, tex_bytes.as_mut_ptr(), tex_size)?;
            let tex: TexHeader = pack::read(tex_bytes, 0).ok_or("atlas header")?;
            if !matches!(tex.format, tex_format::PSP_DXT1 | tex_format::PSP_5650 | tex_format::PSP_T8) || pages_wanted > 4 {
                return Err("the atlas is not a PSP texture");
            }
            let vram = sceGeEdramGetAddr();
            let mut at = (VRAM_TEXTURES + 15) & !15;
            let mut src = core::mem::size_of::<TexHeader>();
            let mut pages = Vec::new();
            for page in 0..pages_wanted {
                let mut levels = Vec::new();
                if tex.format == tex_format::PSP_T8 {
                    src = (src + 15) & !15;
                    if src + tex_format::PALETTE_BYTES > tex_bytes.len() {
                        return Err("the atlas is truncated");
                    }
                    ptr::copy_nonoverlapping(tex_bytes.as_ptr().add(src), ptr::addr_of_mut!(CLUT.0[page]) as *mut u8, tex_format::PALETTE_BYTES);
                    src += tex_format::PALETTE_BYTES;
                }
                for l in 0..tex.mips {
                    src = (src + 15) & !15;
                    let n = tex_format::level_bytes(tex.format, tex.width >> l, tex.height >> l);
                    if at + n > VRAM_BYTES || src + n > tex_bytes.len() {
                        return Err("the atlas does not fit in video memory");
                    }
                    ptr::copy_nonoverlapping(tex_bytes.as_ptr().add(src), vram.add(at), n);
                    levels.push(vram.add(at) as *const u8);
                    at = (at + n + 15) & !15;
                    src += n;
                }
                pages.push(levels);
            }
            Ok((tex, pages))
        })
        .ok_or("no memory to read the atlas")??;

        progress("Reading the town");
        let vtx: &'static [u8] = file.resident(pack::VTX0)?;
        let idx: &'static [u16] = file.resident(pack::IDX0)?;
        let clip: &'static [u8] = file.resident(pack::CLIP)?;

        progress("Reading the models");
        let models: &'static [u8] = file.resident(pack::MODL)?;
        let count: u32 = pack::read(models, 0).ok_or("model count")?;
        let mut scout = Model::default();
        let mut titans: [[Model; 2]; 3] = Default::default();
        let mut o = 4;
        for _ in 0..count {
            let h: ModelHeader = pack::read(models, o).ok_or("model header")?;
            o += core::mem::size_of::<ModelHeader>();
            let mut model = Model::default();
            for _ in 0..h.pad {
                let b: PspBatch = pack::read(models, o).ok_or("model batch")?;
                o += core::mem::size_of::<PspBatch>();
                let vb = b.vtx_count as usize * core::mem::size_of::<PspSkinVertex>();
                let ib = b.idx_count as usize * 2;
                if o + vb + ib > models.len() {
                    return Err("model section is truncated");
                }
                model.batches.push(Batch { bones: b.bones, bone_count: b.bone_count, vtx: models.as_ptr().add(o), idx: models.as_ptr().add(o + vb).cast(), idx_count: b.idx_count });
                o = (o + vb + ib + 3) & !3;
            }
            match h.id {
                0 => scout = model,
                10..=12 => titans[(h.id - 10) as usize][0] = model,
                20..=22 => titans[(h.id - 20) as usize][1] = model,
                _ => {}
            }
        }
        if scout.batches.is_empty() || titans.iter().flatten().any(|m| m.batches.is_empty()) {
            return Err("the pack lacks a skinned model");
        }

        let font_bytes: &'static [u8] = file.resident(pack::FONT)?;
        let (font, tex_at) = Font::parse(font_bytes)?;
        if font.format != tex_format::PSP_4444 {
            return Err("the font atlas is not a PSP texture");
        }
        let font_tex = font_bytes.as_ptr().add(tex_at);

        let mut sky_vb = alloc::vec![ColorVertex::default(); actors::SKY_VERTS];
        let mut sky_ib = alloc::vec![0u16; actors::SKY_INDICES];
        actors::sky(scene, 1000.0, &mut sky_vb, &mut sky_ib);
        let mut sun_vb = alloc::vec![ColorVertex::default(); actors::SUN_VERTS];
        actors::sun(scene, 1000.0, &mut sun_vb);
        let mut cloak_ib = alloc::vec![0u16; actors::CLOAK_INDICES];
        actors::cloak_indices(&mut cloak_ib);
        let mut quad_ib = alloc::vec![0u16; MAX_QUADS * 6];
        actors::quad_indices(&mut quad_ib, MAX_QUADS);
        let mut fan_ib = alloc::vec![0u16; actors::FAN_INDICES];
        actors::fan_indices(&mut fan_ib);

        let resident_bytes = vtx.len() + idx.len() * 2 + clip.len() + models.len() + font_bytes.len();
        sceKernelDcacheWritebackAll();

        Ok(Gfx {
            tex,
            pages,
            vtx,
            idx,
            clip,
            scout,
            titans,
            font_tex,
            font,
            sky_vb,
            sky_ib,
            sun_vb,
            cloak_ib,
            quad_ib,
            fan_ib,
            far: Vec::with_capacity(512),
            near: Vec::with_capacity(128),
            giants: Vec::with_capacity(32),
            stats: Stats::default(),
            picked: Default::default(),
            no_clip: false,
            resident_bytes,
        })
    }

    unsafe fn bind_page(&self, page: usize) {
        let (format, swizzled) = match self.tex.format {
            tex_format::PSP_DXT1 => (TexturePixelFormat::PsmDxt1, 0),
            tex_format::PSP_T8 => {
                sceGuClutMode(ClutPixelFormat::Psm8888, 0, 0xff, 0);
                sceGuClutLoad(32, ptr::addr_of!(CLUT.0[page]) as *const c_void);
                (TexturePixelFormat::PsmT8, 1)
            }
            _ => (TexturePixelFormat::Psm5650, 1),
        };
        sceGuTexMode(format, self.tex.mips as i32 - 1, 0, swizzled);
        const LEVELS: [MipmapLevel; 8] = [MipmapLevel::None, MipmapLevel::Level1, MipmapLevel::Level2, MipmapLevel::Level3, MipmapLevel::Level4, MipmapLevel::Level5, MipmapLevel::Level6, MipmapLevel::Level7];
        for (l, p) in self.pages[page].iter().enumerate() {
            let (w, h) = ((self.tex.width >> l) as i32, (self.tex.height >> l) as i32);
            sceGuTexImage(LEVELS[l], w, h, w, *p as *const c_void);
        }
    }

    /// One static mesh: the GE draws what it can take; large triangles near the eye are tested and cut here.
    #[allow(clippy::too_many_arguments)]
    unsafe fn mesh(&mut self, rec: &HandMesh, vtx: *const u8, idx: *const u16, eye: V3, look: V3, guard: &Guard, u_range: f32, out: *mut ClipVertex, out_n: &mut usize) {
        let (s, t) = mat::dequant(&rec.min, &rec.max, 32768.0);
        let world = ScePspFMatrix4 {
            x: ScePspFVector4 { x: s[0], y: 0.0, z: 0.0, w: 0.0 },
            y: ScePspFVector4 { x: 0.0, y: s[1], z: 0.0, w: 0.0 },
            z: ScePspFVector4 { x: 0.0, y: 0.0, z: s[2], w: 0.0 },
            w: ScePspFVector4 { x: t[0], y: t[1], z: t[2], w: 1.0 },
        };
        sceGuSetMatrix(MatrixMode::Model, &world);
        let nbig = ((rec.idx_count - rec.big_first) / 3) as usize;
        let near_mesh = nbig > 0 && !self.no_clip && mat::box_distance(eye, &rec.min, &rec.max) < rec.clip_radius;
        if !near_mesh {
            sceGuDrawArray(GuPrimitive::Triangles, vtype_world(), rec.idx_count as i32, idx.cast(), vtx.cast());
            self.stats.draws += 1;
            self.stats.tris += rec.idx_count / 3;
            return;
        }
        // The mesh's large triangles, group by group. Inside a group the largest come first, so the
        // ones that could reach the guard band from this distance are a prefix; the rest of the group
        // draws as it is, in one call with the neighbouring groups' rests where they touch.
        let table = self.clip.as_ptr().add(rec.clip_first as usize);
        let group_count = (table as *const u32).read_unaligned() as usize;
        let groups = table.add(4) as *const ClipGroup;
        let codes = table.add(4 + group_count * core::mem::size_of::<ClipGroup>());
        let mut tested = 0usize;
        // The static run being gathered: the small triangles, then untested large ones.
        let (mut run_first, mut run_len) = (0usize, rec.big_first as usize);
        let mut prefix = [0u16; 64];
        let mut at = 0usize;
        for g in 0..group_count {
            let group = groups.add(g).read_unaligned();
            let d = mat::box_distance(eye, &group.min, &group.max);
            let mut n = 0usize;
            while n < group.tris as usize {
                let c = *codes.add(at + n);
                if c != 255 && c as f32 * 2.0 <= d {
                    break;
                }
                n += 1;
            }
            if g < prefix.len() {
                prefix[g] = n as u16;
            }
            tested += n;
            // Indices of this group's untested rest.
            let rest_first = rec.big_first as usize + (at + n) * 3;
            let rest_len = (group.tris as usize - n) * 3;
            if n == 0 && run_first + run_len == rest_first {
                run_len += rest_len;
            } else {
                if run_len > 0 {
                    sceGuDrawArray(GuPrimitive::Triangles, vtype_world(), run_len as i32, idx.add(run_first).cast(), vtx.cast());
                    self.stats.draws += 1;
                    self.stats.tris += (run_len / 3) as u32;
                }
                run_first = rest_first;
                run_len = rest_len;
            }
            at += group.tris as usize;
        }
        if run_len > 0 {
            sceGuDrawArray(GuPrimitive::Triangles, vtype_world(), run_len as i32, idx.add(run_first).cast(), vtx.cast());
            self.stats.draws += 1;
            self.stats.tris += (run_len / 3) as u32;
        }
        if tested == 0 {
            return;
        }

        let verts = vtx as *const PspVertex;
        let k = [s[0] / 32768.0, s[1] / 32768.0, s[2] / 32768.0];
        let pos = |i: u16| {
            let v = &*verts.add(i as usize);
            v3(v.pos[0] as f32 * k[0] + t[0], v.pos[1] as f32 * k[1] + t[1], v.pos[2] as f32 * k[2] + t[2])
        };
        let full = |i: u16, p: V3| {
            let v = &*verts.add(i as usize);
            let c = v.color;
            let (r, g, b) = ((c & 31) as u8, ((c >> 5) & 63) as u8, (c >> 11) as u8);
            ClipVertex { uv: [v.uv[0] as f32 * (u_range / 32768.0), v.uv[1] as f32 / 32768.0], color: [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2), 255], pos: [p.x, p.y, p.z] }
        };
        let near = guard.near();
        let keep = sceGuGetMemory((tested * 6) as i32) as *mut u16;
        let mut kept = 0usize;
        let first = idx.add(rec.big_first as usize);
        let mut at = 0usize;
        for g in 0..group_count {
            let group_tris = groups.add(g).read_unaligned().tris as usize;
            let n = if g < prefix.len() { prefix[g] as usize } else { 0 };
            for j in at..at + n {
                let (a, b, c) = (*first.add(j * 3), *first.add(j * 3 + 1), *first.add(j * 3 + 2));
                let code = *codes.add(j);
                let pa = pos(a);
                let reach = code as f32 * 2.0;
                let to = pa - eye;
                let d2 = to.len2();
                let mut take = code != 255 && d2 > reach * reach;
                // Before three transforms: `edge` bounds the triangle's size, so depth along the view decides
                // most. Wholly behind the near plane, it is not drawn; wholly inside a cone of 82 degrees
                // about the view direction (the guard band is wider than that at every field of view the
                // camera uses), the GE takes it.
                let mut slow = !take;
                if slow && code != 255 {
                    let edge = reach * (1.0 / CLIP_REACH);
                    let z = to.dot(look);
                    if z + edge < near {
                        continue;
                    }
                    if z - edge > near && z - edge > 0.14 * (sqrt(d2) + edge) {
                        take = true;
                        slow = false;
                    }
                }
                if slow {
                    let (pb, pc) = (pos(b), pos(c));
                    let cc = [guard.to_clip(pa), guard.to_clip(pb), guard.to_clip(pc)];
                    match guard.classify(&cc) {
                        Verdict::Safe => take = true,
                        Verdict::Culled => {}
                        Verdict::Clip => {
                            if *out_n + MAX_OUT <= CLIP_CAP {
                                let tri = [full(a, pa), full(b, pb), full(c, pc)];
                                let n = guard.clip(&tri, &cc, core::slice::from_raw_parts_mut(out.add(*out_n), MAX_OUT));
                                *out_n += n;
                                self.stats.clipped += 1;
                            } else {
                                take = true;
                            }
                        }
                    }
                }
                if take {
                    *keep.add(kept) = a;
                    *keep.add(kept + 1) = b;
                    *keep.add(kept + 2) = c;
                    kept += 3;
                }
            }
            at += group_tris;
        }
        self.stats.tested += tested as u32;
        if kept > 0 {
            flush(keep, kept * 2);
            sceGuDrawArray(GuPrimitive::Triangles, vtype_world(), kept as i32, keep.cast(), vtx.cast());
            self.stats.draws += 1;
            self.stats.tris += (kept / 3) as u32;
        }
    }

    /// The meshes of one list, page by page, then the pieces the CPU cut.
    #[allow(clippy::too_many_arguments)]
    unsafe fn meshes(&mut self, world: &World, near_list: bool, eye: V3, look: V3, guard: &Guard, u_range: f32, frame: u32) {
        for page in 0..self.pages.len() {
            let out = sceGuGetMemory((CLIP_CAP * core::mem::size_of::<ClipVertex>()) as i32) as *mut ClipVertex;
            let mut out_n = 0usize;
            let mut bound = false;
            let count = if near_list { self.near.len() } else { self.far.len() };
            for i in 0..count {
                let pick = if near_list { self.near[i] } else { self.far[i] };
                let rec = world.recs[pick.mesh as usize];
                if rec.page as usize != page {
                    continue;
                }
                let (vtx, idx) = if pick.cell == NO_CELL {
                    (self.vtx.as_ptr().add(rec.vtx_first as usize * core::mem::size_of::<PspVertex>()), self.idx.as_ptr().add(rec.idx_first as usize))
                } else {
                    match store::data(pick.cell, frame) {
                        Some((base, offset)) => (base.add((rec.vtx_first - offset) as usize), base.add((rec.idx_first - offset) as usize) as *const u16),
                        None => continue,
                    }
                };
                if !bound {
                    self.bind_page(page);
                    bound = true;
                }
                self.mesh(&rec, vtx, idx, eye, look, guard, u_range, out, &mut out_n);
            }
            if out_n > 0 {
                flush(out, out_n * core::mem::size_of::<ClipVertex>());
                sceGuSetMatrix(MatrixMode::Model, &fmatrix(&mat::IDENTITY));
                sceGuTexScale(1.0, 1.0);
                sceGuDrawArray(GuPrimitive::Triangles, vtype_clip(), out_n as i32, ptr::null(), out.cast());
                sceGuTexScale(u_range, 1.0);
                self.stats.draws += 1;
                self.stats.tris += (out_n / 3) as u32;
            }
        }
    }

    unsafe fn model(&mut self, which: (usize, usize, bool), skin: &[M34; BONES], sink: f32, lights: &maneuver_handheld::scene::Lights) {
        sceGuAmbient(abgr(lights.ambient));
        sceGuLightColor(0, LightComponent::DIFFUSE, abgr(lights.sun));
        sceGuLightColor(1, LightComponent::DIFFUSE, abgr(lights.above));
        let model = if which.2 { &self.scout } else { &self.titans[which.0][which.1] };
        let (mut draws, mut tris) = (0, 0);
        for b in &model.batches {
            for (slot, &bone) in b.bones[..b.bone_count as usize].iter().enumerate() {
                let m = &skin[bone as usize];
                let col = |v: V3, w: f32| ScePspFVector4 { x: v.x, y: v.y, z: v.z, w };
                let fm = ScePspFMatrix4 { x: col(m.r.x, 0.0), y: col(m.r.y, 0.0), z: col(m.r.z, 0.0), w: col(v3(m.t.x, m.t.y - sink, m.t.z), 1.0) };
                sceGuBoneMatrix(slot as u32, &fm);
            }
            sceGuDrawArray(GuPrimitive::Triangles, vtype_skin(), b.idx_count as i32, b.idx.cast(), b.vtx.cast());
            draws += 1;
            tris += b.idx_count / 3;
        }
        self.stats.draws += draws;
        self.stats.tris += tris;
    }

    unsafe fn skinned_state(&self, scene: &maneuver_handheld::scene::Scene, on: bool) {
        if on {
            sceGuDisable(GuState::Texture2D);
            sceGuEnable(GuState::Lighting);
            sceGuEnable(GuState::Light0);
            sceGuEnable(GuState::Light1);
            let d = scene.sun_dir;
            sceGuLight(0, LightType::Directional, LightComponent::DIFFUSE, &ScePspFVector3 { x: d.x, y: d.y, z: d.z });
            sceGuLight(1, LightType::Directional, LightComponent::DIFFUSE, &ScePspFVector3 { x: 0.0, y: 1.0, z: 0.0 });
            sceGuSetMatrix(MatrixMode::Model, &fmatrix(&mat::IDENTITY));
        } else {
            sceGuDisable(GuState::Lighting);
            sceGuDisable(GuState::Light0);
            sceGuDisable(GuState::Light1);
        }
    }

    /// Builds and submits the frame's display list. The GE draws it while the caller prepares the next frame.
    pub unsafe fn frame(&mut self, game: &mut Game, world: &World, ticks: u32, perf: &maneuver_handheld::game::Perf, ui: &Ui) {
        self.stats = Stats::default();
        self.no_clip = game.set.option & 4 != 0;
        let mut mark = sceKernelGetSystemTimeLow();
        let mut phase = [0u32; 8];
        let mut lap = |i: usize| {
            let now = sceKernelGetSystemTimeLow();
            phase[i] = now.wrapping_sub(mark);
            mark = now;
        };
        let cam = game.camera();
        let scene_h = game.scene.h;
        let (lod_near, lod_mid, lod_far) = game.lod();
        let aspect = game.aspect();
        let view = mat::view(cam.eye, cam.look, cam.roll);
        let vp = mat::mul(&mat::perspective(cam.fov, aspect, scene_h.clip_near, scene_h.clip_far), &view);
        let planes = mat::planes(&vp);

        self.far.clear();
        self.near.clear();
        self.giants.clear();
        if game.set.world {
            self.picked = world.pick(&planes, cam.eye, lod_near, lod_mid, lod_far, &|c| store::ready(c), &mut self.far, &mut self.near);
        }
        if game.set.actors {
            game.actors.giants(&game.sim, &game.scene, &planes, cam.eye, &mut self.giants);
        }

        lap(0);
        sceGuStart(GuContextType::Direct, ptr::addr_of_mut!(LIST.0) as *mut c_void);
        sceGuDepthMask(0);
        sceGuClearColor(abgr(game.scene.fog_srgb()));
        sceGuClearDepth(0);
        sceGuClear(ClearBuffer::COLOR_BUFFER_BIT | ClearBuffer::DEPTH_BUFFER_BIT);
        sceGuSetMatrix(MatrixMode::View, &fmatrix(&view));

        let far_near = max(lod_near * 0.8, 8.0);
        let far_proj = fmatrix(&mat::perspective_gl(cam.fov, aspect, far_near, scene_h.clip_far));
        sceGuDisable(GuState::Blend);
        // Front to back: the cells around the eye first, then everything beyond, then the sky where
        // nothing was drawn. The depth test then rejects what is hidden before it is textured; drawn
        // far to near, a street's facades were filled five and six times over.
        let by_distance = |a: &Pick, b: &Pick| a.dist.partial_cmp(&b.dist).unwrap_or(core::cmp::Ordering::Equal);
        self.near.sort_unstable_by(by_distance);
        self.far.sort_unstable_by(by_distance);

        // ------------------------------------------------------------ world state
        let world_state = |on: bool| {
            if on {
                sceGuEnable(GuState::Texture2D);
                sceGuEnable(GuState::Fragment2X);
                sceGuTexFunc(TextureEffect::Modulate, TextureColorComponent::Rgb);
                sceGuTexFilter(TextureFilter::LinearMipmapNearest, TextureFilter::Linear);
                sceGuTexLevelMode(TextureLevelMode::Auto, 0.0);
                sceGuTexWrap(GuTexWrapMode::Repeat, GuTexWrapMode::Clamp);
                sceGuTexScale(scene_h.u_range, 1.0);
                sceGuTexOffset(0.0, 0.0);
            } else {
                sceGuDisable(GuState::Fragment2X);
                sceGuTexFunc(TextureEffect::Modulate, TextureColorComponent::Rgba);
            }
        };
        sceGuEnable(GuState::DepthTest);
        sceGuEnable(GuState::Fog);
        sceGuFog(scene_h.fog_near, scene_h.fog_far, abgr(game.scene.fog_srgb()));
        if game.set.option & 1 == 0 {
            sceGuEnable(GuState::CullFace);
        }
        sceGuFrontFace(if game.set.option & 2 == 0 { FrontFaceDirection::CounterClockwise } else { FrontFaceDirection::Clockwise });

        // ------------------------------------------------------------ near pass
        let frame = game.frame;
        // Giants whose whole body is beyond the near pass's reach draw in the far pass.
        let split = lod_near + 30.0;
        let near_proj = fmatrix(&mat::perspective_gl(cam.fov, aspect, scene_h.clip_near, lod_near + 180.0));
        sceGuSetMatrix(MatrixMode::Projection, &near_proj);
        sceGuDepthRange(65535, DEPTH_SPLIT);
        let show = game.show_character();
        self.skinned_state(&game.scene, true);
        if show && game.set.actors {
            let skin = game.sim.pose.skin;
            let lights = game.scene.lights(game.actors.vis);
            self.model((0, 0, true), &skin, 0.0, &lights);
        }
        for i in 0..self.giants.len() {
            let g = self.giants[i];
            if g.dist > split {
                continue;
            }
            let skin = *game.titan_skin(g.index as usize, g.level == 0);
            self.model((g.variant as usize, g.level as usize, false), &skin, g.sink, &game.scene.lights(g.vis));
        }
        self.skinned_state(&game.scene, false);
        lap(3);
        let near_guard = Guard::new(&vp, scene_h.clip_near, 480.0, 272.0, 2048.0);
        world_state(true);
        for _ in 0..game.set.repeat {
            self.meshes(world, true, cam.eye, cam.look, &near_guard, scene_h.u_range, frame);
        }
        world_state(false);
        lap(4);

        // ------------------------------------------------------------ far pass
        sceGuSetMatrix(MatrixMode::Projection, &far_proj);
        sceGuDepthRange(DEPTH_SPLIT - 1, 0);
        let far_guard = Guard::new(&vp, far_near, 480.0, 272.0, 2048.0);
        world_state(true);
        for _ in 0..game.set.repeat {
            self.meshes(world, false, cam.eye, cam.look, &far_guard, scene_h.u_range, frame);
        }
        world_state(false);
        lap(1);
        self.skinned_state(&game.scene, true);
        for i in 0..self.giants.len() {
            let g = self.giants[i];
            if g.dist <= split {
                continue;
            }
            let skin = *game.titan_skin(g.index as usize, g.level == 0);
            self.model((g.variant as usize, g.level as usize, false), &skin, g.sink, &game.scene.lights(g.vis));
        }
        self.skinned_state(&game.scene, false);
        lap(2);

        // ------------------------------------------------------------ sky
        // At the far end of the depth range, so it fills only what is still clear.
        sceGuDepthRange(0, 0);
        sceGuDisable(GuState::Texture2D);
        sceGuDisable(GuState::Fog);
        sceGuDisable(GuState::CullFace);
        sceGuDepthMask(1);
        let at_eye = fmatrix(&mat::translated(&mat::IDENTITY, cam.eye));
        sceGuSetMatrix(MatrixMode::Model, &at_eye);
        sceGuDrawArray(GuPrimitive::Triangles, vtype_color(), actors::SKY_INDICES as i32, self.sky_ib.as_ptr().cast(), self.sky_vb.as_ptr().cast());
        sceGuEnable(GuState::Blend);
        sceGuDrawArray(GuPrimitive::Triangles, vtype_color(), (2 * actors::FAN * 3) as i32, self.fan_ib.as_ptr().cast(), self.sun_vb.as_ptr().cast());
        sceGuDisable(GuState::Blend);
        sceGuDepthMask(0);
        sceGuEnable(GuState::Fog);
        self.stats.draws += 2;
        self.stats.tris += (actors::SKY_INDICES / 3 + 2 * actors::FAN) as u32;

        // The cloak, the wires and the discs draw with the near pass's frustum again.
        sceGuSetMatrix(MatrixMode::Projection, &near_proj);
        sceGuDepthRange(65535, DEPTH_SPLIT);
        sceGuSetMatrix(MatrixMode::Model, &fmatrix(&mat::IDENTITY));

        // The cloak, the wires and the soft discs, written into the list's own memory.
        if game.set.actors {
            let bytes = (CLOAK_N + actors::ROPE_VERTS + actors::DISC_VERTS) * core::mem::size_of::<ColorVertex>();
            let mem = sceGuGetMemory(bytes as i32) as *mut ColorVertex;
            let cloak = core::slice::from_raw_parts_mut(mem, CLOAK_N);
            let rope = core::slice::from_raw_parts_mut(mem.add(CLOAK_N), actors::ROPE_VERTS);
            let disc = core::slice::from_raw_parts_mut(mem.add(CLOAK_N + actors::ROPE_VERTS), actors::DISC_VERTS);
            let f = game.actors.update(&game.sim, &game.scene, cam.eye, ticks, cloak, rope, disc);
            flush(mem, bytes);
            sceGuDisable(GuState::CullFace);
            if show {
                sceGuDrawArray(GuPrimitive::Triangles, vtype_color(), actors::CLOAK_INDICES as i32, self.cloak_ib.as_ptr().cast(), cloak.as_ptr().cast());
                self.stats.draws += 1;
                self.stats.tris += (actors::CLOAK_INDICES / 3) as u32;
            }
            if f.rope_quads > 0 {
                sceGuDrawArray(GuPrimitive::Triangles, vtype_color(), (f.rope_quads * 6) as i32, self.quad_ib.as_ptr().cast(), rope.as_ptr().cast());
                self.stats.draws += 1;
                self.stats.tris += f.rope_quads * 2;
            }
            if f.discs > 0 {
                sceGuEnable(GuState::Blend);
                sceGuDepthMask(1);
                sceGuDrawArray(GuPrimitive::Triangles, vtype_color(), (f.discs as usize * actors::FAN * 3) as i32, self.fan_ib.as_ptr().cast(), disc.as_ptr().cast());
                sceGuDepthMask(0);
                self.stats.draws += 1;
                self.stats.tris += f.discs * actors::FAN as u32;
            }
        }

        lap(5);
        // ------------------------------------------------------------ marks on the world
        sceGuDisable(GuState::DepthTest);
        sceGuDisable(GuState::Fog);
        sceGuDisable(GuState::CullFace);
        let hv = sceGuGetMemory((MAX_QUADS * 4 * core::mem::size_of::<HudVertex>()) as i32) as *mut HudVertex;
        let quads = {
            let mut p = *perf;
            p.draws = self.stats.draws;
            p.tris = self.stats.tris;
            game.measure(&p);
            let mut hud = Hud::new(&self.font, core::slice::from_raw_parts_mut(hv, MAX_QUADS * 4), [1.0, 1.0], [0.0, 0.0]);
            game.draw_marks(&mut hud, &vp);
            hud.quads
        };
        if quads > 0 {
            sceGuEnable(GuState::Blend);
            sceGuEnable(GuState::Texture2D);
            sceGuTexMode(TexturePixelFormat::Psm4444, 0, 0, 1);
            sceGuTexImage(MipmapLevel::None, self.font.width as i32, self.font.height as i32, self.font.width as i32, self.font_tex.cast());
            sceGuTexFilter(TextureFilter::Nearest, TextureFilter::Nearest);
            sceGuTexWrap(GuTexWrapMode::Clamp, GuTexWrapMode::Clamp);
            flush(hv, quads * 4 * core::mem::size_of::<HudVertex>());
            sceGuDrawArray(GuPrimitive::Triangles, vtype_hud(), (quads * 6) as i32, self.quad_ib.as_ptr().cast(), hv.cast());
            self.stats.draws += 1;
        }
        sceGuDisable(GuState::Blend);
        lap(6);
        // ------------------------------------------------------------ interface
        // PocketJS's GE backend sets blending and texturing for itself and leaves both off; it
        // draws in screen coordinates, which the wrap mode, the texture scale and the matrices do not touch.
        sceGuTexWrap(GuTexWrapMode::Clamp, GuTexWrapMode::Clamp);
        ui.draw();
        sceGuFinish();
        lap(7);
        self.stats.phase = phase;
    }
}
