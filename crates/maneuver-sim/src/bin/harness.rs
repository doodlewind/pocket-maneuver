//! Flies the autopilot through a world and prints what happened. With no
//! argument it uses the synthetic test town; otherwise the path of a world file.
//!
//! `cargo run --release -p maneuver-sim --bin harness -- [world.mvsw] [seconds] [--trace]`

use maneuver_sim::sim::{ev, hook};
use maneuver_sim::{testworld, worldfile};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let trace = args.iter().any(|a| a == "--trace");
    let skip = args.iter().position(|a| a == "--wav").map(|i| i + 1);
    let pos: Vec<&String> = args.iter().enumerate().filter(|(i, a)| !a.starts_with("--") && Some(*i) != skip).map(|(_, a)| a).collect();
    let bytes = match pos.first() {
        Some(p) if p.as_str() != "-" => std::fs::read(p.as_str()).expect("read world file"),
        _ => testworld::build(),
    };
    let seconds: f32 = pos.get(1).and_then(|s| s.parse().ok()).unwrap_or(120.0);
    let t0 = std::time::Instant::now();
    let mut sim = worldfile::load(&bytes).expect("load world");
    let load_ms = t0.elapsed().as_secs_f32() * 1000.0;
    let ticks = (seconds * 60.0) as u32;
    let (mut sum_speed, mut max_speed) = (0.0f32, 0.0f32);
    let (mut sum_alt, mut min_alt, mut max_alt) = (0.0f32, f32::MAX, f32::MIN);
    let (mut grounded, mut hooked) = (0u32, 0u32);
    let mut count = [0u32; 18];
    let mut dist = 0.0f32;
    let mut last = sim.p.pos;
    // `--wav <file>` renders the run's sound at 22.05 kHz.
    let wav = args.iter().position(|a| a == "--wav").and_then(|i| args.get(i + 1)).cloned();
    let mut synth = maneuver_sim::audio::Synth::new();
    let mut pcm: Vec<i16> = Vec::new();
    let t1 = std::time::Instant::now();
    for t in 0..ticks {
        let input = sim.auto_input();
        sim.tick(input);
        if wav.is_some() {
            synth.control(&sim, sim.events);
            let at = pcm.len();
            // 367.5 frames per tick at 22.05 kHz: alternate 367 and 368.
            pcm.resize(at + (367 + (t as usize & 1)) * 2, 0);
            synth.render(&mut pcm[at..], 22050.0);
        }
        let s = sim.speed();
        sum_speed += s;
        max_speed = max_speed.max(s);
        sum_alt += sim.p.pos.y;
        min_alt = min_alt.min(sim.p.pos.y);
        max_alt = max_alt.max(sim.p.pos.y);
        grounded += sim.p.grounded as u32;
        hooked += sim.p.hooks.iter().any(|h| h.state == hook::ATTACHED) as u32;
        dist += (sim.p.pos - last).len();
        last = sim.p.pos;
        for (i, c) in count.iter_mut().enumerate() {
            *c += (sim.events >> i) & 1;
        }
        if trace && t % 6 == 0 {
            println!(
                "{:6} pos {:8.1} {:6.1} {:8.1} v {:5.1} act {} gas {:5.1} wp {} hooks {}{} btn {:02x}",
                t, sim.p.pos.x, sim.p.pos.y, sim.p.pos.z, s, sim.p.act, sim.p.gas, sim.auto.wp, sim.p.hooks[0].state, sim.p.hooks[1].state, input.buttons
            );
        }
    }
    let us = t1.elapsed().as_secs_f32() * 1e6 / ticks as f32;
    let n = ticks as f32;
    println!("world: {} tris, loaded in {load_ms:.1} ms; {ticks} ticks at {us:.1} us/tick", sim.world.tris.len());
    println!("speed: avg {:.1} m/s, max {:.1} m/s; distance {:.0} m", sum_speed / n, max_speed, dist);
    println!("altitude: avg {:.1}, min {:.1}, max {:.1}", sum_alt / n, min_alt, max_alt);
    println!("grounded {:.0}%  hooked {:.0}%  laps {}  waypoint {}", grounded as f32 / n * 100.0, hooked as f32 / n * 100.0, sim.auto.laps, sim.auto.wp);
    let names = ["jump", "fireL", "fireR", "attachL", "attachR", "release", "burst", "slash", "cut", "weak", "land", "landHard", "wallKick", "refill", "respawn", "runDone", "miss", "wallHit"];
    let line: Vec<String> = names.iter().zip(count).filter(|(_, c)| *c > 0).map(|(n, c)| format!("{n} {c}")).collect();
    println!("events: {}", line.join(", "));
    println!("run: kills {}/{}  gas {:.0}", sim.run.kills, sim.dummies.len(), sim.p.gas);
    let _ = ev::JUMP;
    if let Some(path) = wav {
        let peak = pcm.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        let rms = (pcm.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / pcm.len().max(1) as f64).sqrt();
        let mut out = Vec::with_capacity(44 + pcm.len() * 2);
        let bytes = (pcm.len() * 2) as u32;
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + bytes).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        for v in [16u32, 0x0002_0001, 22050, 22050 * 4, 0x0010_0004] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(b"data");
        out.extend_from_slice(&bytes.to_le_bytes());
        for s in &pcm {
            out.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(&path, out).expect("write wav");
        println!("sound: {path}, peak {peak} of 32767, rms {rms:.0}");
    }
}
