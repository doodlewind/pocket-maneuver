//! The device pack (`MVPK`): what the world compiler writes and a runtime
//! loads in one read.
//!
//! Little-endian. Header: `"MVPK"`, version, section count, zero; then one
//! 16-byte entry per section (tag, offset, size, zero). Sections start on
//! 16-byte boundaries.
//!
//! | Tag    | Contents |
//! | ------ | -------- |
//! | `META` | JSON: scene constants, identity, statistics |
//! | `TEX0` | the atlas: `TexHeader`, then BC1 levels, largest first, block rows in order |
//! | `MESH` | `MeshRec` table |
//! | `VTX0` | `Vertex` records of every static mesh |
//! | `IDX0` | `u16` indices, relative to each mesh's first vertex |
//! | `MODL` | skinned models: count, then per model `ModelHeader`, `SkinVertex` records, `u16` indices padded to 4 bytes |
//! | `FONT` | interface glyphs: `FontHeader`, `Glyph` table, 8-bit coverage |
//! | `SIMW` | the simulation's world file, unchanged |

pub const MAGIC: u32 = u32::from_le_bytes(*b"MVPK");
pub const VERSION: u32 = 2;

pub const fn tag(t: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*t)
}
pub const META: u32 = tag(b"META");
pub const TEX0: u32 = tag(b"TEX0");
pub const MESH: u32 = tag(b"MESH");
pub const VTX0: u32 = tag(b"VTX0");
pub const IDX0: u32 = tag(b"IDX0");
pub const MODL: u32 = tag(b"MODL");
pub const FONT: u32 = tag(b"FONT");
pub const SIMW: u32 = tag(b"SIMW");

/// Which mesh of a place in the grid a record is.
pub mod mesh_kind {
    /// A 64 m cell with its detailed geometry.
    pub const NEAR: u32 = 0;
    /// The same cell with its simple geometry.
    pub const MID: u32 = 1;
    /// A 256 m super-cell's far geometry.
    pub const FAR: u32 = 2;
    /// Always drawn.
    pub const BACKDROP: u32 = 3;
}

/// One static mesh: a range of vertices and indices and its bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct MeshRec {
    pub kind: u32,
    pub cx: i32,
    pub cz: i32,
    pub vtx_first: u32,
    pub vtx_count: u32,
    pub idx_first: u32,
    pub idx_count: u32,
    /// Positions dequantize as `min + q / 65535 × (max - min)`.
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub pad: u32,
}

/// Static vertex, 16 bytes: position `u16 × 3` normalized over the mesh's
/// bounds, texture coordinates `i16 × 2` normalized (`u / UV_SCALE`, `v`),
/// colour `u8 × 4` (baked light × tint, half scale, sRGB-encoded).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vertex {
    pub pos: [u16; 3],
    pub pad: u16,
    pub uv: [i16; 2],
    pub color: [u8; 4],
}

/// `u` is stored divided by this, so eight texture repeats fit an `i16`.
pub const UV_SCALE: f32 = 8.0;
/// Colours are stored at half scale: a shader multiplies by this.
pub const COLOR_SCALE: f32 = 2.0;

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct TexHeader {
    pub width: u32,
    pub height: u32,
    pub mips: u32,
    /// 1: BC1.
    pub format: u32,
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct ModelHeader {
    /// 0: the player; 10 to 12: the giants' builds; 20 to 22: the same, coarse.
    pub id: u32,
    pub vtx_count: u32,
    pub idx_count: u32,
    pub pad: u32,
}

/// Skinned vertex, 24 bytes: bind-pose position, normal `i8 × 3` normalized,
/// sRGB tint, two bone indices and their weights (`u8` normalized, summing to 255).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct SkinVertex {
    pub pos: [f32; 3],
    pub normal: [i8; 4],
    pub color: [u8; 4],
    pub bones: [u8; 2],
    pub weights: [u8; 2],
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct FontHeader {
    pub width: u32,
    pub height: u32,
    pub glyphs: u32,
    pub pad: u32,
}

/// One glyph at one size. Sizes are pixel heights at 960 × 544.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Glyph {
    pub code: u16,
    pub size: u16,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub left: i16,
    /// From the baseline up to the glyph's top row.
    pub top: i16,
    pub advance: f32,
}

/// Bytes of a `repr(C)` value without padding surprises: every struct here is
/// made of 4-byte-aligned fields with explicit padding.
pub fn bytes_of<T: Copy>(v: &T) -> &[u8] {
    unsafe { core::slice::from_raw_parts(v as *const T as *const u8, core::mem::size_of::<T>()) }
}
pub fn slice_bytes<T: Copy>(v: &[T]) -> &[u8] {
    unsafe { core::slice::from_raw_parts(v.as_ptr() as *const u8, core::mem::size_of_val(v)) }
}
/// Reads a `repr(C)` record at `at`; the bytes need not be aligned.
pub fn read<T: Copy>(b: &[u8], at: usize) -> Option<T> {
    let n = core::mem::size_of::<T>();
    let s = b.get(at..at + n)?;
    Some(unsafe { core::ptr::read_unaligned(s.as_ptr() as *const T) })
}

/// A parsed pack over borrowed bytes.
pub struct Pack<'a> {
    pub bytes: &'a [u8],
    sections: Vec<(u32, usize, usize)>,
}

impl<'a> Pack<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Pack<'a>, String> {
        let word = |i: usize| read::<u32>(bytes, i).ok_or_else(|| "pack is truncated".to_string());
        if word(0)? != MAGIC {
            return Err("not a pack".into());
        }
        if word(4)? != VERSION {
            return Err(format!("pack version {} (this build reads {VERSION})", word(4)?));
        }
        let n = word(8)? as usize;
        let mut sections = Vec::with_capacity(n);
        for i in 0..n {
            let at = 16 + i * 16;
            let (t, off, size) = (word(at)?, word(at + 4)? as usize, word(at + 8)? as usize);
            if off + size > bytes.len() {
                return Err("pack section runs past the end".into());
            }
            sections.push((t, off, size));
        }
        Ok(Pack { bytes, sections })
    }
    pub fn section(&self, t: u32) -> Result<&'a [u8], String> {
        self.sections.iter().find(|s| s.0 == t).map(|s| &self.bytes[s.1..s.1 + s.2]).ok_or_else(|| format!("pack has no {} section", String::from_utf8_lossy(&t.to_le_bytes())))
    }
    pub fn meshes(&self) -> Result<Vec<MeshRec>, String> {
        let b = self.section(MESH)?;
        let n = core::mem::size_of::<MeshRec>();
        Ok((0..b.len() / n).filter_map(|i| read::<MeshRec>(b, i * n)).collect())
    }
}

/// Assembles a pack from sections.
pub fn write(sections: &[(u32, &[u8])]) -> Vec<u8> {
    let head = 16 + sections.len() * 16;
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(sections.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    let mut at = (head + 15) & !15;
    for (t, data) in sections {
        out.extend_from_slice(&t.to_le_bytes());
        out.extend_from_slice(&(at as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        at = (at + data.len() + 15) & !15;
    }
    for (_, data) in sections {
        while out.len() % 16 != 0 {
            out.push(0);
        }
        out.extend_from_slice(data);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        assert_eq!(core::mem::size_of::<Vertex>(), 16);
        assert_eq!(core::mem::size_of::<MeshRec>(), 56);
        assert_eq!(core::mem::size_of::<SkinVertex>(), 24);
        assert_eq!(core::mem::size_of::<Glyph>(), 20);
    }

    #[test]
    fn round_trip() {
        let rec = MeshRec { kind: mesh_kind::FAR, cx: -2, cz: 3, vtx_count: 9, ..Default::default() };
        let pack = write(&[(META, b"{}"), (MESH, bytes_of(&rec))]);
        let p = Pack::parse(&pack).unwrap();
        assert_eq!(p.section(META).unwrap(), b"{}");
        assert_eq!(p.meshes().unwrap(), vec![rec]);
        assert!(p.section(TEX0).is_err());
    }
}
