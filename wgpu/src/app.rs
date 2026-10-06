//! The shell around the renderer: a frame's steps, in the order the PS Vita's
//! loop takes them (`vita/src/main.rs`), for a tab and for the build machine
//! alike.
//!
//! The simulation is `maneuver_sim` and the game's flow around it is
//! `maneuver_interface::Session`, as on every device. The interface is the
//! PocketJS guest of `ui/`, run by the page. A frame is two calls with the
//! guest's turn between them: [`App::step`] (what the interface asked for,
//! then the simulation's ticks), the turn (the page hands it [`App::heard`]
//! and brings back what it says to [`App::say`]), and [`App::draw`], which
//! lays the interface's picture over the scene. The shell is up before the
//! world is: until [`App::fly`] the frames show the interface alone, which
//! says how much of the pack has arrived.
//!
//! The screen is not one machine's: its size, the interface's turns a second
//! and what its buttons do are a [`Shape`], and change while the game runs.

use maneuver_interface::{channel, pad, Command, Mode, Pace, Pad, Session, Setting};
use maneuver_pack::{self as pack, Pack};
use maneuver_sim::audio::Synth;
use maneuver_sim::collide::mask;
use maneuver_sim::math::*;
use maneuver_sim::sim::btn;
use maneuver_sim::Sim;
use pocket_web_wgpu::gpu::{Gpu, Screen};
use pocket_web_wgpu::overlay::Overlay;
use pocket_web_wgpu::source::Source;
use pocket_web_wgpu::task;
use pocket_web_wgpu::wgpu::TextureFormat;
use serde_json::{json, Value};

use crate::actors::{Actors, Skinned};
use crate::marks::Marks;
use crate::mat;
use crate::pack::Progress;
use crate::render::{Look, Renderer, View};
use crate::scene::Scene;
use crate::world::{self, Draw};

/// Samples a pixel of the scene, as the PS Vita draws it.
pub const SAMPLES: u32 = 4;
/// The ground of the Pocket3D title card: what a frame shows under the interface while there is no world.
const GROUND: [f32; 3] = [0.09, 0.07, 0.15];
/// Frames whose intervals the average and the worst are taken over.
const WINDOW: usize = 240;

/// PocketJS's button bits (`contracts/spec`): what a page holds of a handheld's buttons, and what the
/// interface's guest is handed.
pub mod button {
    pub const SELECT: u32 = 0x0001;
    pub const START: u32 = 0x0008;
    pub const UP: u32 = 0x0010;
    pub const RIGHT: u32 = 0x0020;
    pub const DOWN: u32 = 0x0040;
    pub const LEFT: u32 = 0x0080;
    pub const L: u32 = 0x0100;
    pub const R: u32 = 0x0200;
    /// The face buttons by their place: the PlayStation's △ ○ ✕ □, and the 3DS's X, A, B and Y.
    pub const TOP: u32 = 0x1000;
    pub const RIGHT_FACE: u32 = 0x2000;
    pub const BOTTOM: u32 = 0x4000;
    pub const LEFT_FACE: u32 = 0x8000;
}

/// What a page holds of a handheld's controls: its buttons, and its sticks in -1…1, right and up positive.
#[derive(Clone, Copy, Debug, Default)]
pub struct Held {
    pub buttons: u32,
    pub left: [f32; 2],
    pub right: [f32; 2],
}

/// A screen and what its device's own build does around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    /// The interface's turns a second: what the device's own host tells its guest (`globalThis.__simHz`).
    pub turns: u32,
    /// The numbers in flight reach the interface every this many frames (`Session::numbers_every`).
    pub numbers: u32,
    /// The camera has a stick of its own; without one the direction pad turns it.
    pub camera_stick: bool,
}

/// The screens of the handhelds the game runs on, with the turns a second each one's host gives the
/// interface (`TURN` in `vita/src/interface.rs`, `TURNS` in `psp/src/interface.rs`, `TURN` in
/// `n3ds/src/guest.c`, `TURN_HZ` in `ipod/src/main.c`) and how often its numbers are refreshed
/// (`numbers_every` in `psp/src/main.rs` and `n3ds/core/src/lib.rs`). Whatever the shape, the pack and the
/// passes are the PS Vita's.
pub const SHAPES: [Shape; 4] = [
    Shape { name: "vita", width: 960, height: 544, turns: 30, numbers: 2, camera_stick: true },
    Shape { name: "psp", width: 480, height: 272, turns: 30, numbers: 6, camera_stick: false },
    Shape { name: "3ds", width: 400, height: 240, turns: 30, numbers: 4, camera_stick: false },
    Shape { name: "ipod", width: 480, height: 320, turns: 60, numbers: 4, camera_stick: false },
];

impl Shape {
    pub fn named(name: &str) -> Option<Shape> {
        SHAPES.iter().copied().find(|s| s.name == name)
    }

    /// Sixtieths of a second in one turn of the interface.
    pub fn turn(&self) -> u32 {
        60 / self.turns.clamp(1, 60)
    }

    /// The game's pad from this handheld's controls, as its own build reads them (`session_pad` in
    /// `vita/src/main.rs`, `read_pad` in `psp/src/main.rs` and `n3ds/src/main.c`): the wires on the
    /// shoulders, the gas, the cut, the aimed pair and the dive on the face buttons by their place. The iPod
    /// touch has no pad: its stick and keys are the interface's commands.
    pub fn pad(&self, held: &Held) -> Pad {
        let on = |bit: u32, to: u32| if held.buttons & bit != 0 { to } else { 0 };
        let buttons = on(button::L, btn::HOOK_L) | on(button::R, btn::HOOK_R) | on(button::BOTTOM, btn::GAS) | on(button::LEFT_FACE, btn::SLASH) | on(button::TOP, btn::ZIP) | on(button::RIGHT_FACE, btn::DROP) | on(button::START, pad::START) | on(button::SELECT, pad::SELECT);
        let dir = |neg: u32, pos: u32| ((held.buttons & pos != 0) as i32 - (held.buttons & neg != 0) as i32) as f32 * 0.8;
        let (rx, ry) = if self.camera_stick { (held.right[0], held.right[1]) } else { (dir(button::LEFT, button::RIGHT), dir(button::DOWN, button::UP)) };
        Pad { buttons, lx: held.left[0], ly: held.left[1], rx, ry }
    }
}

/// What a development host and the interface's menu can set.
pub struct Settings {
    /// The marks on the world: where a wire would bite, the nearest target, the streaks of speed.
    pub hud: bool,
    /// The statistics line, which the interface shows.
    pub stats: bool,
    /// The synthesizer plays.
    pub sound: bool,
    pub lod_near: f32,
    pub lod_mid: f32,
    pub world: bool,
    pub actors: bool,
    pub look: Look,
    /// A fixed camera: eye, target, vertical field of view.
    pub view: Option<(V3, V3, f32)>,
    /// A camera that follows the player from a place of its own: the offset from the player, and the
    /// vertical field of view. For a film.
    pub chase: Option<(V3, f32)>,
}

impl Settings {
    /// A switch of this renderer's own, from the interface's menu (the PS Vita's list).
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

/// The world once the pack has been read: its geometry and pictures on the GPU, and the simulation in it.
pub struct World {
    renderer: Renderer,
    scene: Scene,
    cells: world::World,
    actors: Actors,
    marks: Marks,
    sim: Sim,
    meta: Value,
    bytes: usize,
}

impl World {
    /// The world of a whole pack: its programs for a screen of `format`.
    pub fn load(gpu: &Gpu, format: TextureFormat, bytes: &[u8]) -> Result<World, String> {
        let p = Pack::parse(bytes)?;
        let meta: Value = serde_json::from_slice(p.section(pack::META)?).map_err(|e| e.to_string())?;
        if meta["profile"] != "vita60" {
            return Err(format!("the pack is {} (this renderer reads the PS Vita's pack, vita60)", meta["profile"]));
        }
        let scene = Scene::from_meta(&meta);
        let sim = maneuver_sim::worldfile::load(p.section(pack::SIMW)?).map_err(|e| e.to_string())?;
        let actors = Actors::new(&scene, &sim);
        let marks = Marks::load(&p)?;
        let per = (scene.super_cell / scene.cell).round().max(1.0) as i32;
        let cells = world::World::new(p.meshes()?, per);
        let renderer = Renderer::new(gpu, format, SAMPLES, &p, &scene, &actors, &marks)?;
        Ok(World { renderer, scene, cells, actors, marks, sim, meta, bytes: bytes.len() })
    }

    /// Reads the pack from `source`, saying in `progress` how much has arrived, and loads it.
    pub async fn read(gpu: Gpu, format: TextureFormat, source: Source, progress: Progress) -> Result<World, String> {
        let bytes = crate::pack::read(&source, &progress).await?;
        World::load(&gpu, format, &bytes)
    }
}

pub struct App {
    pub gpu: Gpu,
    pub screen: Screen,
    pub shape: Shape,
    /// The interface's picture, laid over every frame.
    pub overlay: Overlay,
    /// How much of the pack has arrived, for the loading screen.
    pub progress: Progress,
    world: Option<World>,
    pub session: Session,
    pub set: Settings,
    synth: Synth,
    pace: Pace,
    /// What the guest is owed: ticks no turn of its has taken, and the buttons and the contact held at any
    /// moment since a turn was last offered.
    guest: (u32, u32, bool),
    /// What the interface asked to have kept, until the page has taken it.
    prefs: Option<String>,
    last: Option<f64>,
    /// Ticks that time has passed for and no frame has taken yet.
    owed: f32,
    /// The ticks of the frame being made, for the gas and the steam.
    ticks: u32,
    intervals: [f32; WINDOW],
    frames: u32,
    late: u32,
    began: f64,
    cpu_ms: f32,
    stats: (world::Stats, u32, u32),
    draws: Vec<Draw>,
    skinned: Vec<Skinned>,
    /// What last went wrong with a read or a frame, for the status.
    pub trouble: String,
}

impl App {
    /// The shell on a screen, with no world yet: its frames show the interface alone. One to a program: the
    /// interface's channel is one.
    pub fn open(gpu: Gpu, screen: Screen, shape: Shape) -> App {
        let overlay = Overlay::new(&gpu, screen.format);
        let mut app = App {
            gpu,
            screen,
            shape,
            overlay,
            progress: Progress::default(),
            world: None,
            session: Session::new(),
            set: Settings { hud: true, stats: false, sound: true, lod_near: 160.0, lod_mid: 560.0, world: true, actors: true, look: Look::DEFAULT, view: None, chase: None },
            synth: Synth::new(),
            pace: Pace::new(),
            guest: (shape.turn(), 0, false),
            prefs: None,
            last: None,
            owed: 0.0,
            ticks: 0,
            intervals: [1000.0 / 60.0; WINDOW],
            frames: 0,
            late: 0,
            began: 0.0,
            cpu_ms: 0.0,
            stats: Default::default(),
            draws: Vec::new(),
            skinned: Vec::new(),
            trouble: String::new(),
        };
        app.stage(Mode::Loading, "Reading the world");
        app.reshape(shape);
        app
    }

    /// The world has been read: the autopilot flies the route behind the title.
    pub fn fly(&mut self, world: World) {
        (self.set.lod_near, self.set.lod_mid) = (world.scene.lod_near, world.scene.lod_mid);
        self.world = Some(world);
        unsafe { channel() }.state.message.clear();
        self.session.mode = Mode::Title;
        self.session.auto = true;
    }

    /// Whether the world is there.
    pub fn flies(&self) -> bool {
        self.world.is_some()
    }

    /// Another screen from the next frame on.
    pub fn reshape(&mut self, shape: Shape) {
        if (self.screen.width, self.screen.height) != (shape.width, shape.height) {
            self.screen.resize(&self.gpu, shape.width, shape.height, 1);
        }
        self.shape = shape;
        self.session.numbers_every = shape.numbers;
    }

    fn stage(&mut self, mode: Mode, message: &str) {
        let state = &mut unsafe { channel() }.state;
        state.mode = mode;
        if state.message != message {
            state.message.clear();
            state.message.push_str(message);
        }
    }

    /// The start failed: the interface says why.
    pub fn fail(&mut self, why: &str) {
        self.trouble = why.into();
        self.stage(Mode::Error, why);
    }

    /// Words for the flow and the renderer, as a development host sends them to a handheld: `mode=title|play|
    /// paused|results`, `auto=0|1`, `ui=start|pause|resume|restart|title`, `reset=1`, `hud= stats= sound=
    /// world= actors= bloom= rays= blur=` (0 or 1), `lodNear= lodMid=` (metres), `view=px,py,pz,tx,ty,tz,fov`
    /// or `view=off` (a fixed camera), `chase=x,y,z,fov` or `chase=off` (a camera that keeps this offset
    /// from the player and looks at it), `skip=N` (N ticks of the simulation at once).
    pub fn control(&mut self, words: &str) {
        for word in words.split_whitespace() {
            let Some((key, value)) = word.split_once('=') else { continue };
            let on = value != "0";
            let numbers: Vec<f32> = value.split(',').filter_map(|n| n.parse().ok()).collect();
            match key {
                "hud" => self.set.hud = on,
                "world" => self.set.world = on,
                "actors" => self.set.actors = on,
                "sound" | "bloom" | "rays" | "blur" | "stats" => self.set.option(key, on),
                "lodNear" => self.set.lod_near = numbers.first().copied().unwrap_or(self.set.lod_near),
                "lodMid" => self.set.lod_mid = numbers.first().copied().unwrap_or(self.set.lod_mid),
                "view" => self.set.view = (numbers.len() >= 6).then(|| (v3(numbers[0], numbers[1], numbers[2]), v3(numbers[3], numbers[4], numbers[5]), numbers.get(6).copied().unwrap_or(62.0))),
                "chase" => self.set.chase = (numbers.len() >= 3).then(|| (v3(numbers[0], numbers[1], numbers[2]), numbers.get(3).copied().unwrap_or(62.0))),
                "auto" => self.session.auto = on,
                "mode" => {
                    if let Some(mode) = Mode::parse(value).filter(|m| !matches!(m, Mode::Loading | Mode::Error)) {
                        self.session.mode = mode;
                        self.session.auto = mode == Mode::Title;
                    }
                }
                _ => {}
            }
            let Some(w) = &mut self.world else { continue };
            match (key, value) {
                ("reset", _) if on => w.sim.reset(),
                ("ui", "start") => drop(self.session.command(&mut w.sim, Command::Start)),
                ("ui", "pause") => drop(self.session.command(&mut w.sim, Command::Pause(true))),
                ("ui", "resume") => drop(self.session.command(&mut w.sim, Command::Pause(false))),
                ("ui", "restart") => drop(self.session.command(&mut w.sim, Command::Restart)),
                ("ui", "title") => drop(self.session.command(&mut w.sim, Command::Title)),
                ("skip", _) => {
                    for _ in 0..numbers.first().copied().unwrap_or(0.0) as u32 {
                        let input = w.sim.auto_input();
                        w.sim.tick(input);
                    }
                }
                _ => {}
            }
        }
    }

    // ---- the interface: a PocketJS guest the page runs, heard and answered through the channel

    /// A guest has opened the interface's channel: it is told the whole state on its next turn. A guest that
    /// replaces another (another screen's presentation) opens it again.
    pub fn interface_opened(&mut self) {
        unsafe { channel() }.open("pocket.overlay");
        // (its first turn is this frame's, and nothing held before it was there is its to hear)
        self.guest = (self.shape.turn(), 0, false);
        self.pace = Pace::new();
        self.session.idle = false;
    }

    /// The guest went with its realm; until another opens the channel the pad keeps the flow.
    pub fn interface_closed(&mut self) {
        unsafe { channel() }.close();
    }

    /// The guest's turn in this frame, once a frame after [`App::step`]: the sixtieths of a second it is for
    /// and the buttons it is handed (PocketJS's bits, held at any moment since a turn was last offered), or
    /// `None` for a frame without one. `touching`: a contact is on a surface the guest draws, or has just
    /// left.
    ///
    /// A turn is offered `shape.turns` times a second, as the device's own host offers it (`Ui::turn` in
    /// `vita/src/interface.rs`), and is always that long; it is taken when it is worth its cost
    /// (`maneuver_interface::Pace`). No more than two are owed: a guest that fell behind does not run after
    /// the time.
    pub fn guest_due(&mut self, touching: bool) -> Option<(u32, u32)> {
        let turn = self.shape.turn();
        let (owed, buttons, touched) = &mut self.guest;
        *touched |= touching;
        *owed = (*owed).min(2 * turn);
        if *owed < turn {
            return None;
        }
        *owed -= turn;
        let (buttons, touched) = (core::mem::take(buttons), core::mem::take(touched));
        // (while the world is read the guest takes every turn, as on the device)
        (self.world.is_none() || self.pace.due(&self.session, buttons, touched)).then_some((turn, buttons))
    }

    /// The line of state the guest has not seen, for its turn (`ui/app/protocol.ts`).
    pub fn heard(&mut self) -> Option<String> {
        unsafe { channel() }.poll()
    }

    /// A line the guest sent: a command for the next step.
    pub fn say(&mut self, line: &str) {
        unsafe { channel() }.receive(line);
    }

    /// The settings the page kept from the last visit, for the interface.
    pub fn prefs_stored(&mut self, text: &str) {
        unsafe { channel() }.state.prefs = text.into();
    }

    /// What the interface asked to have kept since the last call.
    pub fn prefs_take(&mut self) -> Option<String> {
        self.prefs.take()
    }

    // ---- a frame

    /// The first half of a frame at `now` (milliseconds on a clock that goes forward): what the interface
    /// asked for since the last frame, then the simulation's ticks. Returns the sixtieths of a second that
    /// passed.
    pub fn step(&mut self, now: f64, held: &Held) -> u32 {
        self.began = task::now();
        let interval = self.last.map(|last| (now - last) as f32);
        self.last = Some(now);
        let ticks = self.passed(interval.unwrap_or(1000.0 / 60.0));
        if let Some(interval) = interval {
            self.intervals[self.frames as usize % WINDOW] = interval;
            // A frame is shown for one refresh; one that took half a refresh more is late.
            self.late += (self.world.is_some() && interval > 1.5 * 1000.0 / 60.0) as u32;
        }
        self.guest.0 += ticks;
        self.guest.1 |= held.buttons;
        self.ticks = 0;
        let Some(w) = &mut self.world else {
            let (read, of) = self.progress.get();
            if self.trouble.is_empty() {
                let message = if of > 0 { format!("Reading the world · {:.1} of {:.1} MB", read as f32 / 1e6, of as f32 / 1e6) } else { "Reading the world".into() };
                self.stage(Mode::Loading, &message);
            }
            return ticks;
        };
        // What the interface asked for on its last turn. A setting of this renderer's own and the
        // preferences to keep come back from the session.
        while let Some(command) = unsafe { channel() }.next() {
            match self.session.command(&mut w.sim, command) {
                Some(Command::Option { key, value }) => self.set.option(&key, value != 0),
                Some(Command::Prefs(text)) => {
                    unsafe { channel() }.state.prefs.clone_from(&text);
                    self.prefs = Some(text);
                }
                _ => {}
            }
        }
        let before = w.sim.tick;
        let synth = &mut self.synth;
        self.session.run(&mut w.sim, &self.shape.pad(held), ticks, |sim| synth.control(sim, sim.events));
        self.ticks = w.sim.tick.wrapping_sub(before);

        // The interface is shown the run and the settings as they now are.
        let state = &mut unsafe { channel() }.state;
        self.session.publish(&w.sim, state);
        let options = self.set.options(&self.session);
        if state.options != options {
            state.options = options;
        }
        if !self.set.stats {
            state.stats.clear();
        } else if self.frames % 30 == 0 {
            let average = self.intervals.iter().sum::<f32>() / WINDOW as f32;
            state.stats = format!("{:.1} fps · {:.1} ms · late {} · {} draws · {}k tris", 1000.0 / average.max(0.1), self.cpu_ms, self.late, self.stats.0.draws + self.stats.1, (self.stats.0.tris + self.stats.2) / 1000);
        }
        ticks
    }

    /// The second half: the scene, with the interface over it. Before the world is there, the interface
    /// alone over the title card's ground.
    pub fn draw(&mut self) -> Result<(), String> {
        self.frames += 1;
        let Some(w) = &mut self.world else {
            let target = self.screen.frame(&self.gpu)?;
            let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
            drop(target.pass(&mut encoder, GROUND));
            self.overlay.draw(&mut encoder, &target);
            self.gpu.queue.submit([encoder.finish()]);
            target.present();
            return Ok(());
        };
        let (sim, set) = (&w.sim, &self.set);
        // The camera: the simulation's, with its shake; or a held one.
        let fixed = set.view.is_some() || set.chase.is_some();
        let (eye, look, fov, roll) = match (set.view, set.chase) {
            (Some((pos, target, fov)), _) => (pos, (target - pos).norm_or(v3(0.0, 0.0, -1.0)), fov, 0.0),
            (None, Some((offset, fov))) => {
                // The offset is shortened where a wall stands between the player and the camera.
                let at = sim.p.pos + v3(0.0, 0.4, 0.0);
                let (length, away) = (offset.len(), offset.norm_or(v3(0.0, 0.0, 1.0)));
                let clear = sim.world.raycast(at, away, length + 0.6, mask::ALL).map_or(length, |hit| (hit.t - 0.6).clamp(1.5, length));
                (at + away * clear, -away, fov, 0.0)
            }
            (None, None) => {
                let k = sim.cam.shake * 0.25;
                let t = sim.tick as f32;
                (sim.cam.pos + v3(sin(t * 1.7) * k, sin(t * 2.3) * k, cos(t * 1.9) * k), sim.cam.look, sim.cam.fov, sim.cam.roll)
            }
        };
        let (width, height) = (self.screen.width as f32, self.screen.height as f32);
        let vp = mat::mul(&mat::perspective(fov, width / height, w.scene.clip_near, w.scene.clip_far), &mat::view(eye, look, roll));

        self.draws.clear();
        let cells = if set.world { w.cells.choose(&vp, eye, set.lod_near, set.lod_mid, &mut self.draws) } else { world::Stats::default() };
        let moving = w.actors.update(sim, &w.scene, eye, self.ticks);
        self.skinned.clear();
        // Inside the camera's near range the character would fill the frame.
        let character = set.actors && (fixed || (sim.cam.pos - sim.p.pos).len() > 1.7);
        if set.actors {
            w.actors.skinned(&vp, sim, &w.scene, eye, character, &mut self.skinned);
        }
        // The marks belong to play, and to a camera that follows the player.
        w.marks.verts.clear();
        if set.hud && self.session.mode == Mode::Play && !fixed {
            w.marks.draw(sim, &vp, width, height);
        }
        let fast = if fixed { 0.0 } else { smoothstep(26.0, 58.0, sim.speed()) };
        let hidden = crate::actors::Frame::default();
        let view = View { vp, eye, sun: w.scene.sun_dir, world: &self.draws, skinned: &self.skinned, actors: &w.actors, moving: if set.actors { moving } else { hidden }, character, marks: &w.marks, look: set.look, fast };
        let (draws, tris) = w.renderer.frame(&self.gpu, &self.screen, &self.overlay, &view)?;
        self.stats = (cells, draws, tris);
        self.cpu_ms += ((task::now() - self.began) as f32 - self.cpu_ms) * 0.1;
        Ok(())
    }

    /// A whole frame with no guest's turn in it.
    pub fn frame(&mut self, now: f64, held: &Held) -> Result<(), String> {
        self.step(now, held);
        self.draw()
    }

    /// Stereo sound for the page's output: `frames` frames at `rate` a second, from the synthesizer the
    /// simulation's ticks drive. Silence while the game is paused, while there is no world, and with the
    /// sound switched off.
    pub fn audio(&mut self, frames: usize, rate: f32) -> Vec<i16> {
        let mut pcm = vec![0i16; frames * 2];
        if self.world.is_some() && self.set.sound && !self.session.paused() {
            self.synth.render(&mut pcm, rate);
        }
        pcm
    }

    /// Ticks of the simulation (sixtieths of a second) for a frame that comes `interval` milliseconds after
    /// the last. The handhelds' displays refresh 60 times a second and a frame there is a whole number of
    /// ticks; so it is here when the time between two frames is within a twentieth of a whole number of them
    /// (a display of 59.94 a second, or of 120 with every second refresh drawn), and a late frame catches
    /// up. On any other display the part of a tick left over is owed to the next frame, so the game keeps
    /// its speed.
    fn passed(&mut self, interval: f32) -> u32 {
        // (a clock that went back, as after a measurement made ahead of it, is one frame's time)
        let interval = if interval > 0.0 { interval } else { 1000.0 / 60.0 };
        let passed = interval.min(100.0) * 0.06;
        let whole = (passed + 0.5).floor();
        if whole >= 1.0 && (passed - whole).abs() <= 0.05 * whole {
            self.owed = 0.0;
            return whole.min(3.0) as u32;
        }
        self.owed += passed;
        let ticks = self.owed.floor().min(3.0);
        self.owed = (self.owed - ticks).min(1.0);
        ticks as u32
    }

    /// The run as a JSON object.
    pub fn status(&self) -> String {
        let average = self.intervals.iter().sum::<f32>() / WINDOW as f32;
        let worst = self.intervals.iter().fold(0.0f32, |a, &b| a.max(b));
        let mut status = json!({
            "stage": if self.world.is_some() { "running" } else if self.trouble.is_empty() { "loading" } else { "error" },
            "mode": self.session.mode.name(),
            "shape": {"name": self.shape.name, "width": self.screen.width, "height": self.screen.height, "samples": SAMPLES, "turns": self.shape.turns},
            "interface": {"open": unsafe { channel() }.is_open()},
            "frames": self.frames, "frameMs": average, "worstMs": worst, "late": self.late, "cpuMs": self.cpu_ms,
            "settings": {"auto": self.session.auto, "hud": self.set.hud, "stats": self.set.stats, "sound": self.set.sound, "invert": self.session.invert, "lodNear": self.set.lod_near, "lodMid": self.set.lod_mid, "post": {"bloom": self.set.look.bloom, "rays": self.set.look.rays, "speed": self.set.look.speed}},
            "adapter": self.gpu.adapter,
            "trouble": self.trouble,
        });
        if let Some(w) = &self.world {
            let (sim, (cells, draws, tris)) = (&w.sim, &self.stats);
            status["pack"] = json!({"bytes": w.bytes, "name": w.meta["name"], "seed": w.meta["seed"], "source": w.meta["source"], "profile": w.meta["profile"]});
            status["world"] = json!({"draws": cells.draws, "tris": cells.tris, "near": cells.near, "mid": cells.mid, "far": cells.far});
            status["actors"] = json!({"draws": draws, "tris": tris});
            status["gpuBytes"] = json!(w.renderer.bytes);
            status["player"] = json!({"pos": [sim.p.pos.x, sim.p.pos.y, sim.p.pos.z], "speed": sim.speed(), "gas": sim.p.gas, "tick": sim.tick, "kills": sim.run.kills, "laps": sim.auto.laps, "waypoint": sim.auto.wp, "wires": [sim.p.hooks[0].state, sim.p.hooks[1].state]});
            status["camera"] = json!({"pos": [sim.cam.pos.x, sim.cam.pos.y, sim.cam.pos.z], "look": [sim.cam.look.x, sim.cam.look.y, sim.cam.look.z], "yaw": sim.cam.yaw});
            status["giants"] = json!(sim.dummies.iter().map(|d| json!({"pos": [d.pos.x, d.pos.y, d.pos.z], "height": d.height, "alive": d.alive, "yaw": d.yaw})).collect::<Vec<_>>());
        }
        status.to_string()
    }
}
