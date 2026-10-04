//! The interface: one PocketJS guest (`ui/`, shared with the other devices)
//! drawn over the scene through PocketJS's Vita host library. It owns every
//! 2D pixel that is not anchored to the world, and what the buttons and the
//! panel mean outside play; this side owns the simulation and the scene,
//! shows it the game's state and does what it asks
//! (`crates/maneuver-interface`).

use std::collections::VecDeque;

use maneuver_interface::{channel, guest, Pace, Session};
use pocketjs_vita::{input, Runtime};

/// The guest turns this often, and is told so before it mounts.
pub const TURN: f32 = 1.0 / 30.0;
const RATE: &[u8] = b"globalThis.__simHz=30;";
/// The compiled interface, beside the pack.
const SCRIPT: &str = "maneuver.js";
const PAK: &str = "maneuver.pak";

/// A pad nobody touches.
pub const NEUTRAL: input::Pad = input::Pad { buttons: 0, lx: 128, ly: 128, rx: 128, ry: 128 };

pub struct Ui {
    runtime: Option<Runtime>,
    /// Seconds the guest is owed.
    owed: f32,
    /// Buttons seen since the last turn: a press shorter than a turn still reaches the guest.
    latched: u32,
    /// Which of the turns it is offered are taken.
    pace: Pace,
    /// Buttons a control message presses, each held two turns and let go for one.
    presses: VecDeque<(u32, u8)>,
    /// Why there is no interface, for the status report.
    pub error: String,
}

impl Ui {
    /// Boots the guest from `maneuver.js` and `maneuver.pak` (the USB share
    /// in a development build, else the package, else the data folder).
    ///
    /// # Safety
    /// Render thread, after vita2d is up and outside any scene.
    pub unsafe fn boot() -> Self {
        let read = |name: &str| crate::paths::candidates(name).iter().find_map(|p| crate::hostfs::read(p, 16 << 20)).ok_or(format!("{name} is missing"));
        let boot = || -> Result<Runtime, String> {
            // The guest borrows the pak for as long as it lives.
            let pak: &'static [u8] = Box::leak(read(PAK)?.into_boxed_slice());
            let mut script = RATE.to_vec();
            script.extend(read(SCRIPT)?);
            script.push(0);
            let script = String::from_utf8(script).map_err(|e| e.to_string())?;
            let mut runtime = Runtime::new(pak)?;
            // The channel to this renderer takes the place of the host's wire.
            guest::mount(runtime.context(), runtime.global());
            runtime.eval(&script)?;
            Ok(runtime)
        };
        let mut ui = Self { runtime: None, owed: TURN, latched: 0, pace: Pace::default(), presses: VecDeque::new(), error: String::new() };
        match boot() {
            Ok(runtime) => ui.runtime = Some(runtime),
            Err(error) => ui.fail(error),
        }
        ui
    }

    /// The guest is on the screen.
    pub fn live(&self) -> bool {
        self.runtime.is_some()
    }

    /// Without a guest the channel is shut, and the session keeps the game's
    /// flow on START and SELECT.
    unsafe fn fail(&mut self, error: String) {
        pocketjs_vita::vita_log(format_args!("maneuver: interface: {error}"));
        self.error = error;
        self.runtime = None;
        channel().close();
    }

    /// Presses `buttons` on the interface as a thumb would.
    pub fn press(&mut self, buttons: u32) {
        self.presses.push_back((buttons, 3));
    }

    /// The guest's turn when `dt` more seconds make one due: the pad and the
    /// panel go in, the state is read and commands are left on the channel.
    /// `quiet` withholds the panel (another layer has the screen). With a `session` the turn is
    /// taken only when it is worth its cost (`Pace`); without one (the world is loading) always.
    ///
    /// # Safety
    /// Render thread, outside any scene.
    pub unsafe fn turn(&mut self, dt: f32, pad: &input::Pad, quiet: bool, session: Option<&Session>) {
        self.latched |= pad.buttons;
        self.owed = (self.owed + dt).min(2.0 * TURN);
        if self.owed < TURN {
            return;
        }
        self.owed -= TURN;
        let touches = if quiet { input::TouchSnapshot::EMPTY } else { input::read_touches() };
        if let Some(session) = session {
            if !self.pace.due(session, self.latched, !touches.packed().is_empty()) && self.presses.is_empty() {
                self.latched = 0;
                return;
            }
        }
        let Some(runtime) = &mut self.runtime else { return };
        let mut buttons = core::mem::take(&mut self.latched);
        if let Some((pressed, turns)) = self.presses.front_mut() {
            *turns -= 1;
            if *turns > 0 {
                buttons |= *pressed;
            } else {
                self.presses.pop_front();
            }
        }
        if let Err(error) = runtime.frame_with_input(buttons as i32, pad.left_analog(), &touches) {
            self.fail(error);
            return;
        }
        // Two core ticks to a turn: the core counts sixtieths.
        runtime.tick();
        runtime.tick();
    }

    /// Draws the interface into the open vita2d scene, over what is there.
    ///
    /// # Safety
    /// Render thread, inside the display scene.
    pub unsafe fn draw(&mut self) {
        if let Some(runtime) = &mut self.runtime {
            runtime.render_over();
        }
    }
}
