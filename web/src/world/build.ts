// Building blocks: walls, roofs, prisms and the row house. Each helper writes
// render geometry to the layers it is given and, when it has a kind, the
// matching collision surface.

import { BAY_M, BAYS, FACADE, FacadeStyle, facadeV, gableV, STOREY_M, STRIP, Strip, stripV } from "./atlas";
import { add, Collision, cross, Layer, mad, Meshes, norm, Rgb, sub, up, UP, V3 } from "./geo";
import { Rng } from "./rng";

/** Surface kinds of the collision world (`crates/maneuver-sim/src/collide.rs`). */
export const KIND = { GROUND: 0, WALL: 1, ROOF: 2, STONE: 3, WOOD: 4, WATER: 5, NOHOOK: 6 } as const;

export const ALL: readonly Layer[] = [Layer.Near, Layer.Mid, Layer.Far];
export const NEAR_MID: readonly Layer[] = [Layer.Near, Layer.Mid];

/** A horizontal frame: `o` on the ground, `r` to the right seen from the front, `n` out of the front. */
export interface Frame {
  o: V3;
  r: V3;
  n: V3;
}

export function frame(o: V3, n: V3): Frame {
  const nn = norm([n[0], 0, n[2]]);
  return { o, r: cross(UP, nn), n: nn };
}

/** Point at `x` along the frame's right, `y` up and `z` out of the front. */
export function at(f: Frame, x: number, y: number, z: number): V3 {
  return [f.o[0] + f.r[0] * x + f.n[0] * z, f.o[1] + y, f.o[2] + f.r[2] * x + f.n[2] * z];
}

export class World {
  meshes = new Meshes();
  col = new Collision();

  /** Emits a quad to each layer; the bucket is the cell under `key`. */
  quad(layers: readonly Layer[], key: V3, a: V3, b: V3, c: V3, d: V3, uv: readonly [number, number, number, number], tint: Rgb, kind = -1) {
    for (const l of layers) this.meshes.at(l, key[0], key[2]).quad(a, b, c, d, uv, tint);
    if (kind >= 0) this.col.quad(a, b, c, d, kind);
  }
  tri(layers: readonly Layer[], key: V3, a: V3, b: V3, c: V3, ta: readonly [number, number], tb: readonly [number, number], tc: readonly [number, number], tint: Rgb, kind = -1) {
    for (const l of layers) this.meshes.at(l, key[0], key[2]).tri(a, b, c, ta, tb, tc, tint);
    if (kind >= 0) this.col.tri(a, b, c, kind);
  }

  /**
   * A vertical wall from `a` to `b` (left to right seen from the front) between
   * heights `y0` and `y1`, in rows of the strip's height so the texture keeps
   * its scale. `rows` caps the number of quads (the last one stretches).
   */
  stack(layers: readonly Layer[], key: V3, a: V3, b: V3, y0: number, y1: number, strip: Strip, tint: Rgb, kind = -1, rows = 99, u = 0) {
    const [vb, vt] = stripV(strip);
    const width = Math.hypot(b[0] - a[0], b[2] - a[2]);
    const n = Math.max(1, Math.min(rows, Math.round((y1 - y0) / strip.mPerV)));
    const h = (y1 - y0) / n;
    for (let i = 0; i < n; i++) {
      const u0 = (u + (i % 2) * 0.37) % 1;
      const uv: [number, number, number, number] = [u0, u0 + width / strip.mPerU, vb, vt];
      for (const l of layers) this.meshes.at(l, key[0], key[2]).quad(up(a, y0 + i * h), up(b, y0 + i * h), up(b, y0 + (i + 1) * h), up(a, y0 + (i + 1) * h), uv, tint);
    }
    if (kind >= 0) this.col.quad(up(a, y0), up(b, y0), up(b, y1), up(a, y1), kind);
  }

  /** A flat quad seen from above (`a b c d` counter-clockwise from above), textured along `a→b`. */
  floor(layers: readonly Layer[], key: V3, a: V3, b: V3, c: V3, d: V3, strip: Strip, tint: Rgb, kind = -1, u = 0) {
    const [vb, vt] = stripV(strip);
    const width = Math.hypot(b[0] - a[0], b[2] - a[2]);
    this.quad(layers, key, a, b, c, d, [u % 1, (u % 1) + width / strip.mPerU, vb, vt], tint, kind);
  }

  /**
   * A prism on a convex footprint from `y0` to `y1`: stacked walls, and a flat
   * top when `top` is set. `pts` are at ground level, ordered with angles
   * growing from +X toward +Z (front-left, back-left, back-right, front-right on a frame).
   */
  prism(layers: readonly Layer[], key: V3, pts: readonly V3[], y0: number, y1: number, strip: Strip, tint: Rgb, kind: number, opts: { top?: Strip; topTint?: Rgb; topKind?: number; rows?: number } = {}) {
    const n = pts.length;
    for (let i = 0; i < n; i++) {
      // Counter-clockwise from above: the outside is to the right of each edge, so the wall runs from the next point back.
      this.stack(layers, key, pts[(i + 1) % n], pts[i], y0, y1, strip, tint, kind, opts.rows ?? 99, i * 0.21);
    }
    if (opts.top && n === 4) {
      // A four-sided top is laid in rows of the strip's depth, so its texture keeps its scale.
      const [p0, p1, p2, p3] = [up(pts[0], y1), up(pts[1], y1), up(pts[2], y1), up(pts[3], y1)];
      const depth = Math.hypot(p1[0] - p0[0], p1[2] - p0[2]);
      const rows = Math.max(1, Math.round(depth / opts.top.mPerV));
      const mix = (a: V3, b: V3, t: number): V3 => [a[0] + (b[0] - a[0]) * t, a[1], a[2] + (b[2] - a[2]) * t];
      for (let i = 0; i < rows; i++) {
        const [t0, t1] = [i / rows, (i + 1) / rows];
        // Rows run from the edge 0-3 to the edge 1-2; each quad winds upward.
        for (const l of layers) {
          const geo = this.meshes.at(l, key[0], key[2]);
          const [vb, vt] = stripV(opts.top);
          const a = mix(p0, p1, t0);
          const b2 = mix(p3, p2, t0);
          const width = Math.hypot(b2[0] - a[0], b2[2] - a[2]) / opts.top.mPerU;
          geo.quad(a, b2, mix(p3, p2, t1), mix(p0, p1, t1), [i * 0.37, i * 0.37 + width, vb, vt], opts.topTint ?? tint);
        }
      }
      const kind = opts.topKind ?? KIND.GROUND;
      if (kind >= 0) this.col.quad(p0, p3, p2, p1, kind);
    } else if (opts.top) {
      const [vb, vt] = stripV(opts.top);
      const c: V3 = [pts.reduce((s, p) => s + p[0], 0) / n, pts[0][1] + y1, pts.reduce((s, p) => s + p[2], 0) / n];
      for (let i = 0; i < n; i++) {
        const a = up(pts[i], y1);
        const b = up(pts[(i + 1) % n], y1);
        const w = Math.hypot(b[0] - a[0], b[2] - a[2]) / opts.top.mPerU;
        this.tri(layers, key, a, c, b, [0, vb], [w / 2, vt], [w, vb], opts.topTint ?? tint, opts.topKind ?? KIND.GROUND);
      }
    }
  }

  /** A pyramid or cone roof over a convex footprint at height `y0`, apex `h` above its centre. */
  spire(layers: readonly Layer[], key: V3, pts: readonly V3[], y0: number, h: number, tint: Rgb, kind: number = KIND.ROOF) {
    const n = pts.length;
    const [vb, vt] = stripV(STRIP.ROOF);
    const c: V3 = [pts.reduce((s, p) => s + p[0], 0) / n, pts[0][1] + y0 + h, pts.reduce((s, p) => s + p[2], 0) / n];
    for (let i = 0; i < n; i++) {
      const a = up(pts[i], y0);
      const b = up(pts[(i + 1) % n], y0);
      const w = Math.hypot(b[0] - a[0], b[2] - a[2]) / STRIP.ROOF.mPerU;
      this.tri(layers, key, b, a, c, [w, vb], [0, vb], [w / 2, vt], tint, kind);
    }
  }

  /** An axis-free box on a frame: `w` wide, `d` deep behind the front, from `y0` to `y1`. */
  box(layers: readonly Layer[], key: V3, f: Frame, w: number, d: number, y0: number, y1: number, strip: Strip, tint: Rgb, kind: number, opts: { top?: Strip; topTint?: Rgb; topKind?: number; rows?: number } = {}) {
    // Counter-clockwise from above: front-left, back-left, back-right, front-right.
    const pts = [at(f, 0, 0, 0), at(f, 0, 0, -d), at(f, w, 0, -d), at(f, w, 0, 0)];
    this.prism(layers, key, pts, y0, y1, strip, tint, kind, opts);
  }

  /** A regular polygon footprint in the order `prism` takes: angles growing from +X toward +Z. */
  ngon(c: V3, radius: number, sides: number, phase = 0): V3[] {
    const pts: V3[] = [];
    for (let i = 0; i < sides; i++) {
      const a = phase + (i / sides) * Math.PI * 2;
      pts.push([c[0] + Math.cos(a) * radius, c[1], c[2] + Math.sin(a) * radius]);
    }
    return pts;
  }

  /**
   * A gable roof on a frame: `w` wide, `d` deep, eaves at `y`, ridge `h` higher.
   * `along` runs the ridge along the frame's right; otherwise it runs front to
   * back and the front shows a gable. Gable ends take `gable` (a facade style) or stone.
   */
  gableRoof(layers: readonly Layer[], key: V3, f: Frame, w: number, d: number, y: number, h: number, along: boolean, tint: Rgb, gable: { style?: FacadeStyle; tint: Rgb; u?: number }, collide = true) {
    const [vb, vt] = stripV(STRIP.ROOF);
    const roofKind = collide ? KIND.ROOF : -1;
    const wallKind = collide ? KIND.WALL : -1;
    const slope = (a: V3, b: V3, c: V3, dd: V3, run: number, length: number) => {
      const top = vb - Math.min(run / STRIP.ROOF.mPerV, 1) * (vb - vt);
      const u0 = (key[0] * 0.13 + key[2] * 0.07) % 1;
      this.quad(layers, key, a, b, c, dd, [u0, u0 + length / STRIP.ROOF.mPerU, vb, top], tint, roofKind);
    };
    const end = (a: V3, b: V3, apex: V3, span: number) => {
      const u0 = gable.u ?? 0;
      if (gable.style) {
        const bays = Math.max(1, Math.round(span / BAY_M));
        const [gb, ga] = gableV(gable.style, h);
        this.tri(layers, key, a, b, apex, [u0, gb], [u0 + bays / BAYS, gb], [u0 + bays / BAYS / 2, ga], gable.tint, wallKind);
      } else {
        const [sb, st] = stripV(STRIP.STONE);
        const top = sb - Math.min(h / STRIP.STONE.mPerV, 1) * (sb - st);
        this.tri(layers, key, a, b, apex, [u0, sb], [u0 + span / STRIP.STONE.mPerU, sb], [u0 + span / STRIP.STONE.mPerU / 2, top], gable.tint, wallKind);
      }
    };
    if (along) {
      const run = Math.hypot(d / 2, h);
      const rl = at(f, 0, y + h, -d / 2);
      const rr = at(f, w, y + h, -d / 2);
      slope(at(f, 0, y, 0), at(f, w, y, 0), rr, rl, run, w);
      slope(at(f, w, y, -d), at(f, 0, y, -d), rl, rr, run, w);
      end(at(f, w, y, 0), at(f, w, y, -d), rr, d);
      end(at(f, 0, y, -d), at(f, 0, y, 0), rl, d);
    } else {
      const run = Math.hypot(w / 2, h);
      const rf = at(f, w / 2, y + h, 0);
      const rb = at(f, w / 2, y + h, -d);
      slope(at(f, w, y, 0), at(f, w, y, -d), rb, rf, run, d);
      slope(at(f, 0, y, -d), at(f, 0, y, 0), rf, rb, run, d);
      end(at(f, 0, y, 0), at(f, w, y, 0), rf, w);
      end(at(f, w, y, -d), at(f, 0, y, -d), rb, w);
    }
  }
}

export const PLASTER: readonly Rgb[] = [
  [0.97, 0.96, 0.93],
  [1.0, 0.95, 0.84],
  [0.97, 0.86, 0.66],
  [0.95, 0.78, 0.7],
  [0.84, 0.9, 0.82],
  [0.82, 0.87, 0.93],
  [0.88, 0.87, 0.84],
  [0.99, 0.9, 0.76],
];
export const ROOFS: readonly Rgb[] = [
  [0.8, 0.37, 0.27],
  [0.64, 0.29, 0.22],
  [0.86, 0.47, 0.3],
  [0.56, 0.38, 0.3],
  [0.74, 0.34, 0.25],
  [0.45, 0.47, 0.52],
];

export interface House {
  f: Frame;
  w: number;
  d: number;
  storeys: number;
  style: number;
  plaster: Rgb;
  roof: Rgb;
  /** The ridge runs front to back and the street sees a gable. */
  gableFront: boolean;
  /** Ridge height above the eaves. */
  rise: number;
  jetty: number;
  chimney: boolean;
  seed: number;
}

/** Height of the walls and of the ridge. */
export function houseHeights(h: House): [number, number] {
  const wall = h.storeys * STOREY_M;
  return [wall, wall + h.rise];
}

function facadeUV(style: FacadeStyle, storeys: number, width: number, r: Rng): [number, number, number, number] {
  const bays = Math.max(1, Math.round(width / BAY_M));
  const u0 = r.int(0, BAYS - 1) / BAYS;
  const [vb, vt] = facadeV(style, Math.min(storeys, style.storeys));
  return [u0, u0 + bays / BAYS, vb, vt];
}

export function emitHouse(w: World, h: House) {
  const r = new Rng(h.seed);
  const style = FACADE[h.style];
  const storeys = Math.min(h.storeys, style.storeys);
  const H = storeys * STOREY_M;
  const f = h.f;
  const key = at(f, h.w / 2, 0, -h.d / 2);
  const tint = h.plaster;
  const P = (x: number, y: number, z: number) => at(f, x, y, z);

  const front = facadeUV(style, storeys, h.w, r);
  const back = facadeUV(style, storeys, h.w, r);
  const side = facadeUV(style, storeys, h.d, r);

  // Collision: four walls. The roof adds its own.
  w.col.quad(P(0, 0, 0), P(h.w, 0, 0), P(h.w, H, 0), P(0, H, 0), KIND.WALL);
  w.col.quad(P(h.w, 0, -h.d), P(0, 0, -h.d), P(0, H, -h.d), P(h.w, H, -h.d), KIND.WALL);
  w.col.quad(P(h.w, 0, 0), P(h.w, 0, -h.d), P(h.w, H, -h.d), P(h.w, H, 0), KIND.WALL);
  w.col.quad(P(0, 0, -h.d), P(0, 0, 0), P(0, H, 0), P(0, H, -h.d), KIND.WALL);

  // Near: the upper storeys step out over the street on a jetty.
  const j = storeys >= 2 ? h.jetty : 0;
  const near = [Layer.Near] as const;
  if (j > 0) {
    const vs = front[2] + (front[3] - front[2]) / storeys;
    w.quad(near, key, P(0, 0, 0), P(h.w, 0, 0), P(h.w, STOREY_M, 0), P(0, STOREY_M, 0), [front[0], front[1], front[2], vs], tint);
    w.quad(near, key, P(0, STOREY_M, j), P(h.w, STOREY_M, j), P(h.w, H, j), P(0, H, j), [front[0], front[1], vs, front[3]], tint);
    const [wb, wt] = stripV(STRIP.WOOD);
    const dark: Rgb = [0.34, 0.26, 0.2];
    w.quad(near, key, P(0, STOREY_M, 0), P(h.w, STOREY_M, 0), P(h.w, STOREY_M, j), P(0, STOREY_M, j), [0, h.w / STRIP.WOOD.mPerU, wb, wt], dark);
    // The jetty's cheeks.
    w.quad(near, key, P(h.w, STOREY_M, j), P(h.w, STOREY_M, 0), P(h.w, H, 0), P(h.w, H, j), [side[0], side[0] + 0.01, vs, front[3]], tint);
    w.quad(near, key, P(0, STOREY_M, 0), P(0, STOREY_M, j), P(0, H, j), P(0, H, 0), [side[0], side[0] + 0.01, vs, front[3]], tint);
  } else {
    w.quad(near, key, P(0, 0, 0), P(h.w, 0, 0), P(h.w, H, 0), P(0, H, 0), front, tint);
  }
  w.quad([Layer.Mid, Layer.Far], key, P(0, 0, 0), P(h.w, 0, 0), P(h.w, H, 0), P(0, H, 0), front, tint);
  w.quad(ALL, key, P(h.w, 0, -h.d), P(0, 0, -h.d), P(0, H, -h.d), P(h.w, H, -h.d), back, tint);
  w.quad(NEAR_MID, key, P(h.w, 0, 0), P(h.w, 0, -h.d), P(h.w, H, -h.d), P(h.w, H, 0), side, tint);
  w.quad(NEAR_MID, key, P(0, 0, -h.d), P(0, 0, 0), P(0, H, 0), P(0, H, -h.d), side, tint);

  // Roof. Near, it covers the jetty.
  const gable = { style, tint, u: front[0] };
  w.gableRoof(near, key, { o: at(f, 0, 0, j), r: f.r, n: f.n }, h.w, h.d + j, H, h.rise, !h.gableFront, h.roof, gable, false);
  w.gableRoof([Layer.Mid, Layer.Far], key, f, h.w, h.d, H, h.rise, !h.gableFront, h.roof, gable, true);

  if (h.chimney) {
    // On the back slope (or the left slope of a gable-front roof), through the ridge line.
    const cx = h.gableFront ? h.w * 0.28 : h.w * r.range(0.25, 0.75);
    const cz = h.gableFront ? -h.d * r.range(0.4, 0.75) : -h.d * 0.62;
    const s = 0.4;
    const cf: Frame = { o: at(f, cx - s, 0, cz + s), r: f.r, n: f.n };
    w.box(near, key, cf, s * 2, s * 2, H + h.rise * 0.35, H + h.rise + 0.9, STRIP.STONE, [0.62, 0.5, 0.44], -1, { top: STRIP.FLAT, topTint: [0.16, 0.14, 0.13], topKind: -1, rows: 1 });
  }
}

/** A point on the circle of radius `r` at angle `a`; angle 0 is +X and angles grow toward +Z. */
export function polar(r: number, a: number, y = 0): V3 {
  return [Math.cos(a) * r, y, Math.sin(a) * r];
}

export { add, mad, sub };
