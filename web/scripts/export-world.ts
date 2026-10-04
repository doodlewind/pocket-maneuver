// Exports the world for the compiler: WorldIR, a directory of float geometry
// and source images with nothing baked, quantized or compressed.
//
//   bun web/scripts/export-world.ts [--seed 2026] [--out .pocket-build/world/ir]
//
// Files: meshes.bin (render buckets), models.bin (moving models), atlas.rgba,
// world.mvsw (the simulation's world), scene.json (light, air, LOD distances)
// and manifest.json, written last, with the sha256 of each.

import { mkdir } from "node:fs/promises";
import { join, resolve } from "node:path";
import { ATLAS_H, ATLAS_W, paintAtlas, STRIP, FACADE } from "../src/world/atlas";
import { generate } from "../src/world/city";
import { CELL, Geo, Layer, STRIDE, SUPER } from "../src/world/geo";
import { buildScout } from "../src/world/scout";
import { SKIN_STRIDE, SkinModel } from "../src/world/sdf";
import { buildTitan } from "../src/world/titan";
import { Sim } from "../src/sim/sim";
import { SCENE } from "../src/world/scene";
import { skyColor } from "../src/render/sky";
import { writeWorldFile } from "../src/world/worldfile";

const args = process.argv.slice(2);
const arg = (name: string, dflt: string) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : dflt;
};
const seed = Number(arg("seed", "2026"));
const out = resolve(arg("out", join(import.meta.dir, "../../.pocket-build/world/ir")));

function meshBytes(header: number[], entries: { head: number[]; geo: Geo }[]): Uint8Array {
  let size = header.length * 4;
  for (const e of entries) size += e.head.length * 4 + 8 + e.geo.nv * STRIDE * 4 + e.geo.ni * 4;
  const bytes = new Uint8Array(size);
  const view = new DataView(bytes.buffer);
  let at = 0;
  const i32 = (v: number) => {
    view.setInt32(at, v, true);
    at += 4;
  };
  header.forEach(i32);
  for (const e of entries) {
    e.head.forEach(i32);
    i32(e.geo.nv);
    i32(e.geo.ni);
    new Float32Array(bytes.buffer, at, e.geo.nv * STRIDE).set(e.geo.vertices());
    at += e.geo.nv * STRIDE * 4;
    new Uint32Array(bytes.buffer, at, e.geo.ni).set(e.geo.indices());
    at += e.geo.ni * 4;
  }
  return bytes;
}

/** Skinned models: id, vertex count, index count, then 12 floats per vertex and `u32` indices. */
function modelBytes(models: { id: number; model: SkinModel }[]): Uint8Array {
  let size = 12;
  for (const m of models) size += 12 + m.model.v.byteLength + m.model.i.byteLength;
  const bytes = new Uint8Array(size);
  const view = new DataView(bytes.buffer);
  view.setUint32(0, 0x444d564d, true);
  view.setUint32(4, 2, true);
  view.setUint32(8, models.length, true);
  let at = 12;
  for (const m of models) {
    view.setInt32(at, m.id, true);
    view.setInt32(at + 4, m.model.v.length / SKIN_STRIDE, true);
    view.setInt32(at + 8, m.model.i.length, true);
    at += 12;
    bytes.set(new Uint8Array(m.model.v.buffer, m.model.v.byteOffset, m.model.v.byteLength), at);
    at += m.model.v.byteLength;
    bytes.set(new Uint8Array(m.model.i.buffer, m.model.i.byteOffset, m.model.i.byteLength), at);
    at += m.model.i.byteLength;
  }
  return bytes;
}

const t0 = performance.now();
const gen = generate(seed);
const buckets = gen.world.meshes.sorted().filter((b) => b.geo.ni > 0);
// Models are built on the simulation's bind poses: the player (0), then the three giant builds, detailed and coarse.
const wasm = await Bun.file(join(import.meta.dir, "../public/sim/maneuver_sim.wasm")).arrayBuffer();
const sim = await Sim.load(wasm, null, 0);
const models = [{ id: 0, model: buildScout(sim.bind(0)) }];
for (let v = 0; v < 3; v++) {
  models.push({ id: 10 + v, model: buildTitan(v, sim.bind(1 + v), 1 / 100) });
  models.push({ id: 20 + v, model: buildTitan(v, sim.bind(1 + v), 1 / 36) });
}
const files: Record<string, Uint8Array> = {
  "meshes.bin": meshBytes(
    [0x5249564d, 1, buckets.length],
    buckets.map((b) => ({ head: [b.layer, b.cx, b.cz], geo: b.geo })),
  ),
  "models.bin": modelBytes(models),
  "atlas.rgba": new Uint8Array(paintAtlas(seed).buffer),
  "world.mvsw": writeWorldFile(gen.world.col, gen.entities),
};

// Strip boundaries let the compiler filter mips inside each strip.
const edges = new Set<number>([0, ATLAS_H]);
for (const s of Object.values(STRIP)) {
  edges.add(s.y);
  edges.add(s.y + s.h);
}
for (const f of FACADE) {
  edges.add(f.gable);
  edges.add(f.gable + 256);
  edges.add(f.base);
}
// The sky as a table: zenith angle by azimuth relative to the sun.
const sky: number[][] = [];
for (let e = -2; e <= 16; e++) {
  for (let a = 0; a < 24; a++) {
    const el = (e / 16) * (Math.PI / 2);
    const az = (a / 24) * Math.PI * 2;
    sky.push(skyColor([Math.cos(el) * Math.cos(az), Math.sin(el), Math.cos(el) * Math.sin(az)]));
  }
}
const layerCount = (l: Layer) => buckets.filter((b) => b.layer === l).length;
const scene = {
  seed,
  ...SCENE,
  cell: CELL,
  superCell: SUPER,
  atlas: { width: ATLAS_W, height: ATLAS_H, stripEdges: [...edges].sort((a, b) => a - b) },
  skyTable: { elevations: [-2, 16], azimuths: 24, colors: sky },
  entities: { dummies: gen.entities.dummies.length, depots: gen.entities.depots.length, waypoints: gen.entities.waypoints.length, spawn: gen.entities.spawn },
  source: {
    buckets: { base: layerCount(Layer.Base), near: layerCount(Layer.Near), mid: layerCount(Layer.Mid), far: layerCount(Layer.Far), backdrop: layerCount(Layer.Backdrop) },
    triangles: [Layer.Base, Layer.Near, Layer.Mid, Layer.Far, Layer.Backdrop].map((l) => gen.world.meshes.triangles(l)),
    collisionTriangles: gen.world.col.k.length,
  },
};
files["scene.json"] = new TextEncoder().encode(JSON.stringify(scene, null, 1));

await mkdir(out, { recursive: true });
const manifest = { version: 1, name: "walled-town", seed, files: [] as { path: string; bytes: number; sha256: string }[] };
for (const [path, bytes] of Object.entries(files)) {
  await Bun.write(join(out, path), bytes);
  manifest.files.push({ path, bytes: bytes.length, sha256: new Bun.CryptoHasher("sha256").update(bytes).digest("hex") });
}
await Bun.write(join(out, "manifest.json"), JSON.stringify(manifest, null, 1));
console.log(`world IR: ${out}  seed ${seed}  ${buckets.length} buckets  ${(Object.values(files).reduce((n, b) => n + b.length, 0) / 1e6).toFixed(1)} MB  ${(performance.now() - t0).toFixed(0)} ms`);
