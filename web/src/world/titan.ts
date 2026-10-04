// The giants. An original design: ten to sixteen metres of bare muscle and
// bone, hunched, with arms to the knees, clawed hands, a spined back and a
// skull for a face with two small lit eyes. Three builds share the plan:
// 0 gaunt and horned, 1 broad and plated, 2 long-legged, pale and long-haired.
//
// A giant is authored at unit height on its own skeleton (`Skeleton::titan`
// in the simulation) and scaled when drawn. The weak point is the nape: the
// glowing patch at the back of the neck.

import { BONE } from "../sim/abi.gen";
import { Body, ellipse, MeshOut, Rgb, Rigid, SkinModel, V3 } from "./sdf";

interface Build {
  flesh: Rgb;
  dark: Rgb;
  bone: Rgb;
  /** Thickness of the limbs and the trunk. */
  bulk: number;
  horns: boolean;
  plates: boolean;
  hair: Rgb | null;
  /** Length of the spines down the back. */
  spines: number;
}

const BUILDS: readonly Build[] = [
  { flesh: [0.5, 0.21, 0.18], dark: [0.3, 0.12, 0.11], bone: [0.86, 0.81, 0.7], bulk: 1.0, horns: true, plates: false, hair: null, spines: 0.05 },
  { flesh: [0.41, 0.37, 0.34], dark: [0.25, 0.22, 0.21], bone: [0.8, 0.76, 0.67], bulk: 1.32, horns: false, plates: true, hair: null, spines: 0.036 },
  { flesh: [0.66, 0.63, 0.59], dark: [0.38, 0.33, 0.34], bone: [0.9, 0.87, 0.8], bulk: 0.88, horns: false, plates: false, hair: [0.1, 0.09, 0.1], spines: 0.028 },
];

const EYE: Rgb = [1.0, 0.74, 0.2];
const NAPE: Rgb = [1.0, 0.34, 0.14];
const VOID: Rgb = [0.05, 0.03, 0.03];

const dist = (a: V3, b: V3) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);

/** A giant of unit height. `cell` is the mesh resolution: about 1/120 up close, 1/36 far away. */
export function buildTitan(variant: number, bind: Float32Array, cell: number): SkinModel {
  const v = BUILDS[variant % 3];
  const b = new Body(bind, 1, 0.02);
  const B = BONE;
  const k = v.bulk;
  const sw = Math.abs(b.joint(B.ARM_UR)[0]);
  const upper = dist(b.joint(B.ARM_UR), b.joint(B.ARM_LR));
  const fore = dist(b.joint(B.ARM_LR), b.joint(B.HAND_R));
  const thigh = dist(b.joint(B.LEG_UR), b.joint(B.LEG_LR));
  const shin = dist(b.joint(B.LEG_LR), b.joint(B.FOOT_R));
  const neck = dist(b.joint(B.NECK), b.joint(B.HEAD));
  const chest = dist(b.joint(B.CHEST), b.joint(B.NECK));
  const clavY = b.joint(B.CLAV_R)[1] - b.joint(B.CHEST)[1];
  const hr = (1 - b.joint(B.HEAD)[1]) * 0.56;

  // ---- trunk: a wide ribcage over a pinched waist
  b.ell(B.PELVIS, [0, 0, 0.004], [sw * 0.6, 0.058, 0.052 * k], v.flesh);
  b.ell(B.SPINE, [0, 0.04, 0], [sw * 0.52, 0.08, 0.046 * k], v.flesh, { k: 0.035 });
  b.ell(B.CHEST, [0, chest * 0.42, 0.004], [sw * 0.84, chest * 0.7, 0.072 * k], v.flesh, { k: 0.035 });
  b.ell(B.CHEST, [0, chest * 0.5, 0.034 * k], [sw * 0.9, chest * 0.58, 0.05 * k], v.flesh, { k: 0.03 });
  for (const s of [-1, 1]) {
    // Chest, the slope from the neck to the shoulder, and the seat.
    b.ell(B.CHEST, [s * sw * 0.4, chest * 0.6, -0.052 * k], [sw * 0.42, chest * 0.3, 0.034 * k], v.flesh, { k: 0.02 });
    b.cone(B.CHEST, [s * 0.012, chest * 0.96, 0.012], [s * sw * 0.82, clavY + 0.006, 0.004], 0.036 * k, 0.03 * k, v.flesh, { k: 0.03 });
    b.ell(B.PELVIS, [s * sw * 0.36, -0.03, 0.03 * k], [sw * 0.36, 0.05, 0.045 * k], v.flesh, { k: 0.03 });
    // Ribs showing under the skin, and the line of each side of the belly.
    for (let n = 0; n < 4; n++) b.paint(B.CHEST, [s * sw * 0.6, chest * (0.02 + n * 0.1), -0.01], [sw * 0.34, 0.0045, 0.2], v.dark, { rot: [0, 0, s * 0.3] });
    b.paint(B.SPINE, [s * sw * 0.24, 0.045, -0.06 * k], [0.004, 0.07, 0.05], v.dark);
  }
  for (let n = 0; n < 3; n++) b.paint(B.SPINE, [0, 0.005 + n * 0.035, -0.06 * k], [sw * 0.24, 0.0035, 0.05], v.dark);
  b.paint(B.CHEST, [0, chest * 0.55, -0.08 * k], [0.004, chest * 0.3, 0.05], v.dark);

  // ---- neck and the nape
  b.cone(B.NECK, [0, -0.02, 0.004], [0, neck + 0.012, 0], 0.04 * k, 0.034 * k, v.flesh, { k: 0.03, bone2: B.HEAD });
  b.paintEll(B.NECK, [0, neck * 0.35, 0.04 * k], [0.02, neck * 0.34, 0.03], NAPE);

  // ---- arms: long, corded, ending in open clawed hands
  for (const [u, l, h, s] of [
    [B.ARM_UL, B.ARM_LL, B.HAND_L, -1],
    [B.ARM_UR, B.ARM_LR, B.HAND_R, 1],
  ] as const) {
    b.ell(u, [s * 0.006, -0.02, 0], [0.05 * k, 0.062, 0.05 * k], v.plates ? v.bone : v.flesh, { k: 0.02 });
    b.cone(u, [0, -0.02, 0], [0, -upper, 0], 0.03 * k, 0.025 * k, v.flesh, { k: 0.025, bone2: l });
    b.ell(u, [0, -upper * 0.48, -0.012 * k], [0.03 * k, upper * 0.32, 0.03 * k], v.flesh, { k: 0.02 });
    b.ell(u, [0, -upper * 0.45, 0.014 * k], [0.027 * k, upper * 0.36, 0.027 * k], v.flesh, { k: 0.02 });
    b.ell(l, [0, 0, 0.006], [0.026 * k, 0.026, 0.03 * k], v.plates ? v.bone : v.dark, { k: 0.012 });
    b.ell(l, [0, -fore * 0.3, -0.004], [0.03 * k, fore * 0.3, 0.027 * k], v.flesh, { k: 0.02 });
    b.cone(l, [0, -fore * 0.35, 0], [0, -fore, 0], 0.025 * k, 0.016 * k, v.flesh, { k: 0.02, bone2: h });
    b.ell(h, [0, -0.03, 0], [0.027 * k, 0.036, 0.012 * k], v.flesh, { k: 0.012 });
    // The cords of the forearm.
    b.paint(l, [0, -fore * 0.6, 0], [0.0035, fore * 0.36, 0.1], v.dark);
  }

  // ---- legs: bent, heavy thighs over thin shins
  for (const [u, l, f, s] of [
    [B.LEG_UL, B.LEG_LL, B.FOOT_L, -1],
    [B.LEG_UR, B.LEG_LR, B.FOOT_R, 1],
  ] as const) {
    b.cone(u, [0, 0.01, 0], [0, -thigh, 0], 0.044 * k, 0.03 * k, v.flesh, { k: 0.03, bone2: l });
    b.ell(u, [s * 0.004, -thigh * 0.42, -0.014 * k], [0.044 * k, thigh * 0.4, 0.042 * k], v.flesh, { k: 0.025 });
    b.ell(u, [0, -thigh * 0.45, 0.018 * k], [0.038 * k, thigh * 0.38, 0.036 * k], v.flesh, { k: 0.025 });
    b.ell(l, [0, 0, -0.012 * k], [0.03 * k, 0.03, 0.034 * k], v.plates ? v.bone : v.dark, { k: 0.012 });
    b.cone(l, [0, 0, 0], [0, -shin, 0], 0.028 * k, 0.018 * k, v.flesh, { k: 0.02, bone2: f });
    b.ell(l, [0, -shin * 0.3, 0.014 * k], [0.03 * k, shin * 0.28, 0.034 * k], v.flesh, { k: 0.02 });
    b.ell(f, [0, -0.01, -0.03], [0.027 * k, 0.016, 0.06], v.flesh, { k: 0.015 });
    b.paint(u, [0, -thigh * 0.5, -0.03 * k], [0.0035, thigh * 0.34, 0.04], v.dark);
  }

  // ---- head: a skull under a little flesh
  b.ell(B.HEAD, [0, hr * 1.12, hr * 0.06], [hr * 0.78, hr * 0.88, hr * 0.92], v.flesh, { k: 0.02 });
  b.ell(B.HEAD, [0, hr * 1.14, -hr * 0.66], [hr * 0.7, hr * 0.17, hr * 0.24], v.bone, { k: 0.012 });
  for (const s of [-1, 1]) b.ell(B.HEAD, [s * hr * 0.48, hr * 0.8, -hr * 0.56], [hr * 0.25, hr * 0.2, hr * 0.26], v.bone, { k: 0.015 });
  b.ell(B.HEAD, [0, hr * 0.6, -hr * 0.66], [hr * 0.48, hr * 0.22, hr * 0.3], v.bone, { k: 0.015 });
  // The lower jaw hangs a little open.
  b.cone(B.HEAD, [0, hr * 0.3, -hr * 0.26], [0, hr * 0.12, -hr * 0.7], hr * 0.44, hr * 0.3, v.bone, { k: 0.015 });
  for (const s of [-1, 1]) {
    b.carve(B.HEAD, [s * hr * 0.34, hr * 0.96, -hr * 0.9], [hr * 0.2, hr * 0.15, hr * 0.24], { k: 0.006 });
    b.paintEll(B.HEAD, [s * hr * 0.34, hr * 0.96, -hr * 0.8], [hr * 0.22, hr * 0.17, hr * 0.26], VOID);
  }
  // The gap between the jaws, and the hollow of the nose.
  b.paint(B.HEAD, [0, hr * 0.385, -hr * 0.8], [hr * 0.5, hr * 0.05, hr * 0.5], VOID);
  b.paintEll(B.HEAD, [0, hr * 0.78, -hr * 0.98], [hr * 0.09, hr * 0.11, hr * 0.16], VOID);

  const out = new MeshOut();
  b.mesh(out, cell);
  // The coarse mesh keeps the silhouette and drops the small pieces.
  const fine = cell < 1 / 60;

  // ---- face: lit eyes deep in the sockets, long teeth with no lips
  const r = new Rigid(out, b, B.HEAD);
  const into: V3 = [0, 0, 1];
  const z = -hr * 2.2;
  for (const s of [-1, 1]) {
    const cx = s * hr * 0.34;
    const cy = hr * 0.95;
    r.decal(ellipse(cx, cy, hr * 0.085, hr * 0.085, z, 10), into, 0.0008, EYE);
    r.decal(ellipse(cx, cy, hr * 0.036, hr * 0.036, z, 6), into, 0.0014, [1, 1, 0.86]);
  }
  const teeth = fine ? 8 : 4;
  const gw = hr * 0.44;
  for (const [y0, y1] of [
    [hr * 0.435, hr * 0.56],
    [hr * 0.335, hr * 0.2],
  ]) {
    for (let t = -teeth; t < teeth; t++) {
      const x0 = (t / teeth) * gw + hr * 0.008;
      const x1 = ((t + 1) / teeth) * gw - hr * 0.008;
      // Each tooth narrows to its tip: long ones in front, shorter at the back.
      const reach = 1 - 0.4 * Math.abs((t + 0.5) / teeth);
      const tip = y1 + (y0 - y1) * reach * 1.12;
      const w = x1 - x0;
      const upper = y1 > y0;
      r.decal(
        upper
          ? [
              [x1 - w * 0.32, tip, z],
              [x0 + w * 0.32, tip, z],
              [x0, y1, z],
              [x1, y1, z],
            ]
          : [
              [x1, y1, z],
              [x0, y1, z],
              [x0 + w * 0.32, tip, z],
              [x1 - w * 0.32, tip, z],
            ],
        into,
        0.001,
        v.bone,
      );
    }
  }

  // ---- claws, spines, horns, plates: small hard pieces
  for (const [h, s] of [
    [B.HAND_L, -1],
    [B.HAND_R, 1],
  ] as const) {
    const hand = new Rigid(out, b, h);
    for (let f = 0; f < 4; f++) {
      const x = (f - 1.5) * 0.013 * k;
      const curl = 0.01 + f * 0.002;
      hand.tube(
        [
          [x, -0.055, 0],
          [x * 1.25, -0.095, -curl],
          [x * 1.4, -0.128, -curl * 2.6],
        ],
        [0.0075 * k, 0.0062 * k, 0.0045 * k],
        v.flesh,
        fine ? 5 : 3,
      );
      hand.tube(
        [
          [x * 1.4, -0.126, -curl * 2.5],
          [x * 1.5, -0.158, -curl * 4.4],
        ],
        [0.0045 * k, 0.0006],
        v.bone,
        fine ? 5 : 3,
      );
    }
    hand.tube(
      [
        [-s * 0.024 * k, -0.02, -0.004],
        [-s * 0.04 * k, -0.05, -0.016],
        [-s * 0.044 * k, -0.078, -0.03],
      ],
      [0.0085 * k, 0.007 * k, 0.001],
      v.flesh,
      fine ? 5 : 3,
    );
  }
  for (const f of [B.FOOT_L, B.FOOT_R]) {
    const foot = new Rigid(out, b, f);
    for (const x of [-0.016, 0, 0.016]) {
      foot.tube(
        [
          [x * k, -0.012, -0.08],
          [x * k * 1.3, -0.022, -0.115],
        ],
        [0.007 * k, 0.0008],
        v.bone,
        fine ? 5 : 3,
      );
    }
  }
  // Spines down the back, longest between the shoulders.
  const spine = (bone: number, y: number, len: number, back: number) => {
    new Rigid(out, b, bone).tube(
      [
        [0, y, back - 0.01],
        [0, y + len * 0.5, back + len],
      ],
      [0.013 * k, 0.0008],
      v.bone,
      fine ? 6 : 3,
    );
  };
  for (let n = 0; n < 4; n++) spine(B.CHEST, chest * (0.12 + n * 0.24), v.spines * (0.7 + 0.3 * Math.sin((n / 3) * Math.PI)), 0.075 * k);
  spine(B.SPINE, 0.02, v.spines * 0.6, 0.045 * k);
  spine(B.SPINE, 0.065, v.spines * 0.7, 0.048 * k);
  spine(B.PELVIS, 0.01, v.spines * 0.5, 0.05 * k);
  if (v.horns) {
    for (const s of [-1, 1]) {
      r.tube(
        [
          [s * hr * 0.5, hr * 1.36, -hr * 0.36],
          [s * hr * 0.86, hr * 1.78, -hr * 0.06],
          [s * hr * 0.84, hr * 2.2, hr * 0.5],
        ],
        [hr * 0.17, hr * 0.11, hr * 0.012],
        v.bone,
        fine ? 6 : 4,
      );
    }
  }
  if (v.plates) {
    // A crest over the skull and a plate on each shoulder.
    for (let n = 0; n < 3; n++) {
      r.tube(
        [
          [0, hr * 1.82, hr * (-0.36 + n * 0.4)],
          [0, hr * 2.16, hr * (-0.2 + n * 0.46)],
        ],
        [hr * 0.15, hr * 0.012],
        v.bone,
        fine ? 6 : 4,
      );
    }
  }
  if (v.hair) {
    // Long wet strands over the face and down the back.
    for (let n = 0; n < (fine ? 11 : 5); n++) {
      const a = ((n + 0.5) / (fine ? 11 : 5)) * Math.PI * 2;
      const front = Math.cos(a) > 0.3;
      const len = front ? hr * 1.5 : hr * 3.4;
      const x = Math.sin(a) * hr * 0.82;
      const zz = hr * 0.06 - Math.cos(a) * hr * 0.9;
      r.tube(
        [
          [x * 0.3, hr * 1.96, zz * 0.3 + hr * 0.06],
          [x, hr * 1.5, zz],
          [x * 1.06, hr * 1.5 - len * 0.5, zz * 1.08],
          [x * 1.0, hr * 1.5 - len, zz * 1.05],
        ],
        [hr * 0.1, hr * 0.14, hr * 0.1, hr * 0.01],
        v.hair,
        4,
        { axis: [Math.sin(a), 0, -Math.cos(a)], factor: 0.35 },
      );
    }
  }
  return out.model();
}
