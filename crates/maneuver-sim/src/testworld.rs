//! A synthetic town for tests and the tuning harness: a grid of streets lined
//! with gabled houses, a loop of waypoints and a few targets.

use crate::collide::kind;
use crate::math::*;
use crate::worldfile::Builder;

struct Lcg(u32);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.0 >> 8) as f32 / 16777216.0
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
}

/// An axis-aligned house: walls to `h`, a gable roof with the ridge along its longer side.
fn house(b: &mut Builder, x0: f32, z0: f32, x1: f32, z1: f32, h: f32, roof: f32) {
    let p = |x: f32, y: f32, z: f32| v3(x, y, z);
    // Walls, counter-clockwise seen from outside.
    b.quad(p(x0, 0.0, z1), p(x1, 0.0, z1), p(x1, h, z1), p(x0, h, z1), kind::WALL);
    b.quad(p(x1, 0.0, z0), p(x0, 0.0, z0), p(x0, h, z0), p(x1, h, z0), kind::WALL);
    b.quad(p(x1, 0.0, z1), p(x1, 0.0, z0), p(x1, h, z0), p(x1, h, z1), kind::WALL);
    b.quad(p(x0, 0.0, z0), p(x0, 0.0, z1), p(x0, h, z1), p(x0, h, z0), kind::WALL);
    if x1 - x0 >= z1 - z0 {
        let zm = (z0 + z1) * 0.5;
        b.quad(p(x0, h, z1), p(x1, h, z1), p(x1, h + roof, zm), p(x0, h + roof, zm), kind::ROOF);
        b.quad(p(x1, h, z0), p(x0, h, z0), p(x0, h + roof, zm), p(x1, h + roof, zm), kind::ROOF);
        b.tri(p(x1, h, z1), p(x1, h, z0), p(x1, h + roof, zm), kind::WALL);
        b.tri(p(x0, h, z0), p(x0, h, z1), p(x0, h + roof, zm), kind::WALL);
    } else {
        let xm = (x0 + x1) * 0.5;
        b.quad(p(x1, h, z1), p(x1, h, z0), p(xm, h + roof, z0), p(xm, h + roof, z1), kind::ROOF);
        b.quad(p(x0, h, z0), p(x0, h, z1), p(xm, h + roof, z1), p(xm, h + roof, z0), kind::ROOF);
        b.tri(p(x0, h, z1), p(x1, h, z1), p(xm, h + roof, z1), kind::WALL);
        b.tri(p(x1, h, z0), p(x0, h, z0), p(xm, h + roof, z0), kind::WALL);
    }
}

pub fn build() -> Vec<u8> {
    let mut b = Builder::new();
    let mut r = Lcg(7);
    const HALF: f32 = 320.0;
    const BX: f32 = 64.0;
    const BZ: f32 = 52.0;
    const STREET: f32 = 10.0;
    // Ground in 32 m quads.
    let mut z = -HALF;
    while z < HALF {
        let mut x = -HALF;
        while x < HALF {
            b.quad(v3(x, 0.0, z + 32.0), v3(x + 32.0, 0.0, z + 32.0), v3(x + 32.0, 0.0, z), v3(x, 0.0, z), kind::GROUND);
            x += 32.0;
        }
        z += 32.0;
    }
    // Blocks: houses along the two long sides, back to back.
    let mut bz = -HALF + STREET * 0.5;
    while bz + BZ - STREET <= HALF {
        let mut bx = -HALF + STREET * 0.5;
        while bx + BX - STREET <= HALF {
            let (x0, x1) = (bx, bx + BX - STREET);
            let (z0, z1) = (bz, bz + BZ - STREET);
            let depth = (z1 - z0) * 0.5;
            for (za, zb) in [(z0, z0 + depth), (z1 - depth, z1)] {
                let mut x = x0;
                while x < x1 - 4.0 {
                    let w = r.range(6.5, 11.0).min(x1 - x);
                    let h = r.range(7.0, 17.0);
                    house(&mut b, x, za, x + w, zb, h, r.range(3.0, 5.5));
                    x += w;
                }
            }
            bx += BX;
        }
        bz += BZ;
    }
    // A loop of waypoints along the streets, and targets beside it.
    let ring = [(-197.0, -161.0), (187.0, -161.0), (187.0, 151.0), (-5.0, 151.0), (-5.0, -5.0), (-197.0, -5.0)];
    for i in 0..ring.len() {
        let (ax, az) = ring[i];
        let (cx, cz) = ring[(i + 1) % ring.len()];
        let steps = 5;
        for k in 0..steps {
            let t = k as f32 / steps as f32;
            b.waypoints.push([lerp(ax, cx, t), 10.0, lerp(az, cz, t)]);
        }
        let (mx, mz) = ((ax + cx) * 0.5, (az + cz) * 0.5);
        b.dummies.push([mx, 0.0, mz, 0.0, 10.0, mx, 9.0, mz + 0.8]);
    }
    b.depots.push([-5.0, 1.0, -5.0, 6.0]);
    b.spawn = [-197.0, 1.0, -150.0, 0.0];
    b.bounds = HALF - 10.0;
    b.bytes()
}
