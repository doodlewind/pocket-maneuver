//! The game simulation: a sphere on wires.
//!
//! One `tick` advances 1/60 s. The player is a sphere of `RADIUS` moved by
//! gravity, two wire hooks (a ratcheting inextensible rope with a tension
//! pull), compressed-gas thrust and ground contact. All state lives in `Sim`;
//! the same inputs produce the same state on every target.

use crate::collide::{kind, mask, World};
use crate::math::*;
use crate::pose::{Pose, PoseIn};

pub const DT: f32 = 1.0 / 60.0;
const SUBSTEPS: u32 = 2;
/// Collision sphere radius; the feet are `RADIUS` below the centre.
pub const RADIUS: f32 = 0.5;

pub mod btn {
    pub const HOOK_L: u32 = 1;
    pub const HOOK_R: u32 = 2;
    pub const GAS: u32 = 4;
    pub const SLASH: u32 = 8;
    pub const ZIP: u32 = 16;
    pub const DROP: u32 = 32;
    pub const RESET: u32 = 64;
}

pub mod ev {
    pub const JUMP: u32 = 1 << 0;
    pub const HOOK_FIRE_L: u32 = 1 << 1;
    pub const HOOK_FIRE_R: u32 = 1 << 2;
    pub const HOOK_ATTACH_L: u32 = 1 << 3;
    pub const HOOK_ATTACH_R: u32 = 1 << 4;
    pub const RELEASE: u32 = 1 << 5;
    pub const BURST: u32 = 1 << 6;
    pub const SLASH: u32 = 1 << 7;
    pub const SLASH_HIT: u32 = 1 << 8;
    pub const SLASH_WEAK: u32 = 1 << 9;
    pub const LAND: u32 = 1 << 10;
    pub const LAND_HARD: u32 = 1 << 11;
    pub const WALL_KICK: u32 = 1 << 12;
    pub const REFILL: u32 = 1 << 13;
    pub const RESPAWN: u32 = 1 << 14;
    pub const RUN_DONE: u32 = 1 << 15;
    pub const HOOK_MISS: u32 = 1 << 16;
    pub const WALL_HIT: u32 = 1 << 17;
}

pub mod act {
    pub const IDLE: u32 = 0;
    pub const RUN: u32 = 1;
    pub const SLIDE: u32 = 2;
    pub const RISE: u32 = 3;
    pub const FALL: u32 = 4;
    pub const HOOK: u32 = 5;
    pub const REEL: u32 = 6;
    pub const THRUST: u32 = 7;
    pub const WALL: u32 = 8;
}

pub mod hook {
    pub const IDLE: u8 = 0;
    pub const FLYING: u8 = 1;
    pub const ATTACHED: u8 = 2;
    pub const RETRACT: u8 = 3;
    pub const MISS: u8 = 4;
}

/// Tuning. Units: metres, seconds, m/s, m/s².
pub mod tune {
    pub const GRAVITY: f32 = 19.0;
    /// Quadratic drag coefficient (1/m): terminal fall speed is sqrt(GRAVITY / DRAG).
    pub const DRAG: f32 = 0.0085;
    pub const V_MAX: f32 = 85.0;

    pub const RUN_SPEED: f32 = 10.0;
    pub const GROUND_ACCEL: f32 = 48.0;
    pub const GROUND_DECEL: f32 = 60.0;
    pub const SLIDE_FRICTION: f32 = 9.0;
    pub const JUMP_V: f32 = 9.5;
    /// A normal with at least this much `y` is ground.
    pub const WALK_NY: f32 = 0.55;

    pub const AIR_ACCEL: f32 = 11.0;
    pub const AIR_MAX: f32 = 16.0;

    pub const HOOK_RANGE: f32 = 95.0;
    pub const ZIP_RANGE: f32 = 150.0;
    pub const HOOK_SPEED: f32 = 190.0;
    /// Pull toward an anchor while its hook is attached.
    pub const TENSION: f32 = 17.0;
    /// Extra pull while gas reels the wire in.
    pub const REEL: f32 = 30.0;
    /// With two wires attached each pulls this fraction.
    pub const TWO_WIRE: f32 = 0.68;
    /// No pull is added above this closing speed.
    pub const REEL_V_MAX: f32 = 52.0;
    /// Upward assist while hanging on a wire whose anchor is above.
    pub const LIFT: f32 = 8.0;
    /// A wire lets go when its anchor is this close.
    pub const DETACH_DIST: f32 = 2.4;

    pub const THRUST: f32 = 30.0;
    pub const BURST_DV: f32 = 9.0;
    pub const BURST_COOLDOWN: f32 = 0.45;
    pub const DIVE: f32 = 26.0;

    pub const GAS_MAX: f32 = 100.0;
    pub const GAS_REEL: f32 = 4.5;
    pub const GAS_THRUST: f32 = 8.0;
    pub const GAS_BURST: f32 = 2.5;
    pub const GAS_REGEN_GROUND: f32 = 10.0;
    /// Regeneration anywhere, once no gas has been used for `GAS_REST` seconds.
    pub const GAS_REGEN: f32 = 2.5;
    pub const GAS_REST: f32 = 1.2;

    pub const WALL_KICK_OUT: f32 = 8.0;
    pub const WALL_KICK_UP: f32 = 10.0;
    pub const WALL_RUN_TIME: f32 = 1.3;

    pub const SLASH_TIME: f32 = 0.34;
    pub const SLASH_COOLDOWN: f32 = 0.5;
    pub const SLASH_REACH: f32 = 3.4;
    /// A cut needs this much speed.
    pub const CUT_SPEED: f32 = 9.0;

    pub const CAM_YAW_RATE: f32 = 2.7;
    pub const CAM_PITCH_RATE: f32 = 1.9;
    pub const CAM_DIST: f32 = 5.2;
    pub const CAM_DIST_FAST: f32 = 6.8;
    pub const CAM_FOV: f32 = 62.0;
    pub const CAM_FOV_FAST: f32 = 80.0;
}
use tune::*;

#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub buttons: u32,
    /// Left stick, -1..1; `ly` positive is forward.
    pub lx: f32,
    pub ly: f32,
    /// Right stick, -1..1; `ry` positive looks up.
    pub rx: f32,
    pub ry: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Hook {
    pub state: u8,
    pub anchor: V3,
    pub normal: V3,
    pub tip: V3,
    /// Start of a retract, or the direction of a miss.
    pub from: V3,
    pub len: f32,
    pub t: f32,
    pub flight: f32,
    pub zip: bool,
}

impl Hook {
    const NONE: Hook = Hook { state: hook::IDLE, anchor: V3::ZERO, normal: V3::UP, tip: V3::ZERO, from: V3::ZERO, len: 0.0, t: 0.0, flight: 0.0, zip: false };
    #[inline]
    pub fn attached(&self) -> bool {
        self.state == hook::ATTACHED
    }
    #[inline]
    pub fn out(&self) -> bool {
        self.state != hook::IDLE
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Anchor {
    pub valid: bool,
    pub point: V3,
    pub normal: V3,
}

#[derive(Clone, Copy, Debug)]
pub struct Player {
    pub pos: V3,
    pub vel: V3,
    pub grounded: bool,
    pub ground_n: V3,
    pub ground_kind: u8,
    /// Time since the last ground contact.
    pub air_time: f32,
    /// Ground contact is ignored while this runs (after a jump).
    pub no_ground: f32,
    pub wall_n: V3,
    /// Time left in which a wall kick is accepted.
    pub wall_t: f32,
    pub wall_run: f32,
    pub facing: f32,
    pub gas: f32,
    pub hooks: [Hook; 2],
    /// Time into the slash, or negative when idle.
    pub slash_t: f32,
    pub slash_cd: f32,
    pub slash_done: bool,
    pub burst_cd: f32,
    pub thrusting: bool,
    pub reeling: bool,
    pub land_t: f32,
    /// Time since gas was last used.
    pub gas_rest: f32,
    pub run_phase: f32,
    pub act: u32,
    pub act_t: f32,
    pub safe: V3,
    pub refill_cd: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Cam {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub fov: f32,
    pub pivot: V3,
    pub pos: V3,
    pub look: V3,
    pub idle: f32,
    pub shake: f32,
    pub kick: f32,
    pub roll: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Dummy {
    /// Base centre on the ground.
    pub pos: V3,
    pub yaw: f32,
    pub height: f32,
    pub nape: V3,
    pub alive: bool,
    pub cut_tick: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Depot {
    pub pos: V3,
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Run {
    pub started: bool,
    pub done: bool,
    pub ticks: u32,
    pub kills: u32,
    pub max_speed: f32,
    pub last_cut_speed: f32,
    pub last_cut: u32,
}

pub struct Sim {
    pub world: World,
    pub spawn: V3,
    pub spawn_yaw: f32,
    pub bounds: f32,
    pub dummies: Vec<Dummy>,
    pub depots: Vec<Depot>,
    pub waypoints: Vec<V3>,
    pub tick: u32,
    pub p: Player,
    pub cam: Cam,
    pub run: Run,
    pub events: u32,
    pub reticle: [Anchor; 3],
    pub pose: Pose,
    pub auto: crate::auto::Auto,
    prev_buttons: u32,
    hitstop: u32,
}

fn deadzone(x: f32, y: f32) -> (f32, f32) {
    const DEAD: f32 = 0.18;
    let m = sqrt(x * x + y * y);
    if m <= DEAD {
        return (0.0, 0.0);
    }
    let k = min((m - DEAD) / (1.0 - DEAD), 1.0) / m;
    (x * k, y * k)
}

#[inline]
fn right_of(yaw: f32) -> V3 {
    v3(cos(yaw), 0.0, -sin(yaw))
}

/// Distance from point `c` to the segment `a`–`b`.
fn seg_dist(c: V3, a: V3, b: V3) -> f32 {
    let ab = b - a;
    let l2 = ab.len2();
    let t = if l2 > 1e-9 { saturate((c - a).dot(ab) / l2) } else { 0.0 };
    (c - (a + ab * t)).len()
}

impl Sim {
    pub fn new(world: World, spawn: V3, spawn_yaw: f32, bounds: f32) -> Sim {
        let mut s = Sim {
            world,
            spawn,
            spawn_yaw,
            bounds,
            dummies: Vec::new(),
            depots: Vec::new(),
            waypoints: Vec::new(),
            tick: 0,
            p: Player {
                pos: spawn,
                vel: V3::ZERO,
                grounded: false,
                ground_n: V3::UP,
                ground_kind: kind::GROUND,
                air_time: 0.0,
                no_ground: 0.0,
                wall_n: V3::ZERO,
                wall_t: 0.0,
                wall_run: 0.0,
                facing: spawn_yaw,
                gas: GAS_MAX,
                hooks: [Hook::NONE; 2],
                slash_t: -1.0,
                slash_cd: 0.0,
                slash_done: false,
                burst_cd: 0.0,
                thrusting: false,
                reeling: false,
                land_t: 1.0,
                gas_rest: 0.0,
                run_phase: 0.0,
                act: act::IDLE,
                act_t: 0.0,
                safe: spawn,
                refill_cd: 0.0,
            },
            cam: Cam { yaw: spawn_yaw, pitch: -0.14, dist: CAM_DIST, fov: CAM_FOV, pivot: spawn, pos: spawn, look: heading(spawn_yaw), idle: 10.0, shake: 0.0, kick: 0.0, roll: 0.0 },
            run: Run::default(),
            events: 0,
            reticle: [Anchor::default(); 3],
            pose: Pose::new(),
            auto: crate::auto::Auto::new(),
            prev_buttons: 0,
            hitstop: 0,
        };
        s.reset();
        s
    }

    /// Puts the player back at the spawn and restores every target.
    pub fn reset(&mut self) {
        self.respawn(self.spawn, self.spawn_yaw);
        for d in &mut self.dummies {
            d.alive = true;
            d.cut_tick = 0;
        }
        self.run = Run::default();
        self.p.gas = GAS_MAX;
        self.auto = crate::auto::Auto::new();
        self.tick = 0;
    }

    fn respawn(&mut self, at: V3, yaw: f32) {
        let p = &mut self.p;
        p.pos = at;
        p.vel = V3::ZERO;
        p.grounded = false;
        p.air_time = 0.0;
        p.no_ground = 0.0;
        p.wall_t = 0.0;
        p.wall_run = 0.0;
        p.facing = yaw;
        p.hooks = [Hook::NONE; 2];
        p.slash_t = -1.0;
        p.safe = at;
        self.cam.yaw = yaw;
        self.cam.pitch = -0.14;
        self.cam.dist = CAM_DIST;
        self.cam.pivot = at + v3(0.0, 1.1, 0.0);
        self.cam.idle = 10.0;
        self.events |= ev::RESPAWN;
        self.place_camera();
    }

    #[inline]
    pub fn speed(&self) -> f32 {
        self.p.vel.len()
    }

    /// Where the wire of hook `i` leaves the hip gear.
    pub fn hip(&self, i: usize) -> V3 {
        let side = if i == 0 { -1.0 } else { 1.0 };
        self.p.pos + right_of(self.p.facing) * (0.26 * side) + v3(0.0, 0.22, 0.0)
    }

    // ------------------------------------------------------------------ targeting

    /// The best anchor for the hook on `side` (-1 left, +1 right).
    pub fn find_anchor(&self, side: f32, base_yaw: f32) -> Anchor {
        const YAWS: [f32; 6] = [-0.10, 0.14, 0.38, 0.66, 0.96, 1.30];
        const ELEVS: [f32; 7] = [-0.32, -0.08, 0.12, 0.33, 0.56, 0.80, 1.05];
        let o = self.p.pos + v3(0.0, 0.3, 0.0);
        let speed = self.speed();
        let ideal = clamp(24.0 + 0.55 * speed, 24.0, 62.0);
        let bias = clamp(self.cam.pitch, -0.5, 0.7) * 0.7;
        let mut best = Anchor::default();
        let mut best_s = -1e9f32;
        for yo in YAWS {
            for el in ELEVS {
                let e = clamp(el + bias, -0.7, 1.35);
                let d = forward(base_yaw - side * yo, e);
                let Some(h) = self.world.raycast(o, d, HOOK_RANGE, mask::ALL) else { continue };
                if !h.front || mask::ANCHOR & (1 << h.kind) == 0 || h.t < 7.0 {
                    continue;
                }
                let s = 1.0 - abs(h.t - ideal) / ideal - abs(yo) * 0.35 - abs(e - 0.5) * 0.5 - if yo < 0.0 { 0.4 } else { 0.0 };
                if s > best_s {
                    best_s = s;
                    best = Anchor { valid: true, point: o + d * h.t - d * 0.05, normal: h.n };
                }
            }
        }
        best
    }

    /// The surface under the screen centre, for an aimed pair of hooks.
    pub fn find_zip(&self) -> Anchor {
        let f = forward(self.cam.yaw, self.cam.pitch);
        let o = self.cam.pos;
        let skip = max((self.p.pos - o).dot(f), 0.0) + 1.2;
        let start = o + f * skip;
        match self.world.raycast(start, f, ZIP_RANGE, mask::ALL) {
            Some(h) if h.front && mask::AIMED & (1 << h.kind) != 0 && (start + f * h.t - self.p.pos).len() > 6.0 => Anchor { valid: true, point: start + f * h.t - f * 0.05, normal: h.n },
            _ => Anchor::default(),
        }
    }

    fn fire(&mut self, i: usize, a: Anchor, zip: bool) {
        let hip = self.hip(i);
        let h = &mut self.p.hooks[i];
        if a.valid {
            let d = (a.point - hip).len();
            *h = Hook { state: hook::FLYING, anchor: a.point, normal: a.normal, tip: hip, from: hip, len: d, t: 0.0, flight: clamp(d / HOOK_SPEED, 0.05, 0.6), zip };
        } else {
            let side = if i == 0 { -1.0 } else { 1.0 };
            let dir = forward(self.cam.yaw - side * 0.3, clamp(self.cam.pitch + 0.45, -0.3, 1.2));
            *h = Hook { state: hook::MISS, anchor: hip, normal: V3::UP, tip: hip, from: dir, len: 0.0, t: 0.0, flight: 0.0, zip };
            self.events |= ev::HOOK_MISS;
        }
        self.events |= if i == 0 { ev::HOOK_FIRE_L } else { ev::HOOK_FIRE_R };
    }

    fn release(&mut self, i: usize) {
        let h = &mut self.p.hooks[i];
        if h.state == hook::IDLE || h.state == hook::RETRACT {
            return;
        }
        if h.state == hook::ATTACHED {
            self.events |= ev::RELEASE;
        }
        h.from = h.tip;
        h.state = hook::RETRACT;
        h.t = 0.0;
        h.zip = false;
    }

    fn update_hooks(&mut self) {
        for i in 0..2 {
            let hip = self.hip(i);
            let pos = self.p.pos;
            let grounded = self.p.grounded;
            let h = &mut self.p.hooks[i];
            match h.state {
                hook::FLYING => {
                    h.t += DT;
                    let k = min(h.t / h.flight, 1.0);
                    h.tip = hip.lerp(h.anchor, k);
                    if k >= 1.0 {
                        h.state = hook::ATTACHED;
                        h.len = (h.anchor - pos).len();
                        h.tip = h.anchor;
                        self.events |= if i == 0 { ev::HOOK_ATTACH_L } else { ev::HOOK_ATTACH_R };
                        if grounded {
                            // A wire that bites while standing lifts the player off the ground.
                            self.p.vel.y = max(self.p.vel.y, 6.5);
                            self.p.grounded = false;
                            self.p.no_ground = 0.18;
                        }
                    }
                }
                hook::MISS => {
                    h.t += DT;
                    h.tip = hip + h.from * min(h.t * HOOK_SPEED, 42.0);
                    if h.t > 0.28 {
                        h.from = h.tip;
                        h.state = hook::RETRACT;
                        h.t = 0.0;
                    }
                }
                hook::RETRACT => {
                    h.t += DT;
                    let k = min(h.t / 0.12, 1.0);
                    h.tip = h.from.lerp(hip, k);
                    if k >= 1.0 {
                        h.state = hook::IDLE;
                    }
                }
                hook::ATTACHED => {
                    h.tip = h.anchor;
                }
                _ => {
                    h.tip = hip;
                }
            }
        }
        // A wire lets go at its anchor, and bends around what gets in its way.
        for i in 0..2 {
            if !self.p.hooks[i].attached() {
                continue;
            }
            let to = self.p.hooks[i].anchor - self.p.pos;
            let d = to.len();
            if d < DETACH_DIST {
                self.release(i);
                continue;
            }
            if (self.tick + i as u32) % 4 == 0 && d > 3.0 {
                let dir = to * (1.0 / d);
                if let Some(hit) = self.world.raycast(self.p.pos, dir, d - 0.6, mask::ALL) {
                    if hit.front && mask::AIMED & (1 << hit.kind) != 0 {
                        let h = &mut self.p.hooks[i];
                        h.anchor = self.p.pos + dir * (hit.t - 0.05);
                        h.normal = hit.n;
                        h.len = min(h.len, hit.t);
                        h.tip = h.anchor;
                    } else {
                        self.release(i);
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------ physics

    fn step(&mut self, h: f32, move_dir: V3, mag: f32, gas_held: bool, dive: bool) {
        let p = &mut self.p;
        let speed = p.vel.len();
        let mut a = v3(0.0, -GRAVITY, 0.0);

        // Wires.
        let n_att = p.hooks.iter().filter(|k| k.attached()).count();
        let share = if n_att == 2 { TWO_WIRE } else { 1.0 };
        let mut lifted = false;
        p.reeling = false;
        for k in &mut p.hooks {
            if !k.attached() {
                continue;
            }
            let r = k.anchor - p.pos;
            let d = r.len();
            if d < 1e-3 {
                continue;
            }
            let n = r * (1.0 / d);
            k.len = min(k.len, d);
            let mut pull = TENSION;
            if (gas_held || k.zip) && p.gas > 0.0 {
                pull += REEL;
                p.gas = max(p.gas - GAS_REEL * h * share, 0.0);
                p.reeling = true;
            }
            if p.vel.dot(n) < REEL_V_MAX {
                a += n * (pull * share);
            }
            if r.y > 1.0 {
                lifted = true;
            }
        }
        if lifted && !p.grounded {
            a.y += LIFT;
        }

        // Gas thrust with no wire attached.
        p.thrusting = false;
        if gas_held && n_att == 0 && !p.grounded && p.gas > 0.0 && p.no_ground <= 0.0 {
            let up = 0.55 + max(sin(self.cam.pitch), 0.0) * 0.6;
            let dir = (move_dir * mag + v3(0.0, up, 0.0)).norm_or(V3::UP);
            a += dir * THRUST;
            p.gas = max(p.gas - GAS_THRUST * h, 0.0);
            p.thrusting = true;
        }
        if dive && !p.grounded {
            a.y -= DIVE;
        }

        if p.grounded {
            // Ground control acts on the horizontal velocity; the vertical part follows the ground plane.
            let n = p.ground_n;
            let mut vh = p.vel.flat();
            let sp = vh.len();
            let wet = if p.ground_kind == kind::WATER { 0.6 } else { 1.0 };
            let want = move_dir * (RUN_SPEED * mag * wet);
            if n_att > 0 {
                // Dragged along the ground by a wire: no friction.
            } else if sp <= RUN_SPEED * 1.15 {
                let dv = want - vh;
                let rate = if mag > 0.0 { GROUND_ACCEL } else { GROUND_DECEL };
                let l = dv.len();
                vh = if l <= rate * h { want } else { vh + dv * (rate * h / l) };
            } else {
                // Sliding in faster than a run: friction, with a little steering.
                let ns = max(sp - SLIDE_FRICTION * h, 0.0);
                let mut dir = vh * (1.0 / sp);
                if mag > 0.0 {
                    dir = (dir + move_dir * (2.2 * h)).norm_or(dir);
                }
                vh = dir * ns;
            }
            p.vel.x = vh.x;
            p.vel.z = vh.z;
            if n_att == 0 && n.y > 0.2 {
                // Follow the ground plane; a wire is free to lift the player off it.
                let vy = -(n.x * vh.x + n.z * vh.z) / n.y;
                p.vel.y = clamp(p.vel.y, vy - 2.0, vy);
            }
        } else {
            // Air control, limited to a strafe speed.
            if mag > 0.0 && p.vel.dot(move_dir) < AIR_MAX {
                a += move_dir * (AIR_ACCEL * mag);
            }
            // A wall touched at speed carries the player along it for a moment.
            if p.wall_run > 0.0 && n_att == 0 {
                a.y += GRAVITY * 0.62;
            }
        }
        if !p.grounded || n_att > 0 {
            a -= p.vel * (DRAG * speed);
        }

        p.vel += a * h;
        let sp = p.vel.len();
        if sp > V_MAX {
            p.vel *= V_MAX / sp;
        }

        // Move, stopping the centre in front of whatever it would cross.
        let delta = p.vel * h;
        let dist = delta.len();
        if dist > 1e-6 {
            let dir = delta * (1.0 / dist);
            match self.world.raycast(p.pos, dir, dist + 0.06, mask::ALL) {
                Some(hit) if hit.front => {
                    p.pos += dir * max(hit.t - 0.06, 0.0);
                }
                _ => p.pos += delta,
            }
        }

        // Rope: a wire never gets longer.
        for k in &p.hooks {
            if !k.attached() {
                continue;
            }
            let r = k.anchor - p.pos;
            let d = r.len();
            if d > k.len && d > 1e-3 {
                let n = r * (1.0 / d);
                p.pos += n * (d - k.len);
                let vn = p.vel.dot(n);
                if vn < 0.0 {
                    p.vel -= n * vn;
                }
            }
        }

        // Contacts.
        let mut ground: Option<(V3, u8)> = None;
        let mut wall: Option<V3> = None;
        let mut impact = 0.0f32;
        let mut wall_impact = 0.0f32;
        for _ in 0..4 {
            let Some(c) = self.world.deepest_contact(p.pos, RADIUS) else { break };
            p.pos += c.n * (c.depth + 1e-3);
            let vn = p.vel.dot(c.n);
            if vn < 0.0 {
                let before = p.vel.len();
                p.vel -= c.n * vn;
                impact = max(impact, -vn);
                // A glancing hit at speed turns the velocity along the surface instead of
                // cutting it: roofs and walls are skimmed, not stopped at.
                let along = p.vel.len();
                if before > 12.0 && along > 1.0 && p.no_ground <= 0.0 {
                    let keep = lerp(1.0, 0.4, smoothstep(0.3, 0.9, -vn / before));
                    p.vel *= max(before * keep, along) / along;
                }
            }
            if c.n.y >= WALK_NY {
                if ground.map_or(true, |g| c.n.y > g.0.y) {
                    ground = Some((c.n, c.kind));
                }
            } else if abs(c.n.y) < 0.45 {
                wall = Some(c.n);
                if vn < 0.0 {
                    wall_impact = max(wall_impact, -vn);
                }
            }
        }

        if let Some(n) = wall {
            if !p.grounded {
                p.wall_n = n;
                p.wall_t = 0.3;
                if wall_impact > 6.0 {
                    // A glancing hit keeps most of its speed along the wall.
                    p.vel += n * (wall_impact * 0.12);
                    self.events |= ev::WALL_HIT;
                    self.cam.shake = max(self.cam.shake, min(wall_impact / 40.0, 0.6));
                    if p.wall_run <= 0.0 && p.vel.flat().len() > 6.0 {
                        p.wall_run = WALL_RUN_TIME;
                    }
                }
            }
        }

        let was = p.grounded;
        if p.no_ground > 0.0 {
            p.grounded = false;
        } else if let Some((n, kd)) = ground {
            p.grounded = true;
            p.ground_n = n;
            p.ground_kind = kd;
            if !was {
                self.events |= if impact > 15.0 { ev::LAND_HARD } else { ev::LAND };
                p.land_t = 0.0;
                if impact > 15.0 {
                    self.cam.shake = max(self.cam.shake, min(impact / 45.0, 0.8));
                }
            }
        } else if was && p.vel.y <= 2.5 && n_att == 0 {
            // Stay on the ground over small drops and slope changes.
            match self.world.raycast(p.pos, v3(0.0, -1.0, 0.0), RADIUS + 0.45, mask::ALL) {
                Some(hit) if hit.front && hit.n.y >= WALK_NY => {
                    p.pos.y -= hit.t - RADIUS;
                    p.ground_n = hit.n;
                    p.ground_kind = hit.kind;
                }
                _ => p.grounded = false,
            }
        } else {
            p.grounded = false;
        }
    }

    // ------------------------------------------------------------------ camera

    fn orient_camera(&mut self, rx: f32, ry: f32) {
        let c = &mut self.cam;
        let p = &self.p;
        if rx != 0.0 || ry != 0.0 {
            // The stick response is squared: fine aim near the centre, full rate at the rim.
            c.yaw = wrap_angle(c.yaw - rx * abs(rx) * CAM_YAW_RATE * DT);
            c.pitch += ry * abs(ry) * CAM_PITCH_RATE * DT;
            c.idle = 0.0;
        } else {
            c.idle += DT;
        }
        let vh = p.vel.flat();
        let hs = vh.len();
        if c.idle > 0.55 {
            if hs > 5.0 {
                // Follow the direction of travel.
                let rate = saturate((hs - 5.0) / 22.0) * 2.4 * saturate((c.idle - 0.55) * 2.0);
                c.yaw = ease_angle(c.yaw, yaw_of(vh), rate, DT);
            }
            let sp = p.vel.len();
            let rest = if p.grounded || sp < 6.0 { -0.14 } else { clamp(asin(p.vel.y / sp) * 0.45 - 0.1, -0.6, 0.45) };
            c.pitch = ease(c.pitch, rest, 1.4 * saturate((c.idle - 0.55) * 2.0), DT);
        }
        c.pitch = clamp(c.pitch, -1.25, 1.25);
    }

    fn place_camera(&mut self) {
        let p = &self.p;
        let c = &mut self.cam;
        let sp = p.vel.len();
        let lead = p.vel * 0.035;
        let ll = lead.len();
        let lead = if ll > 1.6 { lead * (1.6 / ll) } else { lead };
        let target = p.pos + v3(0.0, 1.1, 0.0) + lead;
        c.pivot = c.pivot.ease(target, 16.0, DT);
        // Never let the pivot fall behind by more than the wire could cover.
        let lag = c.pivot - target;
        let l = lag.len();
        if l > 2.5 {
            c.pivot = target + lag * (2.5 / l);
        }
        let fast = smoothstep(10.0, 55.0, sp);
        let want = lerp(CAM_DIST, CAM_DIST_FAST, fast);
        let f = forward(c.yaw, c.pitch);
        let back = -f;
        let allowed = match self.world.raycast(c.pivot, back, want + 0.5, mask::ALL) {
            Some(h) => clamp(h.t - 0.4, 0.9, want),
            None => want,
        };
        c.dist = if allowed < c.dist { allowed } else { ease(c.dist, allowed, 3.5, DT) };
        c.pos = c.pivot + back * c.dist;
        c.look = f;
        c.kick = ease(c.kick, 0.0, 5.0, DT);
        c.fov = ease(c.fov, lerp(CAM_FOV, CAM_FOV_FAST, fast) + c.kick, 6.0, DT);
        c.shake = max(c.shake - 2.2 * DT, 0.0);
        // Bank a little into turns.
        let side = right_of(c.yaw);
        let lat = p.vel.dot(side);
        c.roll = ease(c.roll, clamp(-lat * 0.004, -0.09, 0.09) * fast, 4.0, DT);
    }

    // ------------------------------------------------------------------ tick

    pub fn tick(&mut self, input: Input) {
        self.events = 0;
        let pressed = input.buttons & !self.prev_buttons;
        let released = !input.buttons & self.prev_buttons;
        self.prev_buttons = input.buttons;
        self.tick = self.tick.wrapping_add(1);

        let (lx, ly) = deadzone(input.lx, input.ly);
        let (rx, ry) = deadzone(input.rx, input.ry);

        if self.hitstop > 0 {
            // A cut freezes the world for a few ticks; the camera still settles.
            self.hitstop -= 1;
            self.orient_camera(rx, ry);
            self.place_camera();
            self.update_pose(V3::ZERO, 0.0);
            return;
        }

        if pressed & btn::RESET != 0 {
            self.respawn(self.spawn, self.spawn_yaw);
        }

        self.orient_camera(rx, ry);
        let mv = heading(self.cam.yaw) * ly + right_of(self.cam.yaw) * lx;
        let mag = min(mv.len(), 1.0);
        let move_dir = mv.norm();

        if !self.run.started && (mag > 0.0 || input.buttons != 0) {
            self.run.started = true;
        }

        // Hooks: an automatic anchor on each shoulder button, an aimed pair on ZIP.
        let base_yaw = if mag > 0.0 { yaw_of(move_dir) } else { self.cam.yaw };
        for i in 0..2 {
            let b = if i == 0 { btn::HOOK_L } else { btn::HOOK_R };
            if pressed & b != 0 && matches!(self.p.hooks[i].state, hook::IDLE | hook::RETRACT) {
                let a = self.find_anchor(if i == 0 { -1.0 } else { 1.0 }, base_yaw);
                self.fire(i, a, false);
            }
            if released & b != 0 && !self.p.hooks[i].zip {
                self.release(i);
            }
        }
        if pressed & btn::ZIP != 0 {
            let a = self.find_zip();
            if a.valid {
                let side = right_of(self.cam.yaw) * 0.25;
                self.fire(0, Anchor { point: a.point - side, ..a }, true);
                self.fire(1, Anchor { point: a.point + side, ..a }, true);
                self.cam.kick = 6.0;
            } else {
                self.events |= ev::HOOK_MISS;
            }
        }
        if released & btn::ZIP != 0 {
            for i in 0..2 {
                if self.p.hooks[i].zip {
                    self.release(i);
                }
            }
        }
        if pressed & btn::DROP != 0 {
            self.release(0);
            self.release(1);
        }
        self.update_hooks();

        // Gas button edge: jump, wall kick or burst.
        let n_att = self.p.hooks.iter().filter(|k| k.attached()).count();
        if pressed & btn::GAS != 0 {
            let p = &mut self.p;
            if p.grounded || p.air_time < 0.12 {
                p.vel.y = JUMP_V;
                p.grounded = false;
                p.no_ground = 0.12;
                p.air_time = 1.0;
                self.events |= ev::JUMP;
            } else if p.wall_t > 0.0 {
                let n = p.wall_n;
                let vn = p.vel.dot(n);
                p.vel -= n * vn;
                p.vel += n * WALL_KICK_OUT + v3(0.0, WALL_KICK_UP, 0.0);
                p.wall_t = 0.0;
                p.wall_run = 0.0;
                self.events |= ev::WALL_KICK;
                self.cam.kick = 4.0;
            } else if n_att == 0 && p.burst_cd <= 0.0 && p.gas >= GAS_BURST {
                let f = forward(self.cam.yaw, self.cam.pitch);
                let dir = if mag > 0.0 { (move_dir + v3(0.0, 0.35, 0.0)).norm() } else { (f + v3(0.0, 0.3, 0.0)).norm() };
                p.vel += dir * BURST_DV;
                p.gas -= GAS_BURST;
                p.burst_cd = BURST_COOLDOWN;
                self.events |= ev::BURST;
                self.cam.kick = 7.0;
                self.cam.shake = max(self.cam.shake, 0.25);
            }
        }

        let prev_pos = self.p.pos;
        let gas_held = input.buttons & btn::GAS != 0;
        let dive = input.buttons & btn::DROP != 0;
        let h = DT / SUBSTEPS as f32;
        for _ in 0..SUBSTEPS {
            self.step(h, move_dir, mag, gas_held, dive);
        }

        // Timers and bookkeeping.
        {
            let p = &mut self.p;
            p.no_ground = max(p.no_ground - DT, 0.0);
            p.wall_t = max(p.wall_t - DT, 0.0);
            p.wall_run = max(p.wall_run - DT, 0.0);
            p.burst_cd = max(p.burst_cd - DT, 0.0);
            p.slash_cd = max(p.slash_cd - DT, 0.0);
            p.refill_cd = max(p.refill_cd - DT, 0.0);
            p.land_t += DT;
            p.gas_rest = if p.thrusting || p.reeling || p.burst_cd > BURST_COOLDOWN - 2.0 * DT { 0.0 } else { p.gas_rest + DT };
            if p.gas_rest > GAS_REST {
                p.gas = min(p.gas + GAS_REGEN * DT, GAS_MAX);
            }
            if p.grounded {
                p.air_time = 0.0;
                p.wall_run = 0.0;
                p.gas = min(p.gas + GAS_REGEN_GROUND * DT, GAS_MAX);
                if p.ground_n.y > 0.9 && p.ground_kind != kind::WATER {
                    p.safe = p.pos;
                }
                p.run_phase += p.vel.flat().len() * DT * 1.9;
            } else {
                p.air_time += DT;
            }
        }

        // World limits: a soft wall at the rim, and a floor under everything.
        let flat = self.p.pos.flat();
        let r = flat.len();
        if r > self.bounds {
            let inward = flat * (-1.0 / r);
            self.p.vel += inward * ((r - self.bounds) * 0.8 + 6.0) * DT * 4.0;
        }
        if self.p.pos.y < -60.0 {
            self.respawn(self.p.safe + v3(0.0, 1.0, 0.0), self.p.facing);
        }

        self.slash(pressed, prev_pos);

        // Depots refill the gas.
        for d in &self.depots {
            if self.p.refill_cd <= 0.0 && self.p.gas < GAS_MAX - 1.0 && (d.pos - self.p.pos).len() < d.radius {
                self.p.gas = GAS_MAX;
                self.p.refill_cd = 2.0;
                self.events |= ev::REFILL;
            }
        }

        // Facing follows travel.
        {
            let p = &mut self.p;
            let vh = p.vel.flat();
            if vh.len() > 1.5 {
                p.facing = ease_angle(p.facing, yaw_of(vh), if p.grounded { 14.0 } else { 9.0 }, DT);
            } else if mag > 0.0 {
                p.facing = ease_angle(p.facing, yaw_of(move_dir), 12.0, DT);
            }
        }

        // The run: time from the first input to the last cut.
        let sp = self.speed();
        if self.run.started && !self.run.done {
            self.run.ticks += 1;
            self.run.max_speed = max(self.run.max_speed, sp);
        }

        let a = self.classify(mag);
        if a != self.p.act {
            self.p.act = a;
            self.p.act_t = 0.0;
        } else {
            self.p.act_t += DT;
        }

        // Reticles: one search per tick, alternating.
        match self.tick % 6 {
            0 => self.reticle[0] = self.find_anchor(-1.0, base_yaw),
            3 => self.reticle[1] = self.find_anchor(1.0, base_yaw),
            1 => self.reticle[2] = self.find_zip(),
            _ => {}
        }

        self.place_camera();
        self.update_pose(move_dir, mag);
    }

    fn slash(&mut self, pressed: u32, prev_pos: V3) {
        if pressed & btn::SLASH != 0 && self.p.slash_cd <= 0.0 {
            self.p.slash_t = 0.0;
            self.p.slash_cd = SLASH_COOLDOWN;
            self.p.slash_done = false;
            self.events |= ev::SLASH;
        }
        if self.p.slash_t < 0.0 {
            return;
        }
        self.p.slash_t += DT;
        if self.p.slash_t > 0.03 && self.p.slash_t < SLASH_TIME - 0.04 && !self.p.slash_done {
            let sp = self.speed();
            let pos = self.p.pos;
            for (i, d) in self.dummies.iter_mut().enumerate() {
                if !d.alive || seg_dist(d.nape, prev_pos, pos) > SLASH_REACH {
                    continue;
                }
                self.p.slash_done = true;
                if sp >= CUT_SPEED {
                    d.alive = false;
                    d.cut_tick = self.tick;
                    self.run.kills += 1;
                    self.run.last_cut_speed = sp;
                    self.run.last_cut = i as u32;
                    self.events |= ev::SLASH_HIT;
                    self.hitstop = 4;
                    self.cam.shake = max(self.cam.shake, 0.7);
                    self.cam.kick = 5.0;
                } else {
                    self.events |= ev::SLASH_WEAK;
                }
                break;
            }
            if self.run.kills as usize == self.dummies.len() && !self.dummies.is_empty() && !self.run.done {
                self.run.done = true;
                self.events |= ev::RUN_DONE;
            }
        }
        if self.p.slash_t >= SLASH_TIME {
            self.p.slash_t = -1.0;
        }
    }

    fn classify(&self, mag: f32) -> u32 {
        let p = &self.p;
        let hs = p.vel.flat().len();
        if p.grounded {
            if hs > RUN_SPEED * 1.25 {
                act::SLIDE
            } else if hs > 0.6 || mag > 0.0 {
                act::RUN
            } else {
                act::IDLE
            }
        } else if p.hooks.iter().any(|h| h.attached()) {
            if p.reeling {
                act::REEL
            } else {
                act::HOOK
            }
        } else if p.thrusting {
            act::THRUST
        } else if p.wall_run > 0.0 && p.wall_t > 0.0 {
            act::WALL
        } else if p.vel.y > 1.0 {
            act::RISE
        } else {
            act::FALL
        }
    }

    fn update_pose(&mut self, _move_dir: V3, _mag: f32) {
        let p = &self.p;
        let mut wire = [V3::ZERO; 2];
        for i in 0..2 {
            if p.hooks[i].attached() {
                wire[i] = (p.hooks[i].anchor - p.pos).norm();
            }
        }
        let input = PoseIn {
            pos: p.pos,
            vel: p.vel,
            facing: p.facing,
            grounded: p.grounded,
            act: p.act,
            act_t: p.act_t,
            run_phase: p.run_phase,
            slash_t: p.slash_t,
            land_t: p.land_t,
            wire,
            tick: self.tick,
        };
        self.pose.update(&input);
    }
}
