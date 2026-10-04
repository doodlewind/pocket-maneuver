//! Pocket Maneuver on PS Vita.
//!
//! One scene per frame at 960 × 544: the sky, the baked world (one program,
//! one texture, one matrix per draw), the moving models, then on the display
//! surface the graded frame, the marks on the world and the interface. The
//! simulation is `maneuver-sim`, the same crate the reference runs as wasm;
//! the interface is the PocketJS guest of `ui/` (`interface.rs`), and the
//! game's flow around the simulation is `maneuver_interface::Session`.
//!
//! Development loop over PocketJS's wired debug transport: the pack and the
//! interface are read from the USB share (`host0:maneuver/`),
//! `host0:maneuver/control.json` steers the run, and status receipts carry
//! frame timings under `engine`.

mod actors;
mod gpu;
mod hostfs;
mod hud;
mod interface;
mod mat;
mod paths;
mod post;
mod world;

use std::io::Read;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use actors::{Actors, Scene};
use gpu::{Gpu, Layout};
use hud::{rgba, Hud};
use maneuver_interface::{channel, pad, Command, Mode, Pad, Session, Setting};
use maneuver_pack::{self as pack, Pack};
use maneuver_sim::abi::snap;
use maneuver_sim::math::*;
use maneuver_sim::sim::btn;
use maneuver_sim::Sim;
use pocket_vita_gxm::mem::{Arena, Kind, Ring};
use pocket_vita_gxm::program::{F32, S16N, S8N, U16N, U8, U8N};
use pocket_vita_gxm::target::{Fence, Msaa};
use post::{Look, Post};
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

/// Samples per pixel of the scene target.
const DEFAULT_MSAA: u64 = 4;

/// vita2d's pool of temporary vertices, which the interface and the Devkit
/// menu draw from. A frame takes half of it, in turn (see the display scene).
const POOL_BYTES: u32 = 2 * 1024 * 1024;

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
const SKIN_V: &str = include_str!("../shaders/skin_v.cg");
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

/// What a frame needs before there is a scene to draw: the interface, the
/// wired-debug host and a count of frames shown.
struct Shell {
    ui: interface::Ui,
    dev: dev::Host,
    /// The system font, loaded when a frame has no interface to draw.
    font: *mut g::vita2d_pgf,
    frame: u32,
}

impl Shell {
    /// A frame of the interface alone, while the pack loads (`Mode::Loading`, `message` the step)
    /// or after the start failed (`Mode::Error`, `message` the reason): the guest turns, then
    /// draws. It also publishes status, so the computer sees the new process come up.
    unsafe fn frame(&mut self, mode: Mode, message: &str) {
        let state = &mut channel().state;
        state.mode = mode;
        if state.message != message {
            state.message.clear();
            state.message.push_str(message);
        }
        self.ui.turn(interface::TURN, &interface::NEUTRAL, true);
        if !self.ui.live() && self.font.is_null() {
            self.font = g::vita2d_load_default_pgf();
        }
        graphics::begin_frame(0xff14_100c);
        if self.ui.live() {
            self.ui.draw();
        } else {
            // No interface: the system font says what is going on.
            text(self.font, 48, 80, 0xffff_ffff, 1.4, "Pocket Maneuver");
            let chars: Vec<char> = message.chars().collect();
            for (i, line) in chars.chunks(90).take(4).enumerate() {
                text(self.font, 48, 130 + i as i32 * 28, 0xffd0_d0d0, 1.0, &line.iter().collect::<String>());
            }
        }
        self.dev.overlay();
        graphics::present();
        self.dev.engine = json!({"stage": if mode == Mode::Error { "error" } else { "loading" }, "message": message, "interface": {"open": channel().is_open(), "error": self.ui.error}});
        self.dev.publish(self.frame, "maneuver");
        serve(&mut self.dev, self.frame, Action::None);
        self.frame += 1;
    }

    /// The start failed: the reason stays on the screen and in the status receipt.
    unsafe fn fail(&mut self, error: String) -> ! {
        pocketjs_vita::vita_log(format_args!("maneuver: {error}"));
        loop {
            self.frame(Mode::Error, &error);
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Reads the pack in pieces, showing the interface's loading screen between them.
unsafe fn read_pack(shell: &mut Shell) -> Result<(Vec<u8>, String, String), String> {
    let mut last = String::new();
    for path in paths::candidates("world.pack") {
        let mut file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                last = format!("{path}: {e}");
                continue;
            }
        };
        let mut bytes = Vec::new();
        let mut hash = Sha256::new();
        let mut chunk = vec![0u8; 512 * 1024];
        loop {
            let n = file.read(&mut chunk).map_err(|e| format!("{path}: {e}"))?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
            hash.update(&chunk[..n]);
            shell.frame(Mode::Loading, &format!("Reading the world · {:.1} MB", bytes.len() as f32 / 1e6));
        }
        let sha: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        return Ok((bytes, path, sha));
    }
    Err(format!("no world pack ({last})"))
}

/// Copies `host0:maneuver/outbox/<name>` to `ux0:data/pocket-maneuver/` (a packaged build to
/// install from VitaShell) and records the result next to the source as `<name>.done`.
fn fetch(name: &str) {
    let result = (|| -> Result<u64, String> {
        if name.is_empty() || name.contains(['/', '\\', ':']) || name.contains("..") {
            return Err(format!("refusing file name {name:?}"));
        }
        let _ = std::fs::create_dir_all(paths::DATA);
        let mut src = std::fs::File::open(format!("{}/outbox/{name}", paths::HOST)).map_err(|e| e.to_string())?;
        let to = format!("{}/{name}", paths::DATA);
        let mut dst = std::fs::File::create(&to).map_err(|e| format!("{to}: {e}"))?;
        std::io::copy(&mut src, &mut dst).map_err(|e| e.to_string())
    })();
    let text = match result {
        Ok(n) => format!("ok {n} {}/{name}", paths::DATA),
        Err(e) => format!("error {e}"),
    };
    let _ = hostfs::write(&format!("{}/outbox/{name}.done", paths::HOST), text.as_bytes());
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
                        if let Some(name) = v["fetch"].as_str() {
                            fetch(name);
                        }
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
    /// The marks on the world: where a wire would bite, the nearest target, the streaks of speed.
    hud: bool,
    /// The statistics line, which the interface shows.
    stats: bool,
    /// The synthesizer plays.
    sound: bool,
    cull_cw: bool,
    /// Wait for the GPU after each scene and time it.
    profile: bool,
    lod_near: f32,
    lod_mid: f32,
    world: bool,
    actors: bool,
    /// Draws the world this many times, to find how much GPU time is left.
    repeat: u32,
    look: Look,
    /// A fixed camera: eye, target, vertical field of view.
    view: Option<(V3, V3, f32)>,
}

impl Settings {
    /// A switch of this device's own, from the interface's menu.
    fn option(&mut self, key: &str, on: bool) {
        match key {
            "sound" => self.sound = on,
            "bloom" => self.look.bloom = on,
            "rays" => self.look.rays = on,
            "blur" => self.look.speed = on,
            "stats" => self.stats = on,
            _ => {}
        }
    }

    /// What the player can set here, in menu order.
    fn options(&self, session: &Session) -> Vec<Setting> {
        let mut out = Vec::with_capacity(6);
        session.settings(&mut out);
        out.push(Setting::switch("sound", self.sound));
        out.push(Setting::switch("bloom", self.look.bloom));
        out.push(Setting::switch("rays", self.look.rays));
        out.push(Setting::switch("blur", self.look.speed));
        out.push(Setting::switch("stats", self.stats));
        out
    }
}

/// A control message's `press`: a mask of PocketJS button bits (the pad's own), or names.
fn press(ui: &mut interface::Ui, v: &Value) {
    if let Some(mask) = v.as_u64() {
        ui.press(mask as u32);
    }
    for name in v.as_array().into_iter().flatten().filter_map(Value::as_str) {
        ui.press(match name {
            "up" => 0x10,
            "right" => 0x20,
            "down" => 0x40,
            "left" => 0x80,
            "l" => 0x100,
            "r" => 0x200,
            "triangle" => P_TRIANGLE,
            "circle" => P_CIRCLE,
            "cross" => P_CROSS,
            "square" => P_SQUARE,
            "start" => P_START,
            "select" => P_SELECT,
            _ => continue,
        });
    }
}

fn apply_control(v: &Value, s: &mut Settings, sim: &mut Sim, session: &mut Session, ui: &mut interface::Ui) {
    let flag = |k: &str, cur: bool| v[k].as_bool().unwrap_or(cur);
    s.hud = flag("hud", s.hud);
    s.stats = flag("stats", s.stats);
    s.sound = flag("sound", s.sound);
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
    if let Some(x) = v["repeat"].as_u64() {
        s.repeat = (x as u32).clamp(1, 8);
    }
    // `post`: {"bloom", "rays", "speed"} switch the passes; the rest are the look's numbers.
    let post = &v["post"];
    if post.is_object() {
        let l = &mut s.look;
        l.bloom = post["bloom"].as_bool().unwrap_or(l.bloom);
        l.rays = post["rays"].as_bool().unwrap_or(l.rays);
        l.speed = post["speed"].as_bool().unwrap_or(l.speed);
        for (key, slot) in [("threshold", &mut l.threshold), ("bloomGain", &mut l.bloom_gain), ("raysGain", &mut l.rays_gain), ("vignette", &mut l.vignette), ("contrast", &mut l.contrast), ("saturation", &mut l.saturation), ("warm", &mut l.warm), ("cool", &mut l.cool)] {
            if let Some(x) = post[key].as_f64() {
                *slot = x as f32;
            }
        }
    }
    if v["reset"].as_bool() == Some(true) {
        sim.reset();
    }
    // The game's flow: `ui` asks as the interface would; `mode` sets it outright, and with
    // `auto` the autopilot plays (`{"mode":"play","auto":true}` is play flown by the autopilot,
    // for a measurement).
    let asked = match v["ui"].as_str() {
        Some("start") => Some(Command::Start),
        Some("pause") => Some(Command::Pause(true)),
        Some("resume") => Some(Command::Pause(false)),
        Some("restart") => Some(Command::Restart),
        Some("title") => Some(Command::Title),
        _ => None,
    };
    if let Some(command) = asked {
        session.command(sim, command);
    }
    if let Some(mode) = v["mode"].as_str().and_then(Mode::parse).filter(|m| !matches!(m, Mode::Loading | Mode::Error)) {
        session.mode = mode;
        session.auto = mode == Mode::Title;
    }
    session.auto = flag("auto", session.auto);
    press(ui, &v["press"]);
    let f = |a: &Value, i: usize| a.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    s.view = match (&v["view"]["pos"], &v["view"]["target"]) {
        (p, t) if p.is_array() && t.is_array() => Some((v3(f(p, 0), f(p, 1), f(p, 2)), v3(f(t, 0), f(t, 1), f(t, 2)), v["view"]["fov"].as_f64().unwrap_or(62.0) as f32)),
        _ => None,
    };
}

/// The pad as the game's flow reads it: the simulation's buttons and both sticks, and START
/// and SELECT for when no interface is on the screen.
fn session_pad(p: &input::Pad) -> Pad {
    let mut b = 0;
    for (bit, to) in [(P_L, btn::HOOK_L), (P_R, btn::HOOK_R), (P_CROSS, btn::GAS), (P_SQUARE, btn::SLASH), (P_TRIANGLE, btn::ZIP), (P_CIRCLE, btn::DROP), (P_START, pad::START), (P_SELECT, pad::SELECT)] {
        if p.buttons & bit != 0 {
            b |= to;
        }
    }
    let axis = |v: u8| (v as f32 - 127.5) / 127.5;
    Pad { buttons: b, lx: axis(p.lx), ly: -axis(p.ly), rx: axis(p.rx), ry: -axis(p.ry) }
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
        // Development builds take boot switches from the USB share: {"msaa": 0 | 2 | 4}.
        let live = cfg!(feature = "usb-debug");
        let boot: Value = if live { hostfs::read(&format!("{}/boot.json", paths::HOST), 4096).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null) } else { Value::Null };
        let samples = boot["msaa"].as_u64().unwrap_or(DEFAULT_MSAA);
        // The scene target is multisampled; the display surface it is composed onto is not.
        let msaa = match samples {
            4 => Msaa::X4,
            2 => Msaa::X2,
            _ => Msaa::None,
        };
        if let Err(error) = graphics::init_with_pool(POOL_BYTES) {
            pocketjs_vita::vita_log(format_args!("maneuver: graphics {error}"));
            return;
        }
        set_clocks();
        input::init();
        // The interface comes up first: it shows the load, and a failure.
        let mut shell = Shell { ui: interface::Ui::boot(), dev: dev::Host::new(), font: core::ptr::null_mut(), frame: 0 };
        channel().state.prefs = std::fs::read_to_string(format!("{}/{}", paths::DATA, paths::INTERFACE_FILE)).unwrap_or_default();

        // ------------------------------------------------------------------ load
        let t_load = Instant::now();
        shell.frame(Mode::Loading, "Reading the world");
        let (bytes, pack_path, pack_sha) = match read_pack(&mut shell) {
            Ok(b) => b,
            Err(e) => shell.fail(e),
        };
        let read_ms = t_load.elapsed().as_millis() as u64;
        let loaded = (|| -> Result<_, String> {
            let p = Pack::parse(&bytes)?;
            let meta: Value = serde_json::from_slice(p.section(pack::META)?).map_err(|e| e.to_string())?;
            let scene = Scene::from_meta(&meta);

            shell.frame(Mode::Loading, "Preparing programs");
            let mut gpu = Gpu::new(live)?;
            let fog = scene.fog_srgb();
            let defines = format!(
                "#define FOG_COLOR half3({:.5}, {:.5}, {:.5})\n#define FOG_DENSITY {:.7}\n#define UV_SCALE {:.1}\n#define COLOR_SCALE {:.1}\n#define BONES {}\n",
                fog[0],
                fog[1],
                fog[2],
                scene.fog_density,
                pack::UV_SCALE,
                pack::COLOR_SCALE,
                maneuver_sim::pose::BONES
            );
            let world_prog = gpu.program("world", &defines, WORLD_V, WORLD_F, &Layout { attrs: &[("aPosition", 0, U16N, 3), ("aUv", 8, S16N, 2), ("aColor", 12, U8N, 4)], stride: 16 }, msaa.gxm())?;
            let color_prog = gpu.program("color", &defines, COLOR_V, COLOR_F, &Layout { attrs: &[("aPosition", 0, F32, 3), ("aColor", 12, U8N, 4)], stride: 16 }, msaa.gxm())?;
            let skin_prog = gpu.program("skin", &defines, SKIN_V, COLOR_F, &Layout { attrs: &[("aPosition", 0, F32, 3), ("aNormal", 12, S8N, 4), ("aColor", 16, U8N, 4), ("aBones", 20, U8, 2), ("aWeights", 22, U8N, 2)], stride: 24 }, msaa.gxm())?;
            let hud_prog = gpu.program("hud", &defines, HUD_V, HUD_F, &Layout { attrs: &[("aPosition", 0, F32, 2), ("aUv", 8, F32, 2), ("aColor", 16, U8N, 4)], stride: 20 }, 0)?;
            let mut vram = Arena::new(Kind::Cdram, 16 * 1024 * 1024);
            let mut targets = Arena::new(Kind::Main, 4 * 1024 * 1024);
            let post = Post::new(&mut gpu, &mut vram, &mut targets, &defines, msaa)?;
            gpu.finish();

            shell.frame(Mode::Loading, "Uploading the world");
            let per = (scene.super_cell / scene.cell).round().max(1.0) as i32;
            let world = world::World::load(&p, &mut vram, per)?;
            let hud = Hud::load(&p, &mut vram)?;
            let sim = maneuver_sim::worldfile::load(p.section(pack::SIMW)?).map_err(|e| e.to_string())?;
            let actors = Actors::load(&p, &scene, &sim)?;
            Ok((meta, scene, gpu, world_prog, color_prog, skin_prog, hud_prog, world, hud, sim, actors, vram, post, targets))
        })();
        let (meta, scene, gpu, world_prog, color_prog, skin_prog, hud_prog, world, mut hud, mut sim, mut actors, vram, mut post, _targets) = match loaded {
            Ok(x) => x,
            Err(e) => shell.fail(e),
        };
        let pack_bytes = bytes.len();
        drop(bytes);
        let load_ms = t_load.elapsed().as_millis() as u64;

        let ring_bytes = actors.frame_bytes() + Hud::VERTEX_BYTES + 4096;
        let mut ring = match Ring::new(ring_bytes, 2) {
            Ok(r) => r,
            Err(e) => shell.fail(e),
        };
        let mut fence = Fence::new(0, 2);
        let control = if live { control_watcher() } else { mpsc::channel().1 };
        let mut set = Settings { hud: true, stats: live, sound: true, cull_cw: true, profile: false, lod_near: scene.lod_near, lod_mid: scene.lod_mid, world: true, actors: true, repeat: 1, look: Look::DEFAULT, view: None };
        // Title, play, pause and the finished run.
        let mut session = Session::new();
        channel().state.message.clear();

        // Sound: the synthesizer renders at 22.05 kHz; the host module doubles it for the port.
        let mut synth = maneuver_sim::audio::Synth::new();
        let sound = pocketjs_vita::audio::start(22050);
        let mut pcm = vec![0i16; 2048];

        let ctx = g::vita2d_get_context();
        let mut timing = Timing { ms: [16.7; Timing::N], at: 0, late: 0, frames: 0 };
        let mut last = Instant::now();
        let mut last_vcount = sceDisplayGetVcount();
        let (mut sim_ms, mut build_ms, mut draw_ms, mut gpu_ms, mut wait_ms, mut ui_ms) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let mut wstats = world::Stats::default();
        let mut actor_stats = (0u32, 0u32);
        let mut snapshot = vec![0.0f32; snap::LEN];
        let mut clock_tick = 0u32;
        let mut frame_no = shell.frame;
        // The last loading frame drew from the start of vita2d's pool: it leaves the GPU before
        // the first frame of the loop writes there.
        g::vita2d_wait_rendering_done();

        loop {
            // -------------------------------------------------------------- input and simulation
            let raw = input::read();
            let (buttons, action) = shell.dev.menu.input(raw.buttons);
            // While the Devkit menu is up the pad is its own.
            let menu = shell.dev.menu.visible;
            let pad = if menu { interface::NEUTRAL } else { input::Pad { buttons, ..raw } };
            while let Ok(v) = control.try_recv() {
                apply_control(&v, &mut set, &mut sim, &mut session, &mut shell.ui);
            }
            // What the interface asked for on its last turn. A setting of this device's own and
            // the preferences to store come back from the session.
            while let Some(command) = channel().next() {
                match session.command(&mut sim, command) {
                    Some(Command::Option { key, value }) => set.option(&key, value != 0),
                    Some(Command::Prefs(text)) => {
                        paths::write_text(paths::INTERFACE_FILE, &text);
                        channel().state.prefs = text;
                    }
                    _ => {}
                }
            }

            // One tick per display refresh: a late frame catches up with two. Paused, none runs.
            let vcount = sceDisplayGetVcount();
            let vblanks = (vcount.wrapping_sub(last_vcount)).clamp(1, 3);
            last_vcount = vcount;
            let t0 = Instant::now();
            session.run(&mut sim, &session_pad(&pad), vblanks as u32, |sim| synth.control(sim, sim.events));
            sim_ms = sim_ms * 0.9 + t0.elapsed().as_secs_f32() * 100.0;

            // Keep about three output blocks queued (1536 frames at 22.05 kHz, 70 ms): the
            // synthesizer's, or silence while the game is paused or the sound is off.
            if sound {
                let queued = 32 * 1024 - pocketjs_vita::audio::free_frames();
                let want = 1536usize.saturating_sub(queued).min(1024);
                if want > 0 {
                    if set.sound && !session.paused() {
                        synth.render(&mut pcm[..want * 2], 22050.0);
                    } else {
                        pcm[..want * 2].fill(0);
                    }
                    pocketjs_vita::audio::push(&pcm[..want * 2], 2);
                }
            }

            // -------------------------------------------------------------- the interface
            // It is shown the run and the settings as they now are, then takes its turn.
            let tu = Instant::now();
            {
                let state = &mut channel().state;
                session.publish(&sim, state);
                let options = set.options(&session);
                if state.options != options {
                    state.options = options;
                }
                if !set.stats {
                    state.stats.clear();
                } else if frame_no % 30 == 0 {
                    state.stats = format!("{:.1} fps · {:.1} ms · late {} · {} draws · {}k tris", 1000.0 / timing.avg().max(0.1), timing.avg(), timing.late, wstats.draws + actor_stats.0, (wstats.tris + actor_stats.1) / 1000);
                }
            }
            shell.ui.turn(vblanks as f32 / 60.0, &pad, menu);
            ui_ms = ui_ms * 0.9 + tu.elapsed().as_secs_f32() * 100.0;

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
            // The marks belong to play, and to a camera that follows the player.
            if set.hud && session.mode == Mode::Play && set.view.is_none() {
                draw_marks(&mut hud, &sim, &vp);
            }
            build_ms = build_ms * 0.9 + t1.elapsed().as_secs_f32() * 100.0;

            // -------------------------------------------------------------- the scene
            let t2 = Instant::now();
            let mut scene_error = post.begin_scene(ctx).err();
            actors.draw_sky(ctx, &color_prog, &vp, eye);
            if set.world {
                world_prog.bind(ctx, false);
                gpu::state_opaque(ctx, set.cull_cw);
                g::sceGxmSetFragmentTexture(ctx, 0, &world.atlas.gxm);
                for _ in 0..set.repeat {
                    wstats = world.draw(ctx, &world_prog, &vp, eye, set.lod_near, set.lod_mid);
                }
            }
            if let (Some(f), true) = (&aframe, set.actors) {
                // Inside the camera's near range the character would fill the frame.
                let show = set.view.is_some() || (sim.cam.pos - sim.p.pos).len() > 1.7;
                actor_stats = actors.draw_skinned(ctx, &skin_prog, &vp, &sim, &scene, eye, set.cull_cw, show);
                actors.draw_cloth(ctx, &color_prog, &vp, f, scene.fog_density, show);
                actors.draw_blend(ctx, &color_prog, &vp, f, scene.fog_density);
            }
            if scene_error.is_none() {
                scene_error = post.finish_scene(ctx, &set.look, &vp, eye, scene.sun_dir).err();
            }
            if let Some(e) = scene_error {
                pocketjs_vita::vita_log(format_args!("maneuver: {e}"));
            }
            // The display scene: the graded frame, the marks, the interface, the Devkit menu.
            // The last two draw from vita2d's one pool of temporary vertices. This slot's
            // frame takes its own half, last written two frames ago, which `fence.wait(slot)`
            // above saw out of the GPU: no frame overwrites vertices still being read.
            g::vita2d_pool_reset();
            if slot == 1 {
                g::vita2d_pool_malloc(POOL_BYTES / 2);
            }
            g::vita2d_start_drawing_advanced(core::ptr::null_mut(), 0);
            post.composite(ctx, &set.look, smoothstep(26.0, 58.0, sim.speed()) * if set.view.is_some() { 0.0 } else { 1.0 });
            hud.flush(ctx, &hud_prog, actors.quad_ib);
            // vita2d draws (the interface, the menu) expect its own viewport and no depth.
            g::sceGxmSetViewport(ctx, 480.0, 480.0, 272.0, -272.0, 0.5, 0.5);
            gpu::state_overlay(ctx, false);
            shell.ui.draw();
            shell.dev.overlay();
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
                shell.dev.engine = json!({
                    "stage": "running",
                    "mode": session.mode.name(),
                    "interface": {"open": channel().is_open(), "error": shell.ui.error},
                    "pack": {"path": pack_path, "bytes": pack_bytes, "sha256": pack_sha, "name": meta["name"], "seed": meta["seed"], "source": meta["source"], "profile": meta["profile"]},
                    "loadMs": load_ms, "readMs": read_ms,
                    "frameMs": timing.avg(), "worstMs": timing.worst(), "late": timing.late, "frames": timing.frames,
                    "cpuMs": {"sim": sim_ms, "interface": ui_ms, "build": build_ms, "draw": draw_ms, "fenceWait": wait_ms},
                    "gpuMs": if set.profile { json!(gpu_ms) } else { Value::Null },
                    "world": {"draws": wstats.draws, "tris": wstats.tris, "near": wstats.near, "mid": wstats.mid, "far": wstats.far},
                    "actors": {"draws": actor_stats.0, "tris": actor_stats.1},
                    "settings": {"auto": session.auto, "hud": set.hud, "stats": set.stats, "sound": set.sound, "invert": session.invert, "lodNear": set.lod_near, "lodMid": set.lod_mid, "cullCw": set.cull_cw, "profile": set.profile, "world": set.world, "actors": set.actors, "repeat": set.repeat, "post": {"bloom": set.look.bloom, "rays": set.look.rays, "speed": set.look.speed}},
                    "player": {"pos": [sim.p.pos.x, sim.p.pos.y, sim.p.pos.z], "speed": sim.speed(), "gas": sim.p.gas, "tick": sim.tick, "kills": sim.run.kills, "laps": sim.auto.laps, "waypoint": sim.auto.wp},
                    "programs": {"compiled": gpu.compiled, "cached": gpu.cached},
                    "msaa": samples,
                    "memory": {"geometry": world.bytes, "vram": vram.reserved()},
                    "clockMhz": [scePowerGetArmClockFrequency(), scePowerGetGpuClockFrequency()],
                });
            }
            shell.dev.publish(frame_no, "maneuver");
            serve(&mut shell.dev, frame_no, action);
            frame_no = frame_no.wrapping_add(1);
        }
    }
}

/// The marks on the world for this frame, in display pixels: where each wire would bite, the
/// streaks of speed and the nearest target. Everything else on the screen is the interface's.
fn draw_marks(h: &mut Hud, sim: &Sim, vp: &mat::Mat4) {
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
    // Streaks from the rim toward the centre at speed.
    let fast = smoothstep(24.0, 60.0, sim.speed());
    if fast > 0.0 {
        for i in 0..18u32 {
            let seed = i.wrapping_mul(2654435761).wrapping_add((sim.tick / 3).wrapping_mul(40503));
            let a = (seed % 6283) as f32 / 1000.0;
            let r0 = 300.0 + ((seed >> 8) % 160) as f32;
            let len = (40.0 + ((seed >> 16) % 90) as f32) * (0.5 + fast);
            let (c, s) = (cos(a), sin(a) * 0.62);
            let (x0, y0) = (480.0 + c * r0, 272.0 + s * r0);
            let (x1, y1) = (480.0 + c * (r0 + len), 272.0 + s * (r0 + len));
            let (nx, ny) = (-s * 1.2, c * 1.2);
            h.poly([(x0, y0), (x1 - nx, y1 - ny), (x1 + nx, y1 + ny), (x0, y0)], rgba(255, 255, 255, (fast * 70.0) as u8));
        }
    }
    // The nearest standing target: a marker on it, or at the rim of the screen toward it.
    let mut best: Option<(f32, V3)> = None;
    for d in sim.dummies.iter().filter(|d| d.alive) {
        let dist = (d.nape - sim.p.pos).len();
        if best.map_or(true, |b| dist < b.0) {
            best = Some((dist, d.nape));
        }
    }
    if let Some((dist, nape)) = best {
        let red = rgba(255, 96, 72, 235);
        let on = mat::project(vp, nape).filter(|(x, y)| (24.0..936.0).contains(x) && (24.0..520.0).contains(y));
        let (x, y) = match on {
            Some(p) => p,
            None => {
                // Off screen: toward it, from the centre, clamped to an ellipse inside the frame.
                let to = nape - sim.cam.pos;
                let right = sim.cam.look.cross(V3::UP).norm_or(v3(1.0, 0.0, 0.0));
                let up = right.cross(sim.cam.look);
                let (dx, dy) = (to.dot(right), to.dot(up));
                let l = sqrt(dx * dx + dy * dy).max(1e-3);
                (480.0 + dx / l * 420.0, 272.0 - dy / l * 230.0)
            }
        };
        h.poly([(x, y - 9.0), (x + 9.0, y), (x, y + 9.0), (x - 9.0, y)], red);
        h.text(18, x, y + 28.0, 0.5, red, &format!("{:.0} m", dist));
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
                request.finish(Err("Pocket Maneuver's interface is read from the share when the app starts; use native to start it again".into()));
            }
        }
        None => {}
    }
}
