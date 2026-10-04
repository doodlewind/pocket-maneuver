//! Lowering for the handhelds (PSP, 3DS).
//!
//! The same baked geometry as the Vita pack, cut to what each machine reads
//! without conversion:
//!
//! - The atlas becomes pages of at most 512 × 512 texels, stacked along `v`;
//!   every mesh is split by the page its triangles sample.
//! - Vertices are quantized into the device's own layout (`PspVertex`,
//!   `PicaVertex`).
//! - PSP: a cell's detailed meshes go to `NEAR` and are read on demand, because
//!   the machine has 24 MB. Large triangles sort to the end of each mesh with a
//!   distance in `CLIP`; inside that distance the runtime clips them on the CPU,
//!   because the GE drops a triangle with a vertex outside its 4096-pixel guard
//!   band instead of clipping it.
//! - PSP: skinned models are cut into draws of at most four bones, the GE's
//!   blend in one draw.
//! - The collision grid is stored built (`SIMG`).

use crate::ir::{self, Mesh, STRIDE};
use maneuver_pack::{self as pack, mesh_kind, tex_format, FontHeader, HandMesh, HandScene, ModelHeader, PicaVertex, PspBatch, PspSkinVertex, PspVertex, SkinVertex, TexHeader, PSP_BATCH_BONES};
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Handheld {
    /// Texels of one atlas page.
    pub page: [u32; 2],
    pub lod: Lod,
    pub fog: Fog,
    pub clip: Clip,
    pub titan: Titan,
    pub models: Models,
    /// A triangle with an edge longer than this many metres is a large triangle (PSP).
    #[serde(default)]
    pub big_edge: f32,
    /// Detailed cell meshes are read on demand (PSP).
    #[serde(default)]
    pub stream_near: bool,
}
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Lod {
    pub near: f32,
    pub mid: f32,
    pub far: f32,
}
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Fog {
    pub near: f32,
    pub far: f32,
    pub density: f32,
}
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub near: f32,
    pub far: f32,
}
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Titan {
    pub near: f32,
    pub far: f32,
    /// The most giants one frame draws.
    pub max: u32,
}
/// Source model ids for each role.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Models {
    pub scout: i32,
    pub titan_near: [i32; 3],
    pub titan_far: [i32; 3],
}

/// Texture repeats a stored `u` covers.
pub const U_RANGE: f32 = 16.0;
/// A triangle farther than this many of its longest edges cannot reach from inside
/// the view to outside the guard band (see `maneuver_handheld::clip`).
pub const CLIP_REACH: f32 = 3.3;

#[derive(Clone, Copy, PartialEq)]
pub enum Target {
    Psp,
    Pica,
}

pub struct Lowered {
    pub rec: HandMesh,
    /// Device vertex bytes.
    pub vtx: Vec<u8>,
    pub idx: Vec<u16>,
    /// One byte per large triangle.
    pub clip: Vec<u8>,
}

fn morton(x: u32, z: u32) -> u32 {
    let spread = |mut v: u32| {
        v &= 0x3ff;
        v = (v | (v << 8)) & 0x00ff_00ff;
        v = (v | (v << 4)) & 0x0f0f_0f0f;
        v = (v | (v << 2)) & 0x3333_3333;
        (v | (v << 1)) & 0x5555_5555
    };
    spread(x) | (spread(z) << 1)
}

/// Splits the parts of one cell and level by atlas page and quantizes each into a device mesh.
pub fn lower(parts: &[(&Mesh, &Vec<[u8; 4]>)], kind: u32, cx: i32, cz: i32, h: &Handheld, pages: u32, target: Target, limit: usize) -> Result<Vec<Lowered>, String> {
    struct Build<'a> {
        src: Vec<(&'a [f32], [u8; 4])>,
        tris: Vec<[u32; 3]>,
    }
    let mut builds: Vec<Build> = (0..pages).map(|_| Build { src: Vec::new(), tris: Vec::new() }).collect();
    for (mesh, colors) in parts {
        let mut maps: Vec<Vec<u32>> = (0..pages).map(|_| vec![u32::MAX; mesh.verts.len() / STRIDE]).collect();
        for tri in mesh.idx.chunks_exact(3) {
            let v = |i: u32| mesh.verts[i as usize * STRIDE + 7];
            let centre = (v(tri[0]) + v(tri[1]) + v(tri[2])) / 3.0;
            let page = ((centre * pages as f32) as u32).min(pages - 1);
            let (lo, hi) = (page as f32 / pages as f32, (page + 1) as f32 / pages as f32);
            if tri.iter().any(|&i| v(i) < lo - 1e-4 || v(i) > hi + 1e-4) {
                return Err(format!("a triangle in cell {cx},{cz} samples two atlas pages"));
            }
            let b = &mut builds[page as usize];
            let mut out = [0u32; 3];
            for (k, &i) in tri.iter().enumerate() {
                let slot = &mut maps[page as usize][i as usize];
                if *slot == u32::MAX {
                    *slot = b.src.len() as u32;
                    b.src.push((&mesh.verts[i as usize * STRIDE..(i as usize + 1) * STRIDE], colors[i as usize]));
                }
                out[k] = *slot;
            }
            b.tris.push(out);
        }
    }
    let mut out = Vec::new();
    for (page, b) in builds.into_iter().enumerate() {
        if b.tris.is_empty() {
            continue;
        }
        if b.src.len() > limit {
            return Err(format!("cell {cx},{cz} page {page} has {} vertices; the profile allows {limit}", b.src.len()));
        }
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        let mut u_min = f32::MAX;
        for (v, _) in &b.src {
            for a in 0..3 {
                min[a] = min[a].min(v[a]);
                max[a] = max[a].max(v[a]);
            }
            u_min = u_min.min(v[6]);
        }
        for a in 0..3 {
            if max[a] - min[a] < 0.01 {
                max[a] = min[a] + 0.01;
            }
        }
        // `u` repeats, so a whole number of repeats comes off for free and the rest is unsigned.
        let u_shift = u_min.floor();
        let mut vtx = Vec::with_capacity(b.src.len() * 16);
        for (v, color) in &b.src {
            let mut pos = [0i16; 4];
            for a in 0..3 {
                pos[a] = (((v[a] - min[a]) / (max[a] - min[a]) * 65535.0 + 0.5).clamp(0.0, 65535.0) as i32 - 32768) as i16;
            }
            let u = (v[6] - u_shift) / U_RANGE;
            if !(0.0..1.0).contains(&u) {
                return Err(format!("texture coordinate u spans more than {U_RANGE} repeats in cell {cx},{cz}"));
            }
            let vv = (v[7] * pages as f32 - page as f32).clamp(0.0, 1.0);
            let uv = [(u * 32768.0).round().min(32767.0) as u16, (vv * 32768.0).round().min(32767.0) as u16];
            match target {
                Target::Psp => {
                    let c = (color[0] as u16 >> 3) | ((color[1] as u16 >> 2) << 5) | ((color[2] as u16 >> 3) << 11);
                    vtx.extend_from_slice(pack::bytes_of(&PspVertex { uv, color: c, pos: [pos[0], pos[1], pos[2]] }));
                }
                Target::Pica => vtx.extend_from_slice(pack::bytes_of(&PicaVertex { uv: [uv[0] as i16, uv[1] as i16], color: *color, pos })),
            }
        }
        // Small triangles first; large ones after, the largest first, so a runtime at a given distance
        // tests a prefix and draws the rest as they are.
        let edge = |t: &[u32; 3]| {
            let p = |i: u32| {
                let v = b.src[i as usize].0;
                [v[0], v[1], v[2]]
            };
            let d = |a: [f32; 3], c: [f32; 3]| ((a[0] - c[0]).powi(2) + (a[1] - c[1]).powi(2) + (a[2] - c[2]).powi(2)).sqrt();
            let (a, c, e) = (p(t[0]), p(t[1]), p(t[2]));
            d(a, c).max(d(c, e)).max(d(e, a))
        };
        let mut small = Vec::new();
        let mut big: Vec<(u32, f32, [u32; 3])> = Vec::new();
        for t in &b.tris {
            let e = edge(t);
            if h.big_edge > 0.0 && e > h.big_edge {
                let v = b.src[t[0] as usize].0;
                let key = morton(((v[0] - min[0]) / (max[0] - min[0]) * 1023.0) as u32, ((v[2] - min[2]) / (max[2] - min[2]) * 1023.0) as u32);
                big.push((key, e, *t));
            } else {
                small.push(*t);
            }
        }
        let code_of = |e: f32| ((e * CLIP_REACH / 2.0).ceil() as u32).clamp(1, 255) as u8;
        big.sort_by_key(|t| (255 - code_of(t.1), t.0));
        let mut idx: Vec<u16> = small.iter().flatten().map(|&i| i as u16).collect();
        let big_first = idx.len() as u32;
        let mut clip = Vec::with_capacity(big.len());
        let mut clip_radius = 0.0f32;
        for (_, e, t) in &big {
            idx.extend(t.iter().map(|&i| i as u16));
            let code = code_of(*e);
            clip.push(code);
            clip_radius = clip_radius.max(if code == 255 { 1e9 } else { code as f32 * 2.0 });
        }
        out.push(Lowered {
            rec: HandMesh { kind, cx, cz, page: page as u32, vtx_first: 0, vtx_count: b.src.len() as u32, idx_first: 0, idx_count: idx.len() as u32, big_first, clip_first: 0, clip_radius, min, max, pad: 0 },
            vtx,
            idx,
            clip,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------- textures

fn psp_swizzle(src: &[u8], row_bytes: usize, rows: usize) -> Vec<u8> {
    let stride = (row_bytes + 15) & !15;
    let padded = (rows + 7) & !7;
    let mut out = vec![0u8; stride * padded];
    for y in 0..rows {
        for x in (0..row_bytes).step_by(16) {
            let n = (row_bytes - x).min(16);
            let dst = ((y / 8) * (stride / 16) + x / 16) * 128 + (y % 8) * 16;
            out[dst..dst + n].copy_from_slice(&src[y * row_bytes + x..y * row_bytes + x + n]);
        }
    }
    out
}

/// 16-bit texels into the PICA's layout: 8 × 8 tiles in row order, Morton order
/// inside a tile, and the image's last row first.
fn pica_tile(texels: &[u16], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 2];
    for y in 0..h {
        for x in 0..w {
            let fy = h - 1 - y;
            let tile = (fy / 8) * (w / 8) + x / 8;
            let (lx, ly) = (x % 8, fy % 8);
            let mut m = 0;
            for b in 0..3 {
                m |= ((lx >> b) & 1) << (2 * b);
                m |= ((ly >> b) & 1) << (2 * b + 1);
            }
            let o = (tile * 64 + m) * 2;
            out[o..o + 2].copy_from_slice(&texels[y * w + x].to_le_bytes());
        }
    }
    out
}

fn encode(format: u32, rgba: &[u8], w: usize, h: usize) -> Result<Vec<u8>, String> {
    let px = |i: usize| (rgba[i * 4] as u16, rgba[i * 4 + 1] as u16, rgba[i * 4 + 2] as u16, rgba[i * 4 + 3] as u16);
    match format {
        tex_format::PSP_DXT1 => {
            let fmt = texpresso::Format::Bc1;
            let mut out = vec![0u8; fmt.compressed_size(w, h)];
            // Every texel opaque: the encoder must not pick the three-colour mode's transparent index.
            let opaque: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
            fmt.compress(&opaque, w, h, texpresso::Params { algorithm: texpresso::Algorithm::ClusterFit, ..Default::default() }, &mut out);
            // The GE reads a block as the index word, then the two colours.
            for b in out.chunks_exact_mut(8) {
                let (colors, lines) = ([b[0], b[1], b[2], b[3]], [b[4], b[5], b[6], b[7]]);
                b[..4].copy_from_slice(&lines);
                b[4..].copy_from_slice(&colors);
            }
            Ok(out)
        }
        tex_format::PSP_5650 | tex_format::PSP_4444 => {
            let mut bytes = Vec::with_capacity(w * h * 2);
            for i in 0..w * h {
                let (r, g, b, a) = px(i);
                let t = if format == tex_format::PSP_5650 { (r >> 3) | ((g >> 2) << 5) | ((b >> 3) << 11) } else { (r >> 4) | ((g >> 4) << 4) | ((b >> 4) << 8) | ((a >> 4) << 12) };
                bytes.extend_from_slice(&t.to_le_bytes());
            }
            if w < 8 || h < 8 {
                return Err(format!("a swizzled level of {w} × {h} texels is smaller than a block"));
            }
            Ok(psp_swizzle(&bytes, w * 2, h))
        }
        tex_format::PICA_RGB565 | tex_format::PICA_RGBA4 => {
            if w < 8 || h < 8 {
                return Err(format!("a tiled level of {w} × {h} texels is smaller than a tile"));
            }
            let texels: Vec<u16> = (0..w * h)
                .map(|i| {
                    let (r, g, b, a) = px(i);
                    if format == tex_format::PICA_RGB565 {
                        ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)
                    } else {
                        ((r >> 4) << 12) | ((g >> 4) << 8) | ((b >> 4) << 4) | (a >> 4)
                    }
                })
                .collect();
            Ok(pica_tile(&texels, w, h))
        }
        f => Err(format!("texture format {f} is not a handheld format")),
    }
}

pub fn format_of(name: &str) -> Result<u32, String> {
    Ok(match name {
        "psp-dxt1" => tex_format::PSP_DXT1,
        "psp-5650" => tex_format::PSP_5650,
        "pica-rgb565" => tex_format::PICA_RGB565,
        other => return Err(format!("texture format {other:?} is not one of psp-dxt1, psp-5650, pica-rgb565")),
    })
}

/// `TexHeader` of one page, then every page's levels.
pub fn atlas(rgba: &[u8], w: usize, h: usize, edges: &[usize], page: [u32; 2], format: u32, max_mips: u32) -> Result<(Vec<u8>, u32, u32), String> {
    let (pw, ph) = (page[0] as usize, page[1] as usize);
    if !pw.is_power_of_two() || !ph.is_power_of_two() || pw > w || w % pw != 0 {
        return Err(format!("atlas page {pw} × {ph} does not divide the {w} × {h} atlas"));
    }
    let base = (w / pw).trailing_zeros();
    let scaled_h = h >> base;
    if scaled_h % ph != 0 {
        return Err(format!("atlas page height {ph} does not divide the atlas at that scale ({scaled_h})"));
    }
    let pages = scaled_h / ph;
    for p in 1..pages {
        if !edges.contains(&(p * ph << base)) {
            return Err(format!("the boundary of atlas page {p} cuts a texture strip"));
        }
    }
    let min_side = if format == tex_format::PSP_DXT1 { 4 } else { 8 };
    let mut levels = 0;
    while levels < max_mips && (pw >> levels) >= min_side && (ph >> levels) >= min_side {
        levels += 1;
    }
    let mips: Vec<(Vec<u8>, usize, usize)> = (0..levels).into_par_iter().map(|l| crate::texture::mip(rgba, w, h, edges, base + l)).collect();
    let mut out = pack::bytes_of(&TexHeader { width: pw as u32, height: ph as u32, mips: levels, format }).to_vec();
    for p in 0..pages {
        for (l, (px, mw, _)) in mips.iter().enumerate() {
            let (lw, lh) = (pw >> l, ph >> l);
            debug_assert_eq!(*mw, lw);
            let rows = &px[p * lh * lw * 4..(p + 1) * lh * lw * 4];
            let bytes = encode(format, rows, lw, lh)?;
            debug_assert_eq!(bytes.len(), tex_format::level_bytes(format, lw as u32, lh as u32));
            while out.len() % 16 != 0 {
                out.push(0);
            }
            out.extend_from_slice(&bytes);
        }
    }
    Ok((out, pages as u32, levels))
}

/// The glyph atlas as a 16-bit texture: white, coverage in alpha, and a solid 2 × 2 block in the last corner.
pub fn font(a8: &[u8], target: Target) -> Result<Vec<u8>, String> {
    let head: FontHeader = pack::read(a8, 0).ok_or("font header")?;
    let table = core::mem::size_of::<FontHeader>() + head.glyphs as usize * core::mem::size_of::<pack::Glyph>();
    let (w, h) = (head.width as usize, head.height as usize);
    let mut rgba = vec![255u8; w * h * 4];
    for i in 0..w * h {
        rgba[i * 4 + 3] = a8[table + i];
    }
    for (x, y) in [(w - 1, h - 1), (w - 2, h - 1), (w - 1, h - 2), (w - 2, h - 2)] {
        rgba[(y * w + x) * 4 + 3] = 255;
    }
    let format = if target == Target::Psp { tex_format::PSP_4444 } else { tex_format::PICA_RGBA4 };
    let mut out = pack::bytes_of(&FontHeader { pad: format, ..head }).to_vec();
    out.extend_from_slice(&a8[core::mem::size_of::<FontHeader>()..table]);
    while out.len() % 16 != 0 {
        out.push(0);
    }
    out.extend_from_slice(&encode(format, &rgba, w, h)?);
    Ok(out)
}

// ---------------------------------------------------------------- models

struct SrcVertex {
    pos: [f32; 3],
    normal: [i8; 4],
    color: [u8; 4],
    bones: [u8; 2],
    w0: u8,
}

fn src_vertices(m: &Mesh) -> Vec<SrcVertex> {
    const S: usize = ir::SKIN_STRIDE;
    (0..m.verts.len() / S)
        .map(|i| {
            let v = &m.verts[i * S..(i + 1) * S];
            let c = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            let n = |x: f32| (x.clamp(-1.0, 1.0) * 127.0).round() as i8;
            SrcVertex { pos: [v[0], v[1], v[2]], normal: [n(v[3]), n(v[4]), n(v[5]), 0], color: [c(v[6]), c(v[7]), c(v[8]), 255], bones: [v[9] as u8, v[10] as u8], w0: (v[11].clamp(0.0, 1.0) * 255.0 + 0.5) as u8 }
        })
        .collect()
}

/// One model as bone batches for the GE.
fn psp_model(id: u32, m: &Mesh) -> Result<Vec<u8>, String> {
    let verts = src_vertices(m);
    struct Batch {
        bones: Vec<u8>,
        verts: Vec<PspSkinVertex>,
        idx: Vec<u16>,
        /// (source vertex, rigid on its first bone) to batch vertex.
        map: HashMap<(u32, bool), u16>,
    }
    // A vertex's bones that carry weight.
    let used = |v: &SrcVertex, rigid: bool| -> Vec<u8> {
        if rigid || v.w0 >= 254 || v.bones[0] == v.bones[1] {
            vec![v.bones[0]]
        } else if v.w0 <= 1 {
            vec![v.bones[1]]
        } else {
            vec![v.bones[0], v.bones[1]]
        }
    };
    let mut tris: Vec<[u32; 3]> = m.idx.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
    tris.sort_by_key(|t| t.iter().map(|&i| verts[i as usize].bones[0]).min());
    let mut batches: Vec<Batch> = Vec::new();
    for t in tris {
        // More bones than one draw blends: the vertex with the weakest second bone follows its first bone alone.
        let mut rigid = [false; 3];
        let set = |rigid: &[bool; 3]| {
            let mut s: Vec<u8> = t.iter().zip(rigid).flat_map(|(&i, &r)| used(&verts[i as usize], r)).collect();
            s.sort();
            s.dedup();
            s
        };
        let mut bones = set(&rigid);
        while bones.len() > PSP_BATCH_BONES {
            let k = (0..3).filter(|&k| !rigid[k]).min_by_key(|&k| 255 - verts[t[k] as usize].w0).ok_or("a triangle's bones cannot be reduced")?;
            rigid[k] = true;
            bones = set(&rigid);
        }
        let growth = |b: &Batch| bones.iter().filter(|x| !b.bones.contains(x)).count();
        let at = match batches.iter().enumerate().filter(|(_, b)| b.bones.len() + growth(b) <= PSP_BATCH_BONES && b.verts.len() + 3 <= 65535).min_by_key(|(_, b)| growth(b)) {
            Some((i, _)) => i,
            None => {
                batches.push(Batch { bones: Vec::new(), verts: Vec::new(), idx: Vec::new(), map: HashMap::new() });
                batches.len() - 1
            }
        };
        let b = &mut batches[at];
        for x in &bones {
            if !b.bones.contains(x) {
                b.bones.push(*x);
            }
        }
        for (k, &i) in t.iter().enumerate() {
            let v = &verts[i as usize];
            let r = rigid[k] || used(v, false).len() == 1;
            let key = (i, r);
            let slot = match b.map.get(&key) {
                Some(&s) => s,
                None => {
                    let mut weights = [0u8; PSP_BATCH_BONES];
                    let at = |bone: u8| b.bones.iter().position(|&x| x == bone).unwrap();
                    if r {
                        weights[at(used(v, rigid[k])[0])] = 128;
                    } else {
                        let w = ((v.w0 as u32 * 128 + 127) / 255) as u8;
                        weights[at(v.bones[0])] = w;
                        weights[at(v.bones[1])] += 128 - w;
                    }
                    let s = b.verts.len() as u16;
                    b.verts.push(PspSkinVertex { weights, color: v.color, normal: v.normal, pos: v.pos });
                    b.map.insert(key, s);
                    s
                }
            };
            b.idx.push(slot);
        }
    }
    let head = ModelHeader { id, vtx_count: batches.iter().map(|b| b.verts.len() as u32).sum(), idx_count: m.idx.len() as u32, pad: batches.len() as u32 };
    let mut out = pack::bytes_of(&head).to_vec();
    for b in &batches {
        let mut bones = [0u8; PSP_BATCH_BONES];
        bones[..b.bones.len()].copy_from_slice(&b.bones);
        out.extend_from_slice(pack::bytes_of(&PspBatch { bone_count: b.bones.len() as u32, vtx_count: b.verts.len() as u32, idx_count: b.idx.len() as u32, bones }));
        out.extend_from_slice(pack::slice_bytes(&b.verts));
        out.extend_from_slice(pack::slice_bytes(&b.idx));
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    Ok(out)
}

fn pica_model(id: u32, m: &Mesh) -> Result<Vec<u8>, String> {
    let verts = src_vertices(m);
    if verts.len() > 65535 {
        return Err(format!("model {id} has {} vertices; indices are 16-bit", verts.len()));
    }
    let mut out = pack::bytes_of(&ModelHeader { id, vtx_count: verts.len() as u32, idx_count: m.idx.len() as u32, pad: 0 }).to_vec();
    for v in &verts {
        out.extend_from_slice(pack::bytes_of(&SkinVertex { pos: v.pos, normal: v.normal, color: v.color, bones: v.bones, weights: [v.w0, 255 - v.w0] }));
    }
    for &i in &m.idx {
        out.extend_from_slice(&(i as u16).to_le_bytes());
    }
    while out.len() % 4 != 0 {
        out.push(0);
    }
    Ok(out)
}

/// The profile's models under the ids a runtime looks for: 0, 10 to 12, 20 to 22.
pub fn models(ir: &ir::Ir, which: &Models, target: Target) -> Result<(Vec<u8>, serde_json::Value), String> {
    let mut roles = vec![(0u32, which.scout)];
    for v in 0..3 {
        roles.push((10 + v as u32, which.titan_near[v]));
        roles.push((20 + v as u32, which.titan_far[v]));
    }
    let mut out = (roles.len() as u32).to_le_bytes().to_vec();
    let mut stats = Vec::new();
    for (id, source) in roles {
        let m = ir.models.iter().find(|m| m.head[0] == source).ok_or(format!("the source has no model {source}"))?;
        let bytes = if target == Target::Psp { psp_model(id, m)? } else { pica_model(id, m)? };
        let head: ModelHeader = pack::read(&bytes, 0).unwrap();
        stats.push(serde_json::json!({"id": id, "source": source, "triangles": m.idx.len() / 3, "vertices": head.vtx_count, "draws": head.pad.max(1)}));
        out.extend_from_slice(&bytes);
    }
    Ok((out, serde_json::Value::Array(stats)))
}

pub fn scene(scene: &serde_json::Value, h: &Handheld, screen: [u32; 2], pages: u32, dummies: u32) -> Result<HandScene, String> {
    let arr = |k: &str| -> Result<[f32; 3], String> {
        let v = scene[k].as_array().filter(|a| a.len() == 3).ok_or(format!("scene.json: {k}"))?;
        Ok([v[0].as_f64().unwrap_or(0.0) as f32, v[1].as_f64().unwrap_or(0.0) as f32, v[2].as_f64().unwrap_or(0.0) as f32])
    };
    let num = |v: &serde_json::Value, k: &str| v.as_f64().map(|x| x as f32).ok_or(format!("scene.json: {k}"));
    Ok(HandScene {
        sun_dir: arr("sunDir")?,
        fog_density: h.fog.density,
        sun: arr("sun")?,
        lod_near: h.lod.near,
        sky: arr("sky")?,
        lod_mid: h.lod.mid,
        bounce: arr("bounce")?,
        lod_far: h.lod.far,
        fog: arr("fog")?,
        clip_near: h.clip.near,
        horizon: arr("horizon")?,
        clip_far: h.clip.far,
        zenith: arr("zenith")?,
        cell: num(&scene["cell"], "cell")?,
        glow: arr("glow")?,
        super_cell: num(&scene["horizonCell"], "horizonCell")?,
        fog_near: h.fog.near,
        fog_far: h.fog.far,
        titan_near: h.titan.near,
        titan_far: h.titan.far,
        u_range: U_RANGE,
        color_scale: pack::COLOR_SCALE,
        screen: [screen[0] as f32, screen[1] as f32],
        pages,
        near_streamed: h.stream_near as u32,
        dummies,
        max_giants: h.titan.max,
    })
}

/// Sections of geometry: resident vertices and indices, the on-demand blob, the clip bytes, the table.
pub struct Geometry {
    pub recs: Vec<HandMesh>,
    pub vtx: Vec<u8>,
    pub idx: Vec<u16>,
    pub near: Vec<u8>,
    pub clip: Vec<u8>,
    pub tris: [usize; 4],
    pub count: [usize; 4],
    pub big: usize,
    pub max_verts: usize,
    pub largest_cell: usize,
}

pub fn assemble(lowered: Vec<Vec<Lowered>>, stream_near: bool, vertex_bytes: usize) -> Geometry {
    let mut g = Geometry { recs: Vec::new(), vtx: Vec::new(), idx: Vec::new(), near: Vec::new(), clip: Vec::new(), tris: [0; 4], count: [0; 4], big: 0, max_verts: 0, largest_cell: 0 };
    for group in lowered {
        // One group is one cell at one level; streamed, its meshes are contiguous: vertices, then indices.
        let streamed = stream_near && group.first().is_some_and(|m| m.rec.kind == mesh_kind::NEAR);
        let start = g.near.len();
        let mut recs: Vec<HandMesh> = Vec::new();
        for m in &group {
            let mut rec = m.rec;
            rec.clip_first = g.clip.len() as u32;
            g.clip.extend_from_slice(&m.clip);
            if streamed {
                rec.vtx_first = g.near.len() as u32;
                g.near.extend_from_slice(&m.vtx);
                while g.near.len() % 4 != 0 {
                    g.near.push(0);
                }
            } else {
                rec.vtx_first = (g.vtx.len() / vertex_bytes) as u32;
                g.vtx.extend_from_slice(&m.vtx);
                rec.idx_first = g.idx.len() as u32;
                g.idx.extend_from_slice(&m.idx);
            }
            g.tris[rec.kind as usize] += m.idx.len() / 3;
            g.count[rec.kind as usize] += 1;
            g.big += m.clip.len();
            g.max_verts = g.max_verts.max(rec.vtx_count as usize);
            recs.push(rec);
        }
        if streamed {
            for (m, rec) in group.iter().zip(&mut recs) {
                rec.idx_first = g.near.len() as u32;
                g.near.extend_from_slice(pack::slice_bytes(&m.idx));
                while g.near.len() % 4 != 0 {
                    g.near.push(0);
                }
            }
            g.largest_cell = g.largest_cell.max(g.near.len() - start);
        }
        g.recs.extend(recs);
    }
    while g.idx.len() % 2 != 0 {
        g.idx.push(0);
    }
    g
}
