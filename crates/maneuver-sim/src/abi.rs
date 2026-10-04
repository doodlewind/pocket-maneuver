//! C interface for the wasm host, and the flat snapshot every host reads.
//!
//! A snapshot is one `f32` array. `layout!` assigns the offsets; the `abi`
//! binary prints them as TypeScript so the reference never hand-copies one.

use crate::math::*;
use crate::pose::PARTS;
use crate::sim::{Input, Sim};

pub const ABI_VERSION: u32 = 1;

macro_rules! layout {
    ($( $name:ident : $n:expr ),* $(,)?) => {
        layout!(@step 0usize; $($name : $n,)*);
        /// Name, offset and length of every snapshot field.
        pub const FIELDS: &[(&str, usize, usize)] = &[ $( (stringify!($name), $name, $n) ),* ];
    };
    (@step $off:expr; $name:ident : $n:expr, $($rest:tt)*) => {
        pub const $name: usize = $off;
        layout!(@step $off + $n; $($rest)*);
    };
    (@step $off:expr;) => {
        pub const LEN: usize = $off;
    };
}

pub mod snap {
    use super::PARTS;
    layout! {
        TICK: 1, POS: 3, VEL: 3, SPEED: 1, FACING: 1, GROUNDED: 1, GAS: 1, ACT: 1, ACT_T: 1, SLASH_T: 1,
        HOOK_L_STATE: 1, HOOK_L_TIP: 3, HOOK_L_ANCHOR: 3,
        HOOK_R_STATE: 1, HOOK_R_TIP: 3, HOOK_R_ANCHOR: 3,
        HIP_L: 3, HIP_R: 3,
        CAM_POS: 3, CAM_LOOK: 3, CAM_FOV: 1, CAM_ROLL: 1, CAM_SHAKE: 1, CAM_YAW: 1, CAM_PITCH: 1,
        RET_L: 4, RET_R: 4, RET_ZIP: 4,
        RUN_STARTED: 1, RUN_DONE: 1, RUN_TICKS: 1, RUN_KILLS: 1, RUN_TOTAL: 1, RUN_MAX_SPEED: 1, LAST_CUT: 1, LAST_CUT_SPEED: 1,
        EVENTS: 1, THRUST: 1, REEL: 1, WALL_RUN: 1, AUTO_WP: 1, AUTO_LAPS: 1,
        BLADE_L: 3, BLADE_R: 3,
        PARTS_AT: PARTS * 12,
    }
}

fn put(out: &mut [f32], at: usize, v: V3) {
    out[at] = v.x;
    out[at + 1] = v.y;
    out[at + 2] = v.z;
}

impl Sim {
    /// Writes the snapshot; `out` must hold `snap::LEN` floats.
    pub fn snapshot(&self, out: &mut [f32]) {
        use snap::*;
        let p = &self.p;
        out[TICK] = self.tick as f32;
        put(out, POS, p.pos);
        put(out, VEL, p.vel);
        out[SPEED] = p.vel.len();
        out[FACING] = p.facing;
        out[GROUNDED] = p.grounded as u32 as f32;
        out[GAS] = p.gas;
        out[ACT] = p.act as f32;
        out[ACT_T] = p.act_t;
        out[SLASH_T] = p.slash_t;
        for (i, (st, tip, anchor)) in [(HOOK_L_STATE, HOOK_L_TIP, HOOK_L_ANCHOR), (HOOK_R_STATE, HOOK_R_TIP, HOOK_R_ANCHOR)].into_iter().enumerate() {
            out[st] = p.hooks[i].state as f32;
            put(out, tip, p.hooks[i].tip);
            put(out, anchor, p.hooks[i].anchor);
        }
        put(out, HIP_L, self.hip(0));
        put(out, HIP_R, self.hip(1));
        put(out, CAM_POS, self.cam.pos);
        put(out, CAM_LOOK, self.cam.look);
        out[CAM_FOV] = self.cam.fov;
        out[CAM_ROLL] = self.cam.roll;
        out[CAM_SHAKE] = self.cam.shake;
        out[CAM_YAW] = self.cam.yaw;
        out[CAM_PITCH] = self.cam.pitch;
        for (i, at) in [RET_L, RET_R, RET_ZIP].into_iter().enumerate() {
            out[at] = self.reticle[i].valid as u32 as f32;
            put(out, at + 1, self.reticle[i].point);
        }
        out[RUN_STARTED] = self.run.started as u32 as f32;
        out[RUN_DONE] = self.run.done as u32 as f32;
        out[RUN_TICKS] = self.run.ticks as f32;
        out[RUN_KILLS] = self.run.kills as f32;
        out[RUN_TOTAL] = self.dummies.len() as f32;
        out[RUN_MAX_SPEED] = self.run.max_speed;
        out[LAST_CUT] = self.run.last_cut as f32;
        out[LAST_CUT_SPEED] = self.run.last_cut_speed;
        out[EVENTS] = self.events as f32;
        out[THRUST] = p.thrusting as u32 as f32;
        out[REEL] = p.reeling as u32 as f32;
        out[WALL_RUN] = p.wall_run;
        out[AUTO_WP] = self.auto.wp as f32;
        out[AUTO_LAPS] = self.auto.laps as f32;
        put(out, BLADE_L, self.pose.blade[0]);
        put(out, BLADE_R, self.pose.blade[1]);
        for (i, m) in self.pose.parts.iter().enumerate() {
            let at = PARTS_AT + i * 12;
            put(out, at, m.r.x);
            put(out, at + 3, m.r.y);
            put(out, at + 6, m.r.z);
            put(out, at + 9, m.t);
        }
    }
}

// ---------------------------------------------------------------------- wasm

static mut SIM: Option<Sim> = None;
static mut SNAP: [f32; snap::LEN] = [0.0; snap::LEN];
static mut DUMMIES: Vec<f32> = Vec::new();
static mut RAY: [f32; 5] = [0.0; 5];

#[allow(static_mut_refs)]
fn sim() -> Option<&'static mut Sim> {
    unsafe { SIM.as_mut() }
}

#[no_mangle]
pub extern "C" fn mv_abi_version() -> u32 {
    ABI_VERSION
}

#[no_mangle]
pub extern "C" fn mv_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    core::mem::forget(v);
    p
}

/// # Safety
/// `ptr` and `len` must come from `mv_alloc`.
#[no_mangle]
pub unsafe extern "C" fn mv_free(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

/// Loads a world file. Returns 0, or a negative code.
///
/// # Safety
/// `ptr` must point at `len` readable bytes.
#[no_mangle]
#[allow(static_mut_refs)]
pub unsafe extern "C" fn mv_load(ptr: *const u8, len: usize) -> i32 {
    match crate::worldfile::load(core::slice::from_raw_parts(ptr, len)) {
        Ok(s) => {
            DUMMIES = vec![0.0; s.dummies.len() * 2];
            SIM = Some(s);
            0
        }
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn mv_reset() {
    if let Some(s) = sim() {
        s.reset();
    }
}

#[no_mangle]
pub extern "C" fn mv_tick(buttons: u32, lx: f32, ly: f32, rx: f32, ry: f32) {
    if let Some(s) = sim() {
        s.tick(Input { buttons, lx, ly, rx, ry });
    }
}

/// One tick flown by the autopilot. Returns the buttons it pressed.
#[no_mangle]
pub extern "C" fn mv_tick_auto() -> u32 {
    match sim() {
        Some(s) => {
            let i = s.auto_input();
            s.tick(i);
            i.buttons
        }
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn mv_snapshot_len() -> u32 {
    snap::LEN as u32
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn mv_snapshot() -> *const f32 {
    unsafe {
        if let Some(s) = SIM.as_ref() {
            s.snapshot(&mut SNAP);
        }
        SNAP.as_ptr()
    }
}

/// Per target: alive (0 or 1), ticks since its cut.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn mv_dummies() -> *const f32 {
    unsafe {
        if let Some(s) = SIM.as_ref() {
            for (i, d) in s.dummies.iter().enumerate() {
                DUMMIES[i * 2] = d.alive as u32 as f32;
                DUMMIES[i * 2 + 1] = if d.alive { -1.0 } else { s.tick.wrapping_sub(d.cut_tick) as f32 };
            }
        }
        DUMMIES.as_ptr()
    }
}

/// Casts a ray against the collision world. Returns `t`, or -1. `mv_ray_hit`
/// then points at the normal, the kind and the front flag.
#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn mv_raycast(ox: f32, oy: f32, oz: f32, dx: f32, dy: f32, dz: f32, tmax: f32) -> f32 {
    let Some(s) = sim() else { return -1.0 };
    match s.world.raycast(v3(ox, oy, oz), v3(dx, dy, dz).norm(), tmax, crate::collide::mask::ALL) {
        Some(h) => {
            unsafe { RAY = [h.n.x, h.n.y, h.n.z, h.kind as f32, h.front as u32 as f32] };
            h.t
        }
        None => -1.0,
    }
}

#[no_mangle]
#[allow(static_mut_refs)]
pub extern "C" fn mv_ray_hit() -> *const f32 {
    unsafe { RAY.as_ptr() }
}
