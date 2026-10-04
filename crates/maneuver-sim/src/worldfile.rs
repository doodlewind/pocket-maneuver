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

use alloc::vec::Vec;

use crate::collide::{Tri, World};
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
    entities(&mut sim, &mut r, nd, np, nw)?;
    Ok(sim)
}

fn entities(sim: &mut Sim, r: &mut Rd, nd: usize, np: usize, nw: usize) -> Result<(), &'static str> {
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
    Ok(())
}

/// The built form (`MVSG`): the collision grid as `World` holds it, so a device
/// reads it straight into place. Header (64 bytes): `magic "MVSG"`, `version`,
/// `tris`, `items`, `dummies`, `depots`, `waypoints`, `nx`, `nz`, then f32
/// `min_x`, `min_z`, `spawn xyz + yaw`, `bounds radius`. Then `tris × Tri`
/// (52 bytes), `(nx × nz + 1) × u32` cell starts, `items × u32`, and the
/// entities as in `MVSW`.
pub const BUILT_MAGIC: u32 = 0x4753_564d;
pub const BUILT_VERSION: u32 = 1;
pub const TRI_BYTES: usize = core::mem::size_of::<Tri>();

#[derive(Clone, Copy, Debug)]
pub struct Built {
    pub tris: usize,
    pub items: usize,
    pub dummies: usize,
    pub depots: usize,
    pub waypoints: usize,
    pub nx: i32,
    pub nz: i32,
    pub min_x: f32,
    pub min_z: f32,
    pub spawn: V3,
    pub yaw: f32,
    pub bounds: f32,
}

impl Built {
    pub fn parse(head: &[u8]) -> Result<Built, &'static str> {
        let mut r = Rd { b: head, at: 0 };
        if r.u32()? != BUILT_MAGIC {
            return Err("not a built world file");
        }
        if r.u32()? != BUILT_VERSION {
            return Err("built world file version mismatch");
        }
        let (tris, items, dummies, depots, waypoints) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
        let (nx, nz) = (r.u32()? as i32, r.u32()? as i32);
        let (min_x, min_z) = (r.f32()?, r.f32()?);
        let spawn = v3(r.f32()?, r.f32()?, r.f32()?);
        let (yaw, bounds) = (r.f32()?, r.f32()?);
        if nx < 1 || nz < 1 || nx > 4096 || nz > 4096 {
            return Err("built world file grid size");
        }
        Ok(Built { tris, items, dummies, depots, waypoints, nx, nz, min_x, min_z, spawn, yaw, bounds })
    }
    pub fn cells(&self) -> usize {
        self.nx as usize * self.nz as usize
    }
    /// Bytes of the three grid arrays, in file order.
    pub fn array_bytes(&self) -> [usize; 3] {
        [self.tris * TRI_BYTES, (self.cells() + 1) * 4, self.items * 4]
    }
    pub fn entity_bytes(&self) -> usize {
        (self.dummies * 8 + self.depots * 4 + self.waypoints * 3) * 4
    }
    /// The simulation over grid arrays already in memory and the entity records.
    pub fn finish(&self, tris: Vec<Tri>, start: Vec<u32>, items: Vec<u32>, entity_bytes: &[u8]) -> Result<Sim, &'static str> {
        let world = World::from_raw(tris, start, items, self.min_x, self.min_z, self.nx, self.nz)?;
        let mut sim = Sim::new(world, self.spawn, self.yaw, self.bounds);
        let mut r = Rd { b: entity_bytes, at: 0 };
        entities(&mut sim, &mut r, self.dummies, self.depots, self.waypoints)?;
        Ok(sim)
    }
}

/// Reads `n` records of plain data from little-endian bytes into a vector.
///
/// # Safety
/// `T` is plain data valid for every bit pattern the file can hold.
unsafe fn records<T: Copy>(bytes: &[u8], n: usize) -> Result<Vec<T>, &'static str> {
    let size = n * core::mem::size_of::<T>();
    if bytes.len() < size {
        return Err("built world file is truncated");
    }
    let mut v = Vec::<T>::with_capacity(n);
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), v.as_mut_ptr() as *mut u8, size);
    v.set_len(n);
    Ok(v)
}

/// Loads the built form from one buffer.
pub fn load_built(bytes: &[u8]) -> Result<Sim, &'static str> {
    let h = Built::parse(bytes.get(..HEADER).ok_or("built world file is truncated")?)?;
    let [a, b, c] = h.array_bytes();
    let rest = &bytes[HEADER..];
    if rest.len() < a + b + c + h.entity_bytes() {
        return Err("built world file is truncated");
    }
    // `Tri` is `repr(C)` floats and one kind byte; the kind is checked by use, not by value.
    let tris = unsafe { records::<Tri>(rest, h.tris)? };
    let start = unsafe { records::<u32>(&rest[a..], h.cells() + 1)? };
    let items = unsafe { records::<u32>(&rest[a + b..], h.items)? };
    h.finish(tris, start, items, &rest[a + b + c..])
}

/// Writes the built form of a loaded world.
pub fn write_built(sim: &Sim) -> Vec<u8> {
    let (tris, start, items, min_x, min_z, nx, nz) = sim.world.raw();
    let mut o = Vec::new();
    for v in [BUILT_MAGIC, BUILT_VERSION, tris.len() as u32, items.len() as u32, sim.dummies.len() as u32, sim.depots.len() as u32, sim.waypoints.len() as u32, nx as u32, nz as u32] {
        o.extend_from_slice(&v.to_le_bytes());
    }
    for v in [min_x, min_z, sim.spawn.x, sim.spawn.y, sim.spawn.z, sim.spawn_yaw, sim.bounds] {
        o.extend_from_slice(&v.to_le_bytes());
    }
    o.resize(HEADER, 0);
    for t in tris {
        for v in [t.v0, t.e1, t.e2, t.n] {
            for c in [v.x, v.y, v.z] {
                o.extend_from_slice(&c.to_le_bytes());
            }
        }
        o.extend_from_slice(&[t.kind, 0, 0, 0]);
    }
    for v in start.iter().chain(items) {
        o.extend_from_slice(&v.to_le_bytes());
    }
    for d in &sim.dummies {
        for v in [d.pos.x, d.pos.y, d.pos.z, d.yaw, d.height, d.nape.x, d.nape.y, d.nape.z] {
            o.extend_from_slice(&v.to_le_bytes());
        }
    }
    for d in &sim.depots {
        for v in [d.pos.x, d.pos.y, d.pos.z, d.radius] {
            o.extend_from_slice(&v.to_le_bytes());
        }
    }
    for w in &sim.waypoints {
        for v in [w.x, w.y, w.z] {
            o.extend_from_slice(&v.to_le_bytes());
        }
    }
    o
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
