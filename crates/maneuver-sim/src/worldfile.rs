//! The simulation's view of a world (`MVSW`): collision triangles and the
//! entities the rules need. The world generator writes it; the compiler copies
//! it into the pack unchanged.
//!
//! Little-endian. Header (64 bytes):
//! `magic "MVSW"`, `version`, `verts`, `tris`, `dummies`, `depots`,
//! `waypoints`, `reserved`, `spawn xyz + yaw`, `bounds radius`, 3 reserved f32.
//! Then `verts × 3 f32`, `tris × 3 u32`, `tris × u8` kinds padded to 4 bytes,
//! `dummies × 8 f32` (base xyz, yaw, height, nape xyz), `depots × 4 f32`
//! (xyz, radius), `waypoints × 3 f32`.

use crate::collide::World;
use crate::math::*;
use crate::sim::{Depot, Dummy, Sim};

pub const MAGIC: u32 = 0x5753_564d;
pub const VERSION: u32 = 1;
pub const HEADER: usize = 64;

struct Rd<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Rd<'a> {
    fn u32(&mut self) -> Result<u32, &'static str> {
        let s = self.b.get(self.at..self.at + 4).ok_or("world file is truncated")?;
        self.at += 4;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f32(&mut self) -> Result<f32, &'static str> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn f32s(&mut self, n: usize) -> Result<Vec<f32>, &'static str> {
        let s = self.b.get(self.at..self.at + n * 4).ok_or("world file is truncated")?;
        self.at += n * 4;
        Ok(s.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    }
    fn u32s(&mut self, n: usize) -> Result<Vec<u32>, &'static str> {
        let s = self.b.get(self.at..self.at + n * 4).ok_or("world file is truncated")?;
        self.at += n * 4;
        Ok(s.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
    }
}

pub fn load(bytes: &[u8]) -> Result<Sim, &'static str> {
    let mut r = Rd { b: bytes, at: 0 };
    if r.u32()? != MAGIC {
        return Err("not a world file");
    }
    if r.u32()? != VERSION {
        return Err("world file version mismatch");
    }
    let (nv, nt, nd, np, nw) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
    r.u32()?;
    let spawn = v3(r.f32()?, r.f32()?, r.f32()?);
    let yaw = r.f32()?;
    let bounds = r.f32()?;
    r.at = HEADER;
    let verts = r.f32s(nv * 3)?;
    let idx = r.u32s(nt * 3)?;
    if idx.iter().any(|&i| i as usize >= nv) {
        return Err("world file index out of range");
    }
    let kinds = r.b.get(r.at..r.at + nt).ok_or("world file is truncated")?.to_vec();
    r.at += (nt + 3) & !3;
    let world = World::build(&verts, &idx, &kinds);
    let mut sim = Sim::new(world, spawn, yaw, bounds);
    for i in 0..nd {
        let f = r.f32s(8)?;
        // The nape follows the giant's build and stance; the file's value is a placeholder.
        let (pos, yaw, height, variant) = (v3(f[0], f[1], f[2]), f[3], f[4], i as u32 % 3);
        let nape = crate::pose::titan_nape(&crate::pose::Skeleton::titan(variant), pos, yaw, height);
        sim.dummies.push(Dummy { pos, yaw, height, nape, alive: true, cut_tick: 0, variant });
    }
    for _ in 0..np {
        let f = r.f32s(4)?;
        sim.depots.push(Depot { pos: v3(f[0], f[1], f[2]), radius: f[3] });
    }
    for _ in 0..nw {
        let f = r.f32s(3)?;
        sim.waypoints.push(v3(f[0], f[1], f[2]));
    }
    Ok(sim)
}

/// Writer used by tests and the test world; the generator has its own in TypeScript.
pub struct Builder {
    pub verts: Vec<f32>,
    pub idx: Vec<u32>,
    pub kinds: Vec<u8>,
    pub dummies: Vec<[f32; 8]>,
    pub depots: Vec<[f32; 4]>,
    pub waypoints: Vec<[f32; 3]>,
    pub spawn: [f32; 4],
    pub bounds: f32,
}

impl Builder {
    pub fn new() -> Builder {
        Builder { verts: Vec::new(), idx: Vec::new(), kinds: Vec::new(), dummies: Vec::new(), depots: Vec::new(), waypoints: Vec::new(), spawn: [0.0, 1.0, 0.0, 0.0], bounds: 1000.0 }
    }
    /// A quad `a b c d`, counter-clockwise seen from its front.
    pub fn quad(&mut self, a: V3, b: V3, c: V3, d: V3, kind: u8) {
        let base = (self.verts.len() / 3) as u32;
        for p in [a, b, c, d] {
            self.verts.extend_from_slice(&[p.x, p.y, p.z]);
        }
        self.idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        self.kinds.extend_from_slice(&[kind, kind]);
    }
    pub fn tri(&mut self, a: V3, b: V3, c: V3, kind: u8) {
        let base = (self.verts.len() / 3) as u32;
        for p in [a, b, c] {
            self.verts.extend_from_slice(&[p.x, p.y, p.z]);
        }
        self.idx.extend_from_slice(&[base, base + 1, base + 2]);
        self.kinds.push(kind);
    }
    pub fn bytes(&self) -> Vec<u8> {
        let mut o = Vec::new();
        let nt = self.idx.len() / 3;
        for v in [MAGIC, VERSION, (self.verts.len() / 3) as u32, nt as u32, self.dummies.len() as u32, self.depots.len() as u32, self.waypoints.len() as u32, 0] {
            o.extend_from_slice(&v.to_le_bytes());
        }
        for v in self.spawn {
            o.extend_from_slice(&v.to_le_bytes());
        }
        o.extend_from_slice(&self.bounds.to_le_bytes());
        o.resize(HEADER, 0);
        for v in &self.verts {
            o.extend_from_slice(&v.to_le_bytes());
        }
        for v in &self.idx {
            o.extend_from_slice(&v.to_le_bytes());
        }
        o.extend_from_slice(&self.kinds);
        while o.len() % 4 != 0 {
            o.push(0);
        }
        for d in &self.dummies {
            for v in d {
                o.extend_from_slice(&v.to_le_bytes());
            }
        }
        for d in &self.depots {
            for v in d {
                o.extend_from_slice(&v.to_le_bytes());
            }
        }
        for d in &self.waypoints {
            for v in d {
                o.extend_from_slice(&v.to_le_bytes());
            }
        }
        o
    }
}
