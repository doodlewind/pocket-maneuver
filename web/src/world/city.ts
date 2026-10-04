// The world: a walled town on a radial plan, its canal, the fields outside
// and a stand of giant trees. Everything derives from one seed.
//
// Plan: ring streets around a central square, radial lanes between them, four
// avenues from the gates to the square. A block is the piece between two rings
// and two lanes; row houses line its four sides around a yard.

import { STRIP, stripV } from "./atlas";
import { ALL, at, emitHouse, frame, Frame, House, KIND, NEAR_MID, PLASTER, polar, ROOFS, World } from "./build";
import { cross, Layer, norm, Rgb, sub, up, UP, V3 } from "./geo";
import { fbm, hash01, Rng } from "./rng";

export const R_WALL = 600;
export const WALL_T = 14;
export const WALL_H = 50;
export const R_WORLD = 1150;
const CANAL_IN = 296;
const CANAL_OUT = 312;
const WATER_Y = -1.5;
const BED_Y = -2.6;
const PLAZA = 42;

interface Band {
  rin: number;
  rout: number;
  /** Lane angles, ascending, and the half width of each lane. */
  spokes: { a: number; hw: number }[];
}

const TAU = Math.PI * 2;
const DEG = Math.PI / 180;

/** Half widths of the ring streets at each band edge are implied by the gaps between bands. */
const BAND_RADII: readonly [number, number][] = [
  [PLAZA + 4, 104],
  [112, 168],
  [176, 233],
  [243, 290],
  [318, 368],
  [376, 433],
  [443, 500],
  [508, 562],
];

export interface Entities {
  spawn: [number, number, number, number];
  dummies: { pos: V3; yaw: number; height: number; nape: V3 }[];
  depots: { pos: V3; radius: number }[];
  waypoints: V3[];
}

export interface Plan {
  bands: Band[];
}

function makePlan(rng: Rng): Plan {
  const bands: Band[] = BAND_RADII.map(([rin, rout], k) => {
    const mid = (rin + rout) / 2;
    // Bands on both sides of the canal share eight lane angles, so their lanes meet at the bridges.
    const step = k === 3 || k === 4 ? 8 : 4;
    const count = Math.max(8, Math.round((TAU * mid) / 70 / step) * step);
    const spokes = [];
    for (let i = 0; i < count; i++) {
      const base = (i / count) * TAU;
      const principal = (i * 8) % count === 0;
      const cardinal = (i * 4) % count === 0;
      const jitter = principal ? 0 : rng.range(-0.18, 0.18) * (TAU / count);
      spokes.push({ a: base + jitter, hw: cardinal ? 6 : principal ? 4 : 3 });
    }
    return { rin, rout, spokes };
  });
  return { bands };
}

/** Signed smallest difference between two angles. */
function angDiff(a: number, b: number): number {
  let d = (a - b) % TAU;
  if (d > Math.PI) d -= TAU;
  if (d < -Math.PI) d += TAU;
  return d;
}

// ---------------------------------------------------------------------------- ground

type Surface = { strip: (typeof STRIP)[keyof typeof STRIP]; tint: Rgb } | null;

function surfaceAt(plan: Plan, r: number, a: number, margin: number): Surface {
  const cobble: Rgb = [0.92, 0.9, 0.87];
  if (r < PLAZA) return { strip: STRIP.COBBLE, tint: [1.0, 0.97, 0.92] };
  if (r > CANAL_IN && r < CANAL_OUT) return null;
  if (r >= R_WALL) return null;
  if (r > 574) return { strip: STRIP.DIRT, tint: [0.95, 0.92, 0.86] };
  for (const b of plan.bands) {
    if (r < b.rin + margin || r > b.rout - margin) continue;
    // Inside a band: a lane, or the yard.
    for (const s of b.spokes) {
      if (Math.abs(angDiff(a, s.a)) * r < s.hw + margin) return { strip: STRIP.COBBLE, tint: cobble };
    }
    const g = fbm(Math.cos(a) * r * 0.05 + 40, Math.sin(a) * r * 0.05 + 40, 11, 3);
    return g > 0.47 ? { strip: STRIP.GRASS, tint: [0.92, 0.95, 0.88] } : { strip: STRIP.DIRT, tint: [0.9, 0.86, 0.8] };
  }
  return { strip: STRIP.COBBLE, tint: cobble };
}

/** A polar grid of ground from `r0` to `r1`, `dr` deep, with arcs near `arc` metres. */
function polarGround(w: World, layers: readonly Layer[], r0: number, r1: number, dr: number, arc: number, surf: (r: number, a: number) => Surface, collide: boolean) {
  const segs = (r: number) => Math.max(8, 2 ** Math.ceil(Math.log2((TAU * Math.max(r, 1)) / arc)));
  for (let r = r0; r < r1 - 1e-6; r += dr) {
    const ro = Math.min(r + dr, r1);
    const ni = r <= 0 ? 0 : segs(r);
    const no = segs(ro);
    const rm = (r + ro) / 2;
    for (let j = 0; j < no; j++) {
      const a0 = (j / no) * TAU;
      const a1 = ((j + 1) / no) * TAU;
      const s = surf(rm, (a0 + a1) / 2);
      if (!s) continue;
      const [vb, vt] = stripV(s.strip);
      const u0 = ((j * rm * (TAU / no)) / s.strip.mPerU) % 1;
      const u1 = u0 + (rm * (TAU / no)) / s.strip.mPerU;
      const o0 = polar(ro, a0);
      const o1 = polar(ro, a1);
      const key = polar(rm, (a0 + a1) / 2);
      const kind = collide ? KIND.GROUND : -1;
      if (ni === 0) {
        w.tri(layers, key, [0, 0, 0], o1, o0, [(u0 + u1) / 2, vb], [u1, vt], [u0, vt], s.tint, kind);
      } else if (ni === no) {
        w.quad(layers, key, polar(r, a0), polar(r, a1), o1, o0, [u0, u1, vb, vt], s.tint, kind);
      } else {
        // The outer ring has twice the segments: two outer edges share one inner edge.
        const i0 = polar(r, (Math.floor(j / 2) / ni) * TAU);
        const i1 = polar(r, ((Math.floor(j / 2) + 1) / ni) * TAU);
        if (j % 2 === 0) {
          w.tri(layers, key, i0, o1, o0, [u0, vb], [u1, vt], [u0, vt], s.tint, kind);
          w.tri(layers, key, i0, i1, o1, [u0, vb], [u1, vb], [u1, vt], s.tint, kind);
        } else {
          w.tri(layers, key, i1, o1, o0, [u0, vb], [u1, vt], [u0, vt], s.tint, kind);
        }
      }
    }
  }
}

function ground(w: World, plan: Plan) {
  // The ground is flat, so the coarser grid carries the collision.
  // Both banks end on the canal's edges at every level of detail.
  for (const [r0, r1] of [
    [0, CANAL_IN],
    [CANAL_OUT, R_WALL],
  ]) {
    polarGround(w, [Layer.Near], r0, r1, 4, 4.6, (r, a) => surfaceAt(plan, r, a, 3), false);
    polarGround(w, [Layer.Mid], r0, r1, 12, 12, (r, a) => surfaceAt(plan, r, a, 4), true);
    polarGround(w, [Layer.Far], r0, r1, 24, 40, (r, a) => surfaceAt(plan, r, a, 6), false);
  }

  // Fields beyond the wall: a patchwork, with a road from each gate.
  const field = (r: number, a: number): Surface => {
    for (let c = 0; c < 4; c++) {
      if (Math.abs(angDiff(a, (c * Math.PI) / 2)) * r < 6) return { strip: STRIP.DIRT, tint: [0.93, 0.88, 0.8] };
    }
    const patch = hash01(Math.floor(r / 96), Math.floor((a / TAU) * 40), 5);
    if (patch < 0.34) return { strip: STRIP.GRASS, tint: [1.42, 1.22, 0.74] };
    if (patch < 0.5) return { strip: STRIP.DIRT, tint: [0.78, 0.7, 0.62] };
    return { strip: STRIP.GRASS, tint: [0.86 + patch * 0.2, 0.98, 0.8] };
  };
  const outer = R_WALL + WALL_T;
  polarGround(w, [Layer.Base], outer, outer + 16 * 34, 16, 20, field, true);
  polarGround(w, [Layer.Far], outer, outer + 16 * 34, 68, 80, field, false);
}

// ---------------------------------------------------------------------------- blocks and houses

interface Block {
  band: number;
  sector: number;
  /** Corners: inner at the first lane, inner at the second, outer at the second, outer at the first. */
  a: V3;
  b: V3;
  c: V3;
  d: V3;
}

function blocks(plan: Plan): Block[] {
  const out: Block[] = [];
  plan.bands.forEach((band, k) => {
    const n = band.spokes.length;
    for (let i = 0; i < n; i++) {
      const s0 = band.spokes[i];
      const s1 = band.spokes[(i + 1) % n];
      const a1 = i + 1 === n ? s1.a + TAU : s1.a;
      out.push({
        band: k,
        sector: i,
        a: polar(band.rin, s0.a + s0.hw / band.rin),
        b: polar(band.rin, a1 - s1.hw / band.rin),
        c: polar(band.rout, a1 - s1.hw / band.rout),
        d: polar(band.rout, s0.a + s0.hw / band.rout),
      });
    }
  });
  return out;
}

/** The frame of a row along the edge `p`–`q` whose front faces away from `inside`. */
function edgeFrame(p: V3, q: V3, inside: V3): { f: Frame; length: number } {
  const t = norm(sub(q, p));
  let n = cross(t, UP);
  const mid: V3 = [(p[0] + q[0]) / 2, 0, (p[2] + q[2]) / 2];
  if ((mid[0] - inside[0]) * n[0] + (mid[2] - inside[2]) * n[2] < 0) n = [-n[0], 0, -n[2]];
  const r = cross(UP, n);
  const left = (q[0] - p[0]) * r[0] + (q[2] - p[2]) * r[2] > 0 ? p : q;
  return { f: { o: left, r, n }, length: Math.hypot(q[0] - p[0], q[2] - p[2]) };
}

function rowHouses(w: World, rng: Rng, f: Frame, from: number, to: number, depth: number, band: number, grand: boolean) {
  let x = from;
  while (to - x > 4.5) {
    let width = rng.range(5.6, 9.6);
    if (to - x - width < 5) width = to - x;
    if (rng.chance(0.035) && width < 8) {
      // A gap into the yard.
      x += width;
      continue;
    }
    const tall = band <= 1 ? rng.pick([3, 3, 4, 4]) : band <= 4 ? rng.pick([2, 3, 3, 4]) : rng.pick([2, 2, 3, 3]);
    const style = rng.chance(grand ? 0.6 : 0.26) ? 1 : 0;
    const storeys = Math.min(grand ? tall + (tall < 4 ? 1 : 0) : tall, style === 1 ? 3 : 4);
    const d = depth * rng.range(0.86, 1.0);
    const gableFront = width < 8.6 && rng.chance(0.36);
    const pitch = Math.tan(rng.range(46, 56) * DEG);
    const house: House = {
      f: { o: at(f, x, 0, 0), r: f.r, n: f.n },
      w: width,
      d,
      storeys,
      style,
      plaster: rng.pick(PLASTER),
      roof: rng.pick(ROOFS),
      gableFront,
      rise: Math.min(((gableFront ? width : d) / 2) * pitch, 7.2),
      jetty: style === 0 && rng.chance(0.65) ? 0.32 : 0,
      chimney: rng.chance(0.6),
      seed: rng.int(0, 1 << 30),
    };
    emitHouse(w, house);
    x += width;
  }
}

function houses(w: World, plan: Plan, rng: Rng, special: Set<string>) {
  for (const blk of blocks(plan)) {
    if (special.has(`${blk.band}:${blk.sector}`)) continue;
    const r = rng.fork(blk.band * 1000 + blk.sector);
    const centre: V3 = [(blk.a[0] + blk.b[0] + blk.c[0] + blk.d[0]) / 4, 0, (blk.a[2] + blk.b[2] + blk.c[2] + blk.d[2]) / 4];
    const band = plan.bands[blk.band];
    const depthT = Math.min(r.range(8.5, 11), (band.rout - band.rin) / 2 - 7);
    const innerW = Math.hypot(blk.b[0] - blk.a[0], blk.b[2] - blk.a[2]);
    const depthR = Math.max(5, Math.min(r.range(8, 10), innerW / 2 - 5));
    const grand = blk.band === 0;
    // The two rows on the ring streets run corner to corner; the rows on the lanes fit between them.
    for (const [p, q, depth, inset] of [
      [blk.a, blk.b, depthT, 0],
      [blk.c, blk.d, depthT, 0],
      [blk.b, blk.c, depthR, depthT],
      [blk.d, blk.a, depthR, depthT],
    ] as const) {
      const e = edgeFrame(p, q, centre);
      if (e.length - inset * 2 > 5) rowHouses(w, r, e.f, inset, e.length - inset, depth, blk.band, grand);
    }
    // A yard tree now and then.
    if (r.chance(0.5)) tree(w, [centre[0] + r.range(-4, 4), 0, centre[2] + r.range(-4, 4)], r.range(7, 11), r);
  }
}

function tree(w: World, p: V3, h: number, r: Rng) {
  const bark: Rgb = [0.5, 0.42, 0.36];
  const trunk = w.ngon(p, 0.35, 5, r.range(0, 6));
  w.prism(NEAR_MID, p, trunk, 0, h * 0.5, STRIP.WOOD, bark, -1, { rows: 1 });
  const leaf: Rgb = [0.5 + r.range(0, 0.15), 0.72 + r.range(0, 0.12), 0.42];
  blob(w, ALL, p, up(p, h * 0.72), h * 0.34, leaf, r);
}

/** A faceted crown: two stacked rings and two caps. */
function blob(w: World, layers: readonly Layer[], key: V3, c: V3, radius: number, tint: Rgb, r: Rng) {
  const [vb, vt] = stripV(STRIP.GRASS);
  const n = 6;
  const phase = r.range(0, 6);
  const ring = (y: number, k: number) => {
    const pts: V3[] = [];
    for (let i = 0; i < n; i++) {
      const a = phase + (i / n) * TAU;
      pts.push([c[0] + Math.cos(a) * radius * k, c[1] + y * radius, c[2] + Math.sin(a) * radius * k]);
    }
    return pts;
  };
  const lo = ring(-0.45, 0.8);
  const hi = ring(0.3, 0.9);
  const top: V3 = [c[0], c[1] + radius, c[2]];
  const bot: V3 = [c[0], c[1] - radius * 0.8, c[2]];
  const dark: Rgb = [tint[0] * 0.72, tint[1] * 0.72, tint[2] * 0.72];
  for (let i = 0; i < n; i++) {
    const j = (i + 1) % n;
    w.quad(layers, key, lo[j], lo[i], hi[i], hi[j], [i / n, (i + 1) / n, vb, vt], tint);
    w.tri(layers, key, hi[j], hi[i], top, [0, vb], [0.3, vb], [0.15, vt], tint);
    w.tri(layers, key, lo[i], lo[j], bot, [0, vb], [0.3, vb], [0.15, vt], dark);
  }
}

// ---------------------------------------------------------------------------- the wall

const WALL_SEGS = 96;
const STONE_DARK: Rgb = [0.8, 0.78, 0.74];
const STONE_LIGHT: Rgb = [0.96, 0.94, 0.9];

function wall(w: World) {
  const step = TAU / WALL_SEGS;
  const rin = R_WALL;
  const rout = R_WALL + WALL_T;
  for (let i = 0; i < WALL_SEGS; i++) {
    const a0 = (i - 0.5) * step;
    const a1 = (i + 0.5) * step;
    const gate = i % (WALL_SEGS / 4) === 0;
    const key = polar(rin + WALL_T / 2, i * step);
    const i0 = polar(rin, a0);
    const i1 = polar(rin, a1);
    const o0 = polar(rout, a0);
    const o1 = polar(rout, a1);
    if (!gate) {
      w.stack([Layer.Base], key, i0, i1, 0, WALL_H, STRIP.STONE, STONE_DARK, KIND.STONE, 8, i * 0.31);
      w.stack([Layer.Base], key, o1, o0, 0, WALL_H, STRIP.STONE, STONE_DARK, KIND.STONE, 8, i * 0.17);
      w.stack([Layer.Far], key, i0, i1, 0, WALL_H, STRIP.STONE, STONE_DARK, -1, 4, i * 0.31);
      w.stack([Layer.Far], key, o1, o0, 0, WALL_H, STRIP.STONE, STONE_DARK, -1, 4, i * 0.17);
    } else {
      gateSegment(w, key, i * step, a0, a1);
    }
    // The walk on top, and a parapet on each edge.
    w.floor([Layer.Base, Layer.Far], key, i0, i1, o1, o0, STRIP.STONE, STONE_LIGHT, KIND.GROUND, i * 0.4);
    for (const [ra, rb] of [
      [rin, rin + 0.7],
      [rout - 0.7, rout],
    ]) {
      const pa0 = polar(ra, a0);
      const pa1 = polar(ra, a1);
      const pb0 = polar(rb, a0);
      const pb1 = polar(rb, a1);
      w.stack([Layer.Base], key, pa0, pa1, WALL_H, WALL_H + 1.2, STRIP.STONE, STONE_DARK, KIND.STONE, 1, i * 0.2);
      w.stack([Layer.Base], key, pb1, pb0, WALL_H, WALL_H + 1.2, STRIP.STONE, STONE_DARK, KIND.STONE, 1, i * 0.2);
      w.floor([Layer.Base], key, up(pa0, WALL_H + 1.2), up(pa1, WALL_H + 1.2), up(pb1, WALL_H + 1.2), up(pb0, WALL_H + 1.2), STRIP.STONE, STONE_LIGHT, -1);
    }
  }
  // Towers between the gates.
  for (let k = 0; k < 16; k++) {
    if (k % 4 === 0) continue;
    const a = (k / 16) * TAU;
    const c = polar(rin + 2, a);
    const pts = w.ngon(c, 11, 8, a);
    w.prism([Layer.Base, Layer.Far], c, pts, 0, WALL_H + 9, STRIP.STONE, STONE_LIGHT, KIND.STONE, { rows: 5 });
    w.spire([Layer.Base, Layer.Far], c, pts, WALL_H + 9, 13, [0.5, 0.36, 0.3]);
  }
}

/** A wall segment with an arched passage, and the gatehouse on top of it. */
function gateSegment(w: World, key: V3, a: number, a0: number, a1: number) {
  const rin = R_WALL;
  const rout = R_WALL + WALL_T;
  const half = 5;
  const high = 11;
  const base: readonly Layer[] = [Layer.Base, Layer.Far];
  // The passage runs along the radius at `a`; `t` is the tangent.
  const rad: V3 = [Math.cos(a), 0, Math.sin(a)];
  const t: V3 = [-Math.sin(a), 0, Math.cos(a)];
  const P = (r: number, s: number): V3 => [rad[0] * r + t[0] * s, 0, rad[2] * r + t[2] * s];
  for (const [r, flip] of [
    [rin, false],
    [rout, true],
  ] as const) {
    const e0 = polar(r, a0);
    const e1 = polar(r, a1);
    const l = P(r, -half);
    const rr = P(r, half);
    const face = (p: V3, q: V3, y0: number, y1: number, rows: number) => (flip ? w.stack(base, key, q, p, y0, y1, STRIP.STONE, STONE_DARK, KIND.STONE, rows) : w.stack(base, key, p, q, y0, y1, STRIP.STONE, STONE_DARK, KIND.STONE, rows));
    face(e0, l, 0, WALL_H, 8);
    face(rr, e1, 0, WALL_H, 8);
    face(l, rr, high, WALL_H, 6);
  }
  // Passage walls and ceiling.
  const shade: Rgb = [0.5, 0.48, 0.46];
  w.stack(base, key, P(rin, -half), P(rout, -half), 0, high, STRIP.STONE, shade, KIND.STONE, 2);
  w.stack(base, key, P(rout, half), P(rin, half), 0, high, STRIP.STONE, shade, KIND.STONE, 2);
  w.floor(base, key, up(P(rin, half), high), up(P(rin, -half), high), up(P(rout, -half), high), up(P(rout, half), high), STRIP.STONE, shade, KIND.STONE);
  w.floor([Layer.Base, Layer.Far], key, P(rin, -half), P(rin, half), P(rout, half), P(rout, -half), STRIP.COBBLE, [0.9, 0.88, 0.85], KIND.GROUND);
  // The gatehouse: a block across the wall, with a flat fighting top.
  const pts = [P(rout + 5, -13), P(rin - 5, -13), P(rin - 5, 13), P(rout + 5, 13)];
  // Order for `prism`: angles growing from +X toward +Z around the footprint.
  const ordered = orderFootprint(pts);
  w.prism(base, key, ordered, WALL_H, WALL_H + 12, STRIP.STONE, STONE_LIGHT, KIND.STONE, { top: STRIP.COBBLE, topTint: STONE_LIGHT, topKind: KIND.GROUND, rows: 2 });
  // Buttresses beside the passage, inside and outside.
  for (const side of [-1, 1]) {
    for (const [r0, r1] of [
      [rin - 5, rin],
      [rout, rout + 5],
    ]) {
      const q = orderFootprint([P(r0, side * half), P(r1, side * half), P(r1, side * 13), P(r0, side * 13)]);
      w.prism(base, key, q, 0, WALL_H, STRIP.STONE, STONE_LIGHT, KIND.STONE, { rows: 8 });
    }
  }
}

/** Orders a convex footprint the way `prism` expects. */
function orderFootprint(pts: V3[]): V3[] {
  const cx = pts.reduce((s, p) => s + p[0], 0) / pts.length;
  const cz = pts.reduce((s, p) => s + p[2], 0) / pts.length;
  return [...pts].sort((p, q) => Math.atan2(p[2] - cz, p[0] - cx) - Math.atan2(q[2] - cz, q[0] - cx));
}

// ---------------------------------------------------------------------------- canal

function canal(w: World, plan: Plan) {
  const n = 192;
  const [vb, vt] = stripV(STRIP.WATER);
  for (let i = 0; i < n; i++) {
    const a0 = (i / n) * TAU;
    const a1 = ((i + 1) / n) * TAU;
    const key = polar(304, (a0 + a1) / 2);
    const arc = (304 * TAU) / n / STRIP.WATER.mPerU;
    w.quad(ALL, key, polar(CANAL_IN, a0, WATER_Y), polar(CANAL_IN, a1, WATER_Y), polar(CANAL_OUT, a1, WATER_Y), polar(CANAL_OUT, a0, WATER_Y), [0, arc, vb, vt], [1, 1, 1]);
    w.col.quad(polar(CANAL_IN, a0, BED_Y), polar(CANAL_IN, a1, BED_Y), polar(CANAL_OUT, a1, BED_Y), polar(CANAL_OUT, a0, BED_Y), KIND.WATER);
    // Quay walls face the water.
    w.stack(NEAR_MID, key, up(polar(CANAL_IN, a1), BED_Y), up(polar(CANAL_IN, a0), BED_Y), 0, -BED_Y, STRIP.STONE, STONE_DARK, KIND.STONE, 1, i * 0.3);
    w.stack(NEAR_MID, key, up(polar(CANAL_OUT, a0), BED_Y), up(polar(CANAL_OUT, a1), BED_Y), 0, -BED_Y, STRIP.STONE, STONE_DARK, KIND.STONE, 1, i * 0.3);
  }
  // Bridges where the lanes of both banks line up.
  const inner = plan.bands[3];
  for (const s of inner.spokes) {
    if (s.hw < 4) continue;
    const a = s.a;
    const rad: V3 = [Math.cos(a), 0, Math.sin(a)];
    const t: V3 = [-Math.sin(a), 0, Math.cos(a)];
    const hw = s.hw + 0.5;
    const P = (r: number, side: number, y = 0): V3 => [rad[0] * r + t[0] * side, y, rad[2] * r + t[2] * side];
    const key = P(304, 0);
    const r0 = CANAL_IN - 1;
    const r1 = CANAL_OUT + 1;
    w.floor(ALL, key, P(r0, -hw, 0.02), P(r0, hw, 0.02), P(r1, hw, 0.02), P(r1, -hw, 0.02), STRIP.COBBLE, [0.95, 0.93, 0.9], KIND.GROUND);
    for (const side of [-1, 1]) {
      // Spandrel under the deck and a parapet above it, both faces.
      const e0 = P(r0, side * hw);
      const e1 = P(r1, side * hw);
      const [p, q] = side > 0 ? [e0, e1] : [e1, e0];
      w.stack(NEAR_MID, key, up(p, WATER_Y), up(q, WATER_Y), 0, 1.0 - WATER_Y, STRIP.STONE, STONE_LIGHT, KIND.STONE, 1);
      const f0 = P(r0, side * (hw - 0.5));
      const f1 = P(r1, side * (hw - 0.5));
      const [p2, q2] = side > 0 ? [f1, f0] : [f0, f1];
      w.stack(NEAR_MID, key, p2, q2, 0, 1.0, STRIP.STONE, STONE_LIGHT, KIND.STONE, 1);
      w.floor(NEAR_MID, key, up(side > 0 ? f0 : e0, 1.0), up(side > 0 ? e0 : f0, 1.0), up(side > 0 ? e1 : f1, 1.0), up(side > 0 ? f1 : e1, 1.0), STRIP.STONE, STONE_LIGHT, -1);
    }
  }
}

// ---------------------------------------------------------------------------- landmarks

const SLATE: Rgb = [0.4, 0.43, 0.5];
const COPPER: Rgb = [0.42, 0.62, 0.55];
const BASE_FAR: readonly Layer[] = [Layer.Base, Layer.Far];

/** A frame for a building in a block: fronted on the block's inner edge, centred, facing the town centre. */
function blockFrame(blk: Block, width: number, setback: number): Frame {
  const mid: V3 = [(blk.a[0] + blk.b[0]) / 2, 0, (blk.a[2] + blk.b[2]) / 2];
  const n = norm([-mid[0], 0, -mid[2]]);
  const r = cross(UP, n);
  return { o: [mid[0] - r[0] * (width / 2) - n[0] * setback, 0, mid[2] - r[2] * (width / 2) - n[2] * setback], r, n };
}

function yard(w: World, blk: Block, tint: Rgb) {
  // Pave the whole block (the fine ground under it stays; this sits a hair above).
  w.floor(NEAR_MID, blk.a, up(blk.a, 0.03), up(blk.b, 0.03), up(blk.c, 0.03), up(blk.d, 0.03), STRIP.COBBLE, tint, -1);
}

function townHall(w: World, blk: Block) {
  yard(w, blk, [1, 0.96, 0.9]);
  const f = blockFrame(blk, 34, 6);
  const key = at(f, 17, 0, -10);
  // An arcaded stone ground storey, three plastered storeys, a steep slate roof.
  w.box(BASE_FAR, key, f, 34, 20, 0, 5, STRIP.STONE, STONE_LIGHT, KIND.STONE, { rows: 1 });
  emitHouse(w, { f: { o: at(f, 0, 5, 0), r: f.r, n: f.n }, w: 34, d: 20, storeys: 3, style: 1, plaster: [1, 0.93, 0.78], roof: SLATE, gableFront: false, rise: 11, jetty: 0, chimney: false, seed: 77 });
  // The clock tower stands at the right corner.
  const tf: Frame = { o: at(f, 34, 0, 2), r: f.r, n: f.n };
  w.box(BASE_FAR, key, tf, 8, 8, 0, 44, STRIP.STONE, STONE_LIGHT, KIND.STONE, { rows: 7 });
  const top = [at(tf, 0, 0, 0), at(tf, 0, 0, -8), at(tf, 8, 0, -8), at(tf, 8, 0, 0)];
  w.spire(BASE_FAR, key, top, 44, 13, COPPER);
  // Clock faces.
  const [vb, vt] = stripV(STRIP.FLAT);
  const dial: Rgb = [0.95, 0.9, 0.7];
  w.quad([Layer.Base], key, at(tf, 2, 34, 0.06), at(tf, 6, 34, 0.06), at(tf, 6, 38, 0.06), at(tf, 2, 38, 0.06), [0, 0.1, vb, vt], dial);
  w.quad([Layer.Base], key, at(tf, 8.06, 34, -2), at(tf, 8.06, 34, -6), at(tf, 8.06, 38, -6), at(tf, 8.06, 38, -2), [0, 0.1, vb, vt], dial);
}

function cathedral(w: World, blk: Block) {
  yard(w, blk, [0.96, 0.94, 0.9]);
  const f = blockFrame(blk, 22, 8);
  const key = at(f, 11, 0, -28);
  const stone: Rgb = [0.98, 0.95, 0.9];
  const length = Math.min(54, Math.hypot(blk.d[0] - blk.a[0], blk.d[2] - blk.a[2]) - 14);
  // Nave.
  w.box(BASE_FAR, key, { o: at(f, 0, 0, -9), r: f.r, n: f.n }, 22, length - 9, 0, 21, STRIP.STONE, stone, KIND.STONE, { rows: 4 });
  w.gableRoof(BASE_FAR, key, { o: at(f, 0, 0, -9), r: f.r, n: f.n }, 22, length - 9, 21, 13, false, SLATE, { tint: stone });
  // Lancet windows down both sides.
  const [vb, vt] = stripV(STRIP.FLAT);
  const glass: Rgb = [0.1, 0.12, 0.2];
  for (let z = 14; z < length - 4; z += 6.5) {
    w.quad([Layer.Base], key, at(f, 22.06, 5, -z), at(f, 22.06, 5, -z - 2), at(f, 22.06, 17, -z - 2), at(f, 22.06, 17, -z), [0, 0.1, vb, vt], glass);
    w.quad([Layer.Base], key, at(f, -0.06, 5, -z - 2), at(f, -0.06, 5, -z), at(f, -0.06, 17, -z), at(f, -0.06, 17, -z - 2), [0, 0.1, vb, vt], glass);
  }
  // Two west towers with spires, and a flèche over the crossing.
  for (const x of [-1, 13]) {
    const tf: Frame = { o: at(f, x, 0, 0), r: f.r, n: f.n };
    w.box(BASE_FAR, key, tf, 10, 10, 0, 50, STRIP.STONE, stone, KIND.STONE, { rows: 8 });
    w.spire(BASE_FAR, key, [at(tf, 0, 0, 0), at(tf, 0, 0, -10), at(tf, 10, 0, -10), at(tf, 10, 0, 0)], 50, 24, SLATE);
    w.quad([Layer.Base], key, at(tf, 3.5, 36, 0.06), at(tf, 6.5, 36, 0.06), at(tf, 6.5, 46, 0.06), at(tf, 3.5, 46, 0.06), [0, 0.1, vb, vt], glass);
  }
  // The portal and the rose window between the towers.
  w.quad([Layer.Base], key, at(f, 8.5, 0, -8.94), at(f, 13.5, 0, -8.94), at(f, 13.5, 9, -8.94), at(f, 8.5, 9, -8.94), [0, 0.1, vb, vt], [0.2, 0.13, 0.09]);
  w.quad([Layer.Base], key, at(f, 8, 13, -8.94), at(f, 14, 13, -8.94), at(f, 14, 19, -8.94), at(f, 8, 19, -8.94), [0, 0.1, vb, vt], glass);
  const c = at(f, 11, 0, -length * 0.62);
  const fl = w.ngon(c, 2.2, 4, 0.6);
  w.prism(BASE_FAR, key, fl, 30, 38, STRIP.STONE, stone, KIND.STONE, { rows: 1 });
  w.spire(BASE_FAR, key, fl, 38, 20, SLATE);
}

function keep(w: World, blk: Block, ent: Entities) {
  yard(w, blk, [0.9, 0.88, 0.84]);
  const f = blockFrame(blk, 26, 12);
  const key = at(f, 13, 0, -13);
  w.box(BASE_FAR, key, f, 26, 26, 0, 30, STRIP.STONE, STONE_LIGHT, KIND.STONE, { top: STRIP.COBBLE, topTint: STONE_LIGHT, rows: 5 });
  for (const [x, z] of [
    [0, 0],
    [26, 0],
    [26, -26],
    [0, -26],
  ]) {
    const c = at(f, x, 0, z);
    const pts = w.ngon(c, 4.6, 6, 0.3);
    w.prism(BASE_FAR, key, pts, 0, 37, STRIP.STONE, STONE_DARK, KIND.STONE, { rows: 6 });
    w.spire(BASE_FAR, key, pts, 37, 9, [0.5, 0.33, 0.27]);
  }
  depot(w, ent, at(f, 13, 30, -13));
}

function marketHall(w: World, blk: Block) {
  yard(w, blk, [1, 0.97, 0.92]);
  const f = blockFrame(blk, 36, 7);
  emitHouse(w, { f, w: 36, d: 15, storeys: 2, style: 0, plaster: [1, 0.95, 0.84], roof: [0.7, 0.33, 0.25], gableFront: false, rise: 7.2, jetty: 0.32, chimney: false, seed: 99 });
}

function watchTower(w: World, c: V3, r: Rng) {
  const pts = w.ngon(c, 4.4, 6, r.range(0, 1));
  w.prism(BASE_FAR, c, pts, 0, 33, STRIP.STONE, STONE_LIGHT, KIND.STONE, { rows: 5 });
  w.spire(BASE_FAR, c, pts, 33, 8, [0.52, 0.34, 0.28]);
}

function plaza(w: World, ent: Entities) {
  const c: V3 = [0, 0, 0];
  const basin = w.ngon(c, 6, 8, 0.2);
  w.prism(NEAR_MID, c, basin, 0, 0.9, STRIP.STONE, STONE_LIGHT, KIND.STONE, { top: STRIP.WATER, topTint: [1, 1, 1], topKind: KIND.STONE, rows: 1 });
  const col = w.ngon(c, 1.3, 4, 0.78);
  w.prism(BASE_FAR, c, col, 0, 17, STRIP.STONE, STONE_LIGHT, KIND.STONE, { rows: 3 });
  w.spire(BASE_FAR, c, col, 17, 5, COPPER);
  depot(w, ent, [14, 0, 10]);
}

/** A gas depot: a rack of cylinders under a tall orange pennant. */
function depot(w: World, ent: Entities, p: V3) {
  const [vb, vt] = stripV(STRIP.FLAT);
  const steel: Rgb = [0.62, 0.66, 0.7];
  const base: V3 = [p[0], p[1], p[2]];
  w.box(NEAR_MID, p, { o: [p[0] - 1.6, p[1], p[2] + 0.8], r: [1, 0, 0], n: [0, 0, 1] }, 3.2, 1.6, 0, 0.35, STRIP.WOOD, [0.7, 0.6, 0.5], KIND.WOOD, { top: STRIP.WOOD, topTint: [0.7, 0.6, 0.5], rows: 1 });
  for (const dx of [-1, 0, 1]) {
    const c: V3 = [p[0] + dx, p[1], p[2]];
    const pts = w.ngon(c, 0.36, 6, 0);
    w.prism(NEAR_MID, p, pts, 0.35, 2.0, STRIP.FLAT, steel, -1, { top: STRIP.FLAT, topTint: [0.4, 0.42, 0.46], topKind: -1, rows: 1 });
  }
  // The pole and its pennant, visible from far off.
  const pole = w.ngon([p[0] + 2, p[1], p[2]], 0.12, 4, 0);
  w.prism(ALL, p, pole, 0, 11, STRIP.FLAT, [0.3, 0.26, 0.22], -1, { rows: 1 });
  const flag: Rgb = [1.6, 0.62, 0.12];
  const a: V3 = [p[0] + 2.1, p[1] + 8.6, p[2]];
  for (const s of [1, -1]) {
    const q: V3[] = [a, [a[0] + 3.2, a[1] + 0.3, a[2]], [a[0] + 3.2, a[1] + 2.0, a[2]], [a[0], a[1] + 2.3, a[2]]];
    if (s > 0) w.quad(ALL, p, q[0], q[1], q[2], q[3], [0, 0.1, vb, vt], flag);
    else w.quad(ALL, p, q[1], q[0], q[3], q[2], [0, 0.1, vb, vt], flag);
  }
  ent.depots.push({ pos: up(base, 1.2), radius: 5.5 });
}

// ---------------------------------------------------------------------------- outside the wall

function forest(w: World, rng: Rng, ent: Entities) {
  const bark: Rgb = [0.56, 0.47, 0.4];
  let placed = 0;
  for (let i = 0; i < 400 && placed < 64; i++) {
    const r = rng.range(700, 1090);
    const a = rng.range(-78, -12) * DEG;
    const p = polar(r, a);
    // Keep the gate road clear.
    if (Math.abs(angDiff(a, -Math.PI / 2)) * r < 22) continue;
    const jitterOk = hash01(Math.floor(p[0] / 34), Math.floor(p[2] / 34), 9) > 0.35;
    if (!jitterOk) continue;
    placed++;
    const h = rng.range(56, 86);
    const rad = rng.range(2.6, 4.4);
    const phase = rng.range(0, 6);
    // The trunk tapers in three lifts.
    const lifts = [0, 0.3, 0.62, 1].map((k) => ({ y: h * k, r: rad * (1 - 0.5 * k) }));
    for (let s = 0; s < 3; s++) {
      const lo = w.ngon(p, lifts[s].r, 8, phase);
      const hi = w.ngon(p, lifts[s + 1].r, 8, phase);
      const [vb, vt] = stripV(STRIP.WOOD);
      for (let e = 0; e < 8; e++) {
        const j = (e + 1) % 8;
        const a0 = up(lo[j], lifts[s].y);
        const b0 = up(lo[e], lifts[s].y);
        const c0 = up(hi[e], lifts[s + 1].y);
        const d0 = up(hi[j], lifts[s + 1].y);
        w.quad(BASE_FAR, p, a0, b0, c0, d0, [e * 0.3, e * 0.3 + 0.3, vb, vt], bark, KIND.WOOD);
      }
    }
    // Great limbs: hook points and perches.
    const limbs = rng.int(3, 5);
    for (let l = 0; l < limbs; l++) {
      const la = rng.range(0, TAU);
      const ly = h * rng.range(0.38, 0.7);
      const ll = rng.range(9, 16);
      const n: V3 = [Math.cos(la), 0, Math.sin(la)];
      const lf: Frame = { o: [p[0] + n[0] * ll - cross(UP, n)[0] * 0.9, 0, p[2] + n[2] * ll - cross(UP, n)[2] * 0.9], r: cross(UP, n), n };
      w.box([Layer.Base], p, lf, 1.8, ll, ly, ly + 1.6, STRIP.WOOD, bark, KIND.WOOD, { top: STRIP.WOOD, topTint: bark, topKind: KIND.WOOD, rows: 1 });
      blob(w, [Layer.Base], p, [p[0] + n[0] * ll, ly + 5, p[2] + n[2] * ll], rng.range(6, 9), [0.42, 0.62, 0.36], rng);
    }
    blob(w, BASE_FAR, p, up(p, h * 0.86), rng.range(13, 18), [0.4, 0.6, 0.34], rng);
    blob(w, BASE_FAR, p, [p[0] + rng.range(-8, 8), h * 0.66, p[2] + rng.range(-8, 8)], rng.range(10, 14), [0.36, 0.56, 0.32], rng);
    if (placed % 9 === 4) {
      ent.dummies.push(dummyAt([p[0] + rad + 9, 0, p[2] + 4], rng.range(0, TAU), 14));
    }
  }
}

function farms(w: World, rng: Rng) {
  for (let c = 0; c < 4; c++) {
    const a = (c * Math.PI) / 2;
    for (let k = 0; k < 5; k++) {
      const r = 690 + k * 86 + rng.range(-10, 10);
      const side = k % 2 === 0 ? 1 : -1;
      const p = polar(r, a + (side * rng.range(18, 34)) / r);
      const n = norm([Math.cos(a + Math.PI / 2) * -side, 0, Math.sin(a + Math.PI / 2) * -side]);
      const f = frame(p, n);
      emitHouse(w, {
        f,
        w: rng.range(9, 14),
        d: rng.range(8, 10),
        storeys: 2,
        style: 0,
        plaster: rng.pick(PLASTER),
        roof: rng.pick(ROOFS),
        gableFront: false,
        rise: rng.range(4.5, 6),
        jetty: 0,
        chimney: true,
        seed: rng.int(0, 1 << 30),
      });
    }
  }
}

/** The horizon: a ring of hills, always drawn. */
function backdrop(w: World) {
  const [vb, vt] = stripV(STRIP.FLAT);
  const n = 72;
  const r = 2700;
  const h = (i: number) => 150 + 330 * fbm((i % n) * 0.21, 3.7, 21, 3, n * 0.21);
  for (let i = 0; i < n; i++) {
    const a0 = (i / n) * TAU;
    const a1 = ((i + 1) / n) * TAU;
    const tint: Rgb = [0.52, 0.6, 0.66];
    w.quad([Layer.Backdrop], [0, 0, 0], up(polar(r, a0), -20), up(polar(r, a1), -20), up(polar(r, a1), h(i + 1)), up(polar(r, a0), h(i)), [0, 0.1, vb, vt], tint);
  }
  // The plain between the fields and the hills.
  const g = stripV(STRIP.GRASS);
  for (let i = 0; i < 24; i++) {
    const a0 = (i / 24) * TAU;
    const a1 = ((i + 1) / 24) * TAU;
    w.quad([Layer.Backdrop], [0, 0, 0], polar(1170, a0, -0.5), polar(1170, a1, -0.5), polar(r, a1, -20), polar(r, a0, -20), [0, 6, g[0], g[1]], [0.8, 0.9, 0.74]);
  }
}

// ---------------------------------------------------------------------------- targets and the route

function dummyAt(p: V3, yaw: number, height: number): Entities["dummies"][number] {
  // The nape sits at the back of the neck; the target faces along `yaw`.
  const back: V3 = [Math.sin(yaw), 0, Math.cos(yaw)];
  return { pos: p, yaw, height, nape: [p[0] + back[0] * height * 0.1, p[1] + height * 0.84, p[2] + back[2] * height * 0.1] };
}

function targets(w: World, plan: Plan, rng: Rng, ent: Entities) {
  // Along the avenues and the ring streets, facing down the street.
  const rings = [108, 172, 238, 372, 438, 504, 568];
  for (let i = 0; i < 26; i++) {
    let p: V3;
    let yaw: number;
    if (i % 3 === 0) {
      const c = (rng.int(0, 3) * Math.PI) / 2;
      const r = rng.range(60, 560);
      if (r > CANAL_IN - 12 && r < CANAL_OUT + 12) continue;
      p = polar(r, c);
      yaw = Math.atan2(-Math.cos(c), -Math.sin(c)) + (rng.chance(0.5) ? Math.PI : 0);
    } else {
      const r = rng.pick(rings);
      const a = rng.range(0, TAU);
      p = polar(r, a);
      yaw = Math.atan2(Math.sin(a), -Math.cos(a)) + (rng.chance(0.5) ? Math.PI : 0);
    }
    ent.dummies.push(dummyAt(p, yaw, rng.pick([8, 9, 10, 12, 15])));
  }
  void plan;
  // Collision: a post for each target, so wires hold on it.
  for (const d of ent.dummies) {
    const f = frame([d.pos[0], d.pos[1], d.pos[2]], [-Math.sin(d.yaw), 0, -Math.cos(d.yaw)]);
    const wd = d.height * 0.34;
    const o = at(f, -wd / 2, 0, d.height * 0.06);
    const pts = [o, at(f, -wd / 2, 0, -d.height * 0.06), at(f, wd / 2, 0, -d.height * 0.06), at(f, wd / 2, 0, d.height * 0.06)];
    const n = pts.length;
    for (let i = 0; i < n; i++) w.col.quad(pts[(i + 1) % n], pts[i], up(pts[i], d.height), up(pts[(i + 1) % n], d.height), KIND.WOOD);
  }
}

function route(ent: Entities) {
  const pts: V3[] = [];
  const radial = (a: number, r0: number, r1: number, y: number) => {
    const n = Math.max(1, Math.round(Math.abs(r1 - r0) / 45));
    for (let i = 0; i < n; i++) pts.push(polar(r0 + ((r1 - r0) * i) / n, a, y));
  };
  const arc = (r: number, a0: number, a1: number, y: number) => {
    const n = Math.max(1, Math.round((Math.abs(a1 - a0) * r) / 45));
    for (let i = 0; i < n; i++) pts.push(polar(r, a0 + ((a1 - a0) * i) / n, y));
  };
  const S = Math.PI / 2;
  const E = 0;
  const N = -Math.PI / 2;
  const W = Math.PI;
  radial(S, 560, 60, 11);
  arc(52, S, E, 13);
  radial(E, 60, 238, 11);
  arc(238, E, N, 11);
  radial(N, 238, 438, 12);
  arc(438, N, -W, 12);
  radial(W, 438, 172, 11);
  arc(172, W, S, 11);
  radial(S, 172, 560, 12);
  ent.waypoints.push(...pts);
}

// ---------------------------------------------------------------------------- assembly

export interface Generated {
  world: World;
  entities: Entities;
  plan: Plan;
}

export function generate(seed: number): Generated {
  const rng = new Rng(seed);
  const plan = makePlan(rng.fork(1));
  const w = new World();
  const ent: Entities = { spawn: [0, WALL_H + 12.6, R_WALL + WALL_T / 2, 0], dummies: [], depots: [], waypoints: [] };

  // Blocks given over to landmarks.
  const sectorAt = (band: number, a: number) => {
    const sp = plan.bands[band].spokes;
    for (let i = 0; i < sp.length; i++) {
      const next = i + 1 === sp.length ? sp[0].a + TAU : sp[i + 1].a;
      const aa = a < sp[0].a ? a + TAU : a;
      if (aa >= sp[i].a && aa < next) return i;
    }
    return 0;
  };
  const special = new Map<string, (b: Block) => void>([
    [`0:${sectorAt(0, -Math.PI / 2 + 0.4)}`, (b) => townHall(w, b)],
    [`2:${sectorAt(2, 0.25)}`, (b) => cathedral(w, b)],
    [`5:${sectorAt(5, Math.PI * 0.75 + 0.1)}`, (b) => keep(w, b, ent)],
    [`1:${sectorAt(1, Math.PI + 0.3)}`, (b) => marketHall(w, b)],
  ]);

  ground(w, plan);
  for (const b of blocks(plan)) special.get(`${b.band}:${b.sector}`)?.(b);
  houses(w, plan, rng.fork(2), new Set(special.keys()));
  wall(w);
  canal(w, plan);
  plaza(w, ent);
  const tr = rng.fork(3);
  for (let i = 0; i < 6; i++) {
    const a = (i / 6) * TAU + 0.5;
    watchTower(w, polar(438, a), tr);
    if (i % 2 === 0) depot(w, ent, polar(446, a + 0.02));
  }
  forest(w, rng.fork(4), ent);
  farms(w, rng.fork(5));
  backdrop(w);
  targets(w, plan, rng.fork(6), ent);
  route(ent);
  return { world: w, entities: ent, plan };
}
