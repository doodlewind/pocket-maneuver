// Mesh and collision builders. The generator writes every surface twice:
// once into a render bucket (cell and layer) and once into the collision soup.

export type V3 = readonly [number, number, number];
export type Rgb = readonly [number, number, number];

/** Near cells: one mesh per layer. */
export const CELL = 64;
/** Far cells: one merged low-detail mesh. */
export const SUPER = 256;

/**
 * Render layers. A near cell draws `Base + Near` or `Base + Mid` by distance;
 * beyond that its super-cell draws `Far`.
 */
export const enum Layer {
  Base = 0,
  Near = 1,
  Mid = 2,
  Far = 3,
  /** One mesh that is always drawn: the horizon. */
  Backdrop = 4,
}

export const add = (a: V3, b: V3): V3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
export const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
export const mul = (a: V3, s: number): V3 => [a[0] * s, a[1] * s, a[2] * s];
export const mad = (a: V3, b: V3, s: number): V3 => [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s];
export const dot = (a: V3, b: V3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
export const cross = (a: V3, b: V3): V3 => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
export const len = (a: V3): number => Math.hypot(a[0], a[1], a[2]);
export const norm = (a: V3): V3 => {
  const l = len(a) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
};
export const lerp3 = (a: V3, b: V3, t: number): V3 => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
export const up = (p: V3, h: number): V3 => [p[0], p[1] + h, p[2]];
export const UP: V3 = [0, 1, 0];

/** Floats per vertex: position 3, normal 3, uv 2, tint 3. */
export const STRIDE = 11;

export class Geo {
  v = new Float32Array(STRIDE * 256);
  i = new Uint32Array(384);
  nv = 0;
  ni = 0;

  private growV(n: number) {
    if ((this.nv + n) * STRIDE > this.v.length) {
      const next = new Float32Array(Math.max(this.v.length * 2, (this.nv + n) * STRIDE));
      next.set(this.v);
      this.v = next;
    }
  }
  private growI(n: number) {
    if (this.ni + n > this.i.length) {
      const next = new Uint32Array(Math.max(this.i.length * 2, this.ni + n));
      next.set(this.i);
      this.i = next;
    }
  }
  vert(p: V3, n: V3, u: number, v: number, tint: Rgb): number {
    this.growV(1);
    const o = this.nv * STRIDE;
    const a = this.v;
    a[o] = p[0];
    a[o + 1] = p[1];
    a[o + 2] = p[2];
    a[o + 3] = n[0];
    a[o + 4] = n[1];
    a[o + 5] = n[2];
    a[o + 6] = u;
    a[o + 7] = v;
    a[o + 8] = tint[0];
    a[o + 9] = tint[1];
    a[o + 10] = tint[2];
    return this.nv++;
  }
  /**
   * A quad `a b c d`, counter-clockwise seen from its front: `a` bottom left,
   * `b` bottom right, `c` top right, `d` top left. `uv` is `[u0, u1, vBottom, vTop]`.
   */
  quad(a: V3, b: V3, c: V3, d: V3, uv: readonly [number, number, number, number], tint: Rgb, normal?: V3) {
    const n = normal ?? norm(cross(sub(b, a), sub(d, a)));
    const [u0, u1, vb, vt] = uv;
    const i0 = this.vert(a, n, u0, vb, tint);
    this.vert(b, n, u1, vb, tint);
    this.vert(c, n, u1, vt, tint);
    this.vert(d, n, u0, vt, tint);
    this.growI(6);
    this.i.set([i0, i0 + 1, i0 + 2, i0, i0 + 2, i0 + 3], this.ni);
    this.ni += 6;
  }
  /** A triangle with explicit texture coordinates per corner. */
  tri(a: V3, b: V3, c: V3, ta: readonly [number, number], tb: readonly [number, number], tc: readonly [number, number], tint: Rgb, normal?: V3) {
    const n = normal ?? norm(cross(sub(b, a), sub(c, a)));
    const i0 = this.vert(a, n, ta[0], ta[1], tint);
    this.vert(b, n, tb[0], tb[1], tint);
    this.vert(c, n, tc[0], tc[1], tint);
    this.growI(3);
    this.i.set([i0, i0 + 1, i0 + 2], this.ni);
    this.ni += 3;
  }
  vertices(): Float32Array {
    return this.v.subarray(0, this.nv * STRIDE);
  }
  indices(): Uint32Array {
    return this.i.subarray(0, this.ni);
  }
}

export interface Bucket {
  layer: Layer;
  cx: number;
  cz: number;
  geo: Geo;
}

export class Meshes {
  buckets = new Map<string, Bucket>();

  /** The bucket of `layer` for the cell that contains `(x, z)`. */
  at(layer: Layer, x: number, z: number): Geo {
    const size = layer === Layer.Far ? SUPER : CELL;
    const cx = layer === Layer.Backdrop ? 0 : Math.floor(x / size);
    const cz = layer === Layer.Backdrop ? 0 : Math.floor(z / size);
    const key = `${layer}:${cx}:${cz}`;
    let b = this.buckets.get(key);
    if (!b) {
      b = { layer, cx, cz, geo: new Geo() };
      this.buckets.set(key, b);
    }
    return b.geo;
  }
  /** Buckets in a stable order. */
  sorted(): Bucket[] {
    return [...this.buckets.values()].sort((a, b) => a.layer - b.layer || a.cz - b.cz || a.cx - b.cx);
  }
  triangles(layer: Layer): number {
    let n = 0;
    for (const b of this.buckets.values()) if (b.layer === layer) n += b.geo.ni / 3;
    return n;
  }
}

/** Collision soup in the layout of the simulation's world file. */
export class Collision {
  v: number[] = [];
  i: number[] = [];
  k: number[] = [];

  quad(a: V3, b: V3, c: V3, d: V3, kind: number) {
    const base = this.v.length / 3;
    this.v.push(...a, ...b, ...c, ...d);
    this.i.push(base, base + 1, base + 2, base, base + 2, base + 3);
    this.k.push(kind, kind);
  }
  tri(a: V3, b: V3, c: V3, kind: number) {
    const base = this.v.length / 3;
    this.v.push(...a, ...b, ...c);
    this.i.push(base, base + 1, base + 2);
    this.k.push(kind);
  }
}
