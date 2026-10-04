//! The loop around the simulation that every handheld shares: pad to input,
//! the autopilot, notes on the screen, the camera, the interface's contents,
//! remote settings and the status record.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use maneuver_sim::audio::Synth;
use maneuver_sim::math::*;
use maneuver_sim::sim::{ev, tune, Input};
use maneuver_sim::Sim;

use maneuver_sim::pose::BONES;

use crate::actors::Actors;
use crate::hud::{rgba, Hud};
use crate::mat::{self, Mat4};
use crate::scene::Scene;

/// Pad buttons beyond the simulation's (`maneuver_sim::sim::btn`, the low bits).
pub mod pad {
    pub const START: u32 = 1 << 16;
    pub const SELECT: u32 = 1 << 17;
    /// The simulation's buttons.
    pub const PLAY: u32 = 0xffff;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Pad {
    pub buttons: u32,
    /// The stick, -1..1; `ly` positive is forward.
    pub lx: f32,
    pub ly: f32,
    /// The camera, -1..1: a second stick, or the direction pad on a machine with one stick.
    pub rx: f32,
    pub ry: f32,
}

pub struct Settings {
    pub auto: bool,
    pub hud: bool,
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
    note: (String, f32),
    prev_buttons: u32,
    pub frame: u32,
    /// The governor's factor on the middle and far distances, `LOD_FLOOR..=1`.
    pub lod_scale: f32,
    /// Frames and late frames in the governor's current window, and windows in a row without a late frame.
    window: (u32, u32, u32),
    /// Each giant's skin matrices and the frame they were computed on.
    skins: Vec<([M34; BONES], u32)>,
    /// The statistics line, rebuilt a few times a second.
    stats_line: String,
    /// The controls, as this machine labels them.
    help: &'static str,
}

impl Game {
    pub fn new(sim: Sim, scene: Scene, help: &'static str) -> Game {
        let actors = Actors::new(&sim, &scene);
        let set = Settings { auto: true, hud: true, stats: false, world: true, actors: true, lod_near: scene.h.lod_near, lod_mid: scene.h.lod_mid, lod_far: scene.h.lod_far, repeat: 1, view: None, option: 0, govern: true };
        let skins = (0..sim.dummies.len()).map(|i| (sim.titan_skin(i), 0)).collect();
        Game { sim, synth: Synth::new(), scene, actors, set, note: (String::new(), 0.0), prev_buttons: u32::MAX, frame: 0, lod_scale: 1.0, window: (0, 0, 0), skins, stats_line: String::new(), help }
    }

    fn say(&mut self, text: &str, seconds: f32) {
        self.note.0.clear();
        self.note.0.push_str(text);
        self.note.1 = seconds;
    }

    /// One displayed frame: `ticks` simulation ticks (one per display refresh since the last frame).
    pub fn step(&mut self, pad: &Pad, ticks: u32) -> u32 {
        let pressed = pad.buttons & !self.prev_buttons;
        self.prev_buttons = pad.buttons;
        if pressed & pad::SELECT != 0 && self.frame > 30 {
            self.sim.reset();
            self.set.auto = false;
            self.say("RESTART", 1.2);
        }
        if pressed & pad::START != 0 {
            self.set.auto = !self.set.auto;
            let text = if self.set.auto { "AUTOPILOT" } else { "MANUAL" };
            self.say(text, 1.5);
        }
        // Any deliberate input takes over from the autopilot.
        if self.set.auto && pressed & pad::PLAY != 0 && self.frame > 30 {
            self.set.auto = false;
            self.say("MANUAL", 1.5);
        }
        let mut events = 0u32;
        for _ in 0..ticks {
            let inp = if self.set.auto { self.sim.auto_input() } else { Input { buttons: pad.buttons & pad::PLAY, lx: pad.lx, ly: pad.ly, rx: pad.rx, ry: pad.ry } };
            self.sim.tick(inp);
            events |= self.sim.events;
            self.synth.control(&self.sim, self.sim.events);
        }
        if events & ev::SLASH_HIT != 0 {
            let text = format!("CUT  {} km/h", F0(self.sim.run.last_cut_speed * 3.6));
            self.say(&text, 1.6);
        } else if events & ev::SLASH_WEAK != 0 {
            self.say("TOO SLOW", 1.0);
        }
        if events & ev::REFILL != 0 {
            self.say("GAS REFILLED", 1.2);
        }
        if events & ev::RUN_DONE != 0 {
            self.say("ALL TARGETS CUT", 5.0);
        }
        self.note.1 -= ticks as f32 / 60.0;
        self.frame = self.frame.wrapping_add(1);
        self.govern(ticks);
        events
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

    /// Hands the device's measurements in for the statistics line, which is rebuilt every 20 frames.
    pub fn measure(&mut self, perf: &Perf) {
        if !self.set.stats || self.frame % 20 != 0 {
            return;
        }
        self.stats_line.clear();
        let _ = write!(
            self.stats_line,
            "{} fps {} ms (worst {}) late {}  cpu {}+{}+{}  {} draws {}k tris",
            F1(1000.0 / max(perf.frame, 0.1)),
            F1(perf.frame),
            F1(perf.worst),
            perf.late,
            F1(perf.sim),
            F1(perf.build),
            F1(perf.draw),
            perf.draws,
            F1(perf.tris as f32 / 1000.0)
        );
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
    /// `auto hud stats world actors govern` take 0 or 1; `lodNear lodMid lodFar repeat option` a number;
    /// `reset=1` restarts; `view=px,py,pz,tx,ty,tz,fov` fixes the camera and `view=off` frees it.
    pub fn control(&mut self, text: &str) {
        for word in text.split_ascii_whitespace() {
            let Some((key, value)) = word.split_once('=') else { continue };
            let on = value == "1" || value == "true";
            let num = parse_f32(value);
            match key {
                "auto" => self.set.auto = on,
                "hud" => self.set.hud = on,
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

    /// The interface for this frame. `sizes` are the font's pixel sizes: small, medium, large.
    pub fn draw_hud(&self, h: &mut Hud, vp: &Mat4) {
        let sim = &self.sim;
        let (w, ht) = (self.scene.h.screen[0], self.scene.h.screen[1]);
        let (sx, sy) = (w / 960.0, ht / 544.0);
        let sizes = &h.font.sizes;
        let (small, medium, large) = (sizes[0], sizes[sizes.len() / 2], sizes[sizes.len() - 1]);
        let fixed = self.set.view.is_some();
        if self.set.hud {
            let white = rgba(244, 241, 232, 255);
            let dim = rgba(244, 241, 232, 190);
            // Gas, bottom left.
            let gas = sim.p.gas / tune::GAS_MAX;
            let (gx, gy, gw, gh) = (libm::roundf(30.0 * sx), libm::roundf(492.0 * sy), libm::roundf(224.0 * sx), max(libm::roundf(14.0 * sy), 6.0));
            h.text(small, gx, gy - 4.0, 0.0, dim, "GAS");
            h.rect(gx, gy, gw, gh, rgba(10, 14, 20, 140));
            h.frame(gx, gy, gw, gh, 1.0, rgba(255, 255, 255, 130));
            let fill = if gas < 0.2 { rgba(255, 122, 60, 255) } else { rgba(233, 240, 244, 255) };
            h.rect(gx + 2.0, gy + 2.0, (gw - 4.0) * gas, gh - 4.0, fill);
            // Speed, bottom right.
            h.text(large, 866.0 * sx, 510.0 * sy, 1.0, white, &format!("{}", F0(sim.speed() * 3.6)));
            h.text(small, 874.0 * sx, 510.0 * sy, 0.0, dim, "km/h");
            // Targets and time, top right.
            h.text(medium, 930.0 * sx, 44.0 * sy, 1.0, white, &format!("{} / {}", sim.run.kills, sim.dummies.len()));
            let t = sim.run.ticks as f32 / 60.0;
            h.text(small, 930.0 * sx, 44.0 * sy + small as f32 + 3.0, 1.0, dim, &clock(t));
            if !fixed {
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
            if sim.run.done {
                let t = sim.run.ticks as f32 / 60.0;
                h.text(large, w * 0.5, 250.0 * sy, 0.5, white, &clock(t));
                h.text(small, w * 0.5, 250.0 * sy + small as f32 + 6.0, 0.5, dim, &format!("every target cut  -  top speed {} km/h  -  SELECT starts again", F0(sim.run.max_speed * 3.6)));
            }
            if self.note.1 > 0.0 {
                let a = (min(self.note.1, 0.3) / 0.3 * 255.0) as u8;
                h.text(medium, w * 0.5, 132.0 * sy, 0.5, rgba(244, 241, 232, a), &self.note.0);
            }
            if self.set.auto {
                if sim.tick < 420 {
                    let a = (smoothstep(420.0, 300.0, sim.tick as f32) * 255.0) as u8;
                    h.text(large, w * 0.5, 200.0 * sy, 0.5, rgba(244, 241, 232, a), "POCKET MANEUVER");
                    h.text(small, w * 0.5, 200.0 * sy + small as f32 + 8.0, 0.5, rgba(244, 241, 232, a), self.help);
                }
                h.text(small, w * 0.5, ht - 8.0, 0.5, dim, "AUTOPILOT  -  press a button to take over");
            }
        }
        if self.set.stats {
            h.text(small, 6.0, small as f32 + 2.0, 0.0, rgba(255, 255, 255, 220), &self.stats_line);
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
            "\"settings\":{{\"auto\":{},\"hud\":{},\"stats\":{},\"world\":{},\"actors\":{},\"lodNear\":{:.1},\"lodMid\":{:.1},\"lodFar\":{:.1},\"repeat\":{},\"option\":{},\"fixedView\":{},\"govern\":{},\"lodScale\":{:.3}}},",
            self.set.auto, self.set.hud, self.set.stats, self.set.world, self.set.actors, self.set.lod_near, self.set.lod_mid, self.set.lod_far, self.set.repeat, self.set.option, self.set.view.is_some(), self.set.govern, self.lod_scale
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

fn clock(t: f32) -> String {
    let tenths = (t * 10.0) as u32;
    let (m, s, d) = (tenths / 600, tenths / 10 % 60, tenths % 10);
    format!("{}:{}{}.{}", m, if s < 10 { "0" } else { "" }, s, d)
}

/// A decimal number: digits, an optional sign and one optional point.
pub fn parse_f32(s: &str) -> Option<f32> {
    let (neg, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    if rest.is_empty() {
        return None;
    }
    let (mut value, mut scale, mut seen_point) = (0.0f32, 1.0f32, false);
    for c in rest.bytes() {
        match c {
            b'0'..=b'9' => {
                value = value * 10.0 + (c - b'0') as f32;
                if seen_point {
                    scale *= 10.0;
                }
            }
            b'.' if !seen_point => seen_point = true,
            _ => return None,
        }
    }
    let v = value / scale;
    Some(if neg { -v } else { v })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_governor_pulls_in_when_frames_are_late_and_lets_out_when_they_are_not() {
        let sim = maneuver_sim::worldfile::load(&maneuver_sim::testworld::build()).unwrap();
        let scene = Scene::new(maneuver_pack::HandScene { lod_near: 40.0, lod_mid: 100.0, lod_far: 400.0, screen: [480.0, 272.0], ..Default::default() });
        let mut g = Game::new(sim, scene, "");
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
    fn numbers_parse() {
        assert_eq!(parse_f32("12.5"), Some(12.5));
        assert_eq!(parse_f32("-3"), Some(-3.0));
        assert_eq!(parse_f32("1e3"), None);
        assert_eq!(parse_f32(""), None);
    }

    #[test]
    fn the_clock_pads_seconds() {
        assert_eq!(clock(65.25), "1:05.2");
        assert_eq!(clock(12.0), "0:12.0");
        assert_eq!(format!("{} {} {}", F1(16.74), F1(-0.26), F0(181.6)), "16.7 -0.3 182");
    }
}
