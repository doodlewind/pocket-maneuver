//! Sound, synthesized: wind that rises with speed, the hiss of gas, the hum
//! of a loaded wire, and a short voice for each event. There are no samples;
//! the reference and the Vita render the same code.
//!
//! The synthesizer listens to the simulation and never feeds back into it.

use crate::math::*;
use crate::sim::{ev, Sim};

const VOICES: usize = 12;

mod kind {
    pub const FIRE: u8 = 1;
    pub const CLANK: u8 = 2;
    pub const WHIP: u8 = 3;
    pub const BURST: u8 = 4;
    pub const SWISH: u8 = 5;
    pub const THUMP: u8 = 6;
    pub const TICK: u8 = 7;
    pub const THUD: u8 = 8;
    pub const CHIME: u8 = 9;
    pub const STEP: u8 = 10;
}

#[derive(Clone, Copy)]
struct Voice {
    kind: u8,
    t: f32,
    gain: f32,
    lp: f32,
    phase: f32,
}

pub struct Synth {
    seed: u32,
    voices: [Voice; VOICES],
    // Continuous layers: current value and target.
    wind: [f32; 2],
    wind_cut: [f32; 2],
    gas: [f32; 2],
    wire: [f32; 2],
    reel: [f32; 2],
    speed: f32,
    wind_lp: [f32; 2],
    gas_lp: f32,
    wire_phase: f32,
    reel_phase: f32,
    step: i32,
}

impl Synth {
    pub fn new() -> Synth {
        Synth { seed: 0x1234_5678, voices: [Voice { kind: 0, t: 0.0, gain: 0.0, lp: 0.0, phase: 0.0 }; VOICES], wind: [0.0; 2], wind_cut: [0.03; 2], gas: [0.0; 2], wire: [0.0; 2], reel: [0.0; 2], speed: 0.0, wind_lp: [0.0; 2], gas_lp: 0.0, wire_phase: 0.0, reel_phase: 0.0, step: 0 }
    }

    #[inline]
    fn noise(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / 8388608.0 - 1.0
    }

    fn play(&mut self, kind: u8, gain: f32) {
        // The oldest voice gives way when all are busy.
        let slot = self.voices.iter().position(|v| v.kind == 0).unwrap_or_else(|| self.voices.iter().enumerate().max_by(|a, b| a.1.t.total_cmp(&b.1.t)).map(|v| v.0).unwrap_or(0));
        self.voices[slot] = Voice { kind, t: 0.0, gain, lp: 0.0, phase: 0.0 };
    }

    /// Takes the simulation's state after a tick, and that tick's events.
    pub fn control(&mut self, sim: &Sim, events: u32) {
        let speed = sim.speed();
        self.speed = speed;
        self.wind[1] = smoothstep(3.0, 55.0, speed) * 0.5;
        self.wind_cut[1] = 0.02 + 0.3 * smoothstep(0.0, 65.0, speed);
        self.gas[1] = if sim.p.thrusting || sim.p.reeling { 0.3 } else { 0.0 };
        let attached = sim.p.hooks.iter().filter(|h| h.attached()).count() as f32;
        self.wire[1] = 0.045 * attached;
        self.reel[1] = if sim.p.reeling { 0.05 } else { 0.0 };
        for (bit, kind, gain) in [
            (ev::HOOK_FIRE_L | ev::HOOK_FIRE_R, kind::FIRE, 0.8),
            (ev::HOOK_ATTACH_L | ev::HOOK_ATTACH_R, kind::CLANK, 0.8),
            (ev::RELEASE, kind::WHIP, 0.7),
            (ev::BURST, kind::BURST, 1.0),
            (ev::SLASH, kind::SWISH, 1.0),
            (ev::SLASH_HIT, kind::THUMP, 1.0),
            (ev::SLASH_WEAK, kind::TICK, 1.0),
            (ev::LAND, kind::THUD, 0.4),
            (ev::LAND_HARD, kind::THUD, 1.0),
            (ev::WALL_HIT, kind::THUD, 0.6),
            (ev::WALL_KICK | ev::JUMP, kind::STEP, 1.2),
            (ev::REFILL | ev::RUN_DONE, kind::CHIME, 1.0),
        ] {
            if events & bit != 0 {
                self.play(kind, gain);
            }
        }
        // Footfalls: two per stride.
        let step = floor(sim.p.run_phase / PI) as i32;
        if sim.p.grounded && speed > 2.0 && step != self.step {
            self.play(kind::STEP, 0.7);
        }
        self.step = step;
    }

    /// Renders interleaved stereo at `rate` Hz into `out` (two values per frame).
    pub fn render(&mut self, out: &mut [i16], rate: f32) {
        let dt = 1.0 / rate;
        let glide = 1.0 - exp(-dt / 0.03);
        let wire_hz = 140.0 + self.speed * 3.5;
        let reel_hz = 300.0 + self.speed * 5.0;
        for frame in out.chunks_exact_mut(2) {
            for layer in [&mut self.wind, &mut self.wind_cut, &mut self.gas, &mut self.wire, &mut self.reel] {
                layer[0] += (layer[1] - layer[0]) * glide;
            }
            // Wind: noise through two low-pass stages that open with speed; a little width between the ears.
            let (n0, n1) = (self.noise(), self.noise());
            let cut = self.wind_cut[0] * (44100.0 / rate).min(2.0);
            self.wind_lp[0] += (n0 - self.wind_lp[0]) * cut;
            self.wind_lp[1] += (n0 * 0.6 + n1 * 0.4 - self.wind_lp[1]) * cut;
            let wind_l = self.wind_lp[0] * self.wind[0] * 2.6;
            let wind_r = self.wind_lp[1] * self.wind[0] * 2.6;
            // Gas: the bright part of noise.
            let n2 = self.noise();
            self.gas_lp += (n2 - self.gas_lp) * 0.3;
            let mut mono = (n2 - self.gas_lp) * self.gas[0];
            // A loaded wire hums; reeling adds a whine.
            self.wire_phase += TAU * wire_hz * dt;
            self.reel_phase += TAU * reel_hz * dt;
            if self.wire_phase > TAU {
                self.wire_phase -= TAU;
            }
            if self.reel_phase > TAU {
                self.reel_phase -= TAU;
            }
            mono += sin(self.wire_phase) * self.wire[0] + (sin(self.reel_phase) + 0.5 * sin(self.reel_phase * 2.0)) * self.reel[0];

            for i in 0..VOICES {
                let v = self.voices[i];
                if v.kind == 0 {
                    continue;
                }
                let t = v.t;
                let n = self.noise();
                let mut lp = v.lp;
                let mut phase = v.phase;
                let (s, dur) = match v.kind {
                    kind::FIRE => {
                        phase += TAU * 1700.0 * exp(-t * 9.0) * dt;
                        ((n * 0.45 + sin(phase) * 0.3) * exp(-t * 28.0), 0.16)
                    }
                    kind::CLANK => ((sin(TAU * 2140.0 * t) + 0.7 * sin(TAU * 3270.0 * t) + 0.4 * sin(TAU * 5120.0 * t)) * 0.2 * exp(-t * 22.0) + n * 0.35 * exp(-t * 300.0), 0.24),
                    kind::WHIP => (n * 0.22 * exp(-t * 45.0), 0.08),
                    kind::BURST => {
                        lp += (n - lp) * 0.12;
                        (lp * 2.4 * exp(-t * 9.0), 0.4)
                    }
                    kind::SWISH => {
                        lp += (n - lp) * (0.08 + 2.6 * t);
                        ((n - lp) * 0.4 * sin(PI * min(t / 0.22, 1.0)), 0.22)
                    }
                    kind::THUMP => {
                        phase += TAU * 95.0 * exp(-t * 6.0) * dt;
                        (sin(phase) * 0.9 * exp(-t * 12.0) + n * 0.45 * exp(-t * 60.0), 0.32)
                    }
                    kind::TICK => (n * 0.3 * exp(-t * 120.0), 0.05),
                    kind::THUD => {
                        lp += (n - lp) * 0.1;
                        (sin(TAU * 70.0 * t) * 0.8 * exp(-t * 25.0) + lp * 0.9 * exp(-t * 40.0), 0.18)
                    }
                    kind::CHIME => ((if t < 0.26 { sin(TAU * 660.0 * t) } else { 0.0 } + if t > 0.12 { sin(TAU * 880.0 * t) } else { 0.0 }) * 0.2 * exp(-t * 5.0), 0.55),
                    _ => {
                        lp += (n - lp) * 0.2;
                        (lp * 0.5 * exp(-t * 90.0), 0.05)
                    }
                };
                mono += s * v.gain;
                let nt = t + dt;
                self.voices[i] = if nt >= dur { Voice { kind: 0, ..v } } else { Voice { t: nt, lp, phase, ..v } };
            }
            // Soft limit, then 16 bits.
            let clip = |x: f32| (x / (1.0 + abs(x)) * 30000.0) as i16;
            frame[0] = clip(mono + wind_l);
            frame[1] = clip(mono + wind_r);
        }
    }
}
