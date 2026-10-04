//! The loop around the simulation that every handheld shares: the game's
//! flow and what the interface is shown of it (`maneuver-interface`), the
//! camera, the marks drawn on the world, remote settings and the status
//! record.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use maneuver_interface::{channel, Command, Mode, Session, Setting};
use maneuver_sim::audio::Synth;
use maneuver_sim::math::*;
use maneuver_sim::Sim;

use maneuver_sim::pose::BONES;

use crate::actors::Actors;
use crate::hud::{rgba, Hud};
use crate::mat::{self, Mat4};
use crate::scene::Scene;

pub use maneuver_interface::{pad, Pad};

pub struct Settings {
    /// The marks on the world: where a wire would bite, the nearest target, the streaks of speed.
    pub hud: bool,
    /// The device plays the synthesizer.
    pub sound: bool,
    pub stats: bool,
    pub world: bool,
    pub actors: bool,
    pub lod_near: f32,
    pub lod_mid: f32,
    pub lod_far: f32,
    /// Draws the world this many times, to find how much time is left.
    pub repeat: u32,
    /// A fixed camera: eye, target, vertical field of view.
    pub view: Option<(V3, V3, f32)>,
    /// A device-specific switch a remote sets, for experiments.
    pub option: i32,
    /// Shrinks the middle and far distances while frames are late.
    pub govern: bool,
}

#[derive(Clone, Copy)]
pub struct Camera {
    pub eye: V3,
    pub look: V3,
    pub fov: f32,
    pub roll: f32,
}

/// Rolling frame statistics over the last `N` frames.
pub struct Timing {
    ms: [f32; Timing::N],
    at: usize,
    pub late: u32,
    pub frames: u32,
}

impl Timing {
    const N: usize = 120;
    pub fn new() -> Timing {
        Timing { ms: [16.7; Timing::N], at: 0, late: 0, frames: 0 }
    }
    pub fn push(&mut self, ms: f32, vblanks: u32) {
        self.ms[self.at] = ms;
        self.at = (self.at + 1) % Self::N;
        self.frames += 1;
        if vblanks > 1 {
            self.late += 1;
        }
    }
    pub fn avg(&self) -> f32 {
        self.ms.iter().sum::<f32>() / Self::N as f32
    }
    pub fn worst(&self) -> f32 {
        self.ms.iter().fold(0.0, |a, &b| max(a, b))
    }
}

impl Default for Timing {
    fn default() -> Self {
        Self::new()
    }
}

/// What a device measured, for the statistics line and the status record. Times are milliseconds.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct Perf {
    pub frame: f32,
    pub worst: f32,
    pub late: u32,
    pub frames: u32,
    pub sim: f32,
    pub build: f32,
    pub draw: f32,
    pub gpu: f32,
    pub draws: u32,
    pub tris: u32,
}

pub struct Game {
    pub sim: Sim,
    pub synth: Synth,
    pub scene: Scene,
    pub actors: Actors,
    pub set: Settings,
    /// Title, play, pause and the finished run.
    pub session: Session,
    /// What the interface asked to have stored, until the device has written it.
    pub prefs: Option<String>,
    pub frame: u32,
    /// The governor's factor on the middle and far distances, `LOD_FLOOR..=1`.
    pub lod_scale: f32,
    /// Frames and late frames in the governor's current window, and windows in a row without a late frame.
    window: (u32, u32, u32),
    /// Each giant's skin matrices and the frame they were computed on.
    skins: Vec<([M34; BONES], u32)>,
}

impl Game {
    pub fn new(sim: Sim, scene: Scene) -> Game {
        let actors = Actors::new(&sim, &scene);
        let set = Settings { hud: true, sound: true, stats: false, world: true, actors: true, lod_near: scene.h.lod_near, lod_mid: scene.h.lod_mid, lod_far: scene.h.lod_far, repeat: 1, view: None, option: 0, govern: true };
        let skins = (0..sim.dummies.len()).map(|i| (sim.titan_skin(i), 0)).collect();
        let mut game = Game { sim, synth: Synth::new(), scene, actors, set, session: Session::new(), prefs: None, frame: 0, lod_scale: 1.0, window: (0, 0, 0), skins };
        game.publish();
        game
    }

    /// What the player can set on a handheld, in menu order.
    fn options(&self) -> Vec<Setting> {
        let mut out = Vec::with_capacity(4);
        self.session.settings(&mut out);
        out.push(Setting::switch("sound", self.set.sound));
        out.push(Setting::switch("govern", self.set.govern));
        out.push(Setting::switch("stats", self.set.stats));
        out
    }

    /// Shows the interface the run and the settings as they now are.
    fn publish(&mut self) {
        let options = self.options();
        // The channel belongs to the thread that steps the game.
        let state = unsafe { &mut channel().state };
        self.session.publish(&self.sim, state);
        if state.options != options {
            state.options = options;
        }
        if !self.set.stats && !state.stats.is_empty() {
            state.stats.clear();
        }
    }

    /// One displayed frame: what the interface asked for since the last one, then `ticks`
    /// simulation ticks (one per display refresh since the last frame) unless the game is paused.
    pub fn step(&mut self, pad: &Pad, ticks: u32) -> u32 {
        while let Some(command) = unsafe { channel() }.next() {
            match self.session.command(&mut self.sim, command) {
                Some(Command::Option { key, value }) => match key.as_str() {
                    "sound" => self.set.sound = value != 0,
                    "govern" => self.set.govern = value != 0,
                    "stats" => self.set.stats = value != 0,
                    _ => {}
                },
                Some(Command::Prefs(text)) => {
                    unsafe { channel() }.state.prefs.clone_from(&text);
                    self.prefs = Some(text);
                }
                _ => {}
            }
        }
        let synth = &mut self.synth;
        let events = self.session.run(&mut self.sim, pad, ticks, |sim| synth.control(sim, sim.events));
        self.publish();
        self.frame = self.frame.wrapping_add(1);
        if !self.session.paused() {
            self.govern(ticks);
        }
        events
    }

    /// The preferences the device read from its storage at the start, for the interface.
    pub fn stored(&mut self, prefs: &str) {
        let state = unsafe { &mut channel().state };
        state.prefs.clear();
        state.prefs.push_str(prefs);
    }

    /// Whether the device plays the synthesizer just now.
    pub fn audible(&self) -> bool {
        self.set.sound && !self.session.paused()
    }

    /// The frame-rate governor. Over each second of frames: more than one late
    /// frame in twenty pulls the middle and far distances in by 8 %; three
    /// seconds in a row without a late frame let them out by 4 %. The near
    /// distance stays, so what is beside the player does not change.
    fn govern(&mut self, ticks: u32) {
        const WINDOW: u32 = 60;
        const LOD_FLOOR: f32 = 0.55;
        if !self.set.govern {
            self.lod_scale = 1.0;
            self.window = (0, 0, 0);
            return;
        }
        self.window.0 += 1;
        if ticks > 1 {
            self.window.1 += 1;
        }
        if self.window.0 < WINDOW {
            return;
        }
        if self.window.1 * 20 > WINDOW {
            self.lod_scale = max(self.lod_scale * 0.92, LOD_FLOOR);
            self.window.2 = 0;
        } else if self.window.1 == 0 {
            self.window.2 += 1;
            if self.window.2 >= 3 {
                self.lod_scale = min(self.lod_scale * 1.04, 1.0);
                self.window.2 = 0;
            }
        } else {
            self.window.2 = 0;
        }
        self.window.0 = 0;
        self.window.1 = 0;
    }

    /// Giant `i`'s skin matrices. A near giant's are computed every frame; a far one's every third
    /// frame, in turn: a pose is 19 bones of trigonometry, and a standing giant sways slowly.
    pub fn titan_skin(&mut self, i: usize, near: bool) -> &[M34; BONES] {
        let due = near || (self.frame.wrapping_add(i as u32)) % 3 == 0 || self.frame.wrapping_sub(self.skins[i].1) > 3;
        if due {
            self.skins[i] = (self.sim.titan_skin(i), self.frame);
        }
        &self.skins[i].0
    }

    /// Hands the device's measurements in for the statistics line the interface shows, which is
    /// rebuilt every 30 frames.
    pub fn measure(&mut self, perf: &Perf) {
        if !self.set.stats || self.frame % 30 != 0 {
            return;
        }
        let line = unsafe { &mut channel().state.stats };
        line.clear();
        let _ = write!(line, "{} fps · {} ms · late {} · {} draws · {}k tris", F1(1000.0 / max(perf.frame, 0.1)), F1(perf.frame), perf.late, perf.draws, F1(perf.tris as f32 / 1000.0));
    }

    /// This frame's near, middle and far distances.
    pub fn lod(&self) -> (f32, f32, f32) {
        (self.set.lod_near, max(self.set.lod_mid * self.lod_scale, self.set.lod_near), self.set.lod_far * self.lod_scale)
    }

    pub fn camera(&self) -> Camera {
        match self.set.view {
            Some((pos, target, fov)) => Camera { eye: pos, look: (target - pos).norm_or(v3(0.0, 0.0, -1.0)), fov, roll: 0.0 },
            None => {
                let c = &self.sim.cam;
                let k = c.shake * 0.25;
                let t = self.sim.tick as f32;
                Camera { eye: c.pos + v3(sin(t * 1.7) * k, sin(t * 2.3) * k, cos(t * 1.9) * k), look: c.look, fov: c.fov, roll: c.roll }
            }
        }
    }

    pub fn aspect(&self) -> f32 {
        self.scene.h.screen[0] / self.scene.h.screen[1]
    }

    /// Projection × view for culling and for placing interface marks.
    pub fn view_proj(&self, cam: &Camera) -> Mat4 {
        mat::mul(&mat::perspective(cam.fov, self.aspect(), self.scene.h.clip_near, self.scene.h.clip_far), &mat::view(cam.eye, cam.look, cam.roll))
    }

    /// Whether the character is drawn: inside the camera's near range it would fill the frame.
    pub fn show_character(&self) -> bool {
        self.set.view.is_some() || (self.sim.cam.pos - self.sim.p.pos).len() > 1.7
    }

    /// How strongly a device should show speed (0..1), for an effect of its own.
    pub fn rush(&self) -> f32 {
        if self.set.view.is_some() {
            0.0
        } else {
            smoothstep(26.0, 58.0, self.sim.speed())
        }
    }

    /// Remote settings: `key=value` words separated by spaces.
    /// `auto hud sound stats world actors govern` take 0 or 1; `lodNear lodMid lodFar repeat option` a number;
    /// `mode=title|play|paused|results` sets the game's flow (`mode=play auto=1` is play flown by the
    /// autopilot, for a measurement); `reset=1` restarts; `view=px,py,pz,tx,ty,tz,fov` fixes the camera
    /// and `view=off` frees it.
    pub fn control(&mut self, text: &str) {
        for word in text.split_ascii_whitespace() {
            let Some((key, value)) = word.split_once('=') else { continue };
            let on = value == "1" || value == "true";
            let num = parse_f32(value);
            match key {
                "auto" => self.session.auto = on,
                "mode" => {
                    if let Some(mode) = Mode::parse(value).filter(|m| !matches!(m, Mode::Loading | Mode::Error)) {
                        self.session.mode = mode;
                        self.session.auto = mode == Mode::Title;
                    }
                }
                "hud" => self.set.hud = on,
                "sound" => self.set.sound = on,
                "stats" => self.set.stats = on,
                "world" => self.set.world = on,
                "actors" => self.set.actors = on,
                "lodNear" => self.set.lod_near = num.unwrap_or(self.set.lod_near),
                "lodMid" => self.set.lod_mid = num.unwrap_or(self.set.lod_mid),
                "lodFar" => self.set.lod_far = num.unwrap_or(self.set.lod_far),
                "repeat" => self.set.repeat = clamp(num.unwrap_or(1.0), 1.0, 8.0) as u32,
                "option" => self.set.option = num.unwrap_or(0.0) as i32,
                "govern" => self.set.govern = on,
                "reset" if on => self.sim.reset(),
                "view" => {
                    let mut f = [0.0f32; 7];
                    let mut n = 0;
                    for part in value.split(',') {
                        if let (Some(x), true) = (parse_f32(part), n < 7) {
                            f[n] = x;
                            n += 1;
                        }
                    }
                    self.set.view = if n >= 6 { Some((v3(f[0], f[1], f[2]), v3(f[3], f[4], f[5]), if n == 7 { f[6] } else { 62.0 })) } else { None };
                }
                _ => {}
            }
        }
    }

    /// The marks on the world for this frame, in screen pixels: where each wire would bite, the
    /// streaks of speed and the nearest target. Everything else on the screen is the interface's.
    pub fn draw_marks(&self, h: &mut Hud, vp: &Mat4) {
        let sim = &self.sim;
        if !self.set.hud || self.set.view.is_some() || self.session.mode != Mode::Play {
            return;
        }
        let (w, ht) = (self.scene.h.screen[0], self.scene.h.screen[1]);
        let sx = w / 960.0;
        let small = h.font.sizes[0];
        // Where each wire would bite, and where an aimed pair would.
        for (i, r) in sim.reticle.iter().enumerate() {
            if !r.valid {
                continue;
            }
            if let Some((x, y)) = mat::project(vp, r.point, w, ht) {
                if (0.0..w).contains(&x) && (0.0..ht).contains(&y) {
                    let (x, y) = (libm::roundf(x), libm::roundf(y));
                    if i < 2 {
                        h.frame(x - 4.0, y - 4.0, 8.0, 8.0, 1.0, rgba(255, 255, 255, 220));
                    } else {
                        h.frame(x - 3.0, y - 3.0, 6.0, 6.0, 1.0, rgba(255, 179, 71, 230));
                    }
                }
            }
        }
        // Streaks from the rim toward the centre at speed.
        let fast = smoothstep(24.0, 60.0, sim.speed());
        if fast > 0.0 {
            for i in 0..14u32 {
                let seed = i.wrapping_mul(2654435761).wrapping_add((sim.tick / 3).wrapping_mul(40503));
                let a = (seed % 6283) as f32 / 1000.0;
                let r0 = (300.0 + ((seed >> 8) % 160) as f32) * sx;
                let len = (40.0 + ((seed >> 16) % 90) as f32) * (0.5 + fast) * sx;
                let (c, s) = (cos(a), sin(a) * 0.62);
                let (x0, y0) = (w * 0.5 + c * r0, ht * 0.5 + s * r0);
                let (x1, y1) = (w * 0.5 + c * (r0 + len), ht * 0.5 + s * (r0 + len));
                let (nx, ny) = (-s * 0.8, c * 0.8);
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
            let on = mat::project(vp, nape, w, ht).filter(|(x, y)| (14.0..w - 14.0).contains(x) && (14.0..ht - 14.0).contains(y));
            let (x, y) = match on {
                Some(p) => p,
                None => {
                    // Off screen: toward it, from the centre, clamped to an ellipse inside the frame.
                    let to = nape - sim.cam.pos;
                    let right = sim.cam.look.cross(V3::UP).norm_or(v3(1.0, 0.0, 0.0));
                    let up = right.cross(sim.cam.look);
                    let (dx, dy) = (to.dot(right), to.dot(up));
                    let l = max(sqrt(dx * dx + dy * dy), 1e-3);
                    (w * 0.5 + dx / l * w * 0.44, ht * 0.5 - dy / l * ht * 0.42)
                }
            };
            let (x, y) = (libm::roundf(x), libm::roundf(y));
            h.poly([(x, y - 5.0), (x + 5.0, y), (x, y + 5.0), (x - 5.0, y)], red);
            h.text(small, x, y + 8.0 + small as f32, 0.5, red, &format!("{} m", F0(dist)));
        }
    }

    /// The status record as JSON. `extra` is the device's own members, without braces (may be empty).
    pub fn status(&self, out: &mut String, target: &str, perf: &Perf, extra: &str) {
        let s = &self.sim;
        let _ = write!(
            out,
            "{{\"target\":\"{target}\",\"stage\":\"running\",\"frame\":{},\"frameMs\":{:.3},\"worstMs\":{:.3},\"late\":{},\"frames\":{},\"cpuMs\":{{\"sim\":{:.3},\"build\":{:.3},\"draw\":{:.3}}},\"gpuMs\":{:.3},\"draws\":{},\"tris\":{},",
            self.frame, perf.frame, perf.worst, perf.late, perf.frames, perf.sim, perf.build, perf.draw, perf.gpu, perf.draws, perf.tris
        );
        let _ = write!(
            out,
            "\"mode\":\"{}\",\"interface\":{},\"settings\":{{\"auto\":{},\"hud\":{},\"stats\":{},\"world\":{},\"actors\":{},\"lodNear\":{:.1},\"lodMid\":{:.1},\"lodFar\":{:.1},\"repeat\":{},\"option\":{},\"fixedView\":{},\"govern\":{},\"lodScale\":{:.3}}},",
            self.session.mode.name(), unsafe { channel() }.is_open(), self.session.auto, self.set.hud, self.set.stats, self.set.world, self.set.actors, self.set.lod_near, self.set.lod_mid, self.set.lod_far, self.set.repeat, self.set.option, self.set.view.is_some(), self.set.govern, self.lod_scale
        );
        let _ = write!(
            out,
            "\"player\":{{\"pos\":[{:.2},{:.2},{:.2}],\"speed\":{:.2},\"gas\":{:.2},\"tick\":{},\"kills\":{},\"laps\":{},\"waypoint\":{}}}",
            s.p.pos.x, s.p.pos.y, s.p.pos.z, s.speed(), s.p.gas, s.tick, s.run.kills, s.auto.laps, s.auto.wp
        );
        if !extra.is_empty() {
            out.push(',');
            out.push_str(extra);
        }
        out.push('}');
    }
}

/// A number rounded to a whole, formatted with integer arithmetic: the float formatter in `core`
/// computes in 64 bits, which a machine without doubles does in software.
struct F0(f32);
impl core::fmt::Display for F0 {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "{}", libm::roundf(self.0) as i32)
    }
}

/// The same with one decimal.
struct F1(f32);
impl core::fmt::Display for F1 {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        let tenths = libm::roundf(self.0 * 10.0) as i32;
        write!(f, "{}{}.{}", if tenths < 0 { "-" } else { "" }, tenths.abs() / 10, tenths.abs() % 10)
    }
}

pub use maneuver_interface::parse_f32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_governor_pulls_in_when_frames_are_late_and_lets_out_when_they_are_not() {
        let sim = maneuver_sim::worldfile::load(&maneuver_sim::testworld::build()).unwrap();
        let scene = Scene::new(maneuver_pack::HandScene { lod_near: 40.0, lod_mid: 100.0, lod_far: 400.0, screen: [480.0, 272.0], ..Default::default() });
        let mut g = Game::new(sim, scene);
        // A second in which every fifth frame is late.
        for i in 0..60 {
            g.govern(if i % 5 == 0 { 2 } else { 1 });
        }
        assert!(g.lod_scale < 1.0);
        let (near, mid, far) = g.lod();
        assert_eq!(near, 40.0);
        assert!(mid < 100.0 && mid >= near && far < 400.0);
        // Many seconds on time: back to the pack's distances.
        for _ in 0..60 * 3 * 4 {
            g.govern(1);
        }
        assert_eq!(g.lod_scale, 1.0);
        // The floor holds however late the frames are.
        for _ in 0..60 * 40 {
            g.govern(3);
        }
        assert!(g.lod_scale >= 0.55);
    }

    #[test]
    fn whole_and_tenth_numbers_format_without_doubles() {
        assert_eq!(format!("{} {} {}", F1(16.74), F1(-0.26), F0(181.6)), "16.7 -0.3 182");
    }
}
