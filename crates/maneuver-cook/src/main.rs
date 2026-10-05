//! World compiler: WorldIR in, device pack out.
//!
//! Passes, in order (`RECIPE`): read-source, bake-lighting, merge-cells,
//! quantize, atlas-mips, interface-font, structural-budgets. The compile
//! receipt next to the pack records the source, the profile, every section's
//! size and hash, and the statistics a frame budget is argued from.
//!
//! The profile's `target` picks the lowering: `vita` (this file), or `psp`,
//! `3ds` and `ipod` (`handheld`).
//!
//! `maneuver-cook --in <WorldIR dir> --out <pack> --profile <json> --font <ttf>`
//! `maneuver-cook --in <WorldIR dir> --map <file> [--map-size <pixels>]` writes the town from above alone.

mod bake;
mod font;
mod handheld;
mod ir;
mod texture;

use ir::{layer, Mesh, STRIDE};
use maneuver_pack::{self as pack, mesh_kind, MeshRec, ModelHeader, SkinVertex, Vertex};
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

const RECIPE: &[(&str, u32)] = &[("read-source", 1), ("bake-lighting", 1), ("merge-cells", 1), ("quantize", 1), ("atlas-mips", 1), ("interface-font", 1), ("structural-budgets", 1)];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Profile {
    name: String,
    target: String,
    presentation: Presentation,
    texture: TextureProfile,
    bake: BakeProfile,
    limits: Limits,
    font: FontProfile,
    /// Present for the `psp`, `3ds` and `ipod` targets.
    #[serde(default)]
    handheld: Option<handheld::Handheld>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Presentation {
    render: [u32; 2],
    display: [u32; 2],
    target_fps: u32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TextureProfile {
    format: String,
    max_mips: u32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BakeProfile {
    sun_rays: u32,
    sun_spread: f32,
    ao_rays: u32,
    ao_reach: f32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Limits {
    max_mesh_vertices: usize,
    max_pack_bytes: usize,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FontProfile {
    sizes: Vec<u32>,
    atlas: [usize; 2],
}

struct OutMesh {
    kind: u32,
    cx: i32,
    cz: i32,
    verts: Vec<Vertex>,
    idx: Vec<u16>,
    min: [f32; 3],
    max: [f32; 3],
}

/// Merges baked source meshes into device meshes of at most `limit` vertices.
fn lower(parts: &[(&Mesh, &Vec<[u8; 4]>)], kind: u32, cx: i32, cz: i32, limit: usize) -> Result<Vec<OutMesh>, String> {
    // Chunk first (source vertex, colour), then quantize each chunk over its own bounds.
    struct Chunk<'a> {
        src: Vec<(&'a [f32], [u8; 4])>,
        idx: Vec<u16>,
    }
    let mut chunks = vec![Chunk { src: Vec::new(), idx: Vec::new() }];
    for (mesh, colors) in parts {
        let mut map = vec![u32::MAX; mesh.verts.len() / STRIDE];
        for tri in mesh.idx.chunks_exact(3) {
            let fresh = tri.iter().filter(|&&i| map[i as usize] == u32::MAX).count();
            if chunks.last().unwrap().src.len() + fresh > limit {
                chunks.push(Chunk { src: Vec::new(), idx: Vec::new() });
                map.iter_mut().for_each(|m| *m = u32::MAX);
            }
            let c = chunks.last_mut().unwrap();
            for &i in tri {
                let i = i as usize;
                if map[i] == u32::MAX {
                    map[i] = c.src.len() as u32;
                    c.src.push((&mesh.verts[i * STRIDE..(i + 1) * STRIDE], colors[i]));
                }
                c.idx.push(map[i] as u16);
            }
        }
    }
    let mut out = Vec::new();
    for c in chunks.into_iter().filter(|c| !c.idx.is_empty()) {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for (v, _) in &c.src {
            for a in 0..3 {
                min[a] = min[a].min(v[a]);
                max[a] = max[a].max(v[a]);
            }
        }
        for a in 0..3 {
            if max[a] - min[a] < 0.01 {
                max[a] = min[a] + 0.01;
            }
        }
        let mut verts = Vec::with_capacity(c.src.len());
        for (v, color) in &c.src {
            let mut pos = [0u16; 3];
            for a in 0..3 {
                pos[a] = ((v[a] - min[a]) / (max[a] - min[a]) * 65535.0 + 0.5).clamp(0.0, 65535.0) as u16;
            }
            let u = v[6] / pack::UV_SCALE;
            if u.abs() > 1.0 || v[7] < 0.0 || v[7] > 1.0 {
                return Err(format!("texture coordinate ({}, {}) is outside the quantized range in cell {cx},{cz}", v[6], v[7]));
            }
            verts.push(Vertex { pos, pad: 0, uv: [(u * 32767.0).round() as i16, (v[7] * 32767.0).round() as i16], color: *color });
        }
        out.push(OutMesh { kind, cx, cz, verts, idx: c.idx, min, max });
    }
    Ok(out)
}

fn models(ir: &ir::Ir) -> Result<Vec<u8>, String> {
    const S: usize = ir::SKIN_STRIDE;
    // The detailed player, and the giants' builds at the two densities the Vita draws.
    let mine: Vec<&Mesh> = ir.models.iter().filter(|m| matches!(m.head[0], 0 | 10..=12 | 20..=22)).collect();
    let mut out = (mine.len() as u32).to_le_bytes().to_vec();
    for m in mine {
        let nv = m.verts.len() / S;
        if nv > 65535 {
            return Err(format!("model {} has {nv} vertices; indices are 16-bit", m.head[0]));
        }
        let head = ModelHeader { id: m.head[0] as u32, vtx_count: nv as u32, idx_count: m.idx.len() as u32, pad: 0 };
        out.extend_from_slice(pack::bytes_of(&head));
        for i in 0..nv {
            let v = &m.verts[i * S..(i + 1) * S];
            let c = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            let n = |x: f32| (x.clamp(-1.0, 1.0) * 127.0).round() as i8;
            let w0 = (v[11].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            let sv = SkinVertex { pos: [v[0], v[1], v[2]], normal: [n(v[3]), n(v[4]), n(v[5]), 0], color: [c(v[6]), c(v[7]), c(v[8]), 255], bones: [v[9] as u8, v[10] as u8], weights: [w0, 255 - w0] };
            out.extend_from_slice(pack::bytes_of(&sv));
        }
        for &i in &m.idx {
            out.extend_from_slice(&(i as u16).to_le_bytes());
        }
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    Ok(out)
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let input = PathBuf::from(arg(&args, "--in").ok_or("--in <WorldIR directory> is required")?);
    // The town from above on its own, for the interface (tools/ui.ts): `--in <WorldIR> --map <file> [--map-size <pixels>]`.
    if let Some(path) = arg(&args, "--map") {
        let ir = ir::load(&input)?;
        let sim = maneuver_sim::worldfile::load(&ir.world).map_err(|e| format!("world.mvsw: {e}"))?;
        let size = arg(&args, "--map-size").and_then(|s| s.parse().ok()).unwrap_or(256);
        return std::fs::write(&path, handheld::map(&sim.world, ir.scene.sun_dir, size, handheld::MAP_EXTENT)).map_err(|e| format!("{path}: {e}"));
    }
    let output = PathBuf::from(arg(&args, "--out").ok_or("--out <pack> is required")?);
    let profile_path = PathBuf::from(arg(&args, "--profile").ok_or("--profile <json> is required")?);
    let font_path = PathBuf::from(arg(&args, "--font").ok_or("--font <ttf> is required")?);

    let t_all = Instant::now();
    let profile_bytes = std::fs::read(&profile_path).map_err(|e| format!("{}: {e}", profile_path.display()))?;
    let profile: Profile = serde_json::from_slice(&profile_bytes).map_err(|e| format!("{}: {e}", profile_path.display()))?;
    let hand = match (profile.target.as_str(), &profile.handheld) {
        ("vita", None) if profile.texture.format == "bc1" => None,
        ("psp", Some(h)) => Some((handheld::Target::Psp, h.clone())),
        ("3ds", Some(h)) => Some((handheld::Target::Pica, h.clone())),
        ("ipod", Some(h)) => Some((handheld::Target::Gles, h.clone())),
        _ => return Err(format!("profile {}: target is vita (bc1 textures, no handheld block), psp, 3ds or ipod (with a handheld block)", profile.name)),
    };
    let ir = ir::load(&input)?;
    let sim = maneuver_sim::worldfile::load(&ir.world).map_err(|e| format!("world.mvsw: {e}"))?;

    // bake-lighting
    let t = Instant::now();
    let settings = bake::Settings { sun_rays: profile.bake.sun_rays, sun_spread: profile.bake.sun_spread, ao_rays: profile.bake.ao_rays, ao_reach: profile.bake.ao_reach };
    let colors: Vec<Vec<[u8; 4]>> = ir.buckets.par_iter().map(|m| bake::bake(m, &ir.scene, &sim.world, &settings)).collect();
    let bake_ms = t.elapsed().as_millis();

    // merge-cells + quantize: a cell's near mesh is its base and near buckets, its middle mesh its base and middle buckets.
    let t = Instant::now();
    let mut cells: BTreeMap<(i32, i32), [Vec<usize>; 3]> = BTreeMap::new();
    let mut plan: Vec<(u32, i32, i32, Vec<usize>)> = Vec::new();
    for (i, b) in ir.buckets.iter().enumerate() {
        match b.head[0] {
            layer::BASE | layer::NEAR | layer::MID => cells.entry((b.head[2], b.head[1])).or_default()[b.head[0] as usize].push(i),
            // A handheld's far meshes are the horizon layer's; the Vita's are the far layer's.
            layer::FAR if hand.is_none() => plan.push((mesh_kind::FAR, b.head[1], b.head[2], vec![i])),
            layer::HORIZON if hand.is_some() => plan.push((mesh_kind::FAR, b.head[1], b.head[2], vec![i])),
            layer::FAR | layer::HORIZON => {}
            layer::BACKDROP => plan.push((mesh_kind::BACKDROP, 0, 0, vec![i])),
            l => return Err(format!("unknown layer {l}")),
        }
    }
    for ((cz, cx), [base, near, mid]) in &cells {
        // A cell with nothing but base geometry has one mesh; the runtime draws it at every distance.
        // A handheld keeps that one as the simple mesh, which is always in memory.
        let plain = near.is_empty() && mid.is_empty();
        if !(plain && hand.is_some()) {
            plan.push((mesh_kind::NEAR, *cx, *cz, base.iter().chain(near).copied().collect()));
        }
        if !plain || hand.is_some() {
            plan.push((mesh_kind::MID, *cx, *cz, base.iter().chain(mid).copied().collect()));
        }
    }
    plan.sort_by_key(|p| (p.0, p.2, p.1));
    let limit = profile.limits.max_mesh_vertices;
    if let Some((target, h)) = hand {
        return cook_handheld(Cooked { profile: &profile, profile_bytes: &profile_bytes, ir: &ir, sim: &sim, colors: &colors, plan: &plan, bake_ms, output: &output, font_path: &font_path, t_all }, target, &h);
    }
    let lowered: Vec<Vec<OutMesh>> = plan
        .par_iter()
        .map(|(kind, cx, cz, parts)| {
            let parts: Vec<(&Mesh, &Vec<[u8; 4]>)> = parts.iter().map(|&i| (&ir.buckets[i], &colors[i])).collect();
            lower(&parts, *kind, *cx, *cz, limit)
        })
        .collect::<Result<_, _>>()?;
    let mut recs = Vec::new();
    let mut vtx = Vec::new();
    let mut idx = Vec::new();
    let mut tris = [0usize; 4];
    let mut count = [0usize; 4];
    let mut max_verts = 0;
    for m in lowered.into_iter().flatten() {
        recs.push(MeshRec { kind: m.kind, cx: m.cx, cz: m.cz, vtx_first: vtx.len() as u32, vtx_count: m.verts.len() as u32, idx_first: idx.len() as u32, idx_count: m.idx.len() as u32, min: m.min, max: m.max, pad: 0 });
        tris[m.kind as usize] += m.idx.len() / 3;
        count[m.kind as usize] += 1;
        max_verts = max_verts.max(m.verts.len());
        vtx.extend_from_slice(&m.verts);
        idx.extend_from_slice(&m.idx);
    }
    while idx.len() % 2 != 0 {
        idx.push(0);
    }
    let lower_ms = t.elapsed().as_millis();

    // atlas-mips-bc1, interface-font
    let t = Instant::now();
    let (tex, mips) = texture::atlas(&ir.atlas, ir.scene.atlas.width, ir.scene.atlas.height, &ir.scene.atlas.strip_edges, profile.texture.max_mips);
    let ttf = std::fs::read(&font_path).map_err(|e| format!("{}: {e}", font_path.display()))?;
    let font = font::bake(&ttf, &profile.font.sizes, profile.font.atlas[0], profile.font.atlas[1])?;
    let tex_ms = t.elapsed().as_millis();

    let stats = json!({
        "meshes": {"near": count[0], "mid": count[1], "far": count[2], "backdrop": count[3]},
        "triangles": {"near": tris[0], "mid": tris[1], "far": tris[2], "backdrop": tris[3]},
        "vertices": vtx.len(),
        "indices": idx.len(),
        "maxMeshVertices": max_verts,
        "collisionTriangles": sim.world.tris.len(),
        "atlasMips": mips,
    });
    let mut scene = ir.scene_json.clone();
    if let Some(o) = scene.as_object_mut() {
        // The runtime computes the sky from the same constants.
        o.remove("skyTable");
        o.remove("source");
    }
    let meta = json!({
        "name": ir.manifest.name,
        "seed": ir.manifest.seed,
        "source": ir.manifest_sha256,
        "profile": profile.name,
        "presentation": {"render": profile.presentation.render, "display": profile.presentation.display, "targetFps": profile.presentation.target_fps},
        "scene": scene,
        "stats": stats,
    });
    let meta_bytes = serde_json::to_vec(&meta).map_err(|e| e.to_string())?;
    let model_bytes = models(&ir)?;
    let sections: Vec<(u32, &[u8])> = vec![
        (pack::META, &meta_bytes),
        (pack::TEX0, &tex),
        (pack::MESH, pack::slice_bytes(&recs)),
        (pack::VTX0, pack::slice_bytes(&vtx)),
        (pack::IDX0, pack::slice_bytes(&idx)),
        (pack::MODL, &model_bytes),
        (pack::FONT, &font),
        (pack::SIMW, &ir.world),
    ];
    let bytes = pack::write(&sections);

    // structural-budgets
    if max_verts > limit {
        return Err(format!("a mesh has {max_verts} vertices; the profile allows {limit}"));
    }
    if bytes.len() > profile.limits.max_pack_bytes {
        return Err(format!("the pack is {} bytes; the profile allows {}", bytes.len(), profile.limits.max_pack_bytes));
    }
    pack::Pack::parse(&bytes)?;

    // Publish the pack, then the receipt that names it.
    let temp = output.with_extension("tmp");
    std::fs::write(&temp, &bytes).map_err(|e| format!("{}: {e}", temp.display()))?;
    std::fs::rename(&temp, &output).map_err(|e| format!("{}: {e}", output.display()))?;
    let receipt = json!({
        "compiler": {"name": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION"), "packVersion": pack::VERSION},
        "recipe": RECIPE.iter().map(|(n, v)| json!({"pass": n, "version": v})).collect::<Vec<_>>(),
        "source": {"name": ir.manifest.name, "seed": ir.manifest.seed, "manifestSha256": ir.manifest_sha256},
        "profile": {"name": profile.name, "sha256": ir::sha256(&profile_bytes)},
        "artifact": {"path": output.file_name().map(|s| s.to_string_lossy()), "bytes": bytes.len(), "sha256": ir::sha256(&bytes)},
        "sections": sections.iter().map(|(t, d)| json!({"tag": String::from_utf8_lossy(&t.to_le_bytes()), "bytes": d.len(), "sha256": ir::sha256(d)})).collect::<Vec<_>>(),
        "stats": stats,
        "frameBudget": {"fps": profile.presentation.target_fps, "milliseconds": 1000.0 / profile.presentation.target_fps as f64},
        "timingsMs": {"bake": bake_ms, "lower": lower_ms, "textures": tex_ms, "total": t_all.elapsed().as_millis()},
    });
    let receipt_path = PathBuf::from(format!("{}.compile.json", output.display()));
    std::fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", receipt_path.display()))?;
    println!("{}: {:.1} MB, {} meshes, {} vertices; bake {bake_ms} ms, lower {lower_ms} ms, textures {tex_ms} ms", output.display(), bytes.len() as f64 / 1e6, recs.len(), vtx.len());
    println!("triangles: near {} mid {} far {} backdrop {}; largest mesh {} vertices", tris[0], tris[1], tris[2], tris[3], max_verts);
    Ok(())
}

struct Cooked<'a> {
    profile: &'a Profile,
    profile_bytes: &'a [u8],
    ir: &'a ir::Ir,
    sim: &'a maneuver_sim::Sim,
    colors: &'a [Vec<[u8; 4]>],
    plan: &'a [(u32, i32, i32, Vec<usize>)],
    bake_ms: u128,
    output: &'a PathBuf,
    font_path: &'a PathBuf,
    t_all: Instant,
}

fn cook_handheld(c: Cooked, target: handheld::Target, h: &handheld::Handheld) -> Result<(), String> {
    let (profile, ir) = (c.profile, c.ir);
    let format = handheld::format_of(&profile.texture.format)?;
    let psp = target == handheld::Target::Psp;
    let fits = match target {
        handheld::Target::Psp => matches!(format, pack::tex_format::PSP_DXT1 | pack::tex_format::PSP_5650 | pack::tex_format::PSP_T8),
        handheld::Target::Pica => format == pack::tex_format::PICA_RGB565,
        handheld::Target::Gles => format == pack::tex_format::GLES_RGB565,
    };
    if !fits {
        return Err(format!("profile {}: texture format {} is not one of target {}", profile.name, profile.texture.format, profile.target));
    }

    // atlas-mips
    let t = Instant::now();
    let (tex, pages, mips) = handheld::atlas(&ir.atlas, ir.scene.atlas.width, ir.scene.atlas.height, &ir.scene.atlas.strip_edges, h.page, format, profile.texture.max_mips)?;
    let ttf = std::fs::read(c.font_path).map_err(|e| format!("{}: {e}", c.font_path.display()))?;
    let font = handheld::font(&font::bake(&ttf, &profile.font.sizes, profile.font.atlas[0], profile.font.atlas[1])?, target)?;
    let tex_ms = t.elapsed().as_millis();

    // merge-cells + quantize
    let t = Instant::now();
    let limit = profile.limits.max_mesh_vertices;
    let lowered: Vec<Vec<handheld::Lowered>> = c
        .plan
        .par_iter()
        .map(|(kind, cx, cz, parts)| {
            let parts: Vec<(&Mesh, &Vec<[u8; 4]>)> = parts.iter().map(|&i| (&ir.buckets[i], &c.colors[i])).collect();
            handheld::lower(&parts, *kind, *cx, *cz, h, pages, target, limit)
        })
        .collect::<Result<_, _>>()?;
    let vertex_bytes = if psp { core::mem::size_of::<pack::PspVertex>() } else { core::mem::size_of::<pack::PicaVertex>() };
    // Blocks of 4 x 4 cells: 256 m of detailed or simple cells, 512 m of far cells.
    let geo = if psp { handheld::assemble(lowered, h.stream_near, vertex_bytes) } else { handheld::assemble_grouped(lowered, vertex_bytes, 4) };
    let lower_ms = t.elapsed().as_millis();

    let (model_bytes, model_stats) = handheld::models(ir, &h.models, target)?;
    // The PSP reads the collision grid as built; the 3DS and the iPod touch have the memory to build it, and the 3DS a 32 MiB package to fit.
    let simg = if psp { maneuver_sim::worldfile::write_built(c.sim) } else { ir.world.clone() };
    let scene = handheld::scene(&ir.scene_json, h, profile.presentation.render, pages, c.sim.dummies.len() as u32)?;

    let stats = json!({
        "meshes": {"near": geo.count[0], "mid": geo.count[1], "far": geo.count[2], "backdrop": geo.count[3]},
        "triangles": {"near": geo.tris[0], "mid": geo.tris[1], "far": geo.tris[2], "backdrop": geo.tris[3]},
        "largeTriangles": geo.big,
        "residentVertexBytes": geo.vtx.len(),
        "residentIndices": geo.idx.len(),
        "onDemandBytes": geo.near.len(),
        "largestOnDemandCell": geo.largest_cell,
        "maxMeshVertices": geo.max_verts,
        "collisionTriangles": c.sim.world.tris.len(),
        "atlas": {"pages": pages, "page": h.page, "mips": mips, "format": profile.texture.format},
        "models": model_stats,
    });
    let meta = json!({
        "name": ir.manifest.name,
        "seed": ir.manifest.seed,
        "source": ir.manifest_sha256,
        "profile": profile.name,
        "target": profile.target,
        "presentation": {"render": profile.presentation.render, "display": profile.presentation.display, "targetFps": profile.presentation.target_fps},
        "stats": stats,
    });
    let meta_bytes = serde_json::to_vec(&meta).map_err(|e| e.to_string())?;
    let mut sections: Vec<(u32, &[u8])> = vec![
        (pack::META, &meta_bytes),
        (pack::HSCN, pack::bytes_of(&scene)),
        (pack::TEX0, &tex),
        (pack::HMSH, pack::slice_bytes(&geo.recs)),
        (pack::VTX0, &geo.vtx),
        (pack::IDX0, pack::slice_bytes(&geo.idx)),
        (pack::CLIP, &geo.clip),
        (pack::MODL, &model_bytes),
        (pack::FONT, &font),
        (if psp { pack::SIMG } else { pack::SIMW }, &simg),
    ];
    // The town from above, in the 3DS pack since its lower screen was drawn from it.
    let map = if target == handheld::Target::Pica { handheld::map(&c.sim.world, ir.scene.sun_dir, 240, handheld::MAP_EXTENT) } else { Vec::new() };
    if target == handheld::Target::Pica {
        sections.push((pack::MAPT, &map));
    }
    if h.stream_near {
        // Last, so the sections a runtime reads at start are contiguous.
        sections.push((pack::NEAR, &geo.near));
    }
    let bytes = pack::write(&sections);
    if bytes.len() > profile.limits.max_pack_bytes {
        return Err(format!("the pack is {} bytes; the profile allows {}", bytes.len(), profile.limits.max_pack_bytes));
    }
    pack::Pack::parse(&bytes)?;

    let temp = c.output.with_extension("tmp");
    std::fs::write(&temp, &bytes).map_err(|e| format!("{}: {e}", temp.display()))?;
    std::fs::rename(&temp, c.output).map_err(|e| format!("{}: {e}", c.output.display()))?;
    let receipt = json!({
        "compiler": {"name": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION"), "packVersion": pack::VERSION},
        "recipe": RECIPE.iter().map(|(n, v)| json!({"pass": n, "version": v})).collect::<Vec<_>>(),
        "source": {"name": ir.manifest.name, "seed": ir.manifest.seed, "manifestSha256": ir.manifest_sha256},
        "profile": {"name": profile.name, "sha256": ir::sha256(c.profile_bytes)},
        "artifact": {"path": c.output.file_name().map(|s| s.to_string_lossy()), "bytes": bytes.len(), "sha256": ir::sha256(&bytes)},
        "sections": sections.iter().map(|(t, d)| json!({"tag": String::from_utf8_lossy(&t.to_le_bytes()), "bytes": d.len(), "sha256": ir::sha256(d)})).collect::<Vec<_>>(),
        "stats": stats,
        "frameBudget": {"fps": profile.presentation.target_fps, "milliseconds": 1000.0 / profile.presentation.target_fps as f64},
        "timingsMs": {"bake": c.bake_ms, "lower": lower_ms, "textures": tex_ms, "total": c.t_all.elapsed().as_millis()},
    });
    let receipt_path = PathBuf::from(format!("{}.compile.json", c.output.display()));
    std::fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", receipt_path.display()))?;
    let resident = bytes.len() - geo.near.len();
    println!("{}: {:.1} MB ({:.1} MB read at start, {:.1} MB on demand), {} meshes", c.output.display(), bytes.len() as f64 / 1e6, resident as f64 / 1e6, geo.near.len() as f64 / 1e6, geo.recs.len());
    println!("triangles: near {} mid {} far {} backdrop {}; {} large; largest mesh {} vertices", geo.tris[0], geo.tris[1], geo.tris[2], geo.tris[3], geo.big, geo.max_verts);
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("maneuver-cook: {e}");
        std::process::exit(1);
    }
}
