//! Procedural animation on a nineteen-bone skeleton.
//!
//! The player's pose is a blend of key poses chosen by the action, with
//! overlays that play on events (an arm thrown toward the anchor when a wire
//! fires, a slash) and springs that carry the body through a jolt: the torso
//! and legs lag behind an acceleration and swing through after it. A cloak of
//! particles and two wire ropes hang off the result. The giants use the same
//! skeleton with their own proportions.
//!
//! Renderers skin with `skin[i] = world[i] × bind[i]⁻¹`; models are built in
//! the bind pose (`Skeleton::bind`, arms out, legs slightly apart).
//!
//! Local frame: +Y up, -Z forward, +X right. A limb points down its -Y axis.

use crate::math::*;
use crate::sim::{act, hook, tune, RADIUS};

pub const BONES: usize = 19;

pub mod bone {
    pub const PELVIS: usize = 0;
    pub const SPINE: usize = 1;
    pub const CHEST: usize = 2;
    pub const NECK: usize = 3;
    pub const HEAD: usize = 4;
    pub const CLAV_L: usize = 5;
    pub const ARM_UL: usize = 6;
    pub const ARM_LL: usize = 7;
    pub const HAND_L: usize = 8;
    pub const CLAV_R: usize = 9;
    pub const ARM_UR: usize = 10;
    pub const ARM_LR: usize = 11;
    pub const HAND_R: usize = 12;
    pub const LEG_UL: usize = 13;
    pub const LEG_LL: usize = 14;
    pub const FOOT_L: usize = 15;
    pub const LEG_UR: usize = 16;
    pub const LEG_LR: usize = 17;
    pub const FOOT_R: usize = 18;
}
use bone::*;

pub const PARENT: [usize; BONES] = [0, 0, 1, 2, 3, 2, 5, 6, 7, 2, 9, 10, 11, 0, 13, 14, 0, 16, 17];

pub type Rot = [Quat; BONES];

/// Joint positions in the parent's frame; the pelvis entry is its height above the feet.
#[derive(Clone, Copy)]
pub struct Skeleton {
    pub offset: [V3; BONES],
}

impl Skeleton {
    fn build(pelvis: f32, spine: f32, chest: f32, neck: f32, head: f32, clav: (f32, f32), shoulder: f32, upper: f32, fore: f32, hip: (f32, f32), thigh: f32, shin: f32) -> Skeleton {
        let mut o = [V3::ZERO; BONES];
        o[PELVIS] = v3(0.0, pelvis, 0.0);
        o[SPINE] = v3(0.0, spine, 0.0);
        o[CHEST] = v3(0.0, chest, 0.0);
        o[NECK] = v3(0.0, neck, 0.0);
        o[HEAD] = v3(0.0, head, 0.0);
        for (s, c, u, l, h) in [(-1.0, CLAV_L, ARM_UL, ARM_LL, HAND_L), (1.0, CLAV_R, ARM_UR, ARM_LR, HAND_R)] {
            o[c] = v3(s * clav.0, clav.1, 0.0);
            o[u] = v3(s * shoulder, 0.0, 0.0);
            o[l] = v3(0.0, -upper, 0.0);
            o[h] = v3(0.0, -fore, 0.0);
        }
        for (s, u, l, f) in [(-1.0, LEG_UL, LEG_LL, FOOT_L), (1.0, LEG_UR, LEG_LR, FOOT_R)] {
            o[u] = v3(s * hip.0, hip.1, 0.0);
            o[l] = v3(0.0, -thigh, 0.0);
            o[f] = v3(0.0, -shin, 0.0);
        }
        Skeleton { offset: o }
    }

    /// The player: 1.74 m.
    pub fn human() -> Skeleton {
        Skeleton::build(0.94, 0.10, 0.16, 0.26, 0.07, (0.04, 0.20), 0.15, 0.27, 0.25, (0.095, -0.06), 0.42, 0.40)
    }

    /// A giant of unit height, standing straight. All three have arms that reach the knee.
    /// Variants: 0 gaunt, 1 broad and heavy-shouldered, 2 long-legged with a long neck.
    pub fn titan(variant: u32) -> Skeleton {
        match variant % 3 {
            0 => Skeleton::build(0.50, 0.075, 0.115, 0.16, 0.04, (0.03, 0.125), 0.12, 0.215, 0.215, (0.06, -0.035), 0.24, 0.205),
            1 => Skeleton::build(0.47, 0.08, 0.12, 0.17, 0.035, (0.04, 0.135), 0.15, 0.21, 0.20, (0.075, -0.03), 0.225, 0.195),
            _ => Skeleton::build(0.54, 0.065, 0.10, 0.135, 0.06, (0.028, 0.105), 0.105, 0.20, 0.21, (0.055, -0.035), 0.265, 0.225),
        }
    }

    /// World transforms from local rotations. `pelvis` places the pelvis joint; `scale` sizes the offsets.
    pub fn fk(&self, pelvis: &M34, rot: &Rot, scale: f32) -> [M34; BONES] {
        let mut w = [M34::ID; BONES];
        w[0] = M34::new(pelvis.r.mul(&rot[0].m3()), pelvis.t);
        for i in 1..BONES {
            let p = w[PARENT[i]];
            w[i] = M34::new(p.r.mul(&rot[i].m3()), p.apply(self.offset[i] * scale));
        }
        w
    }

    /// The pose models are built in: standing at the origin, facing -Z, arms 37° out, legs slightly apart.
    pub fn bind(&self) -> [M34; BONES] {
        let mut r = [Quat::ID; BONES];
        r[ARM_UL] = Quat::euler(0.0, 0.0, -0.65);
        r[ARM_UR] = Quat::euler(0.0, 0.0, 0.65);
        r[ARM_LL] = Quat::euler(0.12, 0.0, 0.0);
        r[ARM_LR] = Quat::euler(0.12, 0.0, 0.0);
        r[LEG_UL] = Quat::euler(0.0, 0.0, -0.1);
        r[LEG_UR] = Quat::euler(0.0, 0.0, 0.1);
        self.fk(&M34::new(M3::ID, self.offset[PELVIS]), &r, 1.0)
    }

    pub fn bind_inverse(&self) -> [M34; BONES] {
        self.bind().map(|m| m.inverse_rigid())
    }
}

/// Sets the same rotation on a left and a right bone, mirrored: positive `z` spreads outward, positive `y` twists outward.
fn sym(r: &mut Rot, left: usize, right: usize, x: f32, y: f32, z: f32) {
    r[left] = Quat::euler(x, y, -z);
    r[right] = Quat::euler(x, -y, z);
}

fn blend(poses: &[(&Rot, f32)]) -> Rot {
    let mut out = [Quat::ID; BONES];
    for b in 0..BONES {
        let mut acc = Quat { x: 0.0, y: 0.0, z: 0.0, w: 0.0 };
        let mut first: Option<Quat> = None;
        for (p, w) in poses {
            if *w <= 0.0 {
                continue;
            }
            let q = p[b];
            let f = *first.get_or_insert(q);
            let s = if q.x * f.x + q.y * f.y + q.z * f.z + q.w * f.w < 0.0 { -*w } else { *w };
            acc = Quat { x: acc.x + q.x * s, y: acc.y + q.y * s, z: acc.z + q.z * s, w: acc.w + q.w * s };
        }
        out[b] = acc.normalized();
    }
    out
}

// ---------------------------------------------------------------------------- key poses

fn stand(t: f32) -> Rot {
    let mut r = [Quat::ID; BONES];
    let breath = sin(t * 1.6) * 0.015;
    r[SPINE] = Quat::euler(0.02 + breath, 0.0, 0.0);
    r[CHEST] = Quat::euler(-0.03 + breath, 0.0, 0.0);
    sym(&mut r, ARM_UL, ARM_UR, 0.06, 0.0, 0.14);
    sym(&mut r, ARM_LL, ARM_LR, 0.3, 0.0, 0.0);
    sym(&mut r, LEG_UL, LEG_UR, 0.02, 0.0, 0.05);
    sym(&mut r, LEG_LL, LEG_LR, -0.05, 0.0, 0.0);
    r
}

fn run(phase: f32, amp: f32) -> Rot {
    let mut r = [Quat::ID; BONES];
    let (s, c) = (sin(phase), cos(phase));
    r[SPINE] = Quat::euler(-0.1 * amp, -0.1 * amp * s, 0.0);
    r[CHEST] = Quat::euler(-0.08 * amp, -0.16 * amp * s, 0.0);
    r[HEAD] = Quat::euler(0.12 * amp, 0.1 * amp * s, 0.0);
    r[LEG_UL] = Quat::euler(0.9 * amp * s, 0.0, -0.03);
    r[LEG_UR] = Quat::euler(-0.9 * amp * s, 0.0, 0.03);
    r[LEG_LL] = Quat::euler(-(0.25 + 1.2 * max(c, 0.0)) * amp, 0.0, 0.0);
    r[LEG_LR] = Quat::euler(-(0.25 + 1.2 * max(-c, 0.0)) * amp, 0.0, 0.0);
    r[FOOT_L] = Quat::euler(0.3 * amp * s, 0.0, 0.0);
    r[FOOT_R] = Quat::euler(-0.3 * amp * s, 0.0, 0.0);
    r[ARM_UL] = Quat::euler(-0.75 * amp * s, 0.0, -0.16);
    r[ARM_UR] = Quat::euler(0.75 * amp * s, 0.0, 0.16);
    sym(&mut r, ARM_LL, ARM_LR, 0.3 + 1.0 * amp, 0.0, 0.0);
    r
}

fn slide() -> Rot {
    let mut r = [Quat::ID; BONES];
    r[SPINE] = Quat::euler(0.25, 0.0, 0.0);
    r[HEAD] = Quat::euler(-0.2, 0.0, 0.0);
    r[LEG_UL] = Quat::euler(1.35, 0.0, -0.1);
    r[LEG_UR] = Quat::euler(0.45, 0.0, 0.15);
    r[LEG_LL] = Quat::euler(-1.6, 0.0, 0.0);
    r[LEG_LR] = Quat::euler(-1.0, 0.0, 0.0);
    sym(&mut r, ARM_UL, ARM_UR, -0.5, 0.0, 0.6);
    sym(&mut r, ARM_LL, ARM_LR, 0.5, 0.0, 0.0);
    r
}

/// Airborne at low speed: knees up, arms out for balance.
fn air() -> Rot {
    let mut r = [Quat::ID; BONES];
    r[SPINE] = Quat::euler(-0.08, 0.0, 0.0);
    r[LEG_UL] = Quat::euler(0.55, 0.0, -0.08);
    r[LEG_UR] = Quat::euler(0.15, 0.0, 0.08);
    r[LEG_LL] = Quat::euler(-0.95, 0.0, 0.0);
    r[LEG_LR] = Quat::euler(-0.5, 0.0, 0.0);
    sym(&mut r, FOOT_L, FOOT_R, -0.4, 0.0, 0.0);
    sym(&mut r, ARM_UL, ARM_UR, -0.15, 0.0, 0.75);
    sym(&mut r, ARM_LL, ARM_LR, 0.5, 0.0, 0.0);
    r
}

/// Fast free flight: the root is pitched forward, the back arches and the head looks ahead.
fn fly() -> Rot {
    let mut r = [Quat::ID; BONES];
    r[SPINE] = Quat::euler(0.14, 0.0, 0.0);
    r[CHEST] = Quat::euler(0.12, 0.0, 0.0);
    r[NECK] = Quat::euler(0.28, 0.0, 0.0);
    r[HEAD] = Quat::euler(0.38, 0.0, 0.0);
    sym(&mut r, ARM_UL, ARM_UR, -0.3, 0.0, 0.3);
    sym(&mut r, ARM_LL, ARM_LR, 0.2, 0.0, 0.0);
    r[LEG_UL] = Quat::euler(-0.12, 0.0, -0.04);
    r[LEG_UR] = Quat::euler(-0.22, 0.0, 0.04);
    r[LEG_LL] = Quat::euler(-0.2, 0.0, 0.0);
    r[LEG_LR] = Quat::euler(-0.38, 0.0, 0.0);
    sym(&mut r, FOOT_L, FOOT_R, -0.6, 0.0, 0.0);
    r
}

/// Hanging from a wire: seated in the harness, legs ahead, hands on the grips at the hips.
fn hang() -> Rot {
    let mut r = [Quat::ID; BONES];
    r[SPINE] = Quat::euler(0.1, 0.0, 0.0);
    r[CHEST] = Quat::euler(0.05, 0.0, 0.0);
    r[HEAD] = Quat::euler(-0.1, 0.0, 0.0);
    r[LEG_UL] = Quat::euler(0.95, 0.0, -0.1);
    r[LEG_UR] = Quat::euler(0.7, 0.0, 0.1);
    r[LEG_LL] = Quat::euler(-0.95, 0.0, 0.0);
    r[LEG_LR] = Quat::euler(-0.6, 0.0, 0.0);
    sym(&mut r, FOOT_L, FOOT_R, -0.3, 0.0, 0.0);
    sym(&mut r, ARM_UL, ARM_UR, 0.25, 0.0, 0.22);
    sym(&mut r, ARM_LL, ARM_LR, 1.05, 0.0, 0.0);
    r
}

/// Reeling in: diving at the anchor, knees bent behind, arms low and forward.
fn reel() -> Rot {
    let mut r = [Quat::ID; BONES];
    r[SPINE] = Quat::euler(0.1, 0.0, 0.0);
    r[NECK] = Quat::euler(0.2, 0.0, 0.0);
    r[HEAD] = Quat::euler(0.3, 0.0, 0.0);
    r[LEG_UL] = Quat::euler(0.35, 0.0, -0.06);
    r[LEG_UR] = Quat::euler(-0.2, 0.0, 0.06);
    r[LEG_LL] = Quat::euler(-1.25, 0.0, 0.0);
    r[LEG_LR] = Quat::euler(-0.7, 0.0, 0.0);
    sym(&mut r, FOOT_L, FOOT_R, -0.6, 0.0, 0.0);
    sym(&mut r, ARM_UL, ARM_UR, 0.5, 0.0, 0.2);
    sym(&mut r, ARM_LL, ARM_LR, 0.7, 0.0, 0.0);
    r
}

/// Thrusting on gas: straight as a dart, arms swept back.
fn thrust() -> Rot {
    let mut r = fly();
    sym(&mut r, ARM_UL, ARM_UR, -0.75, 0.0, 0.25);
    sym(&mut r, ARM_LL, ARM_LR, 0.1, 0.0, 0.0);
    sym(&mut r, LEG_UL, LEG_UR, -0.2, 0.0, 0.03);
    sym(&mut r, LEG_LL, LEG_LR, -0.08, 0.0, 0.0);
    r
}

/// Ground covered by one full stride (two steps) at `speed`. The simulation advances the
/// gait's phase by distance over this, so a planted foot does not slide.
pub fn stride(speed: f32) -> f32 {
    clamp(0.9 + 0.32 * speed, 0.9, 4.2)
}

struct Leg {
    thigh: Quat,
    shin: Quat,
    foot: Quat,
}

/// Walking and running by foot placement. Each foot alternates a stance on the
/// ground, moving back under the body at the body's speed, and a swing forward
/// through the air; the knee comes from two-bone inverse kinematics.
///
/// `pelvis_r` is the pelvis rotation in the character's frame (yaw removed);
/// `moving` fades the stride out to a stand. Returns the pelvis height above
/// the ground and both legs' rotations (thigh relative to the pelvis).
fn gait(skel: &Skeleton, pelvis_r: &M3, phase: f32, speed: f32, moving: f32, crouch: f32) -> (f32, [Leg; 2]) {
    let run = smoothstep(2.5, 7.0, speed) * moving;
    let duty = lerp(0.62, 0.36, run);
    let (l1, l2) = (-skel.offset[LEG_LL].y, -skel.offset[FOOT_L].y);
    let hip = skel.offset[LEG_UR];
    let ankle = skel.offset[PELVIS].y + hip.y - l1 - l2;
    // How far a planted foot travels under the body, within the leg's reach.
    let half = min(duty * stride(speed) * 0.5, 0.46) * moving;
    // Walking, the hips are lowest as the heel strikes; running, in the middle of the stance.
    let bob = lerp(0.02, 0.0, run) * cos(phase * 2.0) + run * 0.05 * cos((phase - PI * duty) * 2.0);
    let pelvis_y = ankle + (l1 + l2) * lerp(0.994, 0.925, run) - hip.y - bob * moving - crouch;
    let leg = |side: f32, offset: f32| {
        let u = (phase / TAU + offset) - floor(phase / TAU + offset);
        let (z, lift, pitch) = if u < duty {
            let s = u / duty;
            // Heel first, flat through the middle, then the heel peels off.
            let toe = if s < 0.15 { 0.3 * (1.0 - s / 0.15) } else if s > 0.68 { -0.8 * (s - 0.68) / 0.32 } else { 0.0 };
            (-half + 2.0 * half * s, max(s - 0.72, 0.0) / 0.28 * 0.06, toe)
        } else {
            let s = (u - duty) / (1.0 - duty);
            let e = s * s * (3.0 - 2.0 * s);
            // The foot swings through; at a run the heel kicks up behind first.
            (half - 2.0 * half * e, 0.06 * (1.0 - s) + lerp(0.07, 0.16, run) * sin(PI * s) + run * 0.2 * sin(PI * min(s * 1.6, 1.0)), lerp(-0.8, 0.3, smoothstep(0.25, 0.95, s)))
        };
        let target = v3(side * lerp(hip.x, 0.055, run), ankle + lift * moving, z);
        let from = v3(0.0, pelvis_y, 0.0) + pelvis_r.apply(v3(side * hip.x, hip.y, 0.0));
        let v = target - from;
        let d = clamp(v.len(), 0.25 * (l1 + l2), (l1 + l2) * 0.999);
        let dir = v.norm_or(v3(0.0, -1.0, 0.0));
        // The knee points forward: the thigh swings ahead of the hip-to-ankle line.
        let fwd = v3(0.0, 0.0, -1.0);
        let f = (fwd - dir * fwd.dot(dir)).norm_or(v3(0.0, 1.0, 0.0));
        let a = acos((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d));
        let knee = PI - acos((l1 * l1 + l2 * l2 - d * d) / (2.0 * l1 * l2));
        let y = -(dir * cos(a) + f * sin(a));
        let x = dir.cross(f).norm_or(v3(1.0, 0.0, 0.0));
        let thigh = M3 { x, y, z: x.cross(y) };
        let shin = thigh.mul(&M3::rot_x(-knee));
        Leg { thigh: pelvis_r.transpose().mul(&thigh).quat(), shin: Quat::axis_angle(v3(1.0, 0.0, 0.0), -knee), foot: shin.transpose().mul(&M3::rot_x(pitch * moving)).quat() }
    };
    (pelvis_y, [leg(-1.0, 0.0), leg(1.0, 0.5)])
}

const POSES: usize = 8;
const W_STAND: usize = 0;
const W_RUN: usize = 1;
const W_SLIDE: usize = 2;
const W_AIR: usize = 3;
const W_FLY: usize = 4;
const W_HANG: usize = 5;
const W_REEL: usize = 6;
const W_THRUST: usize = 7;

// ---------------------------------------------------------------------------- cloak

pub const CLOAK_W: usize = 7;
pub const CLOAK_H: usize = 8;
pub const CLOAK_N: usize = CLOAK_W * CLOAK_H;

/// A short cloak: a grid of particles pinned across the shoulders.
pub struct Cloak {
    /// Points as drawn: the cloth, with the wind's ripple laid over it.
    pub p: [V3; CLOAK_N],
    /// Points as simulated.
    sim: [V3; CLOAK_N],
    prev: [V3; CLOAK_N],
    pub n: [V3; CLOAK_N],
    live: bool,
}

impl Cloak {
    const DROP: f32 = 0.088;

    fn pin(col: usize) -> V3 {
        // Across the shoulders, wrapping forward a little at the ends.
        let u = col as f32 / (CLOAK_W - 1) as f32 * 2.0 - 1.0;
        v3(u * 0.215, 0.235 - 0.03 * u * u, 0.115 - 0.085 * u * u)
    }

    fn rest_width(row: usize) -> f32 {
        // Wider toward the hem.
        (0.43 + 0.2 * row as f32 / (CLOAK_H - 1) as f32) / (CLOAK_W - 1) as f32
    }

    fn update(&mut self, chest: &M34, pelvis: &M34, dt: f32, tick: u32, speed: f32) {
        let at = |r: usize, c: usize| r * CLOAK_W + c;
        if !self.live {
            for r in 0..CLOAK_H {
                for c in 0..CLOAK_W {
                    let p = chest.apply(Self::pin(c) + v3(0.0, -Self::DROP * r as f32, 0.02 * r as f32));
                    self.sim[at(r, c)] = p;
                    self.prev[at(r, c)] = p;
                }
            }
            self.live = true;
        }
        // Verlet: carry the last motion, fall, and lose speed to the air (so the cloth trails a moving body).
        let keep = exp(-2.6 * dt);
        for i in CLOAK_W..CLOAK_N {
            let v = (self.sim[i] - self.prev[i]) * keep;
            self.prev[i] = self.sim[i];
            self.sim[i] = self.sim[i] + v + v3(0.0, -9.0, 0.0) * (dt * dt);
        }
        let back = chest.r.z;
        let (lo, hi) = (pelvis.t - pelvis.r.y * 0.2, chest.apply(v3(0.0, 0.22, 0.0)));
        for _ in 0..4 {
            for c in 0..CLOAK_W {
                self.sim[at(0, c)] = chest.apply(Self::pin(c));
            }
            for r in 0..CLOAK_H {
                for c in 0..CLOAK_W {
                    for (r2, c2, rest) in [(r, c + 1, Self::rest_width(r)), (r + 1, c, Self::DROP)] {
                        if r2 >= CLOAK_H || c2 >= CLOAK_W {
                            continue;
                        }
                        let (i, j) = (at(r, c), at(r2, c2));
                        let d = self.sim[j] - self.sim[i];
                        let l = d.len();
                        if l < 1e-5 {
                            continue;
                        }
                        let fix = d * ((l - rest) / l * 0.5);
                        if r > 0 {
                            self.sim[i] += fix;
                        }
                        if r2 > 0 {
                            self.sim[j] -= fix;
                        }
                    }
                }
            }
            // Keep the cloth outside the torso: a capsule from the hips to the neck.
            for i in CLOAK_W..CLOAK_N {
                let ab = hi - lo;
                let t = saturate((self.sim[i] - lo).dot(ab) / ab.len2().max(1e-6));
                let c = lo + ab * t;
                let d = self.sim[i] - c;
                let l = d.len();
                let radius = 0.17 + 0.03 * (1.0 - t);
                if l < radius {
                    self.sim[i] = c + (if l > 1e-4 { d * (1.0 / l) } else { back }) * radius;
                }
            }
        }
        let normals = |p: &[V3; CLOAK_N], n: &mut [V3; CLOAK_N]| {
            for r in 0..CLOAK_H {
                for c in 0..CLOAK_W {
                    let dx = p[at(r, (c + 1).min(CLOAK_W - 1))] - p[at(r, c.saturating_sub(1))];
                    let dy = p[at((r + 1).min(CLOAK_H - 1), c)] - p[at(r.saturating_sub(1), c)];
                    let mut v = dy.cross(dx).norm_or(back);
                    if v.dot(back) < 0.0 {
                        v = -v;
                    }
                    n[at(r, c)] = v;
                }
            }
        };
        normals(&self.sim, &mut self.n);
        // In the wind the cloth ripples: a wave runs from the shoulders to the hem, growing toward the hem.
        let flutter = saturate(speed / 22.0);
        for r in 0..CLOAK_H {
            for c in 0..CLOAK_W {
                let i = at(r, c);
                let grow = r as f32 / (CLOAK_H - 1) as f32;
                let wave = sin(tick as f32 * 0.6 - r as f32 * 1.3 + c as f32 * 0.7) * 0.05 + sin(tick as f32 * 0.37 + c as f32 * 1.9) * 0.02;
                self.p[i] = self.sim[i] + self.n[i] * (wave * flutter * grow);
            }
        }
        let drawn = self.p;
        normals(&drawn, &mut self.n);
    }
}

// ---------------------------------------------------------------------------- wires

pub const ROPE_N: usize = 14;

/// The visible wire: a chain of points between the hip gear and the hook. It
/// cannot stretch past its length, sags when the player is nearer the anchor
/// than the wire is long, whips while the hook flies and rings when it bites.
pub struct Rope {
    pub p: [V3; ROPE_N],
    prev: [V3; ROPE_N],
    live: bool,
}

impl Rope {
    fn update(&mut self, out: bool, a: V3, b: V3, length: f32, ring: f32, tick: u32, dt: f32) {
        if !out {
            self.live = false;
            for p in &mut self.p {
                *p = a;
            }
            return;
        }
        if !self.live {
            for i in 0..ROPE_N {
                self.p[i] = a.lerp(b, i as f32 / (ROPE_N - 1) as f32);
                self.prev[i] = self.p[i];
            }
            self.live = true;
        }
        let axis = (b - a).norm_or(V3::UP);
        let side = axis.cross(V3::UP).norm_or(v3(1.0, 0.0, 0.0));
        let lift = side.cross(axis);
        for i in 1..ROPE_N - 1 {
            let s = i as f32 / (ROPE_N - 1) as f32;
            let mut v = (self.p[i] - self.prev[i]) * 0.94;
            if ring > 0.0 {
                // A bite sends a wave down the wire; a flying hook drags a whip behind it.
                v += (side * sin(s * PI * 3.0 + tick as f32 * 0.9) + lift * cos(s * PI * 2.0 + tick as f32 * 0.7)) * (ring * sin(s * PI) * dt * dt);
            }
            self.prev[i] = self.p[i];
            self.p[i] = self.p[i] + v + v3(0.0, -7.0, 0.0) * (dt * dt);
        }
        let seg = length / (ROPE_N - 1) as f32;
        for _ in 0..10 {
            self.p[0] = a;
            self.p[ROPE_N - 1] = b;
            for i in 0..ROPE_N - 1 {
                let d = self.p[i + 1] - self.p[i];
                let l = d.len();
                if l <= seg || l < 1e-5 {
                    continue;
                }
                let fix = d * ((l - seg) / l * 0.5);
                if i > 0 {
                    self.p[i] += fix;
                }
                if i + 1 < ROPE_N - 1 {
                    self.p[i + 1] -= fix;
                }
            }
        }
        self.p[0] = a;
        self.p[ROPE_N - 1] = b;
    }
}

// ---------------------------------------------------------------------------- the player's animator

pub struct PoseIn {
    pub pos: V3,
    pub vel: V3,
    pub facing: f32,
    pub grounded: bool,
    pub act: u32,
    pub run_phase: f32,
    pub slash_t: f32,
    pub land_t: f32,
    pub tick: u32,
    /// Per hook: state, tip, anchor and physical wire length.
    pub hooks: [(u8, V3, V3, f32); 2],
    /// Hooks that fired, and hooks that bit, this tick.
    pub fired: [bool; 2],
    pub bit: [bool; 2],
    pub burst: bool,
    pub released: bool,
}

pub struct Pose {
    skeleton: Skeleton,
    bind_inv: [M34; BONES],
    weights: [f32; POSES],
    root: Quat,
    tilt: V3,
    upright: f32,
    crouch: f32,
    /// How much the legs follow the walking solver (1 on the ground, 0 in the air).
    gait: f32,
    fire_t: [f32; 2],
    aim: [V3; 2],
    pitch: Spring,
    roll: Spring,
    drag: Spring,
    accel: V3,
    prev_vel: V3,
    ring: [f32; 2],
    pub world: [M34; BONES],
    pub skin: [M34; BONES],
    /// Where each wire leaves the hip gear.
    pub hip: [V3; 2],
    /// Blade tips in world space (left, right).
    pub blade: [V3; 2],
    pub cloak: Cloak,
    pub ropes: [Rope; 2],
}

/// Hip gear muzzle in the pelvis frame, and the blade's tip in the hand frame.
const MUZZLE: V3 = v3(0.2, -0.05, -0.12);
pub const BLADE_TIP: V3 = v3(0.0, -0.52, -0.78);

impl Pose {
    pub fn new() -> Pose {
        let skeleton = Skeleton::human();
        let mut w = [0.0; POSES];
        w[W_STAND] = 1.0;
        Pose {
            bind_inv: skeleton.bind_inverse(),
            skeleton,
            weights: w,
            root: Quat::ID,
            tilt: V3::UP,
            upright: 1.0,
            crouch: 0.0,
            gait: 1.0,
            fire_t: [9.0; 2],
            aim: [v3(0.0, 0.0, -1.0); 2],
            pitch: Spring::default(),
            roll: Spring::default(),
            drag: Spring::default(),
            accel: V3::ZERO,
            prev_vel: V3::ZERO,
            ring: [0.0; 2],
            world: [M34::ID; BONES],
            skin: [M34::ID; BONES],
            hip: [V3::ZERO; 2],
            blade: [V3::ZERO; 2],
            cloak: Cloak { p: [V3::ZERO; CLOAK_N], sim: [V3::ZERO; CLOAK_N], prev: [V3::ZERO; CLOAK_N], n: [V3::UP; CLOAK_N], live: false },
            ropes: [Rope { p: [V3::ZERO; ROPE_N], prev: [V3::ZERO; ROPE_N], live: false }, Rope { p: [V3::ZERO; ROPE_N], prev: [V3::ZERO; ROPE_N], live: false }],
        }
    }

    /// After a teleport: nothing carries over.
    pub fn reset(&mut self) {
        self.cloak.live = false;
        self.ropes[0].live = false;
        self.ropes[1].live = false;
        self.prev_vel = V3::ZERO;
        self.accel = V3::ZERO;
        self.pitch = Spring::default();
        self.roll = Spring::default();
        self.drag = Spring::default();
    }

    pub fn update(&mut self, i: &PoseIn) {
        let dt = crate::sim::DT;
        let speed = i.vel.len();
        let hs = i.vel.flat().len();
        let t = i.tick as f32 * dt;
        let attached = [i.hooks[0].0 == hook::ATTACHED, i.hooks[1].0 == hook::ATTACHED];
        let n_att = attached.iter().filter(|a| **a).count();

        // Which key poses, by action and speed.
        let fast = smoothstep(10.0, 30.0, speed);
        let mut want = [0.0f32; POSES];
        match i.act {
            act::IDLE => want[W_STAND] = 1.0,
            act::RUN => {
                let amp = saturate(hs / tune::RUN_SPEED);
                want[W_RUN] = amp;
                want[W_STAND] = 1.0 - amp;
            }
            act::SLIDE => want[W_SLIDE] = 1.0,
            act::HOOK => {
                want[W_HANG] = 1.0 - fast * 0.55;
                want[W_FLY] = fast * 0.55;
            }
            act::REEL => {
                want[W_REEL] = 0.75;
                want[W_FLY] = 0.25;
            }
            act::THRUST => want[W_THRUST] = 1.0,
            _ => {
                want[W_FLY] = fast;
                want[W_AIR] = 1.0 - fast;
            }
        }
        let k = 1.0 - exp(-9.0 * dt);
        let mut sum = 0.0;
        for p in 0..POSES {
            self.weights[p] += (want[p] - self.weights[p]) * k;
            sum += self.weights[p];
        }
        let w = self.weights.map(|x| x / sum.max(1e-4));
        let amp = saturate(hs / tune::RUN_SPEED);
        let keys = [stand(t), run(i.run_phase, amp), slide(), air(), fly(), hang(), reel(), thrust()];
        let mut rot = blend(&[(&keys[0], w[0]), (&keys[1], w[1]), (&keys[2], w[2]), (&keys[3], w[3]), (&keys[4], w[4]), (&keys[5], w[5]), (&keys[6], w[6]), (&keys[7], w[7])]);

        // Root: facing, then a forward lean by pose, then a tilt of the whole body toward the wires it hangs from.
        let climb = if speed > 1.0 { i.vel.y / speed } else { 0.0 };
        let lean = w[W_RUN] * 0.2 + w[W_SLIDE] * -0.3 + w[W_AIR] * 0.15 + w[W_FLY] * (1.2 - climb * 0.6) + w[W_HANG] * -0.2 + w[W_REEL] * (1.05 - climb * 0.5) + w[W_THRUST] * (1.25 - climb * 0.7);
        let mut pull = V3::ZERO;
        for h in 0..2 {
            if attached[h] {
                pull += (i.hooks[h].2 - i.pos).norm();
            }
        }
        let hang_w = w[W_HANG] + w[W_REEL] * 0.5;
        let tilt_want = if n_att > 0 { (V3::UP + pull.norm() * (0.9 * hang_w)).norm_or(V3::UP) } else { V3::UP };
        self.tilt = self.tilt.ease(tilt_want, 7.0, dt).norm_or(V3::UP);
        let side_pull = if n_att == 1 { if attached[0] { -0.3 } else { 0.3 } } else { 0.0 };

        // Springs: a jolt along the body pitches it back and swings the legs through; a jolt across rolls it.
        let accel = (i.vel - self.prev_vel) * (1.0 / dt);
        self.prev_vel = i.vel;
        self.accel = self.accel.ease(accel, 14.0, dt);
        let facing = M3::rot_y(i.facing);
        let local = facing.transpose().apply(self.accel);
        let airborne = if i.grounded { 0.25 } else { 1.0 };
        self.pitch.step(clamp(local.z * 0.011, -0.5, 0.5) * airborne, 13.0, 0.32, dt);
        self.roll.step(clamp(local.x * 0.012, -0.5, 0.5) * airborne + side_pull * hang_w, 12.0, 0.4, dt);
        self.drag.step(clamp(local.y * 0.008, -0.4, 0.4) * airborne, 15.0, 0.35, dt);
        for h in 0..2 {
            if i.fired[h] {
                self.fire_t[h] = 0.0;
                self.pitch.v -= 3.4;
                self.roll.v += if h == 0 { 1.6 } else { -1.6 };
            }
            if i.bit[h] {
                // The wire yanks the hips: the torso is left behind, then swings through.
                self.pitch.v -= 5.5;
                self.drag.v += 3.0;
                self.ring[h] = 420.0;
            }
            self.fire_t[h] += dt;
            self.ring[h] *= 0.8;
            if i.hooks[h].0 == hook::FLYING || i.hooks[h].0 == hook::MISS {
                self.aim[h] = (i.hooks[h].1 - i.pos).norm_or(self.aim[h]);
            } else if attached[h] {
                self.aim[h] = (i.hooks[h].2 - i.pos).norm_or(self.aim[h]);
            }
        }
        if i.burst {
            self.pitch.v -= 4.0;
        }
        if i.released {
            self.pitch.v += 2.5;
        }

        let spin = if i.slash_t >= 0.0 && !i.grounded { smoothstep(0.0, 1.0, i.slash_t / tune::SLASH_TIME) * TAU } else { 0.0 };
        let root_want = Quat::from_to(V3::UP, self.tilt)
            .mul(Quat::axis_angle(V3::UP, i.facing))
            .mul(Quat::axis_angle(v3(1.0, 0.0, 0.0), -(lean + self.pitch.x)))
            .mul(Quat::axis_angle(v3(0.0, 0.0, 1.0), self.roll.x));
        self.root = self.root.nlerp(root_want, 1.0 - exp(-14.0 * dt));
        let root = self.root.mul(Quat::axis_angle(v3(0.0, 0.0, 1.0), spin)).m3();

        // The same springs bend the body: legs trail a pull, the spine gives, the head stays level longer.
        let p = self.pitch.x;
        rot[SPINE] = rot[SPINE].mul(Quat::euler(p * 0.35, 0.0, -self.roll.x * 0.3));
        rot[CHEST] = rot[CHEST].mul(Quat::euler(p * 0.25, 0.0, -self.roll.x * 0.2));
        rot[HEAD] = rot[HEAD].mul(Quat::euler(-p * 0.5, 0.0, self.roll.x * 0.3));
        for (u, l) in [(LEG_UL, LEG_LL), (LEG_UR, LEG_LR)] {
            rot[u] = rot[u].mul(Quat::euler(p * 1.1 * airborne - self.drag.x * 0.6, 0.0, 0.0));
            rot[l] = rot[l].mul(Quat::euler(-abs(p) * 0.7 * airborne, 0.0, 0.0));
        }
        for (u, s) in [(ARM_UL, -1.0), (ARM_UR, 1.0)] {
            rot[u] = rot[u].mul(Quat::euler(-p * 0.6, 0.0, s * abs(self.drag.x) * 0.8));
        }

        // Landing: a deep knee bend that recovers.
        let crouch_want = if i.grounded && i.land_t < 0.3 { (1.0 - i.land_t / 0.3) * 0.32 } else if i.act == act::SLIDE { 0.4 } else { 0.0 };
        self.crouch = ease(self.crouch, crouch_want, 16.0, dt);
        let gaited = i.grounded && (i.act == act::IDLE || i.act == act::RUN);
        self.gait = ease(self.gait, if gaited { 1.0 } else { 0.0 }, 16.0, dt);
        if self.crouch > 0.01 && i.act != act::SLIDE && !gaited {
            let c = self.crouch / 0.32;
            for (u, l, f) in [(LEG_UL, LEG_LL, FOOT_L), (LEG_UR, LEG_LR, FOOT_R)] {
                rot[u] = rot[u].mul(Quat::euler(0.9 * c, 0.0, 0.0));
                rot[l] = rot[l].mul(Quat::euler(-1.7 * c, 0.0, 0.0));
                rot[f] = rot[f].mul(Quat::euler(0.8 * c, 0.0, 0.0));
            }
            rot[SPINE] = rot[SPINE].mul(Quat::euler(-0.35 * c, 0.0, 0.0));
        }

        // Firing a wire: that arm is thrown at the anchor, the chest turns with it, the other arm goes back.
        let chest_world = root.mul(&rot[PELVIS].m3()).mul(&rot[SPINE].m3()).mul(&rot[CHEST].m3());
        let mut twist = 0.0;
        for (h, clav, upper, lower, hand, other) in [(0usize, CLAV_L, ARM_UL, ARM_LL, HAND_L, ARM_UR), (1, CLAV_R, ARM_UR, ARM_LR, HAND_R, ARM_UL)] {
            let ft = self.fire_t[h];
            // Up fast, held while the hook flies, then back to the grip.
            let e = smoothstep(0.0, 0.06, ft) * (1.0 - smoothstep(0.2, 0.42, ft));
            if e <= 0.001 {
                continue;
            }
            let parent = chest_world.mul(&rot[clav].m3());
            let dir = parent.transpose().apply(self.aim[h]);
            let reach = Quat::from_to(v3(0.0, -1.0, 0.0), dir.norm_or(v3(0.0, 0.0, -1.0)));
            rot[upper] = rot[upper].nlerp(reach, e);
            rot[lower] = rot[lower].nlerp(Quat::euler(0.12, 0.0, 0.0), e);
            rot[hand] = rot[hand].nlerp(Quat::euler(-0.3, 0.0, 0.0), e);
            rot[clav] = rot[clav].nlerp(Quat::euler(0.0, 0.0, if h == 0 { 0.25 } else { -0.25 }), e);
            if self.fire_t[1 - h] > 0.42 {
                rot[other] = rot[other].nlerp(Quat::euler(-0.9, 0.0, if h == 0 { 0.35 } else { -0.35 }), e * 0.8);
                twist += if h == 0 { 0.45 } else { -0.45 } * e;
            }
        }
        rot[CHEST] = rot[CHEST].mul(Quat::euler(0.0, twist, 0.0));
        rot[SPINE] = rot[SPINE].mul(Quat::euler(0.0, twist * 0.5, 0.0));
        // The shot kicks back: the knees come up while the hook is in the air.
        let kick = [0, 1].map(|h| smoothstep(0.0, 0.06, self.fire_t[h]) * (1.0 - smoothstep(0.14, 0.4, self.fire_t[h]))).into_iter().fold(0.0f32, max);
        if kick > 0.001 && !i.grounded {
            for (u, l) in [(LEG_UL, LEG_LL), (LEG_UR, LEG_LR)] {
                rot[u] = rot[u].mul(Quat::euler(0.55 * kick, 0.0, 0.0));
                rot[l] = rot[l].mul(Quat::euler(-0.8 * kick, 0.0, 0.0));
            }
        }

        // A slash: both blades sweep across, the chest turning through.
        if i.slash_t >= 0.0 {
            let k = saturate(i.slash_t / tune::SLASH_TIME);
            let sweep = lerp(1.0, -1.1, smoothstep(0.1, 0.7, k));
            rot[CHEST] = rot[CHEST].mul(Quat::euler(0.0, sweep * 0.8, 0.0));
            rot[ARM_UL] = Quat::euler(lerp(-0.5, 1.1, k), 0.0, -1.2);
            rot[ARM_UR] = Quat::euler(lerp(-0.5, 1.1, k), 0.0, 1.2);
            sym(&mut rot, ARM_LL, ARM_LR, 0.15, 0.0, 0.0);
        }

        // Standing puts the pelvis above the feet; flying puts it at the sphere's centre.
        let upright_want = 1.0 - (w[W_FLY] + w[W_REEL] + w[W_THRUST] + w[W_HANG] * 0.6 + w[W_AIR] * 0.3);
        self.upright = ease(self.upright, saturate(upright_want), 8.0, dt);
        let rest = self.skeleton.offset[PELVIS].y;
        let mut height = rest * self.upright - self.crouch;
        if self.gait > 0.01 {
            // On the ground the feet are placed and the legs solved; the pelvis rides at the height the stride gives it.
            let frame = M3::rot_y(i.facing).transpose().mul(&root).mul(&rot[PELVIS].m3());
            let moving = smoothstep(0.12, 0.9, hs);
            let (y, legs) = gait(&self.skeleton, &frame, i.run_phase, hs, moving, self.crouch);
            for (leg, (u, l, f)) in legs.iter().zip([(LEG_UL, LEG_LL, FOOT_L), (LEG_UR, LEG_LR, FOOT_R)]) {
                rot[u] = rot[u].nlerp(leg.thigh, self.gait);
                rot[l] = rot[l].nlerp(leg.shin, self.gait);
                rot[f] = rot[f].nlerp(leg.foot, self.gait);
            }
            height = lerp(height, y, self.gait);
        }
        let pelvis = M34::new(root, i.pos + v3(0.0, height - RADIUS, 0.0));
        self.world = self.skeleton.fk(&pelvis, &rot, 1.0);
        for b in 0..BONES {
            self.skin[b] = self.world[b].mul(&self.bind_inv[b]);
        }
        self.hip = [self.world[PELVIS].apply(v3(-MUZZLE.x, MUZZLE.y, MUZZLE.z)), self.world[PELVIS].apply(MUZZLE)];
        self.blade = [self.world[HAND_L].apply(BLADE_TIP), self.world[HAND_R].apply(BLADE_TIP)];

        self.cloak.update(&self.world[CHEST], &self.world[PELVIS], dt, i.tick, speed);
        for h in 0..2 {
            let (state, tip, _, len) = i.hooks[h];
            let out = state != hook::IDLE;
            let chord = (tip - self.hip[h]).len();
            // Flying, the wire pays out loose and whips; attached, it is as long as the physical
            // wire, with a little slack that a swing shows as a curve and a reel pulls straight.
            let reeling = i.act == act::REEL;
            let length = match state {
                hook::ATTACHED => max(len, chord) * if reeling { 1.0008 } else { 1.004 },
                _ => chord * 1.09,
            };
            let ring = if state == hook::FLYING || state == hook::MISS { max(self.ring[h], 160.0) } else { self.ring[h] };
            self.ropes[h].update(out, self.hip[h], tip, length, ring, i.tick, dt);
        }
    }
}

// ---------------------------------------------------------------------------- giants

/// A giant's standing pose: hunched over, arms hanging forward, knees bent.
fn titan_stance(a: f32) -> Rot {
    let mut r = [Quat::ID; BONES];
    let breath = sin(a * 0.8) * 0.035;
    r[SPINE] = Quat::euler(-0.2 + breath * 0.5, 0.0, sin(a * 0.27) * 0.035);
    r[CHEST] = Quat::euler(-0.24 + breath, 0.0, 0.0);
    r[NECK] = Quat::euler(0.08, 0.0, 0.0);
    r[HEAD] = Quat::euler(0.2, 0.0, 0.0);
    r[CLAV_L] = Quat::euler(0.0, 0.25, 0.0);
    r[CLAV_R] = Quat::euler(0.0, -0.25, 0.0);
    r[ARM_UL] = Quat::euler(0.3 + sin(a * 0.45) * 0.05, 0.0, -0.2);
    r[ARM_UR] = Quat::euler(0.3 + sin(a * 0.45 + 2.0) * 0.05, 0.0, 0.2);
    sym(&mut r, ARM_LL, ARM_LR, 0.35, 0.0, 0.0);
    sym(&mut r, HAND_L, HAND_R, 0.25, 0.0, 0.0);
    sym(&mut r, LEG_UL, LEG_UR, 0.32, 0.0, 0.09);
    sym(&mut r, LEG_LL, LEG_LR, -0.5, 0.0, 0.0);
    sym(&mut r, FOOT_L, FOOT_R, 0.18, 0.0, 0.0);
    r
}

/// How far the bent knees lower the pelvis, as a share of the standing height of the hips.
const TITAN_SINK: f32 = 0.93;

/// Where a giant's nape is, standing at rest: the target of a cut.
pub fn titan_nape(skel: &Skeleton, pos: V3, yaw: f32, height: f32) -> V3 {
    let r = titan_stance(0.0);
    let root = M3::rot_y(yaw);
    let pelvis = M34::new(root, pos + v3(0.0, skel.offset[PELVIS].y * height * TITAN_SINK, 0.0));
    let world = skel.fk(&pelvis, &r, height);
    // The back of the neck, a third of the way up it.
    world[NECK].apply(v3(0.0, skel.offset[HEAD].y * 0.35, 0.036) * height)
}

/// Skin matrices for a giant standing at `pos`, facing `yaw`, `height` tall.
/// Alive, it breathes, follows `player` with its head and chest and reaches
/// when the player is close. `fallen` is the time since its nape was cut: it
/// pitches forward onto the ground.
pub fn titan_skin(skel: &Skeleton, bind_inv: &[M34; BONES], pos: V3, yaw: f32, height: f32, player: V3, t: f32, phase: f32, fallen: Option<f32>) -> [M34; BONES] {
    let mut r = titan_stance(t + phase * 7.0);
    let mut root = M3::rot_y(yaw);
    let mut lift = 0.0;
    let mut sink = TITAN_SINK;
    match fallen {
        None => {
            // Turn the chest, neck and head toward the player, within what a neck allows.
            let to = M3::rot_y(yaw).transpose().apply(player - (pos + v3(0.0, height * 0.82, 0.0)));
            let turn = clamp(atan2(-to.x, -to.z), -1.3, 1.3);
            let look = clamp(atan2(to.y, sqrt(to.x * to.x + to.z * to.z)), -0.6, 0.8);
            r[SPINE] = r[SPINE].mul(Quat::euler(0.0, turn * 0.15, 0.0));
            r[CHEST] = r[CHEST].mul(Quat::euler(look * 0.2, turn * 0.25, 0.0));
            r[NECK] = r[NECK].mul(Quat::euler(look * 0.3, turn * 0.25, 0.0));
            r[HEAD] = r[HEAD].mul(Quat::euler(look * 0.5, turn * 0.35, 0.0));
            // Within reach, the arm on the player's side comes up after them, fingers open.
            let dist = to.len();
            let near = smoothstep(height * 2.8, height * 1.2, dist) * saturate(1.3 - abs(turn));
            if near > 0.0 {
                let (clav, upper, lower, hand) = if to.x < 0.0 { (CLAV_L, ARM_UL, ARM_LL, HAND_L) } else { (CLAV_R, ARM_UR, ARM_LR, HAND_R) };
                let chest = M3::rot_y(yaw).mul(&r[PELVIS].m3()).mul(&r[SPINE].m3()).mul(&r[CHEST].m3()).mul(&r[clav].m3());
                let dir = chest.transpose().apply((player - (pos + v3(0.0, height * 0.7, 0.0))).norm());
                r[upper] = r[upper].nlerp(Quat::from_to(v3(0.0, -1.0, 0.0), dir), near * 0.92);
                r[lower] = r[lower].nlerp(Quat::euler(0.3, 0.0, 0.0), near);
                r[hand] = r[hand].nlerp(Quat::euler(-0.4, 0.0, 0.0), near);
            }
        }
        Some(since) => {
            // Cut: the knees give and the body goes down on its face.
            let k = saturate(since / 1.5);
            let fall = k * k * 1.42;
            root = root.mul(&M3::rot_x(-fall));
            r[SPINE] = Quat::euler(-0.2 + 0.25 * k, 0.0, 0.0);
            r[CHEST] = Quat::euler(-0.24 + 0.3 * k, 0.0, 0.0);
            r[HEAD] = Quat::euler(0.2 + 0.2 * k, 0.0, 0.0);
            sym(&mut r, ARM_UL, ARM_UR, 0.3 + 0.5 * k, 0.0, 0.2 + 0.4 * k);
            sym(&mut r, LEG_UL, LEG_UR, 0.32 * (1.0 - k), 0.0, 0.09);
            sym(&mut r, LEG_LL, LEG_LR, -0.5 * (1.0 - k) - 0.4 * sin(k * PI), 0.0, 0.0);
            sink = lerp(TITAN_SINK, 1.0, k);
            // Lying, the pelvis rests a body's depth above the ground.
            lift = k * k * 0.07 * height;
        }
    }
    // The body pivots about the feet.
    let rest = skel.offset[PELVIS] * (height * sink);
    let pelvis = M34::new(root, pos + v3(0.0, lift, 0.0) + root.apply(rest));
    let world = skel.fk(&pelvis, &r, height);
    let mut out = [M34::ID; BONES];
    for b in 0..BONES {
        out[b] = M34::new(world[b].r.scaled(height), world[b].t).mul(&bind_inv[b]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_pose_stands_on_the_ground() {
        for skel in [Skeleton::human(), Skeleton::titan(0), Skeleton::titan(1), Skeleton::titan(2)] {
            let b = skel.bind();
            assert!(b[HEAD].t.y > b[CHEST].t.y && b[CHEST].t.y > b[PELVIS].t.y);
            assert!(b[FOOT_L].t.y > 0.0 && b[FOOT_L].t.y < 0.12 * b[HEAD].t.y, "ankle at {}", b[FOOT_L].t.y);
            assert!(b[HAND_L].t.x < b[ARM_UL].t.x && b[HAND_R].t.x > b[ARM_UR].t.x);
        }
    }

    #[test]
    fn skin_is_identity_in_the_bind_pose() {
        let skel = Skeleton::human();
        let bind = skel.bind();
        let inv = skel.bind_inverse();
        for b in 0..BONES {
            let m = bind[b].mul(&inv[b]);
            assert!((m.t.len()) < 1e-5 && (m.r.x.x - 1.0).abs() < 1e-5 && (m.r.y.y - 1.0).abs() < 1e-5);
        }
    }
}
