// Organic modelling in code: a body is a set of rounded primitives attached
// to bones, blended into one signed distance field and polygonized with
// surface nets. Each vertex takes its colour from the primitive that owns the
// surface there and its skin weights from the bones of the primitives near it.
//
// Primitives are authored in a bone's frame of the bind pose (the simulation
// supplies the bind transforms), so a model follows its skeleton's proportions.

export type V3 = [number, number, number];
export type Rgb = readonly [number, number, number];

/** A skinned mesh: two bones per vertex. */
export interface SkinModel {
  /** Per vertex: position 3, normal 3, sRGB colour 3, bone a, bone b, weight of a. */
  v: Float32Array;
  i: Uint32Array;
}
export const SKIN_STRIDE = 12;

const CONE = 0;
const ELLIPSOID = 1;
const BOX = 2;
const ADD = 0;
const CARVE = 1;
const PAINT = 2;

interface Prim {
  kind: number;
  op: number;
  // Round cone: a, b, radii. Ellipsoid and box: centre, half sizes, rows of the world-to-local rotation.
  a: V3;
  b: V3;
  ra: number;
  rb: number;
  m: number[];
  round: number;
  k: number;
  color: Rgb;
  bone: number;
  bone2: number;
  sigma: number;
  lo: V3;
  hi: V3;
}

export interface PrimOptions {
  /** Blend radius with what is already there (0: a hard union). */
  k?: number;
  /** For a cone: the bone at its far end; weights slide from one to the other along it. */
  bone2?: number;
  /** Euler rotation (x, y, z) of an ellipsoid or a box in its bone's frame. */
  rot?: V3;
  /** Reach of this primitive's bone over the surface, in metres. */
  sigma?: number;
}

function smin(a: number, b: number, k: number): number {
  if (k <= 0) return a < b ? a : b;
  const h = Math.max(k - Math.abs(a - b), 0) / k;
  return Math.min(a, b) - h * h * k * 0.25;
}

function eulerRows(r: V3): number[] {
  // Rz · Rx · Ry, as rows (so applying it takes a local vector to the bone frame).
  const [cx, sx, cy, sy, cz, sz] = [Math.cos(r[0]), Math.sin(r[0]), Math.cos(r[1]), Math.sin(r[1]), Math.cos(r[2]), Math.sin(r[2])];
  const rx = [1, 0, 0, 0, cx, -sx, 0, sx, cx];
  const ry = [cy, 0, sy, 0, 1, 0, -sy, 0, cy];
  const rz = [cz, -sz, 0, sz, cz, 0, 0, 0, 1];
  const mul = (a: number[], b: number[]) => {
    const o = new Array(9).fill(0);
    for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) for (let k = 0; k < 3; k++) o[i * 3 + j] += a[i * 3 + k] * b[k * 3 + j];
    return o;
  };
  return mul(rz, mul(rx, ry));
}

export class Body {
  private prims: Prim[] = [];
  /**
   * `scale` multiplies every size given to the primitives; `sigma` is how far
   * a primitive's bone reaches over the surface around it, in model units.
   */
  constructor(
    readonly bind: Float32Array,
    readonly scale = 1,
    readonly sigma = 0.035,
  ) {}

  /** A bone-frame point in bind-pose space. */
  at(bone: number, p: V3): V3 {
    const m = this.bind;
    const o = bone * 12;
    return [m[o] * p[0] + m[o + 3] * p[1] + m[o + 6] * p[2] + m[o + 9], m[o + 1] * p[0] + m[o + 4] * p[1] + m[o + 7] * p[2] + m[o + 10], m[o + 2] * p[0] + m[o + 5] * p[1] + m[o + 8] * p[2] + m[o + 11]];
  }
  /** A bone-frame direction in bind-pose space. */
  dir(bone: number, d: V3): V3 {
    const m = this.bind;
    const o = bone * 12;
    return [m[o] * d[0] + m[o + 3] * d[1] + m[o + 6] * d[2], m[o + 1] * d[0] + m[o + 4] * d[1] + m[o + 7] * d[2], m[o + 2] * d[0] + m[o + 5] * d[1] + m[o + 8] * d[2]];
  }
  /** Joint position of a bone. */
  joint(bone: number): V3 {
    return this.at(bone, [0, 0, 0]);
  }

  private push(p: Omit<Prim, "lo" | "hi">, reach: number) {
    let lo: V3;
    let hi: V3;
    if (p.kind === CONE) {
      const r = Math.max(p.ra, p.rb) + reach;
      lo = [Math.min(p.a[0], p.b[0]) - r, Math.min(p.a[1], p.b[1]) - r, Math.min(p.a[2], p.b[2]) - r];
      hi = [Math.max(p.a[0], p.b[0]) + r, Math.max(p.a[1], p.b[1]) + r, Math.max(p.a[2], p.b[2]) + r];
    } else {
      const r = Math.hypot(p.b[0], p.b[1], p.b[2]) + p.round + reach;
      lo = [p.a[0] - r, p.a[1] - r, p.a[2] - r];
      hi = [p.a[0] + r, p.a[1] + r, p.a[2] + r];
    }
    this.prims.push({ ...p, lo, hi });
  }

  private add(op: number, kind: number, bone: number, a: V3, b: V3, ra: number, rb: number, round: number, color: Rgb, o: PrimOptions) {
    const sigma = (o.sigma ?? this.sigma) * this.scale;
    const k = (o.k ?? 0) * this.scale;
    let m = [1, 0, 0, 0, 1, 0, 0, 0, 1];
    if (kind !== CONE) {
      // World-to-local rows: (bind rotation × local rotation) transposed.
      const local = eulerRows(o.rot ?? [0, 0, 0]);
      const cols: V3[] = [0, 1, 2].map((c) => this.dir(bone, [local[c], local[3 + c], local[6 + c]]));
      m = [...cols[0], ...cols[1], ...cols[2]];
    }
    this.push({ kind, op, a, b, ra, rb, m, round, k, color, bone, bone2: o.bone2 ?? bone, sigma }, k + sigma * 2.5);
  }

  /** A rounded cone from `a` (radius `ra`) to `b` (radius `rb`), in the bone's frame. */
  cone(bone: number, a: V3, b: V3, ra: number, rb: number, color: Rgb, o: PrimOptions = {}) {
    this.add(ADD, CONE, bone, this.at(bone, a), this.at(bone, b), ra * this.scale, rb * this.scale, 0, color, o);
  }
  /** An ellipsoid with half sizes `r`. */
  ell(bone: number, c: V3, r: V3, color: Rgb, o: PrimOptions = {}) {
    this.add(ADD, ELLIPSOID, bone, this.at(bone, c), [r[0] * this.scale, r[1] * this.scale, r[2] * this.scale], 0, 0, 0, color, o);
  }
  /** A box with half sizes `half` and rounded edges. */
  box(bone: number, c: V3, half: V3, round: number, color: Rgb, o: PrimOptions = {}) {
    this.add(ADD, BOX, bone, this.at(bone, c), [half[0] * this.scale, half[1] * this.scale, half[2] * this.scale], 0, 0, round * this.scale, color, o);
  }
  /** Removes an ellipsoid from the body. */
  carve(bone: number, c: V3, r: V3, o: PrimOptions = {}) {
    this.add(CARVE, ELLIPSOID, bone, this.at(bone, c), [r[0] * this.scale, r[1] * this.scale, r[2] * this.scale], 0, 0, 0, [0, 0, 0], o);
  }
  /** Recolours whatever surface lies inside a box; the shape does not change. */
  paint(bone: number, c: V3, half: V3, color: Rgb, o: PrimOptions = {}) {
    this.add(PAINT, BOX, bone, this.at(bone, c), [half[0] * this.scale, half[1] * this.scale, half[2] * this.scale], 0, 0, 0, color, o);
  }
  /** Recolours inside an ellipsoid. */
  paintEll(bone: number, c: V3, r: V3, color: Rgb, o: PrimOptions = {}) {
    this.add(PAINT, ELLIPSOID, bone, this.at(bone, c), [r[0] * this.scale, r[1] * this.scale, r[2] * this.scale], 0, 0, 0, color, o);
  }

  private dist(p: Prim, x: number, y: number, z: number): number {
    if (p.kind === CONE) {
      // Rounded cone (Quilez), by projecting on the axis.
      const bax = p.b[0] - p.a[0];
      const bay = p.b[1] - p.a[1];
      const baz = p.b[2] - p.a[2];
      const pax = x - p.a[0];
      const pay = y - p.a[1];
      const paz = z - p.a[2];
      const l2 = bax * bax + bay * bay + baz * baz;
      const t = Math.max(0, Math.min(1, (pax * bax + pay * bay + paz * baz) / (l2 || 1)));
      const dx = pax - bax * t;
      const dy = pay - bay * t;
      const dz = paz - baz * t;
      return Math.sqrt(dx * dx + dy * dy + dz * dz) - (p.ra + (p.rb - p.ra) * t);
    }
    const m = p.m;
    const px = x - p.a[0];
    const py = y - p.a[1];
    const pz = z - p.a[2];
    const lx = m[0] * px + m[1] * py + m[2] * pz;
    const ly = m[3] * px + m[4] * py + m[5] * pz;
    const lz = m[6] * px + m[7] * py + m[8] * pz;
    if (p.kind === ELLIPSOID) {
      const k0 = Math.sqrt((lx * lx) / (p.b[0] * p.b[0]) + (ly * ly) / (p.b[1] * p.b[1]) + (lz * lz) / (p.b[2] * p.b[2]));
      const k1 = Math.sqrt((lx * lx) / p.b[0] ** 4 + (ly * ly) / p.b[1] ** 4 + (lz * lz) / p.b[2] ** 4);
      return k1 > 1e-9 ? (k0 * (k0 - 1)) / k1 : -Math.min(p.b[0], p.b[1], p.b[2]);
    }
    const qx = Math.abs(lx) - p.b[0] + p.round;
    const qy = Math.abs(ly) - p.b[1] + p.round;
    const qz = Math.abs(lz) - p.b[2] + p.round;
    const ox = Math.max(qx, 0);
    const oy = Math.max(qy, 0);
    const oz = Math.max(qz, 0);
    return Math.sqrt(ox * ox + oy * oy + oz * oz) + Math.min(Math.max(qx, qy, qz), 0) - p.round;
  }

  /** Signed distance to the body. */
  eval(x: number, y: number, z: number): number {
    let d = 1e9;
    for (const p of this.prims) {
      if (p.op !== ADD || x < p.lo[0] || x > p.hi[0] || y < p.lo[1] || y > p.hi[1] || z < p.lo[2] || z > p.hi[2]) continue;
      d = smin(d, this.dist(p, x, y, z), p.k);
    }
    for (const p of this.prims) {
      if (p.op !== CARVE || x < p.lo[0] || x > p.hi[0] || y < p.lo[1] || y > p.hi[1] || z < p.lo[2] || z > p.hi[2]) continue;
      const c = -this.dist(p, x, y, z);
      // Smooth maximum.
      d = -smin(-d, -c, p.k);
    }
    return d;
  }

  /** Colour and skin weights of the surface at a point. */
  private attributes(x: number, y: number, z: number, weights: Float64Array): Rgb {
    weights.fill(0);
    let owner = 1e9;
    let color: Rgb = [1, 1, 1];
    for (const p of this.prims) {
      if (x < p.lo[0] || x > p.hi[0] || y < p.lo[1] || y > p.hi[1] || z < p.lo[2] || z > p.hi[2]) continue;
      const d = this.dist(p, x, y, z);
      if (p.op === PAINT) {
        if (d < 0) color = p.color;
        continue;
      }
      if (p.op !== ADD) continue;
      if (d < owner) {
        owner = d;
        color = p.color;
      }
      const w = Math.exp(-((Math.max(d, 0) / p.sigma) ** 2));
      if (p.bone2 !== p.bone && p.kind === CONE) {
        const bax = p.b[0] - p.a[0];
        const bay = p.b[1] - p.a[1];
        const baz = p.b[2] - p.a[2];
        const t = Math.max(0, Math.min(1, ((x - p.a[0]) * bax + (y - p.a[1]) * bay + (z - p.a[2]) * baz) / (bax * bax + bay * bay + baz * baz || 1)));
        weights[p.bone] += w * (1 - t);
        weights[p.bone2] += w * t;
      } else {
        weights[p.bone] += w;
      }
    }
    // Paint is applied in order, after ownership: repeat so a later paint wins over an earlier owner.
    for (const p of this.prims) {
      if (p.op !== PAINT || x < p.lo[0] || x > p.hi[0] || y < p.lo[1] || y > p.hi[1] || z < p.lo[2] || z > p.hi[2]) continue;
      if (this.dist(p, x, y, z) < 0) color = p.color;
    }
    return color;
  }

  /** Moves a point onto the surface along `d` (a unit vector pointing into the body), then `lift` back out. */
  snap(p: V3, d: V3, lift: number): V3 {
    let t0 = -0.08 * this.scale;
    let t1 = 0.12 * this.scale;
    const f = (t: number) => this.eval(p[0] + d[0] * t, p[1] + d[1] * t, p[2] + d[2] * t);
    if (f(t0) < 0 || f(t1) > 0) return p;
    for (let i = 0; i < 22; i++) {
      const tm = (t0 + t1) / 2;
      if (f(tm) > 0) t0 = tm;
      else t1 = tm;
    }
    const t = t0 - lift * this.scale;
    return [p[0] + d[0] * t, p[1] + d[1] * t, p[2] + d[2] * t];
  }

  /**
   * Polygonizes the body with surface nets at `cell` units and appends it to
   * `out`. `clip` limits the height range, so a part (the head) can be meshed
   * finer than the rest; the cut edges are left open.
   */
  mesh(out: MeshOut, cell: number, clip: { yMin?: number; yMax?: number } = {}) {
    const adds = this.prims.filter((p) => p.op === ADD);
    const min: V3 = [Math.min(...adds.map((p) => p.lo[0])), clip.yMin ?? Math.min(...adds.map((p) => p.lo[1])), Math.min(...adds.map((p) => p.lo[2]))];
    const max: V3 = [Math.max(...adds.map((p) => p.hi[0])), clip.yMax ?? Math.max(...adds.map((p) => p.hi[1])), Math.max(...adds.map((p) => p.hi[2]))];
    const n = [0, 1, 2].map((a) => Math.ceil((max[a] - min[a]) / cell) + 2);
    const [nx, ny, nz] = n;
    const f = new Float32Array(nx * ny * nz);
    for (let k = 0; k < nz; k++) for (let j = 0; j < ny; j++) for (let i = 0; i < nx; i++) f[(k * ny + j) * nx + i] = this.eval(min[0] + i * cell, min[1] + j * cell, min[2] + k * cell);
    // One vertex per cell the surface crosses, at the mean of its edge crossings.
    const index = new Int32Array(nx * ny * nz).fill(-1);
    const weights = new Float64Array(64);
    const corner = [0, 1, nx, nx + 1, nx * ny, nx * ny + 1, nx * ny + nx, nx * ny + nx + 1];
    const edges = [0, 1, 2, 3, 4, 5, 6, 7, 0, 2, 1, 3, 4, 6, 5, 7, 0, 4, 1, 5, 2, 6, 3, 7];
    for (let k = 0; k < nz - 1; k++) {
      for (let j = 0; j < ny - 1; j++) {
        for (let i = 0; i < nx - 1; i++) {
          const c = (k * ny + j) * nx + i;
          let mask = 0;
          for (let v = 0; v < 8; v++) if (f[c + corner[v]] < 0) mask |= 1 << v;
          if (mask === 0 || mask === 255) continue;
          let sx = 0;
          let sy = 0;
          let sz = 0;
          let count = 0;
          for (let e = 0; e < 24; e += 2) {
            const a = edges[e];
            const b = edges[e + 1];
            const fa = f[c + corner[a]];
            const fb = f[c + corner[b]];
            if (fa < 0 === fb < 0) continue;
            const t = fa / (fa - fb);
            sx += (a & 1) + ((b & 1) - (a & 1)) * t;
            sy += ((a >> 1) & 1) + (((b >> 1) & 1) - ((a >> 1) & 1)) * t;
            sz += ((a >> 2) & 1) + (((b >> 2) & 1) - ((a >> 2) & 1)) * t;
            count++;
          }
          const x = min[0] + (i + sx / count) * cell;
          const y = min[1] + (j + sy / count) * cell;
          const z = min[2] + (k + sz / count) * cell;
          const e = cell * 0.5;
          const gx = this.eval(x + e, y, z) - this.eval(x - e, y, z);
          const gy = this.eval(x, y + e, z) - this.eval(x, y - e, z);
          const gz = this.eval(x, y, z + e) - this.eval(x, y, z - e);
          const gl = Math.hypot(gx, gy, gz) || 1;
          const color = this.attributes(x, y, z, weights);
          index[c] = out.vertexWeighted([x, y, z], [gx / gl, gy / gl, gz / gl], color, weights);
        }
      }
    }
    // One quad per grid edge the surface crosses, between the four cells around it.
    const quad = (a: number, b: number, c: number, d: number, flip: boolean) => {
      if (a < 0 || b < 0 || c < 0 || d < 0) return;
      if (flip) out.i.push(a, d, c, a, c, b);
      else out.i.push(a, b, c, a, c, d);
    };
    for (let k = 1; k < nz - 1; k++) {
      for (let j = 1; j < ny - 1; j++) {
        for (let i = 1; i < nx - 1; i++) {
          const c = (k * ny + j) * nx + i;
          const inside = f[c] < 0;
          if (inside !== f[c + 1] < 0) quad(index[c - nx - nx * ny], index[c - nx * ny], index[c], index[c - nx], !inside);
          if (inside !== f[c + nx] < 0) quad(index[c - 1 - nx * ny], index[c - 1], index[c], index[c - nx * ny], !inside);
          if (inside !== f[c + nx * ny] < 0) quad(index[c - 1 - nx], index[c - nx], index[c], index[c - 1], !inside);
        }
      }
    }
  }
}

/** Growing vertex and index lists for a skinned model. */
export class MeshOut {
  v: number[] = [];
  i: number[] = [];
  get count(): number {
    return this.v.length / SKIN_STRIDE;
  }
  vertex(p: V3, n: V3, color: Rgb, bone: number, bone2 = bone, w = 1): number {
    this.v.push(p[0], p[1], p[2], n[0], n[1], n[2], color[0], color[1], color[2], bone, bone2, w);
    return this.count - 1;
  }
  /** A vertex from a weight per bone: keeps the two largest. */
  vertexWeighted(p: V3, n: V3, color: Rgb, weights: Float64Array): number {
    let a = 0;
    let b = 0;
    let wa = -1;
    let wb = -1;
    for (let k = 0; k < weights.length; k++) {
      const w = weights[k];
      if (w > wa) {
        b = a;
        wb = wa;
        a = k;
        wa = w;
      } else if (w > wb) {
        b = k;
        wb = w;
      }
    }
    if (wa <= 0) return this.vertex(p, n, color, 0);
    if (wb <= 0) return this.vertex(p, n, color, a);
    return this.vertex(p, n, color, a, b, wa / (wa + wb));
  }
  tri(a: number, b: number, c: number) {
    this.i.push(a, b, c);
  }
  model(): SkinModel {
    return { v: new Float32Array(this.v), i: new Uint32Array(this.i) };
  }
}

const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const norm = (a: V3): V3 => {
  const l = Math.hypot(a[0], a[1], a[2]) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
};

/** Hard-surface and flat pieces bound to one bone, authored in that bone's frame. */
export class Rigid {
  constructor(
    private out: MeshOut,
    private body: Body,
    private bone: number,
  ) {}

  private put(p: V3, n: V3, color: Rgb): number {
    return this.out.vertex(this.body.at(this.bone, p), norm(this.body.dir(this.bone, n)), color, this.bone);
  }

  /** A flat polygon (a fan around its first point); the normal follows its winding. */
  poly(pts: V3[], color: Rgb) {
    const n = norm(cross(sub(pts[1], pts[0]), sub(pts[2], pts[0])));
    const first = this.put(pts[0], n, color);
    for (let k = 1; k < pts.length; k++) this.put(pts[k], n, color);
    for (let k = 1; k + 1 < pts.length; k++) this.out.tri(first, first + k, first + k + 1);
  }

  /** A box between two corners, optionally turned by an Euler rotation about its centre. */
  box(lo: V3, hi: V3, color: Rgb, rot: V3 = [0, 0, 0]) {
    const c: V3 = [(lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2];
    const h: V3 = [(hi[0] - lo[0]) / 2, (hi[1] - lo[1]) / 2, (hi[2] - lo[2]) / 2];
    const m = eulerRows(rot);
    const p = (x: number, y: number, z: number): V3 => {
      const l: V3 = [x * h[0], y * h[1], z * h[2]];
      return [c[0] + m[0] * l[0] + m[1] * l[1] + m[2] * l[2], c[1] + m[3] * l[0] + m[4] * l[1] + m[5] * l[2], c[2] + m[6] * l[0] + m[7] * l[1] + m[8] * l[2]];
    };
    this.poly([p(-1, -1, 1), p(1, -1, 1), p(1, 1, 1), p(-1, 1, 1)], color);
    this.poly([p(1, -1, -1), p(-1, -1, -1), p(-1, 1, -1), p(1, 1, -1)], color);
    this.poly([p(1, -1, 1), p(1, -1, -1), p(1, 1, -1), p(1, 1, 1)], color);
    this.poly([p(-1, -1, -1), p(-1, -1, 1), p(-1, 1, 1), p(-1, 1, -1)], color);
    this.poly([p(-1, 1, 1), p(1, 1, 1), p(1, 1, -1), p(-1, 1, -1)], color);
    this.poly([p(-1, -1, -1), p(1, -1, -1), p(1, -1, 1), p(-1, -1, 1)], color);
  }

  /**
   * A tube through `path` with a radius per point and smooth normals; `squash`
   * flattens it along the given axis. Caps close both ends.
   */
  tube(path: V3[], radii: number[], color: Rgb, sides = 8, squash: { axis: V3; factor: number } | null = null, endColor: Rgb = color) {
    const rings: number[] = [];
    for (let k = 0; k < path.length; k++) {
      const t = norm(sub(path[Math.min(k + 1, path.length - 1)], path[Math.max(k - 1, 0)]));
      const ref: V3 = Math.abs(t[1]) < 0.9 ? [0, 1, 0] : [1, 0, 0];
      const u = norm(cross(t, ref));
      const w = cross(t, u);
      rings.push(this.out.count);
      for (let s = 0; s < sides; s++) {
        const a = (s / sides) * Math.PI * 2;
        let d: V3 = [u[0] * Math.cos(a) + w[0] * Math.sin(a), u[1] * Math.cos(a) + w[1] * Math.sin(a), u[2] * Math.cos(a) + w[2] * Math.sin(a)];
        if (squash) {
          const along = d[0] * squash.axis[0] + d[1] * squash.axis[1] + d[2] * squash.axis[2];
          d = [d[0] - squash.axis[0] * along * (1 - squash.factor), d[1] - squash.axis[1] * along * (1 - squash.factor), d[2] - squash.axis[2] * along * (1 - squash.factor)];
        }
        const r = radii[k];
        this.put([path[k][0] + d[0] * r, path[k][1] + d[1] * r, path[k][2] + d[2] * r], norm(d), color);
      }
    }
    for (let k = 0; k + 1 < path.length; k++) {
      for (let s = 0; s < sides; s++) {
        const a = rings[k] + s;
        const b = rings[k] + ((s + 1) % sides);
        const c = rings[k + 1] + ((s + 1) % sides);
        const d = rings[k + 1] + s;
        this.out.i.push(a, b, c, a, c, d);
      }
    }
    for (const [k, dirSign] of [
      [0, -1],
      [path.length - 1, 1],
    ] as const) {
      const t = norm(sub(path[Math.min(k + 1, path.length - 1)], path[Math.max(k - 1, 0)]));
      const n: V3 = [t[0] * dirSign, t[1] * dirSign, t[2] * dirSign];
      const centre = this.put(path[k], n, endColor);
      for (let s = 0; s < sides; s++) {
        const a = rings[k] + s;
        const b = rings[k] + ((s + 1) % sides);
        if (dirSign > 0) this.out.tri(centre, a, b);
        else this.out.tri(centre, b, a);
      }
    }
  }

  /**
   * A flat shape lying on the body: the outline (in the bone's frame, in front
   * of the surface) is pressed onto the surface along `into`, then lifted.
   */
  decal(outline: V3[], into: V3, lift: number, color: Rgb) {
    const d = norm(this.body.dir(this.bone, into));
    const pts = outline.map((p) => this.body.snap(this.body.at(this.bone, p), d, lift));
    const n: V3 = [-d[0], -d[1], -d[2]];
    const first = this.out.count;
    for (const p of pts) this.out.vertex(p, n, color, this.bone);
    for (let k = 1; k + 1 < pts.length; k++) this.out.tri(first, first + k, first + k + 1);
  }
}

/** Points of an ellipse in the XY plane at depth `z`, counter-clockwise seen from -Z looking at +Z. */
export function ellipse(cx: number, cy: number, rx: number, ry: number, z: number, n = 12, from = 0, to = Math.PI * 2): V3[] {
  const pts: V3[] = [];
  const steps = to - from >= Math.PI * 2 - 1e-6 ? n : n + 1;
  for (let k = 0; k < steps; k++) {
    const a = from + ((to - from) * k) / n;
    pts.push([cx - Math.cos(a) * rx, cy + Math.sin(a) * ry, z]);
  }
  return pts;
}
