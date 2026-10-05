//! The interface: one PocketJS guest (`ui/`, shared with the other devices)
//! drawn over the scene through PocketJS's PSP host library. It owns every
//! 2D pixel that is not a mark on the world, and what the buttons mean
//! outside play; this side owns the scene, shows it the game's state and does
//! what it asks (`crates/maneuver-interface`).
//!
//! The guest turns 30 times a second, while the GE draws the previous frame.
//! A turn is the pad going in, the guest's script, and the UI core laying out
//! what it shows; the frames between two turns draw the same list again.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;

use libquickjs_sys::*;
use maneuver_interface::{channel, guest, Pace, Session};
use pocketjs_psp::{arena, ffi, ge, host, pak, qjs_alloc};
use psp::sys::sceKernelGetSystemTimeLow;

// libquickjs-sys omits these; the linked QuickJS provides them.
extern "C" {
    fn JS_NewArrayBuffer(ctx: *mut JSContext, buf: *mut u8, len: usize, free: Option<unsafe extern "C" fn(*mut JSRuntime, *mut c_void, *mut c_void)>, opaque: *mut c_void, shared: i32) -> JSValue;
    fn JS_RunGC(rt: *mut JSRuntime);
}

/// The guest's turns a second; it is told so before it mounts. The UI core counts sixtieths, so a
/// turn is this many of its ticks.
const TURNS: u32 = 30;
const TICKS: u32 = 60 / TURNS;
/// Seconds between two turns.
pub const TURN: f32 = 1.0 / TURNS as f32;
/// The stick at rest, as the guest reads it: x in the high byte, y in the low.
pub const STICK_CENTER: u32 = 0x8080;
/// What the interface takes: its pak, the guest's script and heap, the UI core and its working
/// memory (4.8 MB measured in the emulator with a 245 KiB bundle and a 0.7 MB pak). Memory must
/// have this much left over after the world for the interface to start.
pub const RESERVE: usize = 5 * 1024 * 1024;

pub struct Ui {
    guest: Option<(*mut JSRuntime, *mut JSContext, JSValue, JSValue)>,
    /// Seconds the guest is owed.
    owed: f32,
    /// Buttons held at any frame since the last turn.
    latched: u32,
    /// The arena's high-water mark at the last collection.
    collected: usize,
    /// What it last drew, for the frames between its turns.
    words: (*const u32, usize),
    /// The last turn's script has run and its layout has not.
    laid_out: bool,
    /// Which of the turns it is offered are taken.
    pace: Pace,
    /// Buttons a control message presses, and for how many more turns: a press is held two turns and
    /// let go for one; a rest holds nothing.
    presses: VecDeque<(u32, u8)>,
    /// Why there is no interface, or what its script last threw.
    pub error: String,
    /// Milliseconds of a turn, smoothed over the last ones: the guest's script, then the core's
    /// layout and its list.
    pub script_ms: f32,
    pub layout_ms: f32,
    /// The longest turn so far, milliseconds.
    pub worst_ms: f32,
    pub turns: u32,
}

/// The pending exception as text.
unsafe fn exception(ctx: *mut JSContext) -> String {
    let e = JS_GetException(ctx);
    let mut out = String::new();
    let mut len: size_t = 0;
    let s = JS_ToCStringLen2(ctx, &mut len, e, 0);
    if !s.is_null() {
        if let Ok(text) = core::str::from_utf8(core::slice::from_raw_parts(s as *const u8, len)) {
            out.extend(text.chars().take(160).map(|c| if c == '"' || c == '\\' || c < ' ' { ' ' } else { c }));
        }
        JS_FreeCString(ctx, s);
    }
    JS_FreeValue(ctx, e);
    out
}

impl Ui {
    /// No interface: the pad keeps the game's flow itself.
    pub fn none(why: &str) -> Ui {
        unsafe { channel().close() };
        Ui { guest: None, owed: TURN, latched: 0, collected: 0, words: (core::ptr::null(), 0), laid_out: false, pace: Pace::default(), presses: VecDeque::new(), error: why.into(), script_ms: 0.0, layout_ms: 0.0, worst_ms: 0.0, turns: 0 }
    }

    /// Boots the guest: `script` is the bundle, NUL-terminated (not needed once this returns), `pak`
    /// its styles, fonts and pictures, which the guest borrows for good.
    pub unsafe fn boot(script: Option<Vec<u8>>, pak: Option<&'static [u8]>) -> Ui {
        let (Some(script), Some(pak)) = (script, pak) else {
            return Ui::none("maneuver.js or maneuver.pak is missing");
        };
        let core = ffi::init_ui();
        let (textures, sprites) = pak::feed(core, pak);
        pak::install(pak);
        let rt = qjs_alloc::new_runtime();
        let ctx = if rt.is_null() { core::ptr::null_mut() } else { JS_NewContext(rt) };
        if ctx.is_null() {
            return Ui::none("no memory for the interface");
        }
        let global = JS_GetGlobalObject(ctx);
        ffi::register(ctx, global, &textures, &sprites);
        // The channel to this renderer takes the place of the host's wire.
        guest::mount(ctx, global);
        JS_SetPropertyStr(ctx, global, c"__simHz".as_ptr(), JS_NewInt32(ctx, TURNS as i32));
        JS_SetPropertyStr(ctx, global, c"__pak".as_ptr(), JS_NewArrayBuffer(ctx, pak.as_ptr() as *mut u8, pak.len(), None, core::ptr::null_mut(), 0));
        let result = JS_Eval(ctx, script.as_ptr() as *const _, script.len() - 1, c"maneuver.js".as_ptr(), JS_EVAL_TYPE_GLOBAL as i32);
        let thrown = (JS_ValueGetTag(result) == JS_TAG_EXCEPTION).then(|| exception(ctx));
        JS_FreeValue(ctx, result);
        let frame = JS_GetPropertyStr(ctx, global, c"frame".as_ptr());
        if thrown.is_some() || JS_IsUndefined(frame) {
            let mut ui = Ui::none("the interface did not start");
            if let Some(text) = thrown.filter(|t| !t.is_empty()) {
                ui.error = text;
            }
            return ui;
        }
        host::drain_jobs(rt);
        JS_RunGC(rt);
        Ui { guest: Some((rt, ctx, global, frame)), owed: TURN, latched: 0, collected: arena::stats().bump_bytes, words: (core::ptr::null(), 0), laid_out: false, pace: Pace::default(), presses: VecDeque::new(), error: String::new(), script_ms: 0.0, layout_ms: 0.0, worst_ms: 0.0, turns: 0 }
    }

    /// The guest is on the screen.
    pub fn up(&self) -> bool {
        self.guest.is_some()
    }

    /// Presses `buttons` on the interface as a thumb would, after the presses already waiting.
    pub fn press(&mut self, buttons: u32) {
        self.presses.push_back((buttons, 3));
    }

    /// Leaves the pad alone for `turns` before the next waiting press.
    pub fn rest(&mut self, turns: u32) {
        self.presses.push_back((0, turns.clamp(1, 255) as u8));
    }

    /// The guest's turn when `dt` more seconds make one due: the pad goes in (a button held at any
    /// frame since the last turn counts), and what it shows is laid out. What it asked for waits in
    /// the channel for `Game::step`.
    ///
    /// `session` says whether the turn is worth taking: one costs this CPU 4 to 6 ms however little
    /// changed, so an idle guest is turned only when there is news or a button it listens to moved.
    pub unsafe fn turn(&mut self, dt: f32, buttons: u32, analog: u32, session: &Session) {
        self.latched |= buttons;
        self.owed = (self.owed + dt).min(2.0 * TURN);
        let Some((rt, ctx, global, frame)) = self.guest else { return };
        // A turn takes two frames: the script on one, layout and the list of what it shows on the
        // next. Each half is several milliseconds of this CPU, and the scene's frame has room for one.
        if self.laid_out {
            self.laid_out = false;
            let start = sceKernelGetSystemTimeLow();
            let core = ffi::ui();
            for _ in 0..TICKS {
                core.tick();
            }
            let list = core.draw();
            self.words = (list.words.as_ptr(), list.words.len());
            let layout = sceKernelGetSystemTimeLow().wrapping_sub(start) as f32 / 1000.0;
            if self.turns > 8 {
                self.layout_ms += (layout - self.layout_ms) * 0.1;
                self.worst_ms = self.worst_ms.max(layout);
            }
            return;
        }
        if self.owed < TURN {
            return;
        }
        self.owed -= TURN;
        if !self.pace.due(session, self.latched, false) && self.presses.is_empty() {
            self.latched = 0;
            return;
        }
        let start = sceKernelGetSystemTimeLow();
        let mut buttons = core::mem::take(&mut self.latched);
        if let Some((pressed, turns)) = self.presses.front_mut() {
            *turns -= 1;
            // The last turn of a press is the one that lets go.
            if *turns > 0 || *pressed == 0 {
                buttons |= *pressed;
            }
            if *turns == 0 {
                self.presses.pop_front();
            }
        }
        let mut arguments = [JS_NewInt32(ctx, buttons as i32), JS_NewInt32(ctx, analog as i32)];
        let result = JS_Call(ctx, frame, global, 2, arguments.as_mut_ptr());
        if JS_ValueGetTag(result) == JS_TAG_EXCEPTION {
            self.error = exception(ctx);
        }
        JS_FreeValue(ctx, result);
        host::drain_jobs(rt);
        // Collect when a turn left the arena a quarter megabyte higher (as PocketJS's own host
        // does): a steady interface never does.
        let bump = arena::stats().bump_bytes;
        if bump > self.collected + 256 * 1024 {
            JS_RunGC(rt);
            self.collected = arena::stats().bump_bytes;
        }
        let script = sceKernelGetSystemTimeLow().wrapping_sub(start) as f32 / 1000.0;
        self.laid_out = true;
        self.turns += 1;
        // The first turns mount the screen; the figures are for the ones after.
        if self.turns > 8 {
            self.script_ms += (script - self.script_ms) * 0.1;
            self.worst_ms = self.worst_ms.max(script);
        }
    }

    /// Draws the interface into the open display list, over what is there. The list's vertices come
    /// from the host library's pool: `ge::reset_pool` once the GE has drawn them.
    pub unsafe fn draw(&self) {
        if self.guest.is_some() && self.words.1 > 0 {
            ge::render_over(ffi::ui(), core::slice::from_raw_parts(self.words.0, self.words.1));
        }
    }

    /// The length of the list it last drew, in 32-bit words.
    pub fn words(&self) -> usize {
        self.words.1
    }

    /// Bytes QuickJS holds for the guest.
    pub unsafe fn script_bytes(&self) -> usize {
        if self.guest.is_some() {
            qjs_alloc::stats().live_requested
        } else {
            0
        }
    }
}
