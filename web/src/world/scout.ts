// The player: a soldier in a short jacket, white trousers, knee boots and a
// harness, with the wire gear at the hips and a blade in each hand. The body
// and clothes are one blended field (sdf.ts); the gear, the hair and the face
// are separate crisp pieces on top of it.
//
// Everything is authored in bone frames of the simulation's bind pose: +Y up
// the bone, -Z forward, +X to the figure's right. A limb runs down its -Y.

import { BONE } from "../sim/abi.gen";
import { Body, ellipse, MeshOut, Rgb, Rigid, SkinModel, V3 } from "./sdf";

const SKIN: Rgb = [0.94, 0.77, 0.66];
const SKIN_SHADE: Rgb = [0.86, 0.66, 0.56];
const HAIR: Rgb = [0.2, 0.15, 0.12];
const JACKET: Rgb = [0.7, 0.55, 0.38];
const JACKET_DARK: Rgb = [0.56, 0.43, 0.29];
const SHIRT: Rgb = [0.93, 0.91, 0.86];
const PANTS: Rgb = [0.95, 0.94, 0.9];
const BOOT: Rgb = [0.27, 0.18, 0.13];
const SOLE: Rgb = [0.13, 0.1, 0.09];
const BOOT_CUFF: Rgb = [0.36, 0.25, 0.18];
const STRAP: Rgb = [0.3, 0.2, 0.14];
const SASH: Rgb = [0.5, 0.36, 0.25];
const CLOAK: Rgb = [0.2, 0.42, 0.31];
const STEEL: Rgb = [0.6, 0.63, 0.68];
const STEEL_LIGHT: Rgb = [0.84, 0.86, 0.88];
const STEEL_DARK: Rgb = [0.3, 0.32, 0.36];
const BLADE: Rgb = [0.93, 0.96, 1.0];

/** The cloak's colour, shared with the cloth the simulation moves. */
export const CLOAK_COLOR = CLOAK;
/** The blade's tip in the hand's frame (`BLADE_TIP` in crates/maneuver-sim/src/pose.rs). */
const BLADE_TIP: V3 = [0, -0.52, -0.78];

/** `cells` are the mesh cell sizes of the body and of the head, in metres; a coarser pair is a handheld's model. */
export function buildScout(bind: Float32Array, cells: [number, number] = [0.014, 0.008]): SkinModel {
  const b = new Body(bind);
  const B = BONE;

  // ---- trunk
  b.ell(B.PELVIS, [0, 0, 0.005], [0.168, 0.125, 0.118], PANTS);
  b.ell(B.SPINE, [0, 0.05, 0], [0.142, 0.13, 0.104], SHIRT, { k: 0.05 });
  b.ell(B.CHEST, [0, 0.12, 0.005], [0.172, 0.165, 0.118], SHIRT, { k: 0.05 });
  // A short jacket over the ribs and shoulders, open down the front.
  b.ell(B.CHEST, [0, 0.14, 0.008], [0.186, 0.148, 0.132], JACKET, { k: 0.012 });
  b.cone(B.CLAV_L, [0.02, 0.0, 0], [-0.15, 0.0, 0], 0.07, 0.066, JACKET, { k: 0.03, bone2: B.ARM_UL });
  b.cone(B.CLAV_R, [-0.02, 0.0, 0], [0.15, 0.0, 0], 0.07, 0.066, JACKET, { k: 0.03, bone2: B.ARM_UR });
  b.paint(B.CHEST, [0, 0.1, -0.13], [0.036, 0.16, 0.08], SHIRT);
  // A cloth wrap over the hips.
  b.ell(B.PELVIS, [0, -0.06, 0.016], [0.196, 0.125, 0.142], SASH, { k: 0.01 });
  // The wrap hangs at the sides and the back; the front shows the trousers.
  b.paint(B.PELVIS, [0, -0.08, -0.15], [0.085, 0.16, 0.09], PANTS);
  b.paint(B.PELVIS, [0, 0.07, 0], [0.3, 0.022, 0.3], STRAP);
  // Neck, and the cloak's collar and hood lying on the shoulders.
  b.cone(B.NECK, [0, -0.03, 0.005], [0, 0.085, 0], 0.052, 0.047, SKIN, { k: 0.02, bone2: B.HEAD });
  b.ell(B.CHEST, [0, 0.265, 0.075], [0.15, 0.06, 0.085], CLOAK, { k: 0.02 });
  b.cone(B.CHEST, [-0.1, 0.25, -0.02], [-0.03, 0.2, -0.105], 0.04, 0.028, CLOAK, { k: 0.02 });
  b.cone(B.CHEST, [0.1, 0.25, -0.02], [0.03, 0.2, -0.105], 0.04, 0.028, CLOAK, { k: 0.02 });
  b.paintEll(B.CHEST, [0, 0.198, -0.115], [0.02, 0.02, 0.03], STEEL_LIGHT);

  // ---- arms: muscle under the sleeve, a cuff, a fist around the grip
  for (const [u, l, h, s] of [
    [B.ARM_UL, B.ARM_LL, B.HAND_L, -1],
    [B.ARM_UR, B.ARM_LR, B.HAND_R, 1],
  ] as const) {
    b.ell(u, [s * 0.008, -0.035, 0], [0.062, 0.085, 0.064], JACKET, { k: 0.03 });
    b.cone(u, [0, -0.02, 0], [0, -0.27, 0], 0.045, 0.038, JACKET, { k: 0.03, bone2: l });
    b.ell(u, [0, -0.14, -0.015], [0.043, 0.09, 0.043], JACKET, { k: 0.03 });
    b.ell(u, [0, -0.12, 0.018], [0.04, 0.1, 0.04], JACKET, { k: 0.03 });
    b.ell(l, [0, -0.005, 0.008], [0.039, 0.04, 0.042], JACKET, { k: 0.02 });
    b.ell(l, [0, -0.085, -0.004], [0.046, 0.09, 0.042], JACKET, { k: 0.03 });
    b.cone(l, [0, -0.1, 0], [0, -0.222, 0], 0.04, 0.029, JACKET, { k: 0.03, bone2: h });
    b.cone(l, [0, -0.2, 0], [0, -0.226, 0], 0.036, 0.037, JACKET_DARK, { k: 0.004 });
    b.cone(l, [0, -0.222, 0], [0, -0.252, 0], 0.026, 0.025, SKIN, { k: 0.008, bone2: h });
    // The hand: palm, the knuckles closed on the grip, the thumb over them.
    b.ell(h, [0, -0.04, 0], [0.034, 0.045, 0.024], SKIN, { k: 0.012 });
    b.ell(h, [0, -0.072, -0.014], [0.033, 0.027, 0.03], SKIN, { k: 0.012 });
    b.cone(h, [-s * 0.03, -0.03, -0.012], [-s * 0.02, -0.07, -0.036], 0.013, 0.011, SKIN, { k: 0.008 });
  }

  // ---- legs: white trousers over the thigh's muscle, into knee boots with a heel and a toe
  for (const [u, l, f, s] of [
    [B.LEG_UL, B.LEG_LL, B.FOOT_L, -1],
    [B.LEG_UR, B.LEG_LR, B.FOOT_R, 1],
  ] as const) {
    b.ell(B.PELVIS, [s * 0.086, -0.06, 0.046], [0.088, 0.1, 0.09], PANTS, { k: 0.04 });
    b.cone(u, [0, 0.0, 0], [0, -0.42, 0], 0.07, 0.05, PANTS, { k: 0.04, bone2: l, sigma: 0.028 });
    b.ell(u, [s * 0.006, -0.17, -0.022], [0.07, 0.17, 0.066], PANTS, { k: 0.035, sigma: 0.028 });
    b.ell(u, [0, -0.19, 0.03], [0.06, 0.16, 0.055], PANTS, { k: 0.035, sigma: 0.028 });
    b.ell(l, [0, 0.0, -0.016], [0.049, 0.05, 0.052], PANTS, { k: 0.02 });
    b.cone(l, [0, 0, 0], [0, -0.4, 0], 0.05, 0.034, PANTS, { k: 0.03, bone2: f, sigma: 0.028 });
    b.cone(l, [0, -0.078, 0.004], [0, -0.4, 0], 0.058, 0.041, BOOT, { k: 0.01, bone2: f });
    b.ell(l, [0, -0.15, 0.026], [0.056, 0.1, 0.058], BOOT, { k: 0.02 });
    b.cone(l, [0, -0.064, 0.004], [0, -0.088, 0.004], 0.067, 0.064, BOOT_CUFF, { k: 0.004 });
    b.ell(f, [0, -0.018, 0.022], [0.043, 0.044, 0.05], BOOT, { k: 0.02 });
    b.box(f, [0, -0.035, -0.078], [0.045, 0.026, 0.1], 0.024, BOOT, { k: 0.02 });
    b.paint(f, [0, -0.064, -0.05], [0.08, 0.011, 0.2], SOLE);
    // The harness: two bands on the thigh.
    b.paint(u, [0, -0.12, 0], [0.12, 0.013, 0.12], STRAP);
    b.paint(u, [0, -0.26, 0], [0.12, 0.013, 0.12], STRAP);
  }
  // Harness on the trunk: a chest band and two braces.
  b.paint(B.CHEST, [0, 0.085, 0], [0.3, 0.014, 0.3], STRAP);
  for (const s of [-1, 1]) b.paint(B.CHEST, [s * 0.095, 0.12, 0], [0.014, 0.22, 0.3], STRAP);

  // ---- head
  b.ell(B.HEAD, [0, 0.118, 0.0], [0.083, 0.104, 0.096], SKIN, { k: 0.02 });
  b.cone(B.HEAD, [0, 0.07, -0.03], [0, 0.022, -0.056], 0.062, 0.036, SKIN, { k: 0.03 });
  b.cone(B.HEAD, [0, 0.092, -0.092], [0, 0.066, -0.106], 0.012, 0.009, SKIN, { k: 0.012 });
  for (const s of [-1, 1]) {
    b.ell(B.HEAD, [s * 0.084, 0.092, 0.006], [0.012, 0.03, 0.02], SKIN_SHADE, { k: 0.008 });
    b.carve(B.HEAD, [s * 0.034, 0.104, -0.105], [0.02, 0.014, 0.018], { k: 0.012 });
  }
  // The scalp under the hair.
  b.ell(B.HEAD, [0, 0.14, 0.014], [0.089, 0.094, 0.099], HAIR, { k: 0.006 });

  const out = new MeshOut();
  const neck = b.joint(B.HEAD)[1] - 0.02;
  b.mesh(out, cells[0], { yMax: neck + cells[0] });
  b.mesh(out, cells[1], { yMin: neck - cells[1] });

  face(out, b);
  hair(out, b);
  gear(out, b);
  return out.model();
}

function face(out: MeshOut, b: Body) {
  const r = new Rigid(out, b, BONE.HEAD);
  const into: V3 = [0, 0, 1];
  const z = -0.16;
  const dark: Rgb = [0.14, 0.1, 0.09];
  for (const s of [-1, 1]) {
    const cx = s * 0.034;
    const cy = 0.102;
    // White, iris, pupil, a highlight; a heavy upper lid and a brow.
    r.decal(ellipse(cx, cy, 0.0165, 0.0115, z, 12), into, 0.0012, [0.97, 0.97, 0.96]);
    r.decal(ellipse(cx + s * 0.001, cy - 0.0005, 0.0092, 0.0108, z, 12), into, 0.0018, [0.25, 0.47, 0.5]);
    r.decal(ellipse(cx + s * 0.001, cy - 0.001, 0.0046, 0.006, z, 8), into, 0.0024, [0.06, 0.07, 0.08]);
    r.decal(ellipse(cx - 0.004, cy + 0.004, 0.0024, 0.0024, z, 6), into, 0.003, [1, 1, 1]);
    const lid: V3[] = [];
    for (let k = 0; k <= 8; k++) {
      const a = (k / 8) * Math.PI;
      lid.push([cx - Math.cos(a) * 0.019, cy + Math.sin(a) * 0.0125 + 0.0015, z]);
    }
    for (let k = 8; k >= 0; k--) {
      const a = (k / 8) * Math.PI;
      lid.push([cx - Math.cos(a) * 0.0165, cy + Math.sin(a) * 0.009, z]);
    }
    for (let k = 0; k < 8; k++) r.decal([lid[k + 1], lid[16 - k], lid[17 - k], lid[k]], into, 0.003, dark);
    // Brow: thick at the nose, thin at the temple.
    const inner = cx - s * 0.018;
    const outer = cx + s * 0.02;
    const brow: V3[] = [
      [inner, cy + 0.024, z],
      [outer, cy + 0.029, z],
      [outer, cy + 0.0315, z],
      [inner, cy + 0.03, z],
    ];
    r.decal(s < 0 ? brow : [brow[1], brow[0], brow[3], brow[2]], into, 0.002, HAIR);
  }
  // Mouth: a short line.
  r.decal(
    [
      [0.013, 0.043, z],
      [-0.013, 0.043, z],
      [-0.011, 0.0455, z],
      [0.011, 0.0455, z],
    ],
    into,
    0.0015,
    [0.55, 0.3, 0.27],
  );
}

function hair(out: MeshOut, b: Body) {
  const r = new Rigid(out, b, BONE.HEAD);
  // Locks swept from the crown: over the forehead, down the sides, down the back.
  const crown: V3 = [0, 0.228, 0.01];
  const lock = (end: V3, bend: V3, width: number, squashAxis: V3) => {
    const path: V3[] = [];
    const radii: number[] = [];
    for (let k = 0; k <= 5; k++) {
      const t = k / 5;
      const m = 2 * t * (1 - t);
      path.push([crown[0] + (end[0] - crown[0]) * t + bend[0] * m, crown[1] + (end[1] - crown[1]) * t + bend[1] * m, crown[2] + (end[2] - crown[2]) * t + bend[2] * m]);
      radii.push(width * (0.55 + 1.1 * Math.sin(Math.min(t * 1.25, 1) * Math.PI * 0.5)) * (1 - t * t * 0.92));
    }
    r.tube(path, radii, HAIR, 6, { axis: squashAxis, factor: 0.45 });
  };
  // Fringe.
  for (const [x, drop] of [
    [-0.06, 0.128],
    [-0.035, 0.12],
    [-0.008, 0.112],
    [0.022, 0.118],
    [0.05, 0.126],
  ]) {
    lock([x, drop, -0.1], [x * 0.5, 0.05, -0.085], 0.024, [0, 0, 1]);
  }
  // Sides, over the ears.
  for (const s of [-1, 1]) {
    lock([s * 0.092, 0.075, -0.035], [s * 0.07, 0.045, -0.03], 0.026, [1, 0, 0]);
    lock([s * 0.094, 0.062, 0.02], [s * 0.075, 0.04, 0.02], 0.028, [1, 0, 0]);
    lock([s * 0.075, 0.045, 0.07], [s * 0.06, 0.04, 0.07], 0.028, [1, 0, 0]);
  }
  // Back of the head, to the nape.
  for (const x of [-0.045, 0, 0.045]) lock([x, 0.03, 0.1], [x * 0.6, 0.05, 0.1], 0.03, [0, 0, 1]);
}

function gear(out: MeshOut, b: Body) {
  const p = new Rigid(out, b, BONE.PELVIS);
  for (const s of [-1, 1]) {
    // Blade box along the thigh, tipped back; a gas cylinder on top of it.
    const x = s * 0.238;
    p.box([x - 0.036, -0.2, -0.1], [x + 0.036, -0.045, 0.32], STEEL, [0.26, 0, 0]);
    // The mouths of the blade slots.
    for (let k = 0; k < 4; k++) {
      const y = -0.172 + k * 0.034 + 0.028;
      p.box([x - 0.028, y - 0.01, -0.118], [x + 0.028, y + 0.006, -0.105], STEEL_DARK, [0.26, 0, 0]);
    }
    const cyl: V3[] = [];
    for (let k = 0; k <= 1; k++) cyl.push([x, 0.0 - k * 0.085, -0.06 + k * 0.33]);
    p.tube(cyl, [0.046, 0.046], STEEL_LIGHT, 10, null, STEEL);
    p.tube(
      [
        [x, 0.006, -0.085],
        [x, 0.0, -0.06],
      ],
      [0.02, 0.03],
      STEEL_DARK,
      8,
    );
    // The launcher the wire leaves from, at the front of the hip.
    p.tube(
      [
        [s * 0.2, -0.05, -0.02],
        [s * 0.2, -0.05, -0.13],
      ],
      [0.024, 0.02],
      STEEL_DARK,
      8,
      null,
      [0.1, 0.1, 0.11],
    );
    p.box([s * 0.2 - 0.03, -0.085, -0.03], [s * 0.2 + 0.03, -0.015, 0.05], STEEL);
  }
  // The body of the gear at the small of the back: reels and the fan housing.
  p.box([-0.11, -0.03, 0.105], [0.11, 0.09, 0.2], STEEL);
  p.tube(
    [
      [0, 0.03, 0.2],
      [0, 0.03, 0.25],
    ],
    [0.058, 0.05],
    STEEL_DARK,
    12,
    null,
    [0.12, 0.12, 0.13],
  );
  for (const s of [-1, 1]) {
    p.tube(
      [
        [s * 0.115, 0.03, 0.15],
        [s * 0.165, 0.03, 0.15],
      ],
      [0.04, 0.04],
      STEEL_LIGHT,
      10,
      null,
      STEEL_DARK,
    );
  }

  // A grip and a blade in each hand.
  for (const hand of [BONE.HAND_L, BONE.HAND_R]) {
    const h = new Rigid(out, b, hand);
    const d: V3 = [0, BLADE_TIP[1] / 0.9375, BLADE_TIP[2] / 0.9375];
    const o: V3 = [0, -0.045, -0.005];
    const along = (t: number): V3 => [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t];
    h.tube([along(-0.085), along(0.06)], [0.017, 0.017], STRAP, 8, null, STEEL_DARK);
    h.box([-0.03, along(0.06)[1] - 0.012, along(0.06)[2] - 0.03], [0.03, along(0.06)[1] + 0.012, along(0.06)[2] + 0.03], STEEL, [-0.98, 0, 0]);
    // The blade: a flat tube in segments, as it snaps off.
    const segments = 7;
    for (let k = 0; k < segments; k++) {
      const t0 = 0.075 + (0.86 / segments) * k;
      const t1 = 0.075 + (0.86 / segments) * (k + 1) - 0.006;
      const tint: Rgb = k % 2 === 0 ? BLADE : [0.84, 0.88, 0.93];
      h.tube([along(t0), along(t1)], [0.021, k === segments - 1 ? 0.008 : 0.021], tint, 4, { axis: [1, 0, 0], factor: 0.12 });
    }
  }
}
