// Writes the simulation's world file (`MVSW`, see crates/maneuver-sim/src/worldfile.rs).

import { Collision } from "./geo";
import { Entities, R_WORLD } from "./city";

export function writeWorldFile(col: Collision, ent: Entities): Uint8Array {
  const nv = col.v.length / 3;
  const nt = col.k.length;
  const kindBytes = (nt + 3) & ~3;
  const size = 64 + nv * 12 + nt * 12 + kindBytes + ent.dummies.length * 32 + ent.depots.length * 16 + ent.waypoints.length * 12;
  const out = new Uint8Array(size);
  const view = new DataView(out.buffer);
  let at = 0;
  const u32 = (v: number) => {
    view.setUint32(at, v, true);
    at += 4;
  };
  const f32 = (v: number) => {
    view.setFloat32(at, v, true);
    at += 4;
  };
  u32(0x5753564d);
  u32(1);
  u32(nv);
  u32(nt);
  u32(ent.dummies.length);
  u32(ent.depots.length);
  u32(ent.waypoints.length);
  u32(0);
  ent.spawn.forEach(f32);
  f32(R_WORLD);
  at = 64;
  for (const v of col.v) f32(v);
  for (const i of col.i) u32(i);
  out.set(col.k, at);
  at += kindBytes;
  for (const d of ent.dummies) [...d.pos, d.yaw, d.height, ...d.nape].forEach(f32);
  for (const d of ent.depots) [...d.pos, d.radius].forEach(f32);
  for (const p of ent.waypoints) p.forEach(f32);
  return out;
}
