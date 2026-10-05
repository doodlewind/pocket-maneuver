//! The game's flow around the simulation, the same on every device: the
//! title with the autopilot behind it, play, pause and the finished run; the
//! pad and a touch panel's controls made into the simulation's input; the
//! notes; and what the interface is shown of it all.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use maneuver_sim::math::*;
use maneuver_sim::sim::{ev, hook, tune, Input, DT};
use maneuver_sim::Sim;

use crate::{channel, Command, Mode, Setting, State, Telemetry};

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

/// Radians of view per logical pixel a finger drags: the panel's width turns the view about 220°.
const LOOK: f32 = 0.008;
/// The simulation ignores a stick inside this radius and rescales the rest.
const DEAD: f32 = 0.18;

/// The stick position the simulation reads as `(x, y)` once its dead zone is taken off.
fn past_dead_zone(x: f32, y: f32) -> (f32, f32) {
    let m = sqrt(x * x + y * y);
    if m < 1e-4 {
        return (0.0, 0.0);
    }
    let k = (DEAD + min(m, 1.0) * (1.0 - DEAD)) / m;
    (x * k, y * k)
}

pub struct Session {
    pub mode: Mode,
    /// The autopilot plays: always behind the title, and in play when a measurement asks for it.
    pub auto: bool,
    /// Pushing the camera stick up looks down.
    pub invert: bool,
    /// The interface has nothing scheduled (see [`Pace`]).
    pub idle: bool,
    /// The numbers in flight are refreshed every this many frames: 2 is 30 times a second. Each
    /// refresh is a line the guest reads in a turn, so a slow machine sets more.
    pub numbers_every: u32,
    note: String,
    note_id: u32,
    /// Controls drawn on a touch panel.
    drive: Pad,
    /// Radians a dragging finger still owes the camera: to the right, and up.
    look: (f32, f32),
    prev_buttons: u32,
    frames: u32,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Session {
        Session { mode: Mode::Title, auto: true, invert: false, idle: false, numbers_every: 2, note: String::new(), note_id: 0, drive: Pad::default(), look: (0.0, 0.0), prev_buttons: u32::MAX, frames: 0 }
    }

    /// A line for the middle of the screen.
    pub fn say(&mut self, text: &str) {
        self.note.clear();
        self.note.push_str(text);
        self.note_id = self.note_id.wrapping_add(1);
    }

    fn play(&mut self) {
        self.mode = Mode::Play;
        self.auto = false;
        self.look = (0.0, 0.0);
    }

    /// No tick runs: the world stands still and the device plays no sound.
    pub fn paused(&self) -> bool {
        self.mode == Mode::Paused
    }

    /// Does what the interface asked. A command that is the device's to carry out (a setting of
    /// its own, the preferences to store) comes back.
    pub fn command(&mut self, sim: &mut Sim, command: Command) -> Option<Command> {
        match command {
            Command::Start => {
                if self.mode == Mode::Title {
                    sim.reset();
                    self.play();
                }
            }
            Command::Pause(on) => match (self.mode, on) {
                (Mode::Play, true) => self.mode = Mode::Paused,
                (Mode::Paused, false) => self.mode = Mode::Play,
                _ => {}
            },
            Command::Restart => {
                sim.reset();
                self.play();
            }
            Command::Title => {
                sim.reset();
                self.mode = Mode::Title;
                self.auto = true;
            }
            Command::Option { ref key, value } if key == "invert" => self.invert = value != 0,
            Command::Drive { mx, my, lx, ly, buttons } => self.drive = Pad { buttons: buttons & pad::PLAY, lx: mx, ly: my, rx: lx, ry: ly },
            Command::Idle(on) => self.idle = on,
            Command::Look { dx, dy } => {
                self.look.0 = clamp(self.look.0 + dx * LOOK, -1.5, 1.5);
                self.look.1 = clamp(self.look.1 - dy * LOOK, -1.0, 1.0);
            }
            other => return Some(other),
        }
        None
    }

    /// The settings every device has, for the head of its list.
    pub fn settings(&self, out: &mut Vec<Setting>) {
        out.push(Setting::switch("invert", self.invert));
    }

    /// The simulation's input for one tick of play.
    fn input(&mut self, pad: &Pad) -> Input {
        if self.mode != Mode::Play {
            return Input::default();
        }
        let d = self.drive;
        let touch = d.lx != 0.0 || d.ly != 0.0;
        let (lx, ly) = if touch { past_dead_zone(d.lx, d.ly) } else { (pad.lx, pad.ly) };
        let (mut rx, mut ry) = if d.rx != 0.0 || d.ry != 0.0 { past_dead_zone(d.rx, d.ry) } else { (pad.rx, pad.ry) };
        if self.look.0 != 0.0 || self.look.1 != 0.0 {
            // The simulation turns the view by stick² × rate each tick: spend what the finger is owed at that rate.
            let (yaw, pitch) = (tune::CAM_YAW_RATE * DT, tune::CAM_PITCH_RATE * DT);
            let (sx, sy) = (clamp(self.look.0, -yaw, yaw), clamp(self.look.1, -pitch, pitch));
            self.look.0 -= sx;
            self.look.1 -= sy;
            if abs(self.look.0) < 1e-4 {
                self.look.0 = 0.0;
            }
            if abs(self.look.1) < 1e-4 {
                self.look.1 = 0.0;
            }
            let root = |v: f32, full: f32| if v < 0.0 { -sqrt(-v / full) } else { sqrt(v / full) };
            (rx, ry) = past_dead_zone(root(sx, yaw), root(sy, pitch));
        }
        if self.invert {
            ry = -ry;
        }
        Input { buttons: (pad.buttons | d.buttons) & pad::PLAY, lx, ly, rx, ry }
    }

    /// One displayed frame: `ticks` simulation ticks, unless the game is paused. `each` sees the
    /// simulation after every tick (the synthesizer listens there). Returns the ticks' events.
    pub fn run(&mut self, sim: &mut Sim, pad: &Pad, ticks: u32, mut each: impl FnMut(&Sim)) -> u32 {
        let pressed = pad.buttons & !self.prev_buttons;
        self.prev_buttons = pad.buttons;
        self.frames = self.frames.wrapping_add(1);
        // With no interface on the screen the pad keeps the flow itself: START switches the
        // autopilot, SELECT restarts, any play button takes over.
        if !unsafe { channel() }.is_open() && self.frames > 30 {
            if self.mode == Mode::Results || self.mode == Mode::Paused {
                self.mode = Mode::Play;
            }
            if pressed & pad::SELECT != 0 {
                sim.reset();
                self.play();
            } else if pressed & pad::START != 0 {
                self.auto = !self.auto;
                self.mode = if self.auto { Mode::Title } else { Mode::Play };
            } else if self.mode == Mode::Title && pressed & pad::PLAY != 0 {
                self.play();
            }
        }
        if !matches!(self.mode, Mode::Title | Mode::Play | Mode::Results) {
            return 0;
        }
        let mut events = 0u32;
        for _ in 0..ticks {
            let input = if self.auto { sim.auto_input() } else { self.input(pad) };
            sim.tick(input);
            events |= sim.events;
            each(sim);
        }
        if self.mode == Mode::Play {
            if events & ev::SLASH_HIT != 0 {
                let text = format!("CUT  {} km/h", libm::roundf(sim.run.last_cut_speed * 3.6) as i32);
                self.say(&text);
            } else if events & ev::SLASH_WEAK != 0 {
                self.say("TOO SLOW");
            }
            if events & ev::REFILL != 0 {
                self.say("GAS REFILLED");
            }
            if events & ev::RUN_DONE != 0 && !self.auto {
                self.mode = Mode::Results;
            }
        }
        events
    }

    /// Writes what the interface is shown of the run.
    pub fn publish(&self, sim: &Sim, state: &mut State) {
        state.mode = self.mode;
        state.kills = sim.run.kills;
        state.total = sim.dummies.len() as u32;
        let alive = |i: usize| if sim.dummies[i].alive { b'1' } else { b'0' };
        if state.alive.len() != sim.dummies.len() || state.alive.bytes().enumerate().any(|(i, c)| c != alive(i)) {
            state.alive.clear();
            state.alive.extend((0..sim.dummies.len()).map(|i| alive(i) as char));
        }
        if state.giants.is_empty() {
            for (i, d) in sim.dummies.iter().enumerate() {
                let _ = write!(state.giants, "{}{},{}", if i > 0 { ";" } else { "" }, libm::roundf(d.pos.x) as i32, libm::roundf(d.pos.z) as i32);
            }
        }
        if state.note_id != self.note_id {
            state.note.clone_from(&self.note);
            state.note_id = self.note_id;
        }
        if self.mode == Mode::Results {
            state.result = [sim.run.ticks / 6, libm::roundf(sim.run.max_speed * 3.6) as u32];
        }
        // The numbers in flight show in play; behind the title they would cost the guest a line a turn.
        if self.mode != Mode::Play || self.frames % self.numbers_every.max(1) != 0 {
            return;
        }
        let p = &sim.p;
        let heading = libm::roundf(-sim.cam.yaw * (180.0 / PI)) as i32;
        state.t = Telemetry {
            speed: libm::roundf(sim.speed() * 3.6) as i32,
            gas: libm::roundf(p.gas / tune::GAS_MAX * 100.0) as i32,
            tenths: (sim.run.ticks / 6) as i32,
            x: libm::roundf(p.pos.x) as i32,
            z: libm::roundf(p.pos.z) as i32,
            heading: heading.rem_euclid(360),
            wires: (p.hooks[0].state == hook::ATTACHED) as i32 | ((p.hooks[1].state == hook::ATTACHED) as i32) << 1,
        };
    }
}

/// PocketJS's START button, the one the interface listens to during play.
const GUEST_START: u32 = 0x0008;
/// Turns after the last reason for one: a row's highlight or a switch is still moving.
const LINGER: u8 = 12;

/// Whether the guest's next turn is worth its cost. A turn runs the whole framework's frame, which
/// is milliseconds on a PSP or a 3DS however little changed, so a device offers the guest a turn
/// at its rate and takes it only when this says so.
#[derive(Default)]
pub struct Pace {
    buttons: u32,
    linger: u8,
}

impl Pace {
    pub const fn new() -> Pace {
        Pace { buttons: 0, linger: 0 }
    }

    /// `buttons` are the guest's (PocketJS bits) as held since the last offer; `touching` is any
    /// contact on a surface the guest reads. A turn is due while the guest has something scheduled,
    /// when the renderer has news for it, when what it listens to changed (in play, START alone;
    /// elsewhere every button), while a finger is down, and for a few turns after any of those but
    /// a readout: new numbers are one turn, with nothing left moving after it.
    pub fn due(&mut self, session: &Session, buttons: u32, touching: bool) -> bool {
        let heard = if session.mode == Mode::Play { buttons & GUEST_START } else { buttons };
        let changed = heard != self.buttons;
        self.buttons = heard;
        let interface = unsafe { channel() };
        let news = interface.news();
        if !session.idle || changed || touching || (news && !interface.only_readouts()) {
            self.linger = LINGER;
            return true;
        }
        if self.linger > 0 {
            self.linger -= 1;
            return true;
        }
        news
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maneuver_sim::sim::btn;

    fn world() -> Sim {
        maneuver_sim::worldfile::load(&maneuver_sim::testworld::build()).unwrap()
    }

    #[test]
    fn the_flow_follows_the_interface() {
        let mut sim = world();
        let mut session = Session::new();
        unsafe { channel() }.open("pocket.overlay");
        let pad = Pad::default();
        // Behind the title the autopilot plays, whatever the pad holds.
        session.run(&mut sim, &Pad { buttons: btn::GAS | pad::START, ..pad }, 1, |_| {});
        assert_eq!((session.mode, session.auto), (Mode::Title, true));
        assert_eq!(session.command(&mut sim, Command::Start), None);
        assert_eq!((session.mode, session.auto, sim.tick), (Mode::Play, false, 0));
        session.run(&mut sim, &pad, 2, |_| {});
        assert_eq!(sim.tick, 2);
        // Paused: no tick runs.
        session.command(&mut sim, Command::Pause(true));
        assert!(session.paused());
        assert_eq!(session.run(&mut sim, &pad, 3, |_| {}), 0);
        assert_eq!(sim.tick, 2);
        session.command(&mut sim, Command::Pause(false));
        assert_eq!(session.mode, Mode::Play);
        // A setting of the device's own, and the preferences, are the device's to carry out.
        assert_eq!(session.command(&mut sim, Command::Option { key: "invert".into(), value: 1 }), None);
        assert!(session.invert);
        let bloom = Command::Option { key: "bloom".into(), value: 0 };
        assert_eq!(session.command(&mut sim, bloom.clone()), Some(bloom));
        session.command(&mut sim, Command::Title);
        assert_eq!((session.mode, session.auto, sim.tick), (Mode::Title, true, 0));
        unsafe { channel() }.close();
    }

    #[test]
    fn a_touch_panel_drives_the_simulation() {
        let mut sim = world();
        let mut session = Session::new();
        session.command(&mut sim, Command::Restart);
        let yaw = sim.cam.yaw;
        // A finger dragged 100 pixels to the right turns the view to the right by 100 × LOOK radians.
        session.command(&mut sim, Command::Look { dx: 100.0, dy: 0.0 });
        for _ in 0..40 {
            session.run(&mut sim, &Pad::default(), 1, |_| {});
        }
        let turned = wrap_angle(yaw - sim.cam.yaw);
        assert!((turned - 100.0 * LOOK).abs() < 0.02, "turned {turned}");
        // The drawn stick and buttons reach the simulation; a paused game takes none.
        session.command(&mut sim, Command::Drive { mx: 0.0, my: 1.0, lx: 0.0, ly: 0.0, buttons: btn::GAS });
        let input = session.input(&Pad::default());
        assert_eq!(input.buttons, btn::GAS);
        assert!(input.ly > 0.99);
        session.command(&mut sim, Command::Pause(true));
        assert_eq!(session.input(&Pad::default()).buttons, 0);
    }

    #[test]
    fn an_idle_guest_is_turned_when_there_is_a_reason() {
        let mut sim = world();
        let mut session = Session::new();
        session.command(&mut sim, Command::Restart);
        let mut pace = Pace::default();
        // A guest with something scheduled takes every turn.
        assert!(pace.due(&session, 0, false));
        session.command(&mut sim, Command::Idle(true));
        for _ in 0..LINGER {
            assert!(pace.due(&session, 0, false));
        }
        assert!(!pace.due(&session, 0, false));
        // In play the wires and the gas are not the guest's; START is.
        assert!(!pace.due(&session, 0x4000 | 0x0100, false));
        assert!(pace.due(&session, GUEST_START, false));
        for _ in 0..=LINGER {
            pace.due(&session, GUEST_START, false);
        }
        assert!(!pace.due(&session, GUEST_START, false));
        // A list listens to every button; a finger always counts.
        session.command(&mut sim, Command::Pause(true));
        assert!(pace.due(&session, GUEST_START | 0x0040, false));
        session.command(&mut sim, Command::Pause(false));
        for _ in 0..=LINGER + 1 {
            pace.due(&session, 0, false);
        }
        assert!(!pace.due(&session, 0, false));
        assert!(pace.due(&session, 0, true));
    }

    #[test]
    fn the_interface_is_shown_the_run() {
        let mut sim = world();
        let mut session = Session::new();
        session.command(&mut sim, Command::Restart);
        let mut state = State::default();
        session.publish(&sim, &mut state);
        assert_eq!(state.mode, Mode::Play);
        assert_eq!(state.total as usize, sim.dummies.len());
        assert_eq!(state.alive.len(), sim.dummies.len());
        assert!(state.alive.bytes().all(|c| c == b'1'));
        assert_eq!(state.giants.split(';').count(), sim.dummies.len());
        assert_eq!(state.t.gas, 100);
        sim.dummies[0].alive = false;
        session.say("TOO SLOW");
        session.publish(&sim, &mut state);
        assert!(state.alive.starts_with('0'));
        assert_eq!((state.note.as_str(), state.note_id), ("TOO SLOW", 1));
    }
}
