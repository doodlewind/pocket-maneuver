//! The autopilot in the synthetic town: the run is repeatable, stays finite
//! and inside the world, and covers ground.

use maneuver_sim::abi::snap;
use maneuver_sim::{testworld, worldfile};

fn fly(ticks: u32) -> (Vec<f32>, f32) {
    let mut sim = worldfile::load(&testworld::build()).expect("test world loads");
    let mut distance = 0.0;
    let mut last = sim.p.pos;
    for _ in 0..ticks {
        let input = sim.auto_input();
        sim.tick(input);
        assert!(sim.p.pos.x.is_finite() && sim.p.pos.y.is_finite() && sim.p.pos.z.is_finite(), "position left the reals at tick {}", sim.tick);
        assert!(sim.p.pos.y > -5.0 && sim.p.pos.y < 200.0, "height {} at tick {}", sim.p.pos.y, sim.tick);
        assert!(sim.p.pos.flat().len() < sim.bounds + 80.0, "outside the world at tick {}", sim.tick);
        distance += (sim.p.pos - last).len();
        last = sim.p.pos;
    }
    let mut out = vec![0.0; snap::LEN];
    sim.snapshot(&mut out);
    (out, distance)
}

#[test]
fn autopilot_run_repeats_bit_for_bit() {
    let (a, da) = fly(1800);
    let (b, db) = fly(1800);
    assert_eq!(a.iter().map(|f| f.to_bits()).collect::<Vec<_>>(), b.iter().map(|f| f.to_bits()).collect::<Vec<_>>());
    assert_eq!(da.to_bits(), db.to_bits());
}

#[test]
fn autopilot_covers_ground() {
    let (_, distance) = fly(3600);
    // A minute of flight: well over walking pace on average.
    assert!(distance > 900.0, "only {distance} m in a minute");
}
