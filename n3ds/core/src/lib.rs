//! C interface of the 3DS host (`../src/core.h` declares the same functions).
//!
//! The host owns the GPU, the pad, sound and storage; it hands the pack's
//! tables in once and then asks, each frame, what to draw.

#![no_std]

extern crate alloc;

#[path = "alloc.rs"]
mod allocator;

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_char;

use maneuver_handheld::actors::{self, ColorVertex, Frame, Giant};
use maneuver_handheld::game::{Game, Pad, Perf};
use maneuver_handheld::hud::{Font, Hud, HudVertex};
use maneuver_handheld::mat::{self, Mat4};
use maneuver_handheld::scene::Scene;
use maneuver_handheld::world::{Pick, World};
use maneuver_pack::{HandMesh, HandScene, PicaVertex};
use maneuver_sim::pose::{BONES, CLOAK_N};

const HELP: &str = "L / R wires   B gas   Y cut   X aimed wires   A let go";

struct App {
    game: Game,
    world: World,
    font: Font,
    far: Vec<Pick>,
    near: Vec<Pick>,
    giants: Vec<Giant>,
    vp: Mat4,
    text: String,
}

static mut APP: Option<App> = None;

fn app() -> &'static mut App {
    // The host calls from one thread, after `mh_init` succeeded.
    unsafe { (*core::ptr::addr_of_mut!(APP)).as_mut().unwrap_unchecked() }
}

/// Sizes the host's buffers must have; `mh_sizes` returns them so a mismatch with `core.h` fails at start.
#[repr(C)]
pub struct MhSizes {
    pub bones: u32,
    pub cloak_verts: u32,
    pub cloak_indices: u32,
    pub rope_verts: u32,
    pub disc_verts: u32,
    pub fan: u32,
    pub fan_indices: u32,
    pub sky_verts: u32,
    pub sky_indices: u32,
    pub sun_verts: u32,
    pub scene_bytes: u32,
    pub mesh_bytes: u32,
    pub vertex_bytes: u32,
}

#[no_mangle]
pub extern "C" fn mh_sizes(out: *mut MhSizes) {
    unsafe {
        *out = MhSizes {
            bones: BONES as u32,
            cloak_verts: CLOAK_N as u32,
            cloak_indices: actors::CLOAK_INDICES as u32,
            rope_verts: actors::ROPE_VERTS as u32,
            disc_verts: actors::DISC_VERTS as u32,
            fan: actors::FAN as u32,
            fan_indices: actors::FAN_INDICES as u32,
            sky_verts: actors::SKY_VERTS as u32,
            sky_indices: actors::SKY_INDICES as u32,
            sun_verts: actors::SUN_VERTS as u32,
            scene_bytes: core::mem::size_of::<HandScene>() as u32,
            mesh_bytes: core::mem::size_of::<HandMesh>() as u32,
            vertex_bytes: core::mem::size_of::<PicaVertex>() as u32,
        };
    }
}

fn fail(text: &'static str) -> *const c_char {
    // Every message below ends in a NUL.
    text.as_ptr().cast()
}

/// Builds the game from the pack's sections. Returns null, or a message.
///
/// # Safety
/// The pointers are valid for their lengths. The host may free them afterwards.
#[no_mangle]
pub unsafe extern "C" fn mh_init(scene: *const HandScene, meshes: *const HandMesh, mesh_count: u32, simw: *const u8, simw_len: u32, font: *const u8, font_len: u32) -> *const c_char {
    let scene = Scene::new(core::ptr::read_unaligned(scene));
    let recs: Vec<HandMesh> = (0..mesh_count as usize).map(|i| core::ptr::read_unaligned(meshes.add(i))).collect();
    let per = libm_round(scene.h.super_cell / scene.h.cell).max(1) as i32;
    let world = World::new(recs, per, false, core::mem::size_of::<PicaVertex>() as u32);
    let sim = match maneuver_sim::worldfile::load(core::slice::from_raw_parts(simw, simw_len as usize)) {
        Ok(s) => s,
        Err(_) => return fail("the pack's simulation world does not load\0"),
    };
    let font = match Font::parse(core::slice::from_raw_parts(font, font_len as usize)) {
        Ok((f, _)) => f,
        Err(_) => return fail("the pack's font does not load\0"),
    };
    let game = Game::new(sim, scene, HELP);
    APP = Some(App { game, world, font, far: Vec::with_capacity(1024), near: Vec::with_capacity(256), giants: Vec::with_capacity(32), vp: mat::IDENTITY, text: String::with_capacity(4096) });
    core::ptr::null()
}

fn libm_round(x: f32) -> i32 {
    (x + 0.5) as i32
}

/// Offset of the texture bytes in the `FONT` section, and the atlas size.
#[no_mangle]
pub unsafe extern "C" fn mh_font(font: *const u8, font_len: u32, width: *mut u32, height: *mut u32) -> u32 {
    match Font::parse(core::slice::from_raw_parts(font, font_len as usize)) {
        Ok((f, at)) => {
            *width = f.width;
            *height = f.height;
            at as u32
        }
        Err(_) => 0,
    }
}

/// Static geometry: the sky dome, the sun's discs and the index lists.
#[no_mangle]
pub unsafe extern "C" fn mh_static(radius: f32, sky_v: *mut ColorVertex, sky_i: *mut u16, sun_v: *mut ColorVertex, cloak_i: *mut u16, quad_i: *mut u16, quads: u32, fan_i: *mut u16) {
    let a = app();
    actors::sky(&a.game.scene, radius, core::slice::from_raw_parts_mut(sky_v, actors::SKY_VERTS), core::slice::from_raw_parts_mut(sky_i, actors::SKY_INDICES));
    actors::sun(&a.game.scene, radius, core::slice::from_raw_parts_mut(sun_v, actors::SUN_VERTS));
    actors::cloak_indices(core::slice::from_raw_parts_mut(cloak_i, actors::CLOAK_INDICES));
    actors::quad_indices(core::slice::from_raw_parts_mut(quad_i, quads as usize * 6), quads as usize);
    actors::fan_indices(core::slice::from_raw_parts_mut(fan_i, actors::FAN_INDICES));
}

#[no_mangle]
pub unsafe extern "C" fn mh_control(text: *const u8, len: u32) {
    if let Ok(t) = core::str::from_utf8(core::slice::from_raw_parts(text, len as usize)) {
        app().game.control(t);
    }
}

#[no_mangle]
pub unsafe extern "C" fn mh_step(pad: *const Pad, ticks: u32) -> u32 {
    app().game.step(&*pad, ticks)
}

/// The frame's camera and switches.
#[repr(C)]
pub struct MhView {
    pub eye: [f32; 3],
    pub fov: f32,
    pub look: [f32; 3],
    pub roll: f32,
    pub fog: [f32; 3],
    /// How strongly to show speed, 0..1.
    pub rush: f32,
    pub show_character: u32,
    pub world: u32,
    pub actors: u32,
    pub repeat: u32,
    pub option: i32,
    pub far_count: u32,
    pub near_count: u32,
    pub giant_count: u32,
    /// Sun visibility at the character.
    pub vis: f32,
}

/// Chooses this frame's camera, meshes and giants.
#[no_mangle]
pub unsafe extern "C" fn mh_view(out: *mut MhView) {
    let a = app();
    let cam = a.game.camera();
    a.vp = a.game.view_proj(&cam);
    let planes = mat::planes(&a.vp);
    a.far.clear();
    a.near.clear();
    a.giants.clear();
    let set = &a.game.set;
    if set.world {
        a.world.pick(&planes, cam.eye, set.lod_near, set.lod_mid, set.lod_far, &|_| true, &mut a.far, &mut a.near);
    }
    // In the pack's order, meshes that share a vertex base are adjacent: the host merges them into runs.
    a.far.sort_unstable_by_key(|p| p.mesh);
    a.near.sort_unstable_by_key(|p| p.mesh);
    if set.actors {
        a.game.actors.giants(&a.game.sim, &a.game.scene, &planes, cam.eye, &mut a.giants);
    }
    *out = MhView {
        eye: [cam.eye.x, cam.eye.y, cam.eye.z],
        fov: cam.fov,
        look: [cam.look.x, cam.look.y, cam.look.z],
        roll: cam.roll,
        fog: a.game.scene.fog_srgb(),
        rush: a.game.rush(),
        show_character: a.game.show_character() as u32,
        world: set.world as u32,
        actors: set.actors as u32,
        repeat: set.repeat,
        option: set.option,
        far_count: a.far.len() as u32,
        near_count: a.near.len() as u32,
        giant_count: a.giants.len() as u32,
        vis: a.game.actors.vis,
    };
}

/// The picks of `mh_view`: list 0 is beyond the near distance, list 1 within it.
#[no_mangle]
pub extern "C" fn mh_picks(list: u32) -> *const Pick {
    let a = app();
    if list == 0 {
        a.far.as_ptr()
    } else {
        a.near.as_ptr()
    }
}

#[no_mangle]
pub extern "C" fn mh_giants() -> *const Giant {
    app().giants.as_ptr()
}

/// Three rows per bone of giant `index` (`BONES × 12` floats) and its light table (16 floats).
#[no_mangle]
pub unsafe extern "C" fn mh_giant_pose(index: u32, sink: f32, vis: f32, rows: *mut f32, light: *mut f32) {
    let a = app();
    let skin = a.game.sim.titan_skin(index as usize);
    actors::bone_rows(&skin, sink, &mut *(rows as *mut [f32; BONES * 12]));
    *(light as *mut [f32; 16]) = a.game.scene.light(vis);
}

#[no_mangle]
pub unsafe extern "C" fn mh_scout_pose(rows: *mut f32, light: *mut f32) {
    let a = app();
    actors::bone_rows(&a.game.sim.pose.skin, 0.0, &mut *(rows as *mut [f32; BONES * 12]));
    *(light as *mut [f32; 16]) = a.game.scene.light(a.game.actors.vis);
}

/// Writes the cloak, the wires and the soft discs for this frame.
#[no_mangle]
pub unsafe extern "C" fn mh_actors(ticks: u32, cloak: *mut ColorVertex, rope: *mut ColorVertex, disc: *mut ColorVertex, out: *mut Frame) {
    let a = app();
    let eye = a.game.camera().eye;
    let g = &mut a.game;
    *out = g.actors.update(&g.sim, &g.scene, eye, ticks, core::slice::from_raw_parts_mut(cloak, CLOAK_N), core::slice::from_raw_parts_mut(rope, actors::ROPE_VERTS), core::slice::from_raw_parts_mut(disc, actors::DISC_VERTS));
}

/// Batches the interface into `verts` (four per quad). Texture coordinates leave as `texel × scale + offset`.
#[no_mangle]
pub unsafe extern "C" fn mh_hud(verts: *mut HudVertex, cap: u32, uv_scale: *const f32, uv_offset: *const f32, perf: *const Perf) -> u32 {
    let a = app();
    let mut hud = Hud::new(&a.font, core::slice::from_raw_parts_mut(verts, cap as usize), [*uv_scale, *uv_scale.add(1)], [*uv_offset, *uv_offset.add(1)]);
    a.game.draw_hud(&mut hud, &a.vp, &*perf);
    hud.quads as u32
}

#[no_mangle]
pub unsafe extern "C" fn mh_audio(out: *mut i16, frames: u32, rate: f32) {
    app().game.synth.render(core::slice::from_raw_parts_mut(out, frames as usize * 2), rate);
}

/// The status record as JSON, NUL-terminated. `extra` is the host's members without braces.
#[no_mangle]
pub unsafe extern "C" fn mh_status(out: *mut u8, cap: u32, perf: *const Perf, extra: *const u8, extra_len: u32) -> u32 {
    let a = app();
    a.text.clear();
    let extra = core::str::from_utf8(core::slice::from_raw_parts(extra, extra_len as usize)).unwrap_or("");
    let mut text = core::mem::take(&mut a.text);
    a.game.status(&mut text, "3ds", &*perf, extra);
    let n = text.len().min(cap as usize - 1);
    core::ptr::copy_nonoverlapping(text.as_ptr(), out, n);
    *out.add(n) = 0;
    a.text = text;
    n as u32
}

/// For the lower screen: the player's position and heading, and each giant's position and state.
#[repr(C)]
pub struct MhMap {
    pub player: [f32; 3],
    pub yaw: f32,
    pub speed: f32,
    pub gas: f32,
    pub kills: u32,
    pub giants: u32,
    pub ticks: u32,
    pub auto_on: u32,
    pub bounds: f32,
    pub done: u32,
}

#[no_mangle]
pub unsafe extern "C" fn mh_map(out: *mut MhMap, giant_xz_alive: *mut f32, cap: u32) -> u32 {
    let g = &app().game;
    let s = &g.sim;
    *out = MhMap {
        player: [s.p.pos.x, s.p.pos.y, s.p.pos.z],
        yaw: s.cam.yaw,
        speed: s.speed(),
        gas: s.p.gas / maneuver_sim::sim::tune::GAS_MAX,
        kills: s.run.kills,
        giants: s.dummies.len() as u32,
        ticks: s.run.ticks,
        auto_on: g.set.auto as u32,
        bounds: s.bounds,
        done: s.run.done as u32,
    };
    let n = s.dummies.len().min(cap as usize);
    for (i, d) in s.dummies.iter().take(n).enumerate() {
        *giant_xz_alive.add(i * 3) = d.pos.x;
        *giant_xz_alive.add(i * 3 + 1) = d.pos.z;
        *giant_xz_alive.add(i * 3 + 2) = d.alive as u32 as f32;
    }
    n as u32
}
