//! Autopilot: plays the game from the same inputs a player has. It flies the
//! world's waypoint loop, which gives the attract mode and a repeatable load
//! for frame-time measurements on the device.

use crate::math::*;
use crate::sim::{btn, hook, Input, Sim};

#[derive(Clone, Copy, Debug)]
pub struct Auto {
    pub wp: usize,
    pub laps: u32,
    hold: [u32; 2],
    cool: u32,
    side: u32,
    prev: u32,
    mark: V3,
    mark_tick: u32,
    stuck: u32,
    /// Set when the autopilot gives up on where it is; `tick` then puts the player on the route.
    pub rescue: bool,
}

impl Auto {
    pub fn new() -> Auto {
        Auto { wp: 0, laps: 0, hold: [0; 2], cool: 0, side: 0, prev: 0, mark: V3::ZERO, mark_tick: 0, stuck: 0, rescue: false }
    }
}

impl Sim {
    /// The autopilot's input for the next tick.
    pub fn auto_input(&mut self) -> Input {
        let mut a = self.auto;
        let mut out = Input::default();
        let n = self.waypoints.len();
        if n == 0 {
            return out;
        }
        let p = &self.p;
        let mut tgt = self.waypoints[a.wp % n];
        if (tgt - p.pos).flat().len() < 26.0 {
            a.wp += 1;
            if a.wp >= n {
                a.wp = 0;
                a.laps += 1;
            }
            tgt = self.waypoints[a.wp];
        }
        // A standing target near the route takes over from the waypoint: fly through its nape.
        let along = heading(yaw_of(tgt - p.pos));
        let mut hunting = false;
        let mut best = 90.0f32;
        for d in self.dummies.iter().filter(|d| d.alive) {
            let rel = d.nape - p.pos;
            let dist = rel.len();
            if dist < best && rel.flat().norm().dot(along) > 0.35 {
                best = dist;
                tgt = d.nape;
                hunting = true;
            }
        }
        let to = tgt - p.pos;
        let want_yaw = yaw_of(to);
        let err = wrap_angle(want_yaw - self.cam.yaw);
        // The stick response is squared; take the root so the turn rate is proportional to the error.
        let rx = clamp(-err * 2.4, -1.0, 1.0);
        out.rx = if rx < 0.0 { -sqrt(-rx) } else { sqrt(rx) };
        // Look along the climb to a target; otherwise a little above the horizon.
        let want_pitch = if hunting { clamp(atan2(to.y, to.flat().len()), -0.5, 0.7) } else { 0.14 };
        let ry = clamp((want_pitch - self.cam.pitch) * 3.0, -1.0, 1.0);
        out.ry = if ry < 0.0 { -sqrt(-ry) } else { sqrt(ry) };
        out.ly = 1.0;

        let speed = self.speed();
        let alt = p.pos.y - tgt.y;
        let mut b = 0u32;
        // An aimed pair of wires onto the target once the camera has it: the way a player closes in.
        let aim = self.reticle[2];
        let zip_now = hunting && to.len() > 5.0 && ((a.prev & btn::ZIP != 0 && (p.hooks[0].out() || p.hooks[1].out())) || (to.len() < 80.0 && aim.valid && (aim.point - tgt).len() < 4.0));
        if zip_now {
            b |= btn::ZIP;
        } else if p.grounded {
            if a.prev & btn::GAS == 0 {
                b |= btn::GAS;
            }
        } else {
            let mut any_out = false;
            let mut any_att = false;
            for i in 0..2 {
                let h = &p.hooks[i];
                let bit = if i == 0 { btn::HOOK_L } else { btn::HOOK_R };
                match h.state {
                    hook::FLYING => {
                        b |= bit;
                        any_out = true;
                    }
                    hook::ATTACHED => {
                        any_out = true;
                        let dir = h.anchor - p.pos;
                        let d = dir.len();
                        let ahead = dir.flat().norm().dot(heading(want_yaw));
                        a.hold[i] += 1;
                        if ahead < 0.22 || a.hold[i] > 100 || alt > 16.0 || d < 7.0 {
                            a.hold[i] = 0;
                            a.cool = 5;
                        } else {
                            b |= bit;
                            any_att = true;
                        }
                    }
                    hook::MISS | hook::RETRACT => {
                        any_out = true;
                        a.hold[i] = 0;
                    }
                    _ => {}
                }
            }
            if !any_out && a.cool == 0 && abs(err) < 0.9 {
                if speed < 14.0 {
                    b |= btn::HOOK_L | btn::HOOK_R;
                } else {
                    b |= if a.side == 0 { btn::HOOK_L } else { btn::HOOK_R };
                    a.side ^= 1;
                }
            }
            // Gas while there is some to spare; the wires alone carry the rest.
            let spare = p.gas > 22.0 || (a.prev & btn::GAS != 0 && p.gas > 4.0);
            if any_att {
                if spare && (speed < 30.0 || alt < -3.0) {
                    b |= btn::GAS;
                }
            } else if spare && (alt < -8.0 || speed < 8.0) {
                b |= btn::GAS;
            }
        }
        if a.cool > 0 {
            a.cool -= 1;
        }
        // Cut whatever comes within reach.
        if a.prev & btn::SLASH == 0 && self.dummies.iter().any(|d| d.alive && (d.nape - p.pos).len() < 6.0) {
            b |= btn::SLASH;
        }
        // Stuck: let go of everything, then skip the waypoint, then start over.
        if self.tick.wrapping_sub(a.mark_tick) >= 120 {
            if (p.pos - a.mark).len() < 5.0 {
                a.stuck += 1;
                b &= !(btn::HOOK_L | btn::HOOK_R);
                if a.stuck >= 2 {
                    a.wp = (a.wp + 1) % n;
                }
                if a.stuck >= 3 {
                    // Boxed in: start again from the next waypoint, with a full tank.
                    a.rescue = true;
                    a.stuck = 0;
                }
            } else {
                a.stuck = 0;
            }
            a.mark = p.pos;
            a.mark_tick = self.tick;
        }
        a.prev = b;
        self.auto = a;
        out.buttons = b;
        out
    }
}
