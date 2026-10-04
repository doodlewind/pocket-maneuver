//! Procedural character animation.
//!
//! The character is thirteen rigid parts on a small skeleton. Each tick picks
//! target joint angles from the player's action, eases the current angles
//! toward them and runs forward kinematics. The output is one world transform
//! per part; the reference renderer and the Vita renderer draw the same parts.
//!
//! Local frame: +Y up, -Z forward, +X right. Part meshes are modelled around
//! their joint (see `web/src/world/character.ts`).

use crate::math::*;
use crate::sim::{act, tune, RADIUS};

pub const PARTS: usize = 13;

pub mod part {
    pub const PELVIS: usize = 0;
    pub const CHEST: usize = 1;
    pub const HEAD: usize = 2;
    pub const ARM_UL: usize = 3;
    pub const ARM_LL: usize = 4;
    pub const ARM_UR: usize = 5;
    pub const ARM_LR: usize = 6;
    pub const LEG_UL: usize = 7;
    pub const LEG_LL: usize = 8;
    pub const LEG_UR: usize = 9;
    pub const LEG_LR: usize = 10;
    pub const CAPE_A: usize = 11;
    pub const CAPE_B: usize = 12;
}

/// Skeleton dimensions in metres, exported to the mesh builder.
pub mod dim {
    /// Pelvis centre above the feet when standing.
    pub const PELVIS_Y: f32 = 0.93;
    pub const HIP_X: f32 = 0.10;
    pub const HIP_Y: f32 = -0.07;
    pub const THIGH: f32 = 0.42;
    pub const SHIN: f32 = 0.44;
    /// Chest joint above the pelvis centre.
    pub const WAIST_Y: f32 = 0.10;
    /// Chest joint to neck.
    pub const CHEST: f32 = 0.42;
    pub const SHOULDER_X: f32 = 0.20;
    pub const SHOULDER_Y: f32 = 0.35;
    pub const UPPER_ARM: f32 = 0.27;
    pub const FOREARM: f32 = 0.25;
    pub const CAPE_Y: f32 = 0.37;
    pub const CAPE_Z: f32 = 0.12;
    pub const CAPE_SEG: f32 = 0.42;
}
use dim::*;

pub struct PoseIn {
    pub pos: V3,
    pub vel: V3,
    pub facing: f32,
    pub grounded: bool,
    pub act: u32,
    pub act_t: f32,
    pub run_phase: f32,
    pub slash_t: f32,
    pub land_t: f32,
    /// Unit direction to each attached anchor, or zero.
    pub wire: [V3; 2],
    pub tick: u32,
}

const NJ: usize = 18;
const THIGH_L: usize = 0;
const THIGH_R: usize = 1;
const KNEE_L: usize = 2;
const KNEE_R: usize = 3;
const SPREAD: usize = 4;
const ARM_LX: usize = 5;
const ARM_LZ: usize = 6;
const ARM_RX: usize = 7;
const ARM_RZ: usize = 8;
const ELBOW_L: usize = 9;
const ELBOW_R: usize = 10;
const CHEST_X: usize = 11;
const CHEST_Y: usize = 12;
const HEAD_X: usize = 13;
const LEAN: usize = 14;
const BANK: usize = 15;
const CROUCH: usize = 16;
/// 1 standing (pelvis above the feet), 0 flying (pelvis at the sphere centre).
const UPRIGHT: usize = 17;

pub struct Pose {
    j: [f32; NJ],
    pub parts: [M34; PARTS],
    /// Blade tips in world space (left, right), for trails.
    pub blade: [V3; 2],
}

impl Pose {
    pub fn new() -> Pose {
        let mut j = [0.0; NJ];
        j[UPRIGHT] = 1.0;
        Pose { j, parts: [M34::ID; PARTS], blade: [V3::ZERO; 2] }
    }

    fn target(i: &PoseIn) -> [f32; NJ] {
        let mut t = [0.0f32; NJ];
        let speed = i.vel.len();
        let hs = i.vel.flat().len();
        let fl = smoothstep(9.0, 30.0, speed);
        let climb = if speed > 1.0 { i.vel.y / speed } else { 0.0 };
        t[UPRIGHT] = 1.0;
        t[SPREAD] = 0.06;
        t[ARM_LZ] = 0.12;
        t[ARM_RZ] = 0.12;
        t[ELBOW_L] = 0.25;
        t[ELBOW_R] = 0.25;
        match i.act {
            act::IDLE => {
                t[CHEST_X] = 0.02 * sin(i.tick as f32 * 0.05);
                t[KNEE_L] = -0.05;
                t[KNEE_R] = -0.05;
            }
            act::RUN => {
                let amp = saturate(hs / tune::RUN_SPEED);
                let (s, c) = (sin(i.run_phase), cos(i.run_phase));
                t[THIGH_L] = 0.85 * amp * s;
                t[THIGH_R] = -0.85 * amp * s;
                t[KNEE_L] = -(0.2 + 1.1 * max(c, 0.0)) * amp;
                t[KNEE_R] = -(0.2 + 1.1 * max(-c, 0.0)) * amp;
                t[ARM_LX] = -0.7 * amp * s;
                t[ARM_RX] = 0.7 * amp * s;
                t[ELBOW_L] = 0.2 + 1.2 * amp;
                t[ELBOW_R] = 0.2 + 1.2 * amp;
                t[LEAN] = 0.22 * amp;
                t[CHEST_Y] = 0.18 * amp * s;
                t[CROUCH] = 0.03 + 0.03 * abs(s);
            }
            act::SLIDE => {
                t[THIGH_L] = 1.2;
                t[THIGH_R] = 0.35;
                t[KNEE_L] = -1.5;
                t[KNEE_R] = -0.9;
                t[LEAN] = -0.35;
                t[CROUCH] = 0.42;
                t[ARM_LX] = -0.5;
                t[ARM_RX] = -0.5;
                t[ARM_LZ] = 0.55;
                t[ARM_RZ] = 0.55;
            }
            _ => {
                // Airborne: from a loose jump pose at low speed to a flat flying pose at high speed.
                t[UPRIGHT] = 1.0 - fl;
                t[LEAN] = clamp(0.2 + 1.0 * fl - climb * 0.55, -0.4, 1.4);
                t[THIGH_L] = lerp(0.45, -0.12, fl);
                t[THIGH_R] = lerp(0.1, -0.22, fl);
                t[KNEE_L] = lerp(-0.8, -0.3, fl);
                t[KNEE_R] = lerp(-0.45, -0.42, fl);
                t[ARM_LX] = lerp(-0.2, -0.8, fl);
                t[ARM_RX] = lerp(-0.2, -0.8, fl);
                t[ARM_LZ] = lerp(0.7, 0.32, fl);
                t[ARM_RZ] = lerp(0.7, 0.32, fl);
                t[ELBOW_L] = 0.35;
                t[ELBOW_R] = 0.35;
                t[HEAD_X] = 0.6 * fl;
                match i.act {
                    act::REEL => {
                        t[THIGH_L] = 0.95;
                        t[THIGH_R] = 0.8;
                        t[KNEE_L] = -1.5;
                        t[KNEE_R] = -1.35;
                        t[ARM_LX] = -0.25;
                        t[ARM_RX] = -0.25;
                        t[ARM_LZ] = 0.18;
                        t[ARM_RZ] = 0.18;
                        t[ELBOW_L] = 1.3;
                        t[ELBOW_R] = 1.3;
                    }
                    act::THRUST => {
                        t[THIGH_L] = -0.2;
                        t[THIGH_R] = -0.28;
                        t[KNEE_L] = -0.1;
                        t[KNEE_R] = -0.16;
                        t[ARM_LX] = -0.95;
                        t[ARM_RX] = -0.95;
                    }
                    act::FALL if fl < 0.5 => {
                        t[ARM_LZ] = 1.0;
                        t[ARM_RZ] = 1.0;
                        t[ARM_LX] = 0.15;
                        t[ARM_RX] = 0.15;
                    }
                    _ => {}
                }
                // One wire lifts its hip.
                let l = i.wire[0].len2() > 0.0;
                let r = i.wire[1].len2() > 0.0;
                if l != r {
                    t[BANK] = if l { -0.32 } else { 0.32 };
                }
            }
        }
        if i.grounded && i.land_t < 0.25 {
            let k = 1.0 - i.land_t / 0.25;
            t[CROUCH] += 0.3 * k;
            t[KNEE_L] -= 0.9 * k;
            t[KNEE_R] -= 0.9 * k;
            t[THIGH_L] += 0.7 * k;
            t[THIGH_R] += 0.7 * k;
        }
        if i.slash_t >= 0.0 {
            let k = saturate(i.slash_t / tune::SLASH_TIME);
            t[ARM_LZ] = 1.35;
            t[ARM_RZ] = 1.35;
            t[ARM_LX] = lerp(-0.8, 0.9, k);
            t[ARM_RX] = lerp(-0.8, 0.9, k);
            t[ELBOW_L] = 0.1;
            t[ELBOW_R] = 0.1;
            if i.grounded {
                t[CHEST_Y] = lerp(1.0, -1.0, k);
            }
        }
        t
    }

    pub fn update(&mut self, i: &PoseIn) {
        let dt = crate::sim::DT;
        let t = Self::target(i);
        for k in 0..NJ {
            let rate = match k {
                LEAN | BANK => 9.0,
                UPRIGHT => 7.0,
                ARM_LX | ARM_LZ | ARM_RX | ARM_RZ if i.slash_t >= 0.0 => 40.0,
                _ if i.act == act::RUN => 30.0,
                _ => 14.0,
            };
            self.j[k] = ease(self.j[k], t[k], rate, dt);
        }
        let j = &self.j;

        // A slash in the air spins the body about its direction of travel.
        let spin = if i.slash_t >= 0.0 && !i.grounded { smoothstep(0.0, 1.0, i.slash_t / tune::SLASH_TIME) * TAU } else { 0.0 };
        let root = M3::rot_y(i.facing).mul(&M3::rot_x(-j[LEAN])).mul(&M3::rot_z(j[BANK] + spin));
        let pelvis_pos = i.pos + v3(0.0, (PELVIS_Y - RADIUS) * j[UPRIGHT] - j[CROUCH], 0.0);
        let pelvis = M34::new(root, pelvis_pos);

        let joint = |parent: &M34, at: V3, r: M3| parent.mul(&M34::new(r, at));
        let chest = joint(&pelvis, v3(0.0, WAIST_Y, 0.0), M3::rot_x(j[CHEST_X]).mul(&M3::rot_y(j[CHEST_Y])));
        let head = joint(&chest, v3(0.0, CHEST, 0.0), M3::rot_x(j[HEAD_X]));
        let arm_ul = joint(&chest, v3(-SHOULDER_X, SHOULDER_Y, 0.0), M3::rot_z(-j[ARM_LZ]).mul(&M3::rot_x(j[ARM_LX])));
        let arm_ll = joint(&arm_ul, v3(0.0, -UPPER_ARM, 0.0), M3::rot_x(j[ELBOW_L]));
        let arm_ur = joint(&chest, v3(SHOULDER_X, SHOULDER_Y, 0.0), M3::rot_z(j[ARM_RZ]).mul(&M3::rot_x(j[ARM_RX])));
        let arm_lr = joint(&arm_ur, v3(0.0, -UPPER_ARM, 0.0), M3::rot_x(j[ELBOW_R]));
        let leg_ul = joint(&pelvis, v3(-HIP_X, HIP_Y, 0.0), M3::rot_z(-j[SPREAD]).mul(&M3::rot_x(j[THIGH_L])));
        let leg_ll = joint(&leg_ul, v3(0.0, -THIGH, 0.0), M3::rot_x(j[KNEE_L]));
        let leg_ur = joint(&pelvis, v3(HIP_X, HIP_Y, 0.0), M3::rot_z(j[SPREAD]).mul(&M3::rot_x(j[THIGH_R])));
        let leg_lr = joint(&leg_ur, v3(0.0, -THIGH, 0.0), M3::rot_x(j[KNEE_R]));

        // The cape hangs under gravity and trails against the motion.
        let speed = i.vel.len();
        let wave = sin(i.tick as f32 * 0.37) * 0.10 * saturate(speed / 12.0);
        let back = chest.r.z;
        let side = chest.r.x;
        let cape_at = chest.apply(v3(0.0, CAPE_Y, CAPE_Z));
        let d1 = (v3(0.0, -1.0, 0.0) + i.vel * -0.07 + back * 0.3 + side * wave).norm_or(v3(0.0, -1.0, 0.0));
        let d2 = (d1 + i.vel * -0.035 + v3(0.0, -0.25, 0.0) - side * (wave * 1.6)).norm_or(d1);
        let hang = |d: V3, at: V3| {
            let y = -d;
            let x = (side - y * side.dot(y)).norm_or(v3(1.0, 0.0, 0.0));
            M34::new(M3 { x, y, z: x.cross(y) }, at)
        };
        let cape_a = hang(d1, cape_at);
        let cape_b = hang(d2, cape_at + d1 * CAPE_SEG);

        self.parts = [pelvis, chest, head, arm_ul, arm_ll, arm_ur, arm_lr, leg_ul, leg_ll, leg_ur, leg_lr, cape_a, cape_b];
        // Blade tips: the blades leave the fists forward and down (`BLADE_DIR` in web/src/world/models.ts).
        self.blade = [arm_ll.apply(v3(0.0, -FOREARM - 0.515, -0.823)), arm_lr.apply(v3(0.0, -FOREARM - 0.515, -0.823))];
    }
}
