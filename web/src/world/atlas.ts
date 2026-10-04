// The world's one texture: horizontal strips that repeat along U.
//
// Every static surface samples this atlas, so a cell is one draw. A strip is a
// band of rows; U wraps across the full width, V stays inside the band. The
// rows of a facade style are stacked in storey order, so one quad that spans
// several rows shows several storeys.
//
// The painter is a software rasterizer over a byte array: it runs the same in
// the browser and in Bun, and its output does not depend on a canvas.

import { fbm, hash01, Rng } from "./rng";

export const ATLAS_W = 1024;
export const ATLAS_H = 2048;

export interface Strip {
  /** First pixel row. */
  y: number;
  /** Rows. */
  h: number;
  /** Metres of world for one full U repeat. */
  mPerU: number;
  /** Metres of world the strip's height covers. */
  mPerV: number;
}

/** Pixel rows per storey and per bay; a bay is 3 m wide and a storey 3 m high. */
export const ROW = 128;
export const BAY_M = 3;
export const STOREY_M = 3;
export const BAYS = ATLAS_W / ROW;

export interface FacadeStyle {
  /** Pixel row just below the ground storey. */
  base: number;
  /** Storey rows available, ground included. */
  storeys: number;
  /** First pixel row of the gable band (256 rows, 6 m). */
  gable: number;
}

export const FACADE: readonly FacadeStyle[] = [
  { base: 768, storeys: 4, gable: 0 },
  { base: 1408, storeys: 3, gable: 768 },
];

export const STRIP = {
  ROOF: { y: 1408, h: 128, mPerU: 24, mPerV: 7 },
  COBBLE: { y: 1536, h: 128, mPerU: 32, mPerV: 4 },
  STONE: { y: 1664, h: 128, mPerU: 48, mPerV: 6 },
  GRASS: { y: 1792, h: 64, mPerU: 32, mPerV: 4 },
  DIRT: { y: 1856, h: 64, mPerU: 32, mPerV: 4 },
  WOOD: { y: 1920, h: 48, mPerU: 16, mPerV: 2 },
  FLAT: { y: 1968, h: 16, mPerU: 16, mPerV: 1 },
  WATER: { y: 1984, h: 64, mPerU: 32, mPerV: 8 },
} as const satisfies Record<string, Strip>;

/** V at the bottom and at the top of a strip, half a texel inside it. */
export function stripV(s: Strip): [number, number] {
  return [(s.y + s.h - 0.5) / ATLAS_H, (s.y + 0.5) / ATLAS_H];
}

/** V for a wall of `storeys` rows of a facade style. */
export function facadeV(style: FacadeStyle, storeys: number): [number, number] {
  return [(style.base - 0.5) / ATLAS_H, (style.base - storeys * ROW + 0.5) / ATLAS_H];
}

/** V for a gable of `height` metres: its base row and its apex. */
export function gableV(style: FacadeStyle, height: number): [number, number] {
  const rows = Math.min(height / 6, 1) * 256;
  return [(style.gable + 256 - 0.5) / ATLAS_H, (style.gable + 256 - rows + 0.5) / ATLAS_H];
}

type C = readonly [number, number, number];

class Painter {
  d = new Uint8ClampedArray(ATLAS_W * ATLAS_H * 4);

  set(x: number, y: number, c: C, a = 1) {
    if (y < 0 || y >= ATLAS_H) return;
    const o = (y * ATLAS_W + (((x % ATLAS_W) + ATLAS_W) % ATLAS_W)) * 4;
    const d = this.d;
    d[o] = d[o] + (c[0] - d[o]) * a;
    d[o + 1] = d[o + 1] + (c[1] - d[o + 1]) * a;
    d[o + 2] = d[o + 2] + (c[2] - d[o + 2]) * a;
    d[o + 3] = 255;
  }
  rect(x: number, y: number, w: number, h: number, c: C, a = 1) {
    for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) this.set(x + i, y + j, c, a);
  }
  /** Multiplies a region by `f(x, y)`. */
  shade(x: number, y: number, w: number, h: number, f: (x: number, y: number) => number) {
    const d = this.d;
    for (let j = 0; j < h; j++) {
      const yy = y + j;
      if (yy < 0 || yy >= ATLAS_H) continue;
      for (let i = 0; i < w; i++) {
        const xx = (((x + i) % ATLAS_W) + ATLAS_W) % ATLAS_W;
        const o = (yy * ATLAS_W + xx) * 4;
        const k = f(x + i, yy);
        d[o] *= k;
        d[o + 1] *= k;
        d[o + 2] *= k;
      }
    }
  }
  /** A line of thickness `t` with square ends, clipped to rows `[y0c, y1c)`. */
  line(x0: number, y0: number, x1: number, y1: number, t: number, c: C, y0c = 0, y1c = ATLAS_H) {
    const dx = x1 - x0;
    const dy = y1 - y0;
    const l2 = dx * dx + dy * dy || 1;
    const r = t / 2;
    for (let y = Math.floor(Math.min(y0, y1) - r); y <= Math.ceil(Math.max(y0, y1) + r); y++) {
      if (y < y0c || y >= y1c) continue;
      for (let x = Math.floor(Math.min(x0, x1) - r); x <= Math.ceil(Math.max(x0, x1) + r); x++) {
        const k = Math.max(0, Math.min(1, ((x + 0.5 - x0) * dx + (y + 0.5 - y0) * dy) / l2));
        const d = Math.hypot(x + 0.5 - x0 - dx * k, y + 0.5 - y0 - dy * k);
        if (d <= r + 0.5) this.set(x, y, c, Math.min(1, r + 0.5 - d));
      }
    }
  }
}

const PLASTER: C = [238, 232, 219];
const TIMBER: C = [66, 46, 32];
const GLASS: C = [44, 58, 70];
const FRAME: C = [206, 196, 176];
const STONE: C = [176, 170, 160];

function timber(p: Painter, x: number, y: number, w: number, h: number, seed: number) {
  p.rect(x, y, w, h, TIMBER);
  const along = w >= h;
  p.shade(x, y, w, h, (px, py) => {
    const g = along ? hash01(py, seed, 3) : hash01(px, seed, 3);
    const e = along ? Math.min(py - y, y + h - 1 - py) : Math.min(px - x, x + w - 1 - px);
    return (0.82 + 0.3 * g) * (e < 1 ? 0.75 : 1);
  });
}

function plaster(p: Painter, y: number, h: number, seed: number) {
  p.rect(0, y, ATLAS_W, h, PLASTER);
  p.shade(0, y, ATLAS_W, h, (px, py) => 0.9 + 0.14 * fbm(px / 37, py / 53, seed, 4, ATLAS_W / 37) + 0.04 * hash01(px, py, seed));
}

function windowAt(p: Painter, x: number, y: number, w: number, h: number, r: Rng, opts: { shutters?: C; box?: boolean; arch?: boolean } = {}) {
  // Timber surround, glass with a sky gradient, a light cross.
  timber(p, x - 5, y - 5, w + 10, h + 10, r.int(0, 999));
  p.rect(x, y, w, h, GLASS);
  p.shade(x, y, w, h, (_px, py) => 1.25 - 0.55 * ((py - y) / h));
  // A lit corner, as if the sky were reflected.
  p.line(x + w * 0.15, y + h * 0.45, x + w * 0.4, y + h * 0.1, 3, [120, 150, 170]);
  p.rect(x + Math.floor(w / 2) - 1, y, 3, h, FRAME);
  p.rect(x, y + Math.floor(h * 0.42), w, 3, FRAME);
  p.rect(x, y, w, 2, FRAME);
  p.rect(x, y + h - 2, w, 2, FRAME);
  p.rect(x, y, 2, h, FRAME);
  p.rect(x + w - 2, y, 2, h, FRAME);
  if (opts.shutters) {
    for (const sx of [x - 5 - 13, x + w + 5]) {
      p.rect(sx, y - 2, 13, h + 4, opts.shutters);
      p.shade(sx, y - 2, 13, h + 4, (px, py) => ((py - y) % 6 === 0 ? 0.72 : 1) * (px === sx || px === sx + 12 ? 0.7 : 1));
    }
  }
  if (opts.box) {
    p.rect(x - 4, y + h + 3, w + 8, 7, [92, 62, 40]);
    for (let i = 0; i < w + 6; i += 3) {
      const g = r.next();
      p.rect(x - 3 + i, y + h - 1 - Math.floor(g * 3), 3, 5, g > 0.62 ? [188, 52, 48] : [74, 112, 52]);
    }
  }
}

function upperStorey(p: Painter, y: number, seed: number, timbered: boolean) {
  const r = new Rng(seed);
  plaster(p, y, ROW, seed);
  const shutter: C[] = [
    [70, 104, 84],
    [128, 54, 44],
    [58, 84, 118],
    [108, 88, 52],
  ];
  for (let b = 0; b < BAYS; b++) {
    const x = b * ROW;
    const kind = r.int(0, 9);
    if (timbered) {
      if (kind === 0) {
        // A braced panel without a window.
        p.line(x + 6, y + ROW - 8, x + ROW / 2, y + 8, 7, TIMBER, y, y + ROW);
        p.line(x + ROW - 6, y + ROW - 8, x + ROW / 2, y + 8, 7, TIMBER, y, y + ROW);
      } else {
        if (kind <= 3) {
          // St Andrew's cross under the window.
          p.line(x + 34, y + 98, x + 94, y + ROW - 8, 5, TIMBER, y, y + ROW);
          p.line(x + 94, y + 98, x + 34, y + ROW - 8, 5, TIMBER, y, y + ROW);
        } else if (kind <= 5) {
          p.line(x + 6, y + ROW - 8, x + 30, y + 40, 6, TIMBER, y, y + ROW);
          p.line(x + ROW - 6, y + ROW - 8, x + ROW - 30, y + 40, 6, TIMBER, y, y + ROW);
        }
        timber(p, x + 28, y + 6, 6, ROW - 12, seed + b);
        timber(p, x + 94, y + 6, 6, ROW - 12, seed + b + 50);
        timber(p, x + 6, y + 92, ROW - 12, 6, seed + b + 70);
      }
    } else {
      // Stone-dressed: a moulded sill line.
      p.rect(x, y + 96, ROW, 4, [198, 190, 176]);
      p.rect(x, y + 100, ROW, 2, [150, 142, 130]);
    }
    if (kind !== 0 || !timbered) {
      const two = !timbered && kind > 6;
      if (two) {
        windowAt(p, x + 24, y + 30, 30, 62, r);
        windowAt(p, x + 74, y + 30, 30, 62, r);
      } else {
        windowAt(p, x + 40, y + 30, 48, 62, r, { shutters: kind % 2 === 1 ? r.pick(shutter) : undefined, box: kind === 6 || kind === 8 });
      }
    }
    // Posts on the bay lines.
    if (timbered) {
      timber(p, x - 6, y, 12, ROW, seed + b * 3);
    }
  }
  // Plate and sill beams run the full width; with the storey below they read as one jetty beam.
  timber(p, 0, y, ATLAS_W, 8, seed + 1);
  timber(p, 0, y + ROW - 7, ATLAS_W, 7, seed + 2);
  if (!timbered) {
    p.rect(0, y + ROW - 7, ATLAS_W, 7, [196, 188, 172]);
    p.rect(0, y, ATLAS_W, 8, [214, 206, 192]);
    p.shade(0, y, ATLAS_W, 8, (_px, py) => (py - y > 5 ? 0.78 : 1));
  }
}

function stoneCourses(p: Painter, y: number, h: number, course: number, seed: number, base: C) {
  const r = new Rng(seed);
  p.rect(0, y, ATLAS_W, h, base);
  for (let cy = 0; cy < h; cy += course) {
    let x = Math.floor(r.range(0, 40));
    const first = x;
    while (x < ATLAS_W + first) {
      const w = Math.floor(r.range(course * 1.2, course * 2.6));
      const k = r.range(0.84, 1.06);
      const hue = r.range(-6, 6);
      const ch = Math.min(course, h - cy);
      p.rect(x, y + cy, w, ch, [base[0] * k + hue, base[1] * k, base[2] * k - hue]);
      p.rect(x, y + cy, 2, ch, [base[0] * 0.5, base[1] * 0.5, base[2] * 0.5], 0.8);
      x += w;
    }
    p.rect(0, y + cy, ATLAS_W, 2, [base[0] * 0.5, base[1] * 0.5, base[2] * 0.5], 0.8);
    p.rect(0, y + cy + 2, ATLAS_W, 1, [255, 255, 255], 0.12);
  }
  p.shade(0, y, ATLAS_W, h, (px, py) => 0.86 + 0.2 * fbm(px / 61, py / 23, seed + 5, 4, ATLAS_W / 61) + 0.05 * hash01(px, py, seed));
}

function groundStorey(p: Painter, y: number, seed: number, timbered: boolean) {
  const r = new Rng(seed);
  if (timbered) {
    plaster(p, y, ROW, seed);
    // A stone plinth.
    const sy = y + ROW - 30;
    stoneCourses(p, sy, 30, 15, seed + 9, STONE);
  } else {
    stoneCourses(p, y, ROW, 22, seed + 9, [190, 182, 168]);
  }
  for (let b = 0; b < BAYS; b++) {
    const x = b * ROW;
    const door = b === 1 || b === 5;
    if (door) {
      // A plank door under an arch.
      const dx = x + 40;
      const dw = 48;
      const dy = y + 22;
      const dh = ROW - 22;
      timber(p, dx - 6, dy - 6, dw + 12, dh + 6, seed + b);
      p.rect(dx, dy, dw, dh, [104, 70, 42]);
      p.shade(dx, dy, dw, dh, (px, py) => ((px - dx) % 8 === 0 ? 0.6 : 0.9 + 0.2 * hash01(px, 7, seed)) * (py - dy < 10 ? 0.55 + 0.045 * (py - dy) : 1));
      p.rect(dx + dw - 12, dy + Math.floor(dh / 2), 4, 4, [40, 36, 32]);
      // A lantern bracket beside it.
      p.rect(dx + dw + 12, dy + 4, 3, 12, [40, 36, 32]);
      p.rect(dx + dw + 9, dy + 16, 9, 10, [236, 196, 110]);
    } else {
      const wide = r.chance(0.5);
      const ww = wide ? 72 : 48;
      windowAt(p, x + (ROW - ww) / 2, y + 28, ww, 58, r, { box: !wide && r.chance(0.4) });
    }
    if (timbered) timber(p, x - 6, y, 12, ROW - 30, seed + b * 3);
  }
  timber(p, 0, y, ATLAS_W, 8, seed + 1);
  if (!timbered) {
    p.rect(0, y, ATLAS_W, 8, [214, 206, 192]);
  }
  // Grime rising from the street.
  p.shade(0, y + ROW - 40, ATLAS_W, 40, (px, py) => 1 - 0.3 * ((py - (y + ROW - 40)) / 40) * (0.6 + 0.8 * fbm(px / 29, 0, seed + 3, 3, ATLAS_W / 29)));
}

function gableBand(p: Painter, y: number, seed: number, timbered: boolean) {
  const r = new Rng(seed);
  const H = 256;
  plaster(p, y, H, seed);
  for (let b = 0; b < BAYS; b++) {
    const x = b * ROW;
    if (timbered) {
      // Chevron struts in two tiers.
      for (const ty of [y + 136, y + 16]) {
        p.line(x + 6, ty + 104, x + ROW / 2, ty + 8, 6, TIMBER, y, y + H);
        p.line(x + ROW - 6, ty + 104, x + ROW / 2, ty + 8, 6, TIMBER, y, y + H);
      }
      timber(p, x - 6, y, 12, H, seed + b);
    }
    if (b % 2 === 0) windowAt(p, x + 44, y + 168, 40, 50, r);
    else if (!timbered) windowAt(p, x + 48, y + 176, 32, 40, r);
  }
  timber(p, 0, y + 124, ATLAS_W, 8, seed + 4);
  timber(p, 0, y + H - 8, ATLAS_W, 8, seed + 5);
  if (!timbered) {
    p.rect(0, y + 124, ATLAS_W, 8, [204, 196, 180]);
    p.rect(0, y + H - 8, ATLAS_W, 8, [204, 196, 180]);
  }
}

function roof(p: Painter, s: Strip, seed: number) {
  const r = new Rng(seed);
  const th = 8;
  const tw = 12;
  for (let row = 0; row * th < s.h; row++) {
    const off = row % 2 === 0 ? 0 : tw / 2;
    for (let x = 0; x < ATLAS_W; x += tw) {
      const k = r.range(0.74, 1.0) * (r.chance(0.04) ? 0.72 : 1);
      p.rect(x + off, s.y + row * th, tw, th, [232 * k, 222 * k, 212 * k]);
      p.rect(x + off, s.y + row * th, 1, th, [120, 112, 104], 0.55);
    }
    // The row above overlaps this one: a shadow line under its edge, a lit lip above the next.
    p.rect(0, s.y + row * th, ATLAS_W, 2, [84, 78, 72], 0.75);
    p.rect(0, s.y + row * th + th - 1, ATLAS_W, 1, [255, 250, 240], 0.25);
  }
  p.shade(0, s.y, ATLAS_W, s.h, (px, py) => 0.82 + 0.3 * fbm(px / 71, py / 31, seed, 4, ATLAS_W / 71));
}

function cobble(p: Painter, s: Strip, seed: number) {
  p.rect(0, s.y, ATLAS_W, s.h, [74, 68, 62]);
  const cell = 8;
  for (let j = 0; j < s.h / cell; j++) {
    for (let i = 0; i < ATLAS_W / cell; i++) {
      const k = 0.78 + 0.3 * hash01(i, j, seed);
      const ox = Math.floor(hash01(i, j, seed + 1) * 3) + (j % 2) * 4;
      const w = cell - 2 + Math.floor(hash01(i, j, seed + 2) * 2);
      const warm = hash01(i, j, seed + 3) * 12;
      p.rect(i * cell + ox, s.y + j * cell + 1, w, cell - 2, [160 * k + warm, 154 * k, 146 * k - warm * 0.5]);
      p.set(i * cell + ox, s.y + j * cell + 1, [74, 68, 62], 0.6);
      p.set(i * cell + ox + w - 1, s.y + j * cell + cell - 2, [74, 68, 62], 0.6);
    }
  }
  p.shade(0, s.y, ATLAS_W, s.h, (px, py) => 0.8 + 0.36 * fbm(px / 83, py / 47, seed + 4, 4, ATLAS_W / 83));
}

function soil(p: Painter, s: Strip, seed: number, base: C, speck: C, amount: number) {
  p.rect(0, s.y, ATLAS_W, s.h, base);
  p.shade(0, s.y, ATLAS_W, s.h, (px, py) => 0.74 + 0.5 * fbm(px / 43, py / 19, seed, 5, ATLAS_W / 43));
  const r = new Rng(seed);
  for (let i = 0; i < ATLAS_W * s.h * amount; i++) {
    const x = r.int(0, ATLAS_W - 1);
    const y = s.y + r.int(0, s.h - 2);
    p.set(x, y, speck, r.range(0.25, 0.7));
    p.set(x, y + 1, speck, 0.3);
  }
}

function wood(p: Painter, s: Strip, seed: number) {
  p.rect(0, s.y, ATLAS_W, s.h, [150, 112, 74]);
  p.shade(0, s.y, ATLAS_W, s.h, (px, py) => (px % 16 === 0 ? 0.5 : 1) * (0.8 + 0.3 * hash01(Math.floor(px / 16), 0, seed)) * (0.9 + 0.16 * fbm(px / 5, py / 37, seed, 3, ATLAS_W / 5)));
}

function water(p: Painter, s: Strip, seed: number) {
  p.rect(0, s.y, ATLAS_W, s.h, [62, 98, 108]);
  p.shade(0, s.y, ATLAS_W, s.h, (px, py) => 0.8 + 0.5 * fbm(px / 59, py / 7, seed, 4, ATLAS_W / 59));
  const r = new Rng(seed);
  for (let i = 0; i < 260; i++) {
    p.rect(r.int(0, ATLAS_W), s.y + r.int(1, s.h - 2), r.int(6, 28), 1, [186, 212, 214], r.range(0.2, 0.55));
  }
}

/** Paints the atlas: RGBA, `ATLAS_W × ATLAS_H`, row 0 first. */
export function paintAtlas(seed: number): Uint8ClampedArray {
  const p = new Painter();
  FACADE.forEach((f, i) => {
    const timbered = i === 0;
    gableBand(p, f.gable, seed + i * 100 + 7, timbered);
    for (let s = 1; s < f.storeys; s++) upperStorey(p, f.base - (s + 1) * ROW, seed + i * 100 + s, timbered);
    groundStorey(p, f.base - ROW, seed + i * 100, timbered);
  });
  roof(p, STRIP.ROOF, seed + 300);
  cobble(p, STRIP.COBBLE, seed + 310);
  stoneCourses(p, STRIP.STONE.y, STRIP.STONE.h, 32, seed + 320, [184, 178, 166]);
  // Rain streaks down the masonry.
  p.shade(0, STRIP.STONE.y, ATLAS_W, STRIP.STONE.h, (px, py) => 1 - 0.22 * Math.max(0, fbm(px / 9, py / 140, seed + 321, 3, ATLAS_W / 9) - 0.45) * 2);
  soil(p, STRIP.GRASS, seed + 330, [112, 138, 70], [150, 172, 88], 0.05);
  soil(p, STRIP.DIRT, seed + 340, [150, 126, 94], [98, 82, 62], 0.03);
  wood(p, STRIP.WOOD, seed + 350);
  p.rect(0, STRIP.FLAT.y, ATLAS_W, STRIP.FLAT.h, [255, 255, 255]);
  water(p, STRIP.WATER, seed + 360);
  return p.d;
}
