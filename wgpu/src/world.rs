//! The static world: which of the pack's meshes a frame draws
//! (`vita/src/world.rs`).
//!
//! A frame walks the 256 m super-cells. One beyond the middle distance draws
//! its far mesh; a closer one draws each of its 64 m cells, detailed inside
//! the near distance and simple outside it. Everything is culled against the
//! frustum by its bounds first.

use std::collections::BTreeMap;

use maneuver_pack::{mesh_kind, MeshRec};
use maneuver_sim::math::V3;

use crate::mat::{self, Mat4};

#[derive(Clone, Copy, Default)]
struct Range {
    first: u32,
    count: u32,
}

struct Cell {
    min: [f32; 3],
    max: [f32; 3],
    near: Range,
    mid: Range,
}

struct Super {
    min: [f32; 3],
    max: [f32; 3],
    far: Range,
    cells: Range,
}

#[derive(Clone, Copy, Default)]
pub struct Stats {
    pub draws: u32,
    pub tris: u32,
    pub near: u32,
    pub mid: u32,
    pub far: u32,
}

/// One mesh of a frame: its matrix with the mesh's bounds folded in, and where its indices and vertices are.
#[derive(Clone, Copy)]
pub struct Draw {
    pub mvp: Mat4,
    pub idx_first: u32,
    pub idx_count: u32,
    pub vtx_first: u32,
}

pub struct World {
    recs: Vec<MeshRec>,
    /// Mesh indices, grouped: every range above points in here.
    lists: Vec<u32>,
    cells: Vec<Cell>,
    supers: Vec<Super>,
    backdrop: Range,
}

fn grow(min: &mut [f32; 3], max: &mut [f32; 3], r: &MeshRec) {
    for a in 0..3 {
        min[a] = min[a].min(r.min[a]);
        max[a] = max[a].max(r.max[a]);
    }
}

impl World {
    /// Groups the pack's meshes: by cell for near and middle, by super-cell for far.
    pub fn new(recs: Vec<MeshRec>, super_per_cell: i32) -> World {
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
            for &i in near.iter().chain(mid) {
                grow(&mut min, &mut max, &recs[i as usize]);
            }
            let near_r = push(near);
            // A cell with one level of detail draws it at every distance.
            let mid_r = if mid.is_empty() { near_r } else { push(mid) };
            super_cells.entry((cz.div_euclid(super_per_cell), cx.div_euclid(super_per_cell))).or_default().push(Cell { min, max, near: near_r, mid: mid_r });
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
                        grow(&mut min, &mut max, &recs[i as usize]);
                    }
                    push(v)
                }
                None => Range::default(),
            };
            let first = cells.len() as u32;
            for c in super_cells.remove(&k).unwrap_or_default() {
                for a in 0..3 {
                    min[a] = min[a].min(c.min[a]);
                    max[a] = max[a].max(c.max[a]);
                }
                cells.push(c);
            }
            supers.push(Super { min, max, far, cells: Range { first, count: cells.len() as u32 - first } });
        }
        World { recs, lists, cells, supers, backdrop }
    }

    fn range(&self, vp: &Mat4, planes: &[[f32; 4]; 6], r: Range, test: bool, out: &mut Vec<Draw>, stats: &mut Stats) {
        for &i in &self.lists[r.first as usize..(r.first + r.count) as usize] {
            let m = &self.recs[i as usize];
            if test && !mat::visible(planes, &m.min, &m.max) {
                continue;
            }
            let mvp = mat::with_bounds(vp, m.min, [m.max[0] - m.min[0], m.max[1] - m.min[1], m.max[2] - m.min[2]]);
            out.push(Draw { mvp, idx_first: m.idx_first, idx_count: m.idx_count, vtx_first: m.vtx_first });
            stats.draws += 1;
            stats.tris += m.idx_count / 3;
        }
    }

    /// The frame's meshes from `eye`, appended to `out`.
    pub fn choose(&self, vp: &Mat4, eye: V3, lod_near: f32, lod_mid: f32, out: &mut Vec<Draw>) -> Stats {
        let planes = mat::planes(vp);
        let mut stats = Stats::default();
        self.range(vp, &planes, self.backdrop, false, out, &mut stats);
        for s in &self.supers {
            if !mat::visible(&planes, &s.min, &s.max) {
                continue;
            }
            if s.far.count > 0 && mat::box_distance(eye, &s.min, &s.max) > lod_mid {
                let before = stats.draws;
                self.range(vp, &planes, s.far, true, out, &mut stats);
                stats.far += stats.draws - before;
                continue;
            }
            for c in &self.cells[s.cells.first as usize..(s.cells.first + s.cells.count) as usize] {
                if !mat::visible(&planes, &c.min, &c.max) {
                    continue;
                }
                let before = stats.draws;
                if mat::box_distance(eye, &c.min, &c.max) < lod_near {
                    self.range(vp, &planes, c.near, false, out, &mut stats);
                    stats.near += stats.draws - before;
                } else {
                    self.range(vp, &planes, c.mid, false, out, &mut stats);
                    stats.mid += stats.draws - before;
                }
            }
        }
        stats
    }
}
