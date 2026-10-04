//! Lighting baked into vertex colours.
//!
//! Each vertex takes `tint × (sun × max(N·L, 0) × visibility + hemisphere(N) × openness)`:
//! visibility from rays toward the sun, openness from cosine-weighted rays
//! over the hemisphere. The rays run against the simulation's collision
//! world, so the bake and the game agree on where the walls are.

use crate::ir::{layer, Mesh, Scene, STRIDE};
use maneuver_sim::collide::{mask, World};
use maneuver_sim::math::{v3, V3};
use rayon::prelude::*;

pub struct Settings {
    pub sun_rays: u32,
    pub sun_spread: f32,
    pub ao_rays: u32,
    pub ao_reach: f32,
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^ (x >> 16)
}

fn unit(seed: u32) -> f32 {
    (hash(seed) >> 8) as f32 / 16777216.0
}

fn basis(n: V3) -> (V3, V3) {
    let a = if n.y.abs() < 0.9 { v3(0.0, 1.0, 0.0) } else { v3(1.0, 0.0, 0.0) };
    let t = n.cross(a).norm();
    (t, n.cross(t))
}

fn srgb(x: f32) -> f32 {
    x.max(0.0).powf(1.0 / 2.2)
}

/// One colour per vertex: sRGB-encoded `tint × light`, at half scale.
pub fn bake(mesh: &Mesh, scene: &Scene, world: &World, s: &Settings) -> Vec<[u8; 4]> {
    let sun = v3(scene.sun_dir[0], scene.sun_dir[1], scene.sun_dir[2]).norm();
    let (st, sb) = basis(sun);
    let backdrop = mesh.head[0] == layer::BACKDROP;
    (0..mesh.verts.len() / STRIDE)
        .into_par_iter()
        .map(|i| {
            let v = &mesh.verts[i * STRIDE..(i + 1) * STRIDE];
            let p = v3(v[0], v[1], v[2]);
            let n = v3(v[3], v[4], v[5]).norm();
            let seed = hash(v[0].to_bits() ^ hash(v[1].to_bits() ^ hash(v[2].to_bits() ^ (i as u32).wrapping_mul(0x9e37_79b9))));
            let o = p + n * 0.07;
            let ndl = n.dot(sun).max(0.0);

            // A ground vertex under a roof sees the inside of a house; give it plain shade so
            // the dark does not bleed out under the walls.
            let inside = !backdrop && n.y > 0.9 && matches!(world.raycast(o, v3(0.0, 1.0, 0.0), 40.0, mask::ALL), Some(h) if !h.front);

            let mut vis = 0.0;
            if ndl > 0.0 && !backdrop && !inside {
                for k in 0..s.sun_rays {
                    let (a, b) = (unit(seed ^ (k * 2 + 1)) - 0.5, unit(seed ^ (k * 2 + 2)) - 0.5);
                    let d = (sun + st * (a * 2.0 * s.sun_spread) + sb * (b * 2.0 * s.sun_spread)).norm();
                    if world.raycast(o, d, 700.0, mask::ALL).is_none() {
                        vis += 1.0;
                    }
                }
                vis /= s.sun_rays as f32;
            } else if backdrop {
                vis = 1.0;
            }

            let mut open = 1.0;
            if inside {
                open = 0.55;
            } else if !backdrop {
                let (t, b) = basis(n);
                let mut occ = 0.0;
                for k in 0..s.ao_rays {
                    // Stratified over the disc, cosine-weighted over the hemisphere.
                    let u1 = (k as f32 + unit(seed ^ (0x100 + k))) / s.ao_rays as f32;
                    let u2 = unit(seed ^ (0x200 + k));
                    let r = u1.sqrt();
                    let phi = u2 * std::f32::consts::TAU;
                    let d = (t * (r * phi.cos()) + b * (r * phi.sin()) + n * (1.0 - u1).max(0.0).sqrt()).norm();
                    if let Some(h) = world.raycast(o, d, s.ao_reach, mask::ALL) {
                        let k = 1.0 - h.t / s.ao_reach;
                        occ += k * k;
                    }
                }
                open = 1.0 - 0.85 * occ / s.ao_rays as f32;
            }

            let up = 0.5 + 0.5 * n.y;
            let mut out = [0u8; 4];
            for c in 0..3 {
                let hemi = scene.bounce[c] + (scene.sky[c] - scene.bounce[c]) * up;
                let light = scene.sun[c] * ndl * vis + hemi * open;
                let tint = v[8 + c].max(0.0).powf(2.2);
                out[c] = (srgb(tint * light) / maneuver_pack::COLOR_SCALE * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            }
            out[3] = 255;
            out
        })
        .collect()
}
