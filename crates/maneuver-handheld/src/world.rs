//! The static world's table of meshes and the per-frame choice of what to draw.
//!
//! A frame walks the 256 m super-cells. One beyond the middle distance draws
//! its far mesh, unless it is beyond the far distance too; a closer one draws
//! each of its 64 m cells, detailed inside the near distance and simple outside
//! it. Everything is culled against the frustum by its bounds first.
//!
//! Where detailed meshes are read on demand (PSP), a cell whose data has not
//! arrived draws its simple mesh.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use maneuver_pack::{mesh_kind, HandMesh};
use maneuver_sim::math::V3;

use crate::mat;

#[derive(Clone, Copy, Default, Debug)]
pub struct Range {
    pub first: u32,
    pub count: u32,
}

pub struct Cell {
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub near: Range,
    pub mid: Range,
    /// Where the detailed meshes' bytes are in `NEAR`: offset and length (0 when resident or absent).
    pub blob: (u32, u32),
}

struct Super {
    min: [f32; 3],
    max: [f32; 3],
    far: Range,
    cells: Range,
}

#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Stats {
    pub near: u32,
    pub mid: u32,
    pub far: u32,
    /// Detailed cells drawn simple because their data is not in memory.
    pub waiting: u32,
}

/// One mesh to draw: its record, and for a detailed mesh the cell it belongs to.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Pick {
    pub mesh: u32,
    pub cell: u32,
    /// Distance from the eye to the mesh's cell, for drawing front to back.
    pub dist: f32,
}

pub const NO_CELL: u32 = u32::MAX;

pub struct World {
    pub recs: Vec<HandMesh>,
    /// Mesh indices, grouped: every range points in here.
    lists: Vec<u32>,
    pub cells: Vec<Cell>,
    supers: Vec<Super>,
    backdrop: Range,
}

fn grow(min: &mut [f32; 3], max: &mut [f32; 3], lo: &[f32; 3], hi: &[f32; 3]) {
    for a in 0..3 {
        if lo[a] < min[a] {
            min[a] = lo[a];
        }
        if hi[a] > max[a] {
            max[a] = hi[a];
        }
    }
}

impl World {
    /// `streamed`: detailed meshes address `NEAR` by byte. `vertex_bytes` and 2 are the sizes they count in.
    pub fn new(recs: Vec<HandMesh>, super_per_cell: i32, streamed: bool, vertex_bytes: u32) -> World {
        let mut by_cell: BTreeMap<(i32, i32), (Vec<u32>, Vec<u32>)> = BTreeMap::new();
        let mut by_super: BTreeMap<(i32, i32), Vec<u32>> = BTreeMap::new();
        let mut back = Vec::new();
        for (i, r) in recs.iter().enumerate() {
            match r.kind {
                mesh_kind::NEAR => by_cell.entry((r.cz, r.cx)).or_default().0.push(i as u32),
                mesh_kind::MID => by_cell.entry((r.cz, r.cx)).or_default().1.push(i as u32),
                mesh_kind::FAR => by_super.entry((r.cz, r.cx)).or_default().push(i as u32),
                _ => back.push(i as u32),
            }
        }
        let mut lists = Vec::new();
        let mut push = |v: &[u32]| {
            let r = Range { first: lists.len() as u32, count: v.len() as u32 };
            lists.extend_from_slice(v);
            r
        };
        let backdrop = push(&back);
        let empty = || ([f32::MAX; 3], [f32::MIN; 3]);
        let mut super_cells: BTreeMap<(i32, i32), Vec<Cell>> = BTreeMap::new();
        for ((cz, cx), (near, mid)) in &by_cell {
            let (mut min, mut max) = empty();
            let (mut lo, mut hi) = (u32::MAX, 0u32);
            for &i in near.iter().chain(mid) {
                let r = &recs[i as usize];
                grow(&mut min, &mut max, &r.min, &r.max);
            }
            if streamed {
                for &i in near {
                    let r = &recs[i as usize];
                    lo = lo.min(r.vtx_first).min(r.idx_first);
                    hi = hi.max(r.vtx_first + r.vtx_count * vertex_bytes).max(r.idx_first + r.idx_count * 2);
                }
            }
            let near_r = push(near);
            let mid_r = push(mid);
            let blob = if streamed && !near.is_empty() { (lo, hi - lo) } else { (0, 0) };
            super_cells.entry((cz.div_euclid(super_per_cell), cx.div_euclid(super_per_cell))).or_default().push(Cell { min, max, near: near_r, mid: mid_r, blob });
        }
        let mut keys: Vec<(i32, i32)> = super_cells.keys().chain(by_super.keys()).copied().collect();
        keys.sort();
        keys.dedup();
        let mut cells = Vec::new();
        let mut supers = Vec::new();
        for k in keys {
            let (mut min, mut max) = empty();
            let far = match by_super.get(&k) {
                Some(v) => {
                    for &i in v {
                        grow(&mut min, &mut max, &recs[i as usize].min, &recs[i as usize].max);
                    }
                    push(v)
                }
                None => Range::default(),
            };
            let first = cells.len() as u32;
            for c in super_cells.remove(&k).unwrap_or_default() {
                grow(&mut min, &mut max, &c.min, &c.max);
                cells.push(c);
            }
            supers.push(Super { min, max, far, cells: Range { first, count: cells.len() as u32 - first } });
        }
        World { recs, lists, cells, supers, backdrop }
    }

    fn list(&self, r: Range) -> &[u32] {
        &self.lists[r.first as usize..(r.first + r.count) as usize]
    }

    /// The meshes of this frame, appended to `near` (the cells within `lod_near`
    /// of the eye) and `far` (everything else): two lists because a device may
    /// draw them with two depth ranges. `ready(cell)` says whether a cell's
    /// detailed data is in memory.
    #[allow(clippy::too_many_arguments)]
    pub fn pick(&self, planes: &[[f32; 4]; 6], eye: V3, lod_near: f32, lod_mid: f32, lod_far: f32, ready: &dyn Fn(u32) -> bool, far: &mut Vec<Pick>, near: &mut Vec<Pick>) -> Stats {
        let mut stats = Stats::default();
        for &i in self.list(self.backdrop) {
            far.push(Pick { mesh: i, cell: NO_CELL, dist: f32::MAX });
        }
        for s in &self.supers {
            if !mat::visible(planes, &s.min, &s.max) {
                continue;
            }
            let d = mat::box_distance(eye, &s.min, &s.max);
            if s.far.count > 0 && d > lod_mid {
                if d > lod_far {
                    continue;
                }
                for &i in self.list(s.far) {
                    let m = &self.recs[i as usize];
                    if mat::visible(planes, &m.min, &m.max) {
                        far.push(Pick { mesh: i, cell: NO_CELL, dist: d });
                        stats.far += 1;
                    }
                }
                continue;
            }
            for ci in s.cells.first..s.cells.first + s.cells.count {
                let c = &self.cells[ci as usize];
                if !mat::visible(planes, &c.min, &c.max) {
                    continue;
                }
                // The list follows the distance, whichever mesh is drawn: a cell around the eye is in `near`.
                let dist = mat::box_distance(eye, &c.min, &c.max);
                let close = dist < lod_near;
                if close && c.near.count > 0 && ready(ci) {
                    for &i in self.list(c.near) {
                        near.push(Pick { mesh: i, cell: ci, dist });
                        stats.near += 1;
                    }
                } else {
                    if close && c.near.count > 0 {
                        stats.waiting += 1;
                    }
                    for &i in self.list(c.mid) {
                        if close { &mut *near } else { &mut *far }.push(Pick { mesh: i, cell: NO_CELL, dist });
                        stats.mid += 1;
                    }
                }
            }
        }
        stats
    }

    /// Cells whose detailed data should be in memory: those within `radius` of
    /// the eye, with their distance, in no order.
    pub fn wanted(&self, eye: V3, radius: f32, out: &mut Vec<(f32, u32)>) {
        for s in &self.supers {
            if mat::box_distance(eye, &s.min, &s.max) > radius {
                continue;
            }
            for ci in s.cells.first..s.cells.first + s.cells.count {
                let c = &self.cells[ci as usize];
                if c.blob.1 == 0 {
                    continue;
                }
                let d = mat::box_distance(eye, &c.min, &c.max);
                if d <= radius {
                    out.push((d, ci));
                }
            }
        }
    }
}
