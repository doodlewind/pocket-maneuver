//! Pocket Maneuver on PS Vita.
//!
//! One scene per frame, straight into the display surface at 960 × 544: the
//! sky, the baked world (one program, one texture, one matrix per draw), the
//! moving models, the interface. The simulation is `maneuver-sim`, the same
//! crate the reference runs as wasm.
//!
//! Development loop over PocketJS's wired debug transport: the pack is read
//! from the USB share (`host0:maneuver/world.pack`), `host0:maneuver/control.json`
//! steers the run, and status receipts carry frame timings under `engine`.

mod actors;
mod gpu;
mod hostfs;
mod hud;
mod mat;
mod paths;
mod world;

use std::io::Read;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use actors::{Actors, Scene};
use gpu::{Gpu, Layout};
use hud::{rgba, Hud};
use maneuver_pack::{self as pack, Pack};
use maneuver_sim::abi::snap;
use maneuver_sim::math::*;
use maneuver_sim::sim::{btn, ev, tune, Input};
use maneuver_sim::Sim;
use pocket_vita_gxm::mem::{Arena, Kind, Ring};
use pocket_vita_gxm::program::{F32, S16N, U16N, U8N};
use pocket_vita_gxm::target::Fence;
use pocketjs_vita::{dev, dev_protocol::Op, devmenu::Action, graphics, input};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use vita2d_sys as g;

#[no_mangle]
#[used]
pub static sceUserMainThreadStackSize: u32 = 1024 * 1024;

#[no_mangle]
#[used]
pub static _newlib_heap_size_user: u32 = 128 * 1024 * 1024;

extern "C" {
    fn scePowerSetArmClockFrequency(freq: i32) -> i32;
    fn scePowerSetBusClockFrequency(freq: i32) -> i32;
    fn scePowerSetGpuClockFrequency(freq: i32) -> i32;
    fn scePowerSetGpuXbarClockFrequency(freq: i32) -> i32;
    fn scePowerGetArmClockFrequency() -> i32;
    fn scePowerGetGpuClockFrequency() -> i32;
    fn sceDisplayGetVcount() -> i32;
}

/// ARM, bus, GPU and GPU crossbar clocks (MHz) the frame budget assumes.
const CLOCKS: [i32; 4] = [444, 222, 222, 166];

unsafe fn set_clocks() {
    scePowerSetArmClockFrequency(CLOCKS[0]);
    scePowerSetBusClockFrequency(CLOCKS[1]);
    scePowerSetGpuClockFrequency(CLOCKS[2]);
    scePowerSetGpuXbarClockFrequency(CLOCKS[3]);
}

const WORLD_V: &str = include_str!("../shaders/world_v.cg");
const WORLD_F: &str = include_str!("../shaders/world_f.cg");
const COLOR_V: &str = include_str!("../shaders/color_v.cg");
const COLOR_F: &str = include_str!("../shaders/color_f.cg");
const HUD_V: &str = include_str!("../shaders/hud_v.cg");
const HUD_F: &str = include_str!("../shaders/hud_f.cg");

// Vita controller bits.
const P_SELECT: u32 = 0x1;
const P_START: u32 = 0x8;
const P_L: u32 = 0x100 | 0x400;
const P_R: u32 = 0x200 | 0x800;
const P_TRIANGLE: u32 = 0x1000;
const P_CIRCLE: u32 = 0x2000;
const P_CROSS: u32 = 0x4000;
const P_SQUARE: u32 = 0x8000;

unsafe fn text(font: *mut g::vita2d_pgf, x: i32, y: i32, color: u32, scale: f32, s: &str) {
    let c = std::ffi::CString::new(s.replace('\0', " ")).unwrap();
    g::vita2d_pgf_draw_text(font, x, y, color, scale, c.as_ptr());
}

/// A frame of the loading screen; it also publishes status, so the computer sees the new process come up.
unsafe fn loading(font: *mut g::vita2d_pgf, dev: &mut dev::Host, frame: &mut u32, lines: &[String]) {
    graphics::begin_frame(0xff14_100c);
    text(font, 48, 80, 0xffff_ffff, 1.4, "Pocket Maneuver");
    for (i, l) in lines.iter().enumerate() {
        text(font, 48, 130 + i as i32 * 28, 0xffd0_d0d0, 1.0, l);
    }
    dev.overlay();
    graphics::present();
    dev.engine = json!({"stage": "loading", "lines": lines});
    dev.publish(*frame, "maneuver");
    serve(dev, *frame, Action::None);
    *frame += 1;
}

/// Reads the pack in pieces, drawing the loading screen between them.
unsafe fn read_pack(font: *mut g::vita2d_pgf, dev: &mut dev::Host, frame: &mut u32) -> Result<(Vec<u8>, &'static str, String), String> {
    let mut last = String::new();
    for path in paths::PACKS {
        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                last = format!("{path}: {e}");
                continue;
            }
        };
        let mut bytes = Vec::new();
        let mut hash = Sha256::new();
        let mut chunk = vec![0u8; 512 * 1024];
        let t = Instant::now();
        loop {
            let n = file.read(&mut chunk).map_err(|e| format!("{path}: {e}"))?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
            hash.update(&chunk[..n]);
            let secs = t.elapsed().as_secs_f32().max(0.001);
            loading(font, dev, frame, &[format!("Reading the world: {:.1} MB", bytes.len() as f32 / 1e6), format!("{:.2} MB/s from {path}", bytes.len() as f32 / 1e6 / secs)]);
        }
        let sha: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        return Ok((bytes, path, sha));
    }
    Err(format!("no world pack ({last})"))
}

/// Remote control: `host0:maneuver/control.json`, polled off the render thread.
fn control_watcher() -> mpsc::Receiver<Value> {
    let (tx, rx) = mpsc::channel();
    let _ = std::thread::Builder::new().name("maneuver-control".into()).stack_size(256 * 1024).spawn(move || {
        // What is there at launch is left over from an earlier run: only changes after it count.
        let path = format!("{}/control.json", paths::HOST);
        let mut last = hostfs::read(&path, 64 * 1024).unwrap_or_default();
        loop {
            std::thread::sleep(Duration::from_millis(300));
            if let Some(bytes) = hostfs::read(&path, 64 * 1024) {
                if bytes != last {
                    last = bytes.clone();
                    if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                        if tx.send(v).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
    rx
}

struct Settings {
    auto: bool,
    hud: bool,
    stats: bool,
    cull_cw: bool,
    /// Wait for the GPU after each scene and time it.
    profile: bool,
    lod_near: f32,
    lod_mid: f32,
    world: bool,
    actors: bool,
    /// A fixed camera: eye, target, vertical field of view.
    view: Option<(V3, V3, f32)>,
}

fn apply_control(v: &Value, s: &mut Settings, sim: &mut Sim) {
    let flag = |k: &str, cur: bool| v[k].as_bool().unwrap_or(cur);
    s.auto = flag("auto", s.auto);
    s.hud = flag("hud", s.hud);
    s.stats = flag("stats", s.stats);
    s.cull_cw = flag("cullCw", s.cull_cw);
    s.profile = flag("profile", s.profile);
    s.world = flag("world", s.world);
    s.actors = flag("actors", s.actors);
    if let Some(x) = v["lodNear"].as_f64() {
        s.lod_near = x as f32;
    }
    if let Some(x) = v["lodMid"].as_f64() {
        s.lod_mid = x as f32;
    }
    if v["reset"].as_bool() == Some(true) {
        sim.reset();
    }
    let f = |a: &Value, i: usize| a.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    s.view = match (&v["view"]["pos"], &v["view"]["target"]) {
        (p, t) if p.is_array() && t.is_array() => Some((v3(f(p, 0), f(p, 1), f(p, 2)), v3(f(t, 0), f(t, 1), f(t, 2)), v["view"]["fov"].as_f64().unwrap_or(62.0) as f32)),
        _ => None,
    };
}

fn pad_input(pad: &input::Pad) -> Input {
    let mut b = 0;
    for (bit, to) in [(P_L, btn::HOOK_L), (P_R, btn::HOOK_R), (P_CROSS, btn::GAS), (P_SQUARE, btn::SLASH), (P_TRIANGLE, btn::ZIP), (P_CIRCLE, btn::DROP), (P_SELECT, btn::RESET)] {
        if pad.buttons & bit != 0 {
            b |= to;
        }
    }
    let axis = |v: u8| (v as f32 - 127.5) / 127.5;
    Input { buttons: b, lx: axis(pad.lx), ly: -axis(pad.ly), rx: axis(pad.rx), ry: -axis(pad.ry) }
}

/// Rolling frame statistics over the last `N` frames.
struct Timing {
    ms: [f32; Timing::N],
    at: usize,
    late: u32,
    frames: u32,
}

impl Timing {
    const N: usize = 240;
    fn push(&mut self, ms: f32, vblanks: i32) {
        self.ms[self.at] = ms;
        self.at = (self.at + 1) % Self::N;
        self.frames += 1;
        if vblanks > 1 {
            self.late += 1;
        }
    }
    fn avg(&self) -> f32 {
        self.ms.iter().sum::<f32>() / Self::N as f32
    }
    fn worst(&self) -> f32 {
        self.ms.iter().fold(0.0, |a, &b| a.max(b))
    }
}

fn main() {
    unsafe {
        if let Err(error) = graphics::init_with_pool(1024 * 1024) {
            pocketjs_vita::vita_log(format_args!("maneuver: graphics {error}"));
            return;
        }
        set_clocks();
        input::init();
        let mut dev = dev::Host::new();
        let font = g::vita2d_load_default_pgf();
        let live = cfg!(feature = "usb-debug");
        let mut frame_no = 0u32;
        let fail = |font, dev: &mut dev::Host, frame_no: &mut u32, e: String| -> ! {
            pocketjs_vita::vita_log(format_args!("maneuver: {e}"));
            loop {
                loading(font, dev, frame_no, &["Could not start.".into(), e.chars().take(90).collect(), e.chars().skip(90).take(90).collect()]);
                std::thread::sleep(Duration::from_millis(100));
            }
        };

        // ------------------------------------------------------------------ load
        let t_load = Instant::now();
        loading(font, &mut dev, &mut frame_no, &["Reading the world".into()]);
        let (bytes, pack_path, pack_sha) = match read_pack(font, &mut dev, &mut frame_no) {
            Ok(b) => b,
            Err(e) => fail(font, &mut dev, &mut frame_no, e),
        };
        let read_ms = t_load.elapsed().as_millis() as u64;
        let loaded = (|| -> Result<_, String> {
            let p = Pack::parse(&bytes)?;
            let meta: Value = serde_json::from_slice(p.section(pack::META)?).map_err(|e| e.to_string())?;
            let scene = Scene::from_meta(&meta);

            loading(font, &mut dev, &mut frame_no, &["Preparing programs".into()]);
            let mut gpu = Gpu::new(live)?;
            let fog = scene.fog_srgb();
            let defines = format!(
                "#define FOG_COLOR half3({:.5}, {:.5}, {:.5})\n#define FOG_DENSITY {:.7}\n#define UV_SCALE {:.1}\n#define COLOR_SCALE {:.1}\n",
                fog[0],
                fog[1],
                fog[2],
                scene.fog_density,
                pack::UV_SCALE,
                pack::COLOR_SCALE
            );
            let world_prog = gpu.program("world", &defines, WORLD_V, WORLD_F, &Layout { attrs: &[("aPosition", 0, U16N, 3), ("aUv", 8, S16N, 2), ("aColor", 12, U8N, 4)], stride: 16 })?;
            let color_prog = gpu.program("color", &defines, COLOR_V, COLOR_F, &Layout { attrs: &[("aPosition", 0, F32, 3), ("aColor", 12, U8N, 4)], stride: 16 })?;
            let hud_prog = gpu.program("hud", &defines, HUD_V, HUD_F, &Layout { attrs: &[("aPosition", 0, F32, 2), ("aUv", 8, F32, 2), ("aColor", 16, U8N, 4)], stride: 20 })?;
            gpu.finish();

            loading(font, &mut dev, &mut frame_no, &["Uploading the world".into()]);
            let mut vram = Arena::new(Kind::Cdram, 8 * 1024 * 1024);
            let per = (scene.super_cell / scene.cell).round().max(1.0) as i32;
            let world = world::World::load(&p, &mut vram, per)?;
            let hud = Hud::load(&p, &mut vram)?;
            let sim = maneuver_sim::worldfile::load(p.section(pack::SIMW)?).map_err(|e| e.to_string())?;
            let actors = Actors::load(&p, &scene, &sim)?;
            Ok((meta, scene, gpu, world_prog, color_prog, hud_prog, world, hud, sim, actors, vram))
        })();
        let (meta, scene, gpu, world_prog, color_prog, hud_prog, world, mut hud, mut sim, mut actors, vram) = match loaded {
            Ok(x) => x,
            Err(e) => fail(font, &mut dev, &mut frame_no, e),
        };
        let pack_bytes = bytes.len();
        drop(bytes);
        let load_ms = t_load.elapsed().as_millis() as u64;

        let ring_bytes = actors.frame_bytes(sim.dummies.len()) + Hud::VERTEX_BYTES + 4096;
        let mut ring = match Ring::new(ring_bytes, 2) {
            Ok(r) => r,
            Err(e) => fail(font, &mut dev, &mut frame_no, e),
        };
        let mut fence = Fence::new(0, 2);
        let control = if live { control_watcher() } else { mpsc::channel().1 };
        let mut set = Settings { auto: true, hud: true, stats: live, cull_cw: true, profile: false, lod_near: scene.lod_near, lod_mid: scene.lod_mid, world: true, actors: true, view: None };

        let ctx = g::vita2d_get_context();
        let mut timing = Timing { ms: [16.7; Timing::N], at: 0, late: 0, frames: 0 };
        let mut last = Instant::now();
        let mut last_vcount = sceDisplayGetVcount();
        let mut prev_buttons = u32::MAX;
        let mut note: (String, f32) = (String::new(), 0.0);
        let (mut sim_ms, mut build_ms, mut draw_ms, mut gpu_ms, mut wait_ms) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let mut wstats = world::Stats::default();
        let mut actor_stats = (0u32, 0u32);
        let mut snapshot = vec![0.0f32; snap::LEN];
        let mut clock_tick = 0u32;

        loop {
            // -------------------------------------------------------------- input and simulation
            let pad = input::read();
            let (buttons, action) = dev.menu.input(pad.buttons);
            let pressed = buttons & !prev_buttons;
            prev_buttons = buttons;
            while let Ok(v) = control.try_recv() {
                apply_control(&v, &mut set, &mut sim);
            }
            if pressed & P_START != 0 {
                set.auto = !set.auto;
                note = (if set.auto { "AUTOPILOT".into() } else { "MANUAL".into() }, 1.5);
            }
            // Any deliberate input takes over from the autopilot.
            if set.auto && buttons & (P_L | P_R | P_CROSS | P_SQUARE | P_TRIANGLE) != 0 && prev_buttons != u32::MAX {
                set.auto = false;
                note = ("MANUAL".into(), 1.5);
            }

            // One tick per display refresh: a late frame catches up with two.
            let vcount = sceDisplayGetVcount();
            let vblanks = (vcount.wrapping_sub(last_vcount)).clamp(1, 3);
            last_vcount = vcount;
            let t0 = Instant::now();
            let mut events = 0u32;
            for _ in 0..vblanks {
                let inp = if set.auto { sim.auto_input() } else if dev.menu.visible { Input::default() } else { pad_input(&input::Pad { buttons, ..pad }) };
                sim.tick(inp);
                events |= sim.events;
            }
            sim_ms = sim_ms * 0.9 + t0.elapsed().as_secs_f32() * 100.0;

            if events & ev::SLASH_HIT != 0 {
                note = (format!("CUT  {:.0} km/h", sim.run.last_cut_speed * 3.6), 1.6);
            } else if events & ev::SLASH_WEAK != 0 {
                note = ("TOO SLOW".into(), 1.0);
            }
            if events & ev::REFILL != 0 {
                note = ("GAS REFILLED".into(), 1.2);
            }
            if events & ev::RUN_DONE != 0 {
                note = ("ALL TARGETS CUT".into(), 5.0);
            }

            // -------------------------------------------------------------- camera
            let (eye, look, fov, roll) = match set.view {
                Some((pos, target, fov)) => (pos, (target - pos).norm_or(v3(0.0, 0.0, -1.0)), fov, 0.0),
                None => {
                    let k = sim.cam.shake * 0.25;
                    let t = sim.tick as f32;
                    (sim.cam.pos + v3(sin(t * 1.7) * k, sin(t * 2.3) * k, cos(t * 1.9) * k), sim.cam.look, sim.cam.fov, sim.cam.roll)
                }
            };
            let vp = mat::mul(&mat::perspective(fov, 960.0 / 544.0, scene.clip_near, scene.clip_far), &mat::view(eye, look, roll));

            // -------------------------------------------------------------- moving geometry
            let t1 = Instant::now();
            let slot = (frame_no % 2) as usize;
            let tw = Instant::now();
            // This slot's ring segment was last used two frames ago: its GPU work must be done.
            fence.wait(slot);
            wait_ms = wait_ms * 0.9 + tw.elapsed().as_secs_f32() * 100.0;
            ring.next_frame();
            let aframe = actors.update(&sim, &scene, &mut ring, eye, vblanks as u32);
            let hud_verts = ring.alloc(Hud::VERTEX_BYTES, 16);
            hud.begin(hud_verts.unwrap_or(core::ptr::null_mut()));
            if set.hud {
                note.1 -= vblanks as f32 / 60.0;
                draw_hud(&mut hud, &sim, &vp, &note, set.auto, set.view.is_some());
            }
            if set.stats {
                let line = format!("{:.1} fps  {:.1} ms (worst {:.1})  late {}  cpu {:.1}+{:.1}+{:.1}  {} draws  {}k tris", 1000.0 / timing.avg().max(0.1), timing.avg(), timing.worst(), timing.late, sim_ms, build_ms, draw_ms, wstats.draws + actor_stats.0, (wstats.tris + actor_stats.1) / 1000);
                hud.text(18, 12.0, 20.0, 0.0, rgba(255, 255, 255, 220), &line);
            }
            build_ms = build_ms * 0.9 + t1.elapsed().as_secs_f32() * 100.0;

            // -------------------------------------------------------------- the scene
            let t2 = Instant::now();
            g::vita2d_pool_reset();
            g::vita2d_start_drawing_advanced(core::ptr::null_mut(), 0);
            g::sceGxmSetViewport(ctx, 480.0, 480.0, 272.0, -272.0, 0.0, 1.0);
            g::sceGxmSetRegionClip(ctx, g::SceGxmRegionClipMode_SCE_GXM_REGION_CLIP_OUTSIDE, 0, 0, 959, 543);
            actors.draw_sky(ctx, &color_prog, &vp, eye);
            if set.world {
                world_prog.bind(ctx, false);
                gpu::state_opaque(ctx, set.cull_cw);
                g::sceGxmSetFragmentTexture(ctx, 0, &world.atlas.gxm);
                wstats = world.draw(ctx, &world_prog, &vp, eye, set.lod_near, set.lod_mid);
            }
            if let (Some(f), true) = (&aframe, set.actors) {
                actor_stats = actors.draw_opaque(ctx, &color_prog, &vp, f, scene.fog_density, set.cull_cw);
                actors.draw_blend(ctx, &color_prog, &vp, f, scene.fog_density);
            }
            hud.flush(ctx, &hud_prog, actors.quad_ib);
            // vita2d's overlay (the debug menu) expects its own viewport and no depth.
            g::sceGxmSetViewport(ctx, 480.0, 480.0, 272.0, -272.0, 0.5, 0.5);
            gpu::state_overlay(ctx, false);
            dev.overlay();
            g::sceGxmEndScene(ctx, core::ptr::null(), fence.signal(slot));
            draw_ms = draw_ms * 0.9 + t2.elapsed().as_secs_f32() * 100.0;
            if set.profile {
                let tg = Instant::now();
                fence.wait(slot);
                gpu_ms = gpu_ms * 0.9 + tg.elapsed().as_secs_f32() * 100.0;
            }
            g::vita2d_swap_buffers();

            let now = Instant::now();
            timing.push((now - last).as_secs_f32() * 1000.0, vblanks);
            last = now;

            // -------------------------------------------------------------- status
            clock_tick += 1;
            if clock_tick % 60 == 0 {
                // The system lowers the clocks after a suspend; set them again.
                if scePowerGetArmClockFrequency() < CLOCKS[0] - 20 {
                    set_clocks();
                }
            }
            if frame_no % 20 == 0 {
                sim.snapshot(&mut snapshot);
                dev.engine = json!({
                    "stage": "running",
                    "pack": {"path": pack_path, "bytes": pack_bytes, "sha256": pack_sha, "name": meta["name"], "seed": meta["seed"], "source": meta["source"], "profile": meta["profile"]},
                    "loadMs": load_ms, "readMs": read_ms,
                    "frameMs": timing.avg(), "worstMs": timing.worst(), "late": timing.late, "frames": timing.frames,
                    "cpuMs": {"sim": sim_ms, "build": build_ms, "draw": draw_ms, "fenceWait": wait_ms},
                    "gpuMs": if set.profile { json!(gpu_ms) } else { Value::Null },
                    "world": {"draws": wstats.draws, "tris": wstats.tris, "near": wstats.near, "mid": wstats.mid, "far": wstats.far},
                    "actors": {"draws": actor_stats.0, "tris": actor_stats.1},
                    "settings": {"auto": set.auto, "lodNear": set.lod_near, "lodMid": set.lod_mid, "cullCw": set.cull_cw, "profile": set.profile, "world": set.world, "actors": set.actors},
                    "player": {"pos": [sim.p.pos.x, sim.p.pos.y, sim.p.pos.z], "speed": sim.speed(), "gas": sim.p.gas, "tick": sim.tick, "kills": sim.run.kills, "laps": sim.auto.laps, "waypoint": sim.auto.wp},
                    "programs": {"compiled": gpu.compiled, "cached": gpu.cached},
                    "memory": {"geometry": world.bytes, "vram": vram.reserved()},
                    "clockMhz": [scePowerGetArmClockFrequency(), scePowerGetGpuClockFrequency()],
                });
            }
            dev.publish(frame_no, "maneuver");
            serve(&mut dev, frame_no, action);
            frame_no = frame_no.wrapping_add(1);
        }
    }
}

fn draw_hud(h: &mut Hud, sim: &Sim, vp: &mat::Mat4, note: &(String, f32), auto: bool, fixed_view: bool) {
    let white = rgba(244, 241, 232, 255);
    let dim = rgba(244, 241, 232, 190);
    // Gas, bottom left.
    let gas = sim.p.gas / tune::GAS_MAX;
    h.text(18, 30.0, 484.0, 0.0, dim, "GAS");
    h.rect(30.0, 492.0, 224.0, 14.0, rgba(10, 14, 20, 140));
    h.frame(30.0, 492.0, 224.0, 14.0, 1.5, rgba(255, 255, 255, 130));
    let fill = if gas < 0.2 { rgba(255, 122, 60, 255) } else { rgba(233, 240, 244, 255) };
    h.rect(33.0, 495.0, 218.0 * gas, 8.0, fill);
    // Speed, bottom right.
    let kmh = format!("{:.0}", sim.speed() * 3.6);
    h.text(44, 866.0, 510.0, 1.0, white, &kmh);
    h.text(18, 874.0, 510.0, 0.0, dim, "km/h");
    // Targets and time, top right.
    let score = format!("{} / {}", sim.run.kills, sim.dummies.len());
    h.text(26, 930.0, 44.0, 1.0, white, &score);
    let t = sim.run.ticks as f32 / 60.0;
    h.text(18, 930.0, 68.0, 1.0, dim, &format!("{}:{:04.1}", (t / 60.0) as u32, t % 60.0));
    if !fixed_view {
        // Where each wire would bite, and where an aimed pair would.
        for (i, r) in sim.reticle.iter().enumerate() {
            if !r.valid {
                continue;
            }
            if let Some((x, y)) = mat::project(vp, r.point) {
                if (0.0..960.0).contains(&x) && (0.0..544.0).contains(&y) {
                    if i < 2 {
                        h.frame(x - 6.0, y - 6.0, 12.0, 12.0, 2.0, rgba(255, 255, 255, 220));
                    } else {
                        h.frame(x - 4.0, y - 4.0, 8.0, 8.0, 2.0, rgba(255, 179, 71, 230));
                    }
                }
            }
        }
    }
    if note.1 > 0.0 {
        let a = (note.1.min(0.3) / 0.3 * 255.0) as u8;
        h.text(26, 480.0, 132.0, 0.5, rgba(244, 241, 232, a), &note.0);
    }
    if auto {
        h.text(18, 480.0, 528.0, 0.5, dim, "AUTOPILOT  -  press a button to take over");
    }
}

/// Answers wired-debug requests at a frame boundary.
unsafe fn serve(dev: &mut dev::Host, frame: u32, action: Action) {
    let mut request = dev.poll();
    let op = request.as_ref().map(|r| r.command.op).or(match action {
        Action::Capture => Some(Op::Capture),
        _ => None,
    });
    match op {
        Some(Op::Status) => request.take().unwrap().finish(Ok(dev.status(frame, "maneuver"))),
        Some(Op::Menu) => {
            dev.menu.visible = !dev.menu.visible;
            request.take().unwrap().finish(Ok(json!({"menu": dev.menu.visible})));
        }
        Some(Op::Capture) => {
            if let Some(request) = request.take() {
                let _ = request.reply.try_send(dev.capture(frame));
            } else {
                dev.capture_from_menu(frame);
            }
        }
        Some(Op::Native) => {
            let request = request.take().unwrap();
            g::vita2d_wait_rendering_done();
            let result = dev::exec_native(request.native_path.as_ref().unwrap());
            request.finish(result.map(|_| json!({})));
        }
        Some(Op::Push | Op::Reload | Op::Reset) => {
            if let Some(request) = request.take() {
                request.finish(Err("Pocket Maneuver has no JS guest; use native".into()));
            }
        }
        None => {}
    }
}
