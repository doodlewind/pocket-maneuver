//! WorldIR: the float geometry, source atlas and scene constants the
//! reference exports (`web/scripts/export-world.ts`). Import checks every
//! file against the manifest and changes nothing.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Floats per source vertex: position 3, normal 3, uv 2, tint 3.
pub const STRIDE: usize = 11;
/// Floats per skinned model vertex: position 3, normal 3, tint 3, two bones, the first bone's weight.
pub const SKIN_STRIDE: usize = 12;

pub mod layer {
    pub const BASE: i32 = 0;
    pub const NEAR: i32 = 1;
    pub const MID: i32 = 2;
    pub const FAR: i32 = 3;
    pub const BACKDROP: i32 = 4;
    /// The far layer for the handhelds, in 128 m cells.
    pub const HORIZON: i32 = 5;
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub name: String,
    pub seed: u32,
    pub files: Vec<FileRef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRef {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub sun_dir: [f32; 3],
    pub sun: [f32; 3],
    pub sky: [f32; 3],
    pub bounce: [f32; 3],
    pub atlas: Atlas,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Atlas {
    pub width: usize,
    pub height: usize,
    pub strip_edges: Vec<usize>,
}

pub struct Mesh {
    /// Layer, cell x, cell z for a render bucket; the model id for a model.
    pub head: [i32; 3],
    pub verts: Vec<f32>,
    pub idx: Vec<u32>,
}

pub struct Ir {
    pub manifest: Manifest,
    pub manifest_sha256: String,
    pub scene: Scene,
    pub scene_json: serde_json::Value,
    pub buckets: Vec<Mesh>,
    pub models: Vec<Mesh>,
    pub atlas: Vec<u8>,
    pub world: Vec<u8>,
}

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn meshes(bytes: &[u8], magic: u32, version: i32, head_len: usize, stride: usize) -> Result<Vec<Mesh>, String> {
    let word = |at: usize| bytes.get(at..at + 4).map(|s| i32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or("mesh file is truncated".to_string());
    if word(0)? as u32 != magic || word(4)? != version {
        return Err("mesh file has the wrong magic or version".into());
    }
    let n = word(8)? as usize;
    let mut at = 12;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let mut head = [0i32; 3];
        for h in head.iter_mut().take(head_len) {
            *h = word(at)?;
            at += 4;
        }
        let nv = word(at)? as usize;
        let ni = word(at + 4)? as usize;
        at += 8;
        let vb = bytes.get(at..at + nv * stride * 4).ok_or("mesh file is truncated")?;
        at += nv * stride * 4;
        let ib = bytes.get(at..at + ni * 4).ok_or("mesh file is truncated")?;
        at += ni * 4;
        let verts = vb.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let idx: Vec<u32> = ib.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        if idx.iter().any(|&i| i as usize >= nv) {
            return Err("mesh index out of range".into());
        }
        out.push(Mesh { head, verts, idx });
    }
    Ok(out)
}

pub fn load(dir: &Path) -> Result<Ir, String> {
    let read = |name: &str| std::fs::read(dir.join(name)).map_err(|e| format!("{}: {e}", dir.join(name).display()));
    let manifest_bytes = read("manifest.json")?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|e| format!("manifest.json: {e}"))?;
    if manifest.version != 1 {
        return Err(format!("WorldIR version {} (this compiler reads 1)", manifest.version));
    }
    let file = |name: &str| -> Result<Vec<u8>, String> {
        let r = manifest.files.iter().find(|f| f.path == name).ok_or(format!("manifest.json does not list {name}"))?;
        let bytes = read(name)?;
        if bytes.len() as u64 != r.bytes || sha256(&bytes) != r.sha256 {
            return Err(format!("{name} does not match manifest.json"));
        }
        Ok(bytes)
    };
    let scene_bytes = file("scene.json")?;
    let scene: Scene = serde_json::from_slice(&scene_bytes).map_err(|e| format!("scene.json: {e}"))?;
    let atlas = file("atlas.rgba")?;
    if atlas.len() != scene.atlas.width * scene.atlas.height * 4 {
        return Err("atlas.rgba does not match the atlas size in scene.json".into());
    }
    Ok(Ir {
        manifest_sha256: sha256(&manifest_bytes),
        scene_json: serde_json::from_slice(&scene_bytes).map_err(|e| e.to_string())?,
        buckets: meshes(&file("meshes.bin")?, u32::from_le_bytes(*b"MVIR"), 1, 3, STRIDE)?,
        models: meshes(&file("models.bin")?, u32::from_le_bytes(*b"MVMD"), 2, 1, SKIN_STRIDE)?,
        world: file("world.mvsw")?,
        scene,
        atlas,
        manifest,
    })
}
