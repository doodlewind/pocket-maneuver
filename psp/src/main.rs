//! Pocket Maneuver on PSP.
//!
//! The same simulation as the reference and the Vita build (`maneuver-sim`),
//! the same world, lowered by the world compiler for this machine
//! (`profiles/psp60.json`): 480 × 272, the GE's fixed-function pipeline,
//! 24 MB of memory, one analog stick.
//!
//! Controls in play: the stick moves, the direction pad turns the camera
//! (left alone, it follows the direction of travel), L and R fire the wires,
//! cross is gas, square cuts, triangle fires an aimed pair, circle lets go.
//! Everything else on the pad is the interface's (`interface`): a PocketJS
//! guest shared with the other devices, drawn over the scene. Without it
//! (its files are missing, or a 24 MB machine has no room for it beside the
//! world) START switches the autopilot and SELECT restarts.
//!
//! Development loop over PSPLINK: the pack and the interface's bundle are
//! read from `host0:/maneuver/`, `control.txt` there steers the run and
//! `status.json` reports it.

#![no_std]
#![no_main]

extern crate alloc;

mod audio;
mod gfx;
mod interface;
mod mem;
mod store;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::fmt::Write;
use core::mem::ManuallyDrop;

use maneuver_handheld::game::{pad, parse_f32, Game, Pad, Perf, Timing};
use maneuver_handheld::scene::Scene;
use maneuver_handheld::world::World;
use maneuver_interface::{channel, Mode};
use maneuver_pack::{self as pack, HandMesh, HandScene, PspVertex};
use maneuver_sim::collide::Tri;
use maneuver_sim::sim::btn;
use maneuver_sim::worldfile::{Built, HEADER};
use pocketjs_psp::{arena, host};
use psp::sys::*;

psp::module!("PocketManeuver", 1, 0);

fn psp_main() {
    psp::enable_home_button();
    unsafe {
        // This thread has the EBOOT's directory as its working directory, and no other thread does:
        // it opens what lies there, and once the frame thread runs it serves storage below it.
        store::locate();
        // The frame thread, with this thread's attributes and four times its stack: the interface's
        // guest compiles its bundle and mounts its screens on the thread that runs it, and that
        // takes more than 256 KB.
        let id = sceKernelCreateThread(b"maneuver_frame\0".as_ptr(), frame_thread, 32, 1024 * 1024, ThreadAttributes::USER | ThreadAttributes::VFPU, core::ptr::null_mut());
        if id.0 < 0 {
            psp::dprintln!("Could not start: the frame thread was not created ({:08x})", id.0 as u32);
            loop {
                sceKernelDelayThread(1_000_000);
            }
        }
        sceKernelStartThread(id, 0, core::ptr::null_mut());
        sceKernelChangeThreadPriority(SceUid(sceKernelGetThreadId()), 40);
        store::serve()
    }
}

unsafe extern "C" fn frame_thread(_: usize, _: *mut c_void) -> i32 {
    start();
    0
}

unsafe fn start() {
    // PSPLINK can start a thread with floating-point exceptions on; the interface's layout computes with NaN.
    host::reset_fpu_status();
    let mut ui = interface::Ui::none("");
    let mut on_screen = false;
    if let Err(e) = run(&mut ui, &mut on_screen) {
        store::note(&format!("{{\"target\":\"psp\",\"stage\":\"failed\",\"error\":\"{e}\",\"code\":\"{:08x}\"}}", core::ptr::addr_of!(store::LAST_CODE).read() as u32));
        if !on_screen {
            psp::dprintln!("Could not start: {}", e);
        }
        let state = &mut channel().state;
        state.mode = Mode::Error;
        state.message.clear();
        state.message.push_str(e);
        let mut frames = 0u32;
        // No game to pace the guest by: it takes every turn.
        let waking = maneuver_interface::Session::new();
        loop {
            if on_screen {
                // A turn's two halves: its script, then its layout.
                ui.turn(interface::TURN, 0, interface::STICK_CENTER, &waking);
                ui.turn(0.0, 0, interface::STICK_CENTER, &waking);
                gfx::interlude(&ui);
                frames += 1;
                // A test run (`boot.txt` names a frame to leave at) ends here, with this screen as its frame.
                let mut buf = [0u8; 512];
                if frames == 30 && store::boot_text(&mut buf).is_some_and(|text| text.contains("exit=")) {
                    store::write_shot(&gfx::pixels(gfx::DRAW_BUFFER ^ 1));
                    sceKernelExitGame();
                }
            } else {
                sceKernelDelayThread(1_000_000);
            }
        }
    }
}

/// A vector of `n` records read straight from the file into memory of exactly their size, which keeps
/// them for the rest of the run (`mem::vec`). The allocator did not hand that memory out, so the
/// vector is wrapped against a drop on the way out of a failed load.
unsafe fn read_vec<T: Copy>(file: &store::PackFile, tag: u32, offset: usize, n: usize) -> Result<ManuallyDrop<Vec<T>>, &'static str> {
    let mut v = ManuallyDrop::new(mem::vec::<T>(n).ok_or("no memory for the collision world")?);
    file.read_into(tag, offset, v.as_mut_ptr().cast(), n * core::mem::size_of::<T>())?;
    v.set_len(n);
    Ok(v)
}

unsafe fn load_sim(file: &store::PackFile) -> Result<maneuver_sim::Sim, &'static str> {
    let mut head = [0u8; HEADER];
    file.read_into(pack::SIMG, 0, head.as_mut_ptr(), HEADER)?;
    let h = Built::parse(&head)?;
    let [a, b, c] = h.array_bytes();
    let tris = read_vec::<Tri>(file, pack::SIMG, HEADER, h.tris)?;
    let start = read_vec::<u32>(file, pack::SIMG, HEADER + a, h.cells() + 1)?;
    let items = read_vec::<u32>(file, pack::SIMG, HEADER + a + b, h.items)?;
    // The giants, the depots and the route are parsed into their own lists: this buffer is given back.
    let mut entities = alloc::vec![0u8; h.entity_bytes()];
    file.read_into(pack::SIMG, HEADER + a + b + c, entities.as_mut_ptr(), entities.len())?;
    let sim = h.finish(ManuallyDrop::into_inner(tris), ManuallyDrop::into_inner(start), ManuallyDrop::into_inner(items), &entities);
    if sim.is_err() {
        // The three arrays were dropped with the world that failed, and the allocator now lists memory
        // it never handed out among its free blocks. Asking for the same sizes takes those blocks off
        // its lists again, for good: the screen that reports the failure still allocates.
        for bytes in [a, b, c] {
            core::mem::forget(Vec::<u8>::with_capacity(bytes));
        }
    }
    sim
}

fn read_pad(data: &SceCtrlData) -> Pad {
    let b = data.buttons;
    let mut buttons = 0;
    for (from, to) in [
        (CtrlButtons::LTRIGGER, btn::HOOK_L),
        (CtrlButtons::RTRIGGER, btn::HOOK_R),
        (CtrlButtons::CROSS, btn::GAS),
        (CtrlButtons::SQUARE, btn::SLASH),
        (CtrlButtons::TRIANGLE, btn::ZIP),
        (CtrlButtons::CIRCLE, btn::DROP),
        (CtrlButtons::START, pad::START),
        (CtrlButtons::SELECT, pad::SELECT),
    ] {
        if b.contains(from) {
            buttons |= to;
        }
    }
    let axis = |v: u8| (v as f32 - 127.5) / 127.5;
    let dir = |neg: CtrlButtons, pos: CtrlButtons| (b.contains(pos) as i32 - b.contains(neg) as i32) as f32 * 0.8;
    Pad { buttons, lx: axis(data.lx), ly: -axis(data.ly), rx: dir(CtrlButtons::LEFT, CtrlButtons::RIGHT), ry: dir(CtrlButtons::DOWN, CtrlButtons::UP) }
}

/// The Pocket3D title card, drawn into video memory before the GE is set up.
unsafe fn title() {
    // the uncached mirror of video memory: what is written is what the display reads
    let vram = (sceGeEdramGetAddr() as usize | 0x4000_0000) as *mut u8;
    sceDisplaySetMode(DisplayMode::Lcd, 480, 272);
    let mut surface = pocket3d_title::Surface {
        pixels: core::slice::from_raw_parts_mut(vram, 512 * 272 * 4),
        width: 480,
        height: 272,
        stride: 512,
        layout: pocket3d_title::Layout::Rgba8,
    };
    pocket3d_title::play(&mut surface, |_| {
        sceDisplaySetFrameBuf(vram, 512, DisplayPixelFormat::Psm8888, DisplaySetBufSync::NextFrame);
        sceDisplayWaitVblankStart();
    });
}

/// The words of a control text that are the interface's: `press=<mask>` presses buttons on it as a
/// thumb would (PocketJS's button bits, the pad's own), `rest=<turns>` leaves the pad alone that
/// many of its turns before the next press, and `ui=start|pause|resume|restart|title` asks what its
/// menus ask.
unsafe fn remote(text: &str, ui: &mut interface::Ui) {
    for word in text.split_ascii_whitespace() {
        match word.split_once('=') {
            Some(("press", mask)) => {
                if let Some(mask) = parse_f32(mask) {
                    ui.press(mask as u32);
                }
            }
            Some(("rest", turns)) => {
                if let Some(turns) = parse_f32(turns) {
                    ui.rest(turns as u32);
                }
            }
            Some(("ui", what)) => channel().receive(match what {
                "start" => r#"{"type":"start"}"#,
                "pause" => r#"{"type":"pause","on":true}"#,
                "resume" => r#"{"type":"pause","on":false}"#,
                "restart" => r#"{"type":"restart"}"#,
                "title" => r#"{"type":"title"}"#,
                _ => continue,
            }),
            _ => {}
        }
    }
}

unsafe fn run(ui: &mut interface::Ui, on_screen: &mut bool) -> Result<(), &'static str> {
    scePowerSetClockFrequency(333, 333, 166);
    title();
    psp::dprintln!("Pocket Maneuver\n");
    let t_load = sceKernelGetSystemTimeLow();
    let free_at_start = sceKernelTotalFreeMemSize();
    let file = store::PackFile::open()?;

    // The interface starts first when memory has room for it beside the world: the arena's tail and
    // what the kernel still has, against the pack's resident sections, the world's working memory
    // (its tables, the game, the atlas on its way to video memory) and what the interface takes. A
    // PSP-1000's 24 MB hold the world alone; the pad then keeps the game's flow itself.
    let room = arena::stats().tail_free_bytes + mem::kernel_room();
    let world_bytes = file.resident_bytes() + 3 * 512 * 1024;
    if room < world_bytes + interface::RESERVE {
        *ui = interface::Ui::none("no memory for the interface beside the world");
        store::leave_beside(store::SCRIPT);
        store::leave_beside(store::PAK);
    } else {
        psp::dprintln!("  Starting the interface");
        *ui = interface::Ui::boot(store::read_beside(store::SCRIPT, &[0]), store::keep_beside(store::PAK));
    }
    let arena_after_ui = arena::stats().bump_bytes;
    if ui.up() {
        // From here the screen belongs to the GE, and the loading steps to the interface.
        gfx::init();
        *on_screen = true;
    }
    // While the world loads the guest takes every turn it is offered.
    let waking = maneuver_interface::Session::new();
    let mut stage = |name: &str| {
        if ui.up() {
            let state = &mut channel().state;
            state.message.clear();
            state.message.push_str(name);
            ui.turn(interface::TURN, 0, interface::STICK_CENTER, &waking);
            ui.turn(0.0, 0, interface::STICK_CENTER, &waking);
            gfx::interlude(ui);
        } else {
            psp::dprintln!("  {}", name);
        }
        if file.host {
            store::note(&format!("{{\"target\":\"psp\",\"stage\":\"loading\",\"step\":\"{name}\"}}"));
        }
    };

    stage("Reading the world");
    let hs: Vec<HandScene> = file.records(pack::HSCN)?;
    let scene = Scene::new(*hs.first().ok_or("scene section")?);
    if scene.h.near_streamed != 1 {
        return Err("the pack is not a PSP pack");
    }
    let recs: Vec<HandMesh> = file.records(pack::HMSH)?;
    let per = libm::roundf(scene.h.super_cell / scene.h.cell).max(1.0) as i32;
    let world = World::new(recs, per, true, core::mem::size_of::<PspVertex>() as u32);
    let largest = world.cells.iter().map(|c| c.blob.1 as usize).max().unwrap_or(0);

    let mut gfx = gfx::Gfx::load(&file, &scene, &mut stage)?;
    stage("Reading the collision grid");
    let sim = load_sim(&file)?;
    stage("Starting");
    store::start(&file, largest)?;
    let sound = audio::start();
    if !*on_screen {
        // No interface: the debug text stood until here.
        gfx::init();
        *on_screen = true;
    }
    let mut game = Game::new(sim, scene);
    // The numbers in flight ten times a second: each refresh is a turn of the guest, 4 to 6 ms here.
    game.session.numbers_every = 6;
    game.set.stats = file.host;
    {
        // What the interface asked to have kept in an earlier run.
        let mut buf = [0u8; store::KEPT_MAX];
        if let Some(text) = store::kept(&mut buf) {
            game.stored(text);
        }
    }
    // Test runs: `boot.txt` on the share holds control words for the start, plus `shot=N` (write frame N
    // to `shot.raw`) and `exit=N` (leave at frame N).
    let (mut shot_at, mut exit_at) = (u32::MAX, u32::MAX);
    if file.host {
        let mut buf = [0u8; 512];
        if let Some(text) = store::boot_text(&mut buf) {
            game.control(text);
            remote(text, ui);
            for word in text.split_ascii_whitespace() {
                match word.split_once('=') {
                    Some(("shot", n)) => shot_at = parse_f32(n).unwrap_or(-1.0) as u32,
                    Some(("exit", n)) => exit_at = parse_f32(n).unwrap_or(-1.0) as u32,
                    _ => {}
                }
            }
        }
    }
    let load_ms = sceKernelGetSystemTimeLow().wrapping_sub(t_load) / 1000;
    let free_after = sceKernelTotalFreeMemSize();
    let arena_after_load = arena::stats().bump_bytes;

    sceCtrlSetSamplingCycle(0);
    sceCtrlSetSamplingMode(CtrlMode::Analog);
    let mut data: SceCtrlData = core::mem::zeroed();
    let mut pcm = alloc::vec![0i16; 2048];
    let mut wanted: Vec<(f32, u32)> = Vec::with_capacity(64);
    let mut status = String::with_capacity(4096);
    let mut extra = String::with_capacity(2048);
    let mut timing = Timing::new();
    let mut perf = Perf::default();
    let (mut sim_ms, mut build_ms, mut gpu_ms, mut audio_ms, mut ui_ms) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut phase_ms = [0.0f32; 8];
    let mut last_swap = sceKernelGetSystemTimeLow();
    let mut last_vcount = sceDisplayGetVcount();
    let mut ticks = 1u32;
    let ms = |from: u32| sceKernelGetSystemTimeLow().wrapping_sub(from) as f32 / 1000.0;

    loop {
        // -------------------------------------------------------------- input and simulation
        sceCtrlPeekBufferPositive(&mut data, 1);
        store::control(|text| {
            game.control(text);
            remote(text, ui);
        });
        let t0 = sceKernelGetSystemTimeLow();
        game.step(&read_pad(&data), ticks);
        sim_ms = sim_ms * 0.9 + ms(t0) * 0.1;
        // What the interface asked to have kept goes to the reader thread, which writes it beside the pack.
        if let Some(text) = game.prefs.take() {
            if !store::keep(&text) {
                game.prefs = Some(text);
            }
        }
        // The interface's turn, while the GE draws the previous frame: the pad as PocketJS's hosts
        // pass it (the pad's own bits, the stick packed x then y).
        let tu = sceKernelGetSystemTimeLow();
        // `option=16` withholds the turn, for measuring what it costs.
        if game.set.option & 16 == 0 {
            ui.turn(ticks as f32 / 60.0, data.buttons.bits(), (data.lx as u32) << 8 | data.ly as u32, &game.session);
        }
        ui_ms = ui_ms * 0.9 + ms(tu) * 0.1;
        let ta = sceKernelGetSystemTimeLow();
        if sound && audio::running() && game.audible() {
            let want = audio::wanted();
            if want > 0 {
                game.synth.render(&mut pcm[..want * 2], audio::RATE);
                audio::push(&pcm[..want * 2]);
            }
        }
        audio_ms = audio_ms * 0.9 + ms(ta) * 0.1;
        for (slot, us) in phase_ms.iter_mut().zip(gfx.stats.phase) {
            *slot = *slot * 0.9 + us as f32 * 0.0001;
        }

        // -------------------------------------------------------------- the previous frame leaves the GE
        let t1 = sceKernelGetSystemTimeLow();
        sceGuSync(GuSyncMode::Finish, GuSyncBehavior::Wait);
        gpu_ms = gpu_ms * 0.9 + ms(t1) * 0.1;
        // The GE has read the vertices of the interface's last list.
        pocketjs_psp::ge::reset_pool();
        if game.frame == shot_at.wrapping_add(1) {
            store::write_shot(&gfx::pixels(gfx::DRAW_BUFFER));
        }
        if game.frame >= exit_at {
            status.clear();
            extra.clear();
            let a = arena::stats();
            let _ = write!(
                extra,
                "\"exited\":true,\"clip\":{{\"tested\":{},\"cut\":{}}},\"meshes\":{{\"near\":{},\"mid\":{},\"far\":{},\"waiting\":{}}},\"cells\":{},\"memory\":{{\"freeAtStart\":{},\"freeAfterLoad\":{},\"free\":{},\"kernelBlocks\":{},\"arena\":{{\"capacity\":{},\"used\":{},\"afterInterface\":{},\"afterLoad\":{}}}}},\"interface\":{{\"up\":{},\"error\":\"{}\",\"turns\":{},\"turnMs\":{{\"script\":{:.2},\"layout\":{:.2},\"worst\":{:.2}}},\"words\":{},\"scriptBytes\":{}}},\"phaseMs\":{{\"marks\":{:.2},\"ui\":{:.2}}}",
                gfx.stats.tested,
                gfx.stats.clipped,
                gfx.picked.near,
                gfx.picked.mid,
                gfx.picked.far,
                gfx.picked.waiting,
                store::LOADED.load(core::sync::atomic::Ordering::Relaxed),
                free_at_start,
                free_after,
                sceKernelTotalFreeMemSize(),
                core::ptr::addr_of!(mem::KERNEL_BYTES).read(),
                a.capacity_bytes,
                a.bump_bytes,
                arena_after_ui,
                arena_after_load,
                ui.up(),
                ui.error,
                ui.turns,
                ui.script_ms,
                ui.layout_ms,
                ui.worst_ms,
                ui.words(),
                ui.script_bytes(),
                phase_ms[6],
                phase_ms[7]
            );
            game.status(&mut status, "psp", &perf, &extra);
            // Let the reader thread finish a status write of its own first.
            sceKernelDelayThread(300_000);
            store::note(&status);
            sceKernelExitGame();
        }
        // Present on a display refresh: wait for the next one unless one has passed since the last frame.
        if sceDisplayGetVcount() == last_vcount {
            sceDisplayWaitVblankStart();
        }
        gfx::swap();
        let vcount = sceDisplayGetVcount();
        // One tick per display refresh: a late frame catches up.
        ticks = vcount.wrapping_sub(last_vcount).clamp(1, 3);
        last_vcount = vcount;
        let now = sceKernelGetSystemTimeLow();
        timing.push(now.wrapping_sub(last_swap) as f32 / 1000.0, ticks);
        last_swap = now;

        // The GE is idle: cells may change buffers.
        let cam = game.camera();
        store::update(&world, cam.eye, game.set.lod_near + 26.0, game.frame, &mut wanted);

        // -------------------------------------------------------------- this frame's list
        let t2 = sceKernelGetSystemTimeLow();
        perf = Perf { frame: timing.avg(), worst: timing.worst(), late: timing.late, frames: timing.frames, sim: sim_ms, build: build_ms, draw: 0.0, gpu: gpu_ms, draws: perf.draws, tris: perf.tris };
        gfx.frame(&mut game, &world, ticks, &perf, ui);
        perf.draws = gfx.stats.draws;
        perf.tris = gfx.stats.tris;
        build_ms = build_ms * 0.9 + ms(t2) * 0.1;

        if file.host && game.frame % 30 == 0 {
            status.clear();
            extra.clear();
            let a = arena::stats();
            let _ = write!(
                extra,
                "\"loadMs\":{},\"pack\":{{\"bytes\":{},\"resident\":{}}},\"memory\":{{\"freeAtStart\":{},\"freeAfterLoad\":{},\"free\":{},\"kernelBlocks\":{},\"arena\":{{\"capacity\":{},\"used\":{},\"afterInterface\":{},\"afterLoad\":{}}}}},\"cells\":{{\"loaded\":{},\"bytes\":{}}},\"clip\":{{\"tested\":{},\"cut\":{}}},\"meshes\":{{\"near\":{},\"mid\":{},\"far\":{},\"waiting\":{}}},\"sound\":{},\"audioMs\":{:.2},\"ticks\":{},",
                load_ms,
                file.bytes,
                gfx.resident_bytes,
                free_at_start,
                free_after,
                sceKernelTotalFreeMemSize(),
                core::ptr::addr_of!(mem::KERNEL_BYTES).read(),
                a.capacity_bytes,
                a.bump_bytes,
                arena_after_ui,
                arena_after_load,
                store::LOADED.load(core::sync::atomic::Ordering::Relaxed),
                store::LOADED_BYTES.load(core::sync::atomic::Ordering::Relaxed),
                gfx.stats.tested,
                gfx.stats.clipped,
                gfx.picked.near,
                gfx.picked.mid,
                gfx.picked.far,
                gfx.picked.waiting,
                sound && audio::running(),
                audio_ms,
                ticks
            );
            // The interface: a turn's parts (30 turns a second), its cost per frame, the length of its list.
            let _ = write!(
                extra,
                "\"interface\":{{\"up\":{},\"error\":\"{}\",\"turns\":{},\"turnMs\":{{\"script\":{:.2},\"layout\":{:.2},\"worst\":{:.2}}},\"cpuMs\":{:.2},\"words\":{},\"scriptBytes\":{}}},",
                ui.up(),
                ui.error,
                ui.turns,
                ui.script_ms,
                ui.layout_ms,
                ui.worst_ms,
                ui_ms,
                ui.words(),
                ui.script_bytes()
            );
            let _ = write!(
                extra,
                "\"phaseMs\":{{\"pick\":{:.2},\"far\":{:.2},\"farGiants\":{:.2},\"models\":{:.2},\"near\":{:.2},\"actors\":{:.2},\"marks\":{:.2},\"ui\":{:.2}}}",
                phase_ms[0], phase_ms[1], phase_ms[2], phase_ms[3], phase_ms[4], phase_ms[5], phase_ms[6], phase_ms[7]
            );
            game.status(&mut status, "psp", &perf, &extra);
            store::publish(&status);
        }
    }
}
