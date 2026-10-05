//! Pocket Maneuver on PSP.
//!
//! The same simulation as the reference and the Vita build (`maneuver-sim`),
//! the same world, lowered by the world compiler for this machine
//! (`profiles/psp60.json`): 480 × 272, the GE's fixed-function pipeline,
//! 24 MB of memory, one analog stick.
//!
//! Controls: the stick moves, the direction pad turns the camera (left alone,
//! it follows the direction of travel), L and R fire the wires, cross is gas,
//! square cuts, triangle fires an aimed pair, circle lets go. START switches
//! the autopilot, SELECT restarts.
//!
//! Development loop over PSPLINK: the pack is read from `host0:/maneuver/`,
//! `control.txt` there steers the run and `status.json` reports it.

#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

mod allocator;
mod audio;
mod gfx;
mod store;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use maneuver_handheld::game::{pad, Game, Pad, Perf, Timing};
use maneuver_handheld::scene::Scene;
use maneuver_handheld::world::World;
use maneuver_pack::{self as pack, HandMesh, HandScene, PspVertex};
use maneuver_sim::collide::Tri;
use maneuver_sim::sim::btn;
use maneuver_sim::worldfile::{Built, HEADER};
use psp::sys::*;

psp::module!("PocketManeuver", 1, 0);

const HELP: &str = "L / R wires   X gas   SQUARE cut   TRIANGLE aimed wires   CIRCLE let go";

fn psp_main() {
    psp::enable_home_button();
    unsafe {
        if let Err(e) = run() {
            psp::dprintln!("Could not start: {}", e);
            store::note(&format!("{{\"target\":\"psp\",\"stage\":\"failed\",\"error\":\"{e}\",\"code\":\"{:08x}\"}}", core::ptr::addr_of!(store::LAST_CODE).read() as u32));
            loop {
                sceKernelDelayThread(1_000_000);
            }
        }
    }
}

/// A vector of `n` records read straight from the file into the memory that keeps them.
unsafe fn read_vec<T: Copy>(file: &store::PackFile, tag: u32, offset: usize, n: usize) -> Result<Vec<T>, &'static str> {
    let mut v = Vec::<T>::with_capacity(n.max(1));
    file.read_into(tag, offset, v.as_mut_ptr().cast(), n * core::mem::size_of::<T>())?;
    v.set_len(n);
    Ok(v)
}

unsafe fn load_sim(file: &store::PackFile) -> Result<maneuver_sim::Sim, &'static str> {
    let mut head = [0u8; HEADER];
    file.read_into(pack::SIMG, 0, head.as_mut_ptr(), HEADER)?;
    let h = Built::parse(&head)?;
    let [a, b, c] = h.array_bytes();
    let tris: Vec<Tri> = read_vec(file, pack::SIMG, HEADER, h.tris)?;
    let start: Vec<u32> = read_vec(file, pack::SIMG, HEADER + a, h.cells() + 1)?;
    let items: Vec<u32> = read_vec(file, pack::SIMG, HEADER + a + b, h.items)?;
    let entities: Vec<u8> = read_vec(file, pack::SIMG, HEADER + a + b + c, h.entity_bytes())?;
    h.finish(tris, start, items, &entities)
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

unsafe fn run() -> Result<(), &'static str> {
    scePowerSetClockFrequency(333, 333, 166);
    title();
    psp::dprintln!("Pocket Maneuver\n");
    let t_load = sceKernelGetSystemTimeLow();
    let free_at_start = sceKernelTotalFreeMemSize();
    let file = store::PackFile::open()?;
    let mut stage = |name: &str| {
        psp::dprintln!("  {}", name);
        if file.host {
            store::note(&format!("{{\"target\":\"psp\",\"stage\":\"loading\",\"step\":\"{name}\"}}"));
        }
    };

    stage("scene");
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
    // From here the screen belongs to the GE; messages go to the computer only.
    let stage = |name: &str| {
        if file.host {
            store::note(&format!("{{\"target\":\"psp\",\"stage\":\"loading\",\"step\":\"{name}\"}}"));
        }
    };
    stage("collision");
    let sim = load_sim(&file)?;
    stage("start");
    store::start(&file, largest)?;
    let sound = audio::start();
    let mut game = Game::new(sim, scene, HELP);
    game.set.stats = file.host;
    // Test runs: `boot.txt` on the share holds control words for the start, plus `shot=N` (write frame N
    // to `shot.raw`) and `exit=N` (leave at frame N).
    let (mut shot_at, mut exit_at) = (u32::MAX, u32::MAX);
    if file.host {
        let mut buf = [0u8; 512];
        if let Some(text) = store::boot_text(&mut buf) {
            game.control(text);
            for word in text.split_ascii_whitespace() {
                match word.split_once('=') {
                    Some(("shot", n)) => shot_at = maneuver_handheld::game::parse_f32(n).unwrap_or(-1.0) as u32,
                    Some(("exit", n)) => exit_at = maneuver_handheld::game::parse_f32(n).unwrap_or(-1.0) as u32,
                    _ => {}
                }
            }
        }
    }
    // Which of the two frame buffers the list in flight draws into.
    let mut drawing = 0usize;
    let load_ms = sceKernelGetSystemTimeLow().wrapping_sub(t_load) / 1000;
    let free_after = sceKernelTotalFreeMemSize();

    sceCtrlSetSamplingCycle(0);
    sceCtrlSetSamplingMode(CtrlMode::Analog);
    let mut data: SceCtrlData = core::mem::zeroed();
    let mut pcm = alloc::vec![0i16; 2048];
    let mut wanted: Vec<(f32, u32)> = Vec::with_capacity(64);
    let mut status = String::with_capacity(2048);
    let mut extra = String::with_capacity(512);
    let mut timing = Timing::new();
    let mut perf = Perf::default();
    let (mut sim_ms, mut build_ms, mut gpu_ms, mut audio_ms) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut phase_ms = [0.0f32; 7];
    let mut last_swap = sceKernelGetSystemTimeLow();
    let mut last_vcount = sceDisplayGetVcount();
    let mut ticks = 1u32;
    let ms = |from: u32| sceKernelGetSystemTimeLow().wrapping_sub(from) as f32 / 1000.0;

    loop {
        // -------------------------------------------------------------- input and simulation
        sceCtrlPeekBufferPositive(&mut data, 1);
        store::control(|text| game.control(text));
        let t0 = sceKernelGetSystemTimeLow();
        game.step(&read_pad(&data), ticks);
        sim_ms = sim_ms * 0.9 + ms(t0) * 0.1;
        let ta = sceKernelGetSystemTimeLow();
        if sound && audio::running() {
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
        if game.frame == shot_at.wrapping_add(1) {
            // The frame just drawn, out of video memory first: host I/O cannot take a video memory address.
            let vram = sceGeEdramGetAddr().add(drawing * gfx::FB_BYTES);
            let mut pixels = alloc::vec![0u8; 480 * 272 * 2];
            for y in 0..272 {
                core::ptr::copy_nonoverlapping(vram.add(y * 512 * 2), pixels.as_mut_ptr().add(y * 480 * 2), 480 * 2);
            }
            store::write_shot(&pixels);
        }
        if game.frame >= exit_at {
            status.clear();
            extra.clear();
            let _ = write!(extra, "\"exited\":true,\"clip\":{{\"tested\":{},\"cut\":{}}},\"meshes\":{{\"near\":{},\"mid\":{},\"far\":{},\"waiting\":{}}},\"cells\":{}", gfx.stats.tested, gfx.stats.clipped, gfx.picked.near, gfx.picked.mid, gfx.picked.far, gfx.picked.waiting, store::LOADED.load(core::sync::atomic::Ordering::Relaxed));
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
        sceGuSwapBuffers();
        drawing ^= 1;
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
        gfx.frame(&mut game, &world, ticks, &perf);
        perf.draws = gfx.stats.draws;
        perf.tris = gfx.stats.tris;
        build_ms = build_ms * 0.9 + ms(t2) * 0.1;

        if file.host && game.frame % 30 == 0 {
            status.clear();
            extra.clear();
            let _ = write!(
                extra,
                "\"loadMs\":{},\"pack\":{{\"bytes\":{},\"resident\":{}}},\"memory\":{{\"freeAtStart\":{},\"freeAfterLoad\":{},\"free\":{},\"small\":{},\"large\":{}}},\"cells\":{{\"loaded\":{},\"bytes\":{}}},\"clip\":{{\"tested\":{},\"cut\":{}}},\"meshes\":{{\"near\":{},\"mid\":{},\"far\":{},\"waiting\":{}}},\"sound\":{},\"audioMs\":{:.2},\"ticks\":{},\"phaseMs\":{{\"pick\":{:.2},\"far\":{:.2},\"farGiants\":{:.2},\"models\":{:.2},\"near\":{:.2},\"actors\":{:.2},\"hud\":{:.2}}}",
                load_ms,
                file.bytes,
                gfx.resident_bytes,
                free_at_start,
                free_after,
                sceKernelTotalFreeMemSize(),
                allocator::arena_used(),
                core::ptr::addr_of!(allocator::LARGE_BYTES).read(),
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
                ticks,
                phase_ms[0],
                phase_ms[1],
                phase_ms[2],
                phase_ms[3],
                phase_ms[4],
                phase_ms[5],
                phase_ms[6]
            );
            game.status(&mut status, "psp", &perf, &extra);
            store::publish(&status);
        }
    }
}
