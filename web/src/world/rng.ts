// Deterministic randomness: every part of the world draws from a seeded
// stream. No Math.random, no wall time.

export class Rng {
  private s: number;
  constructor(seed: number) {
    this.s = seed >>> 0;
  }
  /** Uniform in [0, 1). */
  next(): number {
    let t = (this.s += 0x6d2b79f5);
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }
  range(a: number, b: number): number {
    return a + (b - a) * this.next();
  }
  int(a: number, b: number): number {
    return a + Math.floor(this.next() * (b - a + 1));
  }
  chance(p: number): boolean {
    return this.next() < p;
  }
  pick<T>(items: readonly T[]): T {
    return items[Math.floor(this.next() * items.length)];
  }
  /** An independent stream for a numbered child. */
  fork(n: number): Rng {
    return new Rng(hash2(this.s, n));
  }
}

export function hash2(a: number, b: number): number {
  let h = (a | 0) ^ Math.imul(b | 0, 0x9e3779b1);
  h = Math.imul(h ^ (h >>> 16), 0x85ebca6b);
  h = Math.imul(h ^ (h >>> 13), 0xc2b2ae35);
  return (h ^ (h >>> 16)) >>> 0;
}

/** Value in [0, 1) for a lattice point. */
export function hash01(x: number, y: number, seed = 0): number {
  return hash2(hash2(x | 0, y | 0), seed) / 4294967296;
}

/** Smooth value noise in [0, 1). `period` wraps x for tileable textures. */
export function noise2(x: number, y: number, seed = 0, period = 0): number {
  const xi = Math.floor(x);
  const yi = Math.floor(y);
  const fx = x - xi;
  const fy = y - yi;
  const wrap = (v: number) => (period > 0 ? ((v % period) + period) % period : v);
  const a = hash01(wrap(xi), yi, seed);
  const b = hash01(wrap(xi + 1), yi, seed);
  const c = hash01(wrap(xi), yi + 1, seed);
  const d = hash01(wrap(xi + 1), yi + 1, seed);
  const ux = fx * fx * (3 - 2 * fx);
  const uy = fy * fy * (3 - 2 * fy);
  return a + (b - a) * ux + (c - a) * uy + (a - b - c + d) * ux * uy;
}

export function fbm(x: number, y: number, seed = 0, octaves = 4, period = 0): number {
  let sum = 0;
  let amp = 0.5;
  let f = 1;
  for (let i = 0; i < octaves; i++) {
    sum += amp * noise2(x * f, y * f, seed + i * 17, period > 0 ? period * f : 0);
    amp *= 0.5;
    f *= 2;
  }
  return sum;
}
