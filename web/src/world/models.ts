// Models that move: the character's thirteen rigid parts and the training
// targets. Part meshes are modelled around their joint, in the pose module's
// frame (+Y up, -Z forward), with dimensions from the simulation's skeleton.

import { STRIP, stripV } from "./atlas";
import { Geo, Rgb, V3 } from "./geo";
import { DIM, PART, PARTS } from "../sim/abi.gen";

const [VB, VT] = stripV(STRIP.FLAT);
const UV: [number, number, number, number] = [0.01, 0.05, VB, VT];

/** A box from `y0` to `y1`, centred on `(cx, cz)`, with half sizes that may differ at the two ends. */
function box(g: Geo, cx: number, cz: number, y0: number, y1: number, hx0: number, hz0: number, hx1: number, hz1: number, tint: Rgb, cz1 = cz) {
  const lo = (sx: number, sz: number): V3 => [cx + sx * hx0, y0, cz + sz * hz0];
  const hi = (sx: number, sz: number): V3 => [cx + sx * hx1, y1, cz1 + sz * hz1];
  // Front is -Z.
  g.quad(lo(1, -1), lo(-1, -1), hi(-1, -1), hi(1, -1), UV, tint);
  g.quad(lo(-1, 1), lo(1, 1), hi(1, 1), hi(-1, 1), UV, tint);
  g.quad(lo(1, 1), lo(1, -1), hi(1, -1), hi(1, 1), UV, tint);
  g.quad(lo(-1, -1), lo(-1, 1), hi(-1, 1), hi(-1, -1), UV, tint);
  g.quad(hi(-1, 1), hi(1, 1), hi(1, -1), hi(-1, -1), UV, tint);
  g.quad(lo(-1, -1), lo(1, -1), lo(1, 1), lo(-1, 1), UV, tint);
}

const SKIN: Rgb = [0.9, 0.72, 0.6];
const HAIR: Rgb = [0.24, 0.17, 0.13];
const JACKET: Rgb = [0.66, 0.52, 0.36];
const SHIRT: Rgb = [0.9, 0.88, 0.82];
const PANTS: Rgb = [0.92, 0.9, 0.85];
const BOOT: Rgb = [0.3, 0.2, 0.14];
const STRAP: Rgb = [0.26, 0.18, 0.13];
const STEEL: Rgb = [0.6, 0.63, 0.68];
const BLADE: Rgb = [0.92, 0.95, 1.0];
const CAPE: Rgb = [0.2, 0.42, 0.3];

/** Direction of a blade in the forearm's frame, and where it starts and ends along it. */
export const BLADE_DIR: V3 = [0, -0.5, -0.866];

export function characterParts(): Geo[] {
  const parts: Geo[] = [];
  for (let i = 0; i < PARTS; i++) parts.push(new Geo());
  const g = (i: number) => parts[i];

  // Pelvis, belt and the gear on both hips: blade boxes with a gas cylinder on top.
  box(g(PART.PELVIS), 0, 0, -0.1, 0.1, 0.15, 0.1, 0.15, 0.1, PANTS);
  box(g(PART.PELVIS), 0, 0, 0.06, 0.11, 0.158, 0.108, 0.158, 0.108, STRAP);
  for (const s of [-1, 1]) {
    box(g(PART.PELVIS), s * 0.215, 0.17, -0.3, -0.02, 0.045, 0.2, 0.045, 0.2, STEEL);
    box(g(PART.PELVIS), s * 0.215, 0.16, -0.02, 0.08, 0.04, 0.17, 0.04, 0.17, [0.8, 0.82, 0.85]);
    box(g(PART.PELVIS), s * 0.215, -0.06, -0.01, 0.07, 0.03, 0.05, 0.03, 0.05, STRAP);
  }
  // Chest: shirt at the waist, a short jacket above, straps across.
  box(g(PART.CHEST), 0, 0, 0, 0.16, 0.14, 0.095, 0.16, 0.1, SHIRT);
  box(g(PART.CHEST), 0, 0, 0.16, DIM.CHEST, 0.165, 0.105, 0.2, 0.11, JACKET);
  box(g(PART.CHEST), 0, -0.004, 0.2, 0.24, 0.17, 0.108, 0.176, 0.11, STRAP);
  // Head: neck, face, hair.
  box(g(PART.HEAD), 0, 0, 0, 0.06, 0.045, 0.045, 0.045, 0.045, SKIN);
  box(g(PART.HEAD), 0, -0.005, 0.05, 0.27, 0.082, 0.095, 0.088, 0.1, SKIN);
  box(g(PART.HEAD), 0, 0.02, 0.17, 0.3, 0.094, 0.1, 0.09, 0.095, HAIR);
  for (const [upper, lower, s] of [
    [PART.ARM_UL, PART.ARM_LL, -1],
    [PART.ARM_UR, PART.ARM_LR, 1],
  ] as const) {
    box(g(upper), 0, 0, -DIM.UPPER_ARM, 0.03, 0.045, 0.05, 0.055, 0.058, JACKET);
    box(g(lower), 0, 0, -DIM.FOREARM, 0, 0.04, 0.045, 0.046, 0.05, JACKET);
    box(g(lower), 0, 0, -DIM.FOREARM - 0.08, -DIM.FOREARM, 0.036, 0.042, 0.04, 0.045, SKIN);
    // The blade leaves the fist forward and down.
    const y0 = -DIM.FOREARM - 0.04;
    const tip: V3 = [0, y0 + BLADE_DIR[1] * 0.95, BLADE_DIR[2] * 0.95];
    box(g(lower), s * 0.0, tip[2], tip[1], y0, 0.006, 0.02, 0.008, 0.03, BLADE, 0);
    box(g(lower), 0, -0.02, y0 - 0.02, y0 + 0.03, 0.02, 0.05, 0.02, 0.05, STEEL);
  }
  for (const [upper, lower] of [
    [PART.LEG_UL, PART.LEG_LL],
    [PART.LEG_UR, PART.LEG_LR],
  ] as const) {
    box(g(upper), 0, 0, -DIM.THIGH, 0.02, 0.06, 0.07, 0.078, 0.088, PANTS);
    box(g(upper), 0, 0, -0.2, -0.16, 0.082, 0.092, 0.082, 0.092, STRAP);
    box(g(lower), 0, 0, -0.2, 0, 0.055, 0.062, 0.06, 0.07, PANTS);
    box(g(lower), 0, 0, -DIM.SHIN, -0.18, 0.058, 0.066, 0.066, 0.074, BOOT);
    box(g(lower), 0, -0.07, -DIM.SHIN, -DIM.SHIN + 0.09, 0.055, 0.13, 0.055, 0.12, BOOT);
  }
  box(g(PART.CAPE_A), 0, 0, -DIM.CAPE_SEG, 0, 0.235, 0.012, 0.2, 0.012, CAPE);
  box(g(PART.CAPE_B), 0, 0, -DIM.CAPE_SEG, 0, 0.27, 0.012, 0.235, 0.012, CAPE);
  return parts;
}

const WOOD: Rgb = [0.74, 0.6, 0.44];
const WOOD_DARK: Rgb = [0.5, 0.39, 0.28];

/**
 * A training target, one unit tall, facing -Z: a plank giant on two posts.
 * Scale it by the target's height. `nape` is the pad at the back of its neck.
 */
export function targetModel(): { body: Geo; nape: Geo } {
  const body = new Geo();
  const nape = new Geo();
  for (const s of [-1, 1]) box(body, s * 0.09, 0, 0, 0.46, 0.035, 0.03, 0.04, 0.03, WOOD_DARK);
  box(body, 0, 0, 0.44, 0.8, 0.15, 0.03, 0.19, 0.035, WOOD);
  for (const s of [-1, 1]) box(body, s * 0.235, 0, 0.42, 0.78, 0.035, 0.028, 0.045, 0.03, WOOD);
  box(body, 0, 0, 0.8, 0.86, 0.05, 0.03, 0.05, 0.03, WOOD_DARK);
  box(body, 0, 0, 0.86, 1.0, 0.075, 0.035, 0.07, 0.035, WOOD);
  // A cross-brace on the back.
  box(body, 0, 0.04, 0.5, 0.54, 0.2, 0.012, 0.2, 0.012, WOOD_DARK);
  box(nape, 0, 0.065, 0.79, 0.89, 0.06, 0.03, 0.06, 0.03, [0.86, 0.3, 0.24]);
  return { body, nape };
}
