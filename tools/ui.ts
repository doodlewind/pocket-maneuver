#!/usr/bin/env bun
/**
 * Compiles the interface (`ui/`, one PocketJS app) for a device. PocketJS
 * resolves `ui/pocket.json` against the device's profile, picks the
 * presentation its modality asks for and writes the bundle and its pak.
 *
 *   bun tools/ui.ts <psp|vita|3ds|ipod>     → .pocket-build/ui/<device>/maneuver.{js,pak}, plan.json
 *   bun tools/ui.ts prepare                 → the generated map only (ui/app/generated/)
 *   bun tools/ui.ts preview [device…]       → pictures of every screen, .pocket-build/ui/preview/
 *   bun tools/ui.ts test                    → the interface driven through a mock renderer
 */
import { existsSync, mkdirSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { POCKET_CAPABILITIES, definePlatformContractRegistry, defineTargetRegistry } from "../vendor/pocketjs/contracts/spec/platforms.ts";
import { extractHostBuildInputs, type HostBuildInputs } from "../vendor/pocketjs/framework/src/manifest/index.ts";
import { validateAndResolveBuildPlan } from "../vendor/pocketjs/framework/src/manifest/resolve.ts";
import { resolve3dsBuildPlan } from "../vendor/pocketjs/tools/3ds-profile.ts";
import { IPODTOUCH4_DEV_HOST_ABI } from "../vendor/pocketjs/tools/ipodtouch4-profile.ts";
import { encodePng } from "./png.ts";

export const DEVICES = ["psp", "vita", "3ds", "ipod"] as const;
export type Device = (typeof DEVICES)[number];

const root = resolve(import.meta.dir, "..");
const pocket = join(root, "vendor/pocketjs");
const project = join(root, "ui");
const manifest = join(project, "pocket.json");
const generated = join(project, "app/generated");

/**
 * The iPod touch 4 as Pocket Maneuver presents it. PocketJS's own profile for
 * the device describes its host, which draws at the panel's 640×960; here the
 * interface shares the game's drawable, 480×320 on its side at one sample per
 * logical pixel.
 */
export const IPOD_TARGET = "ipodtouch4-maneuver";
const IPOD_CONTRACTS = definePlatformContractRegistry(POCKET_CAPABILITIES, defineTargetRegistry({
  [IPOD_TARGET]: {
    hostAbi: IPODTOUCH4_DEV_HOST_ABI,
    platform: "ios",
    form: "takeover",
    display: { physicalViewport: [480, 320], logicalViewports: [[480, 320]], presentations: ["native"], rasterDensity: 1 },
    capabilities: ["input.touch", "text.glyphs.baked"],
  },
}));
function resolveIPodBuildPlan(manifest: unknown): unknown {
  const resolution = validateAndResolveBuildPlan(manifest, { target: IPOD_TARGET }, IPOD_CONTRACTS);
  if (!resolution.ok) throw new Error(`ui: ${resolution.diagnostics.map((d) => `${d.path || "/"}: ${d.message}`).join("; ")}`);
  return resolution.plan;
}
/** Devices outside PocketJS's public registry resolve through a profile kept with their tool. */
const PRIVATE: Partial<Record<Device, (manifest: unknown) => unknown>> = { "3ds": resolve3dsBuildPlan, ipod: resolveIPodBuildPlan };

export interface Interface {
  /** Directory holding `maneuver.js`, `maneuver.pak` and `plan.json`. */
  directory: string;
  inputs: HostBuildInputs;
  plan: { features: Record<string, boolean> };
}

function run(args: string[], cwd = pocket) {
  const done = Bun.spawnSync(args, { cwd, stdout: "inherit", stderr: "inherit" });
  if (done.exitCode !== 0) throw new Error(`ui: ${args.slice(0, 3).join(" ")} failed`);
}

/** Half as wide and as high: each texel the mean of four. */
function halve(rgba: Uint8Array, size: number): Uint8Array {
  const half = size / 2, out = new Uint8Array(half * half * 4);
  for (let y = 0; y < half; y++)
    for (let x = 0; x < half; x++)
      for (let c = 0; c < 4; c++) {
        const at = (yy: number, xx: number) => rgba[((y * 2 + yy) * size + x * 2 + xx) * 4 + c];
        out[(y * half + x) * 4 + c] = (at(0, 0) + at(0, 1) + at(1, 0) + at(1, 1) + 2) >> 2;
      }
  return out;
}

/**
 * Writes what the interface compiles from the world: the town from above at
 * both raster densities and the square of world it shows
 * (`ui/app/generated/`, ignored). The world compiler draws it from the
 * exported world (`bun tools/maneuver.ts export`).
 */
export async function prepareInterface(device?: Device): Promise<void> {
  const ir = join(root, ".pocket-build/world/ir");
  if (!existsSync(join(ir, "world.mvsw"))) throw new Error("ui: no exported world: run `bun tools/maneuver.ts export` first");
  mkdirSync(generated, { recursive: true });
  const raw = join(root, ".pocket-build/ui/map.bin");
  const stale = !existsSync(raw) || !existsSync(join(generated, "map.png")) || statSync(raw).mtimeMs < statSync(join(ir, "world.mvsw")).mtimeMs;
  if (stale) {
    mkdirSync(join(root, ".pocket-build/ui"), { recursive: true });
    run(["cargo", "run", "--release", "-q", "-p", "maneuver-cook", "--", "--in", ir, "--map", raw, "--map-size", "512"], root);
    const bytes = new Uint8Array(await Bun.file(raw).arrayBuffer());
    const view = new DataView(bytes.buffer);
    const size = view.getUint32(0, true), extent = view.getFloat32(8, true);
    const rgba = new Uint8Array(size * size * 4);
    for (let i = 0; i < size * size; i++) {
      const p = view.getUint16(16 + i * 2, true);
      rgba.set([((p >> 11) * 255 + 15) / 31, (((p >> 5) & 63) * 255 + 31) / 63, ((p & 31) * 255 + 15) / 31, 255], i * 4);
    }
    writeFileSync(join(generated, "map@2x.png"), encodePng(rgba, size, size));
    writeFileSync(join(generated, "map.png"), encodePng(halve(rgba, size), size / 2, size / 2));
    writeFileSync(join(generated, "map.ts"),
      `// Generated by tools/ui.ts from the exported world.\n\n/** Half the side of the square of world the map shows, metres. */\nexport const MAP_EXTENT = ${extent};\nexport const MAP_IMAGE = "generated/map.png";\n`);
  }
  // The PSP keeps the map as 16-bit texels: it shares 24 MB with the world.
  writeFileSync(join(project, "app/images.json"), JSON.stringify(device === "psp" ? { "generated/map.png": { psm: 0 } } : {}, null, 2) + "\n");
}

export async function compileInterface(device: Device): Promise<Interface> {
  await prepareInterface(device);
  const directory = join(root, ".pocket-build/ui", device);
  mkdirSync(directory, { recursive: true });
  const planPath = join(directory, "plan.json");
  const resolver = PRIVATE[device];
  if (resolver) {
    writeFileSync(planPath, JSON.stringify(resolver(await Bun.file(manifest).json()), null, 2) + "\n");
    run(["bun", "tools/build.ts", `--plan=${planPath}`, `--project-root=${project}`, `--outdir=${directory}`]);
  } else {
    run(["bun", "tools/pocket.ts", "compile", "--target", device, "--manifest", manifest, "--project-root", project, "--outdir", directory]);
    await Bun.write(planPath, Bun.file(join(project, ".pocket", device, "plan.json")));
  }
  const plan = await Bun.file(planPath).json();
  return { directory, plan, inputs: extractHostBuildInputs(plan) };
}

if (import.meta.main) {
  const [command, ...rest] = process.argv.slice(2);
  if (command === "prepare") await prepareInterface();
  else if (command === "preview" || command === "test") {
    // One process per device: a bundle owns `globalThis.frame`.
    for (const device of rest.length ? (rest as Device[]) : DEVICES) {
      await compileInterface(device);
      run(["bun", `ui/test/${command === "test" ? "flow" : "preview"}.ts`, device], root);
    }
  } else if (DEVICES.includes(command as Device)) {
    const built = await compileInterface(command as Device);
    console.log(`${command}: ${built.inputs.target}, ${built.inputs.viewport.logical.join("×")} @${built.inputs.viewport.rasterDensity}x → ${built.directory}`);
  } else throw new Error(`usage: bun tools/ui.ts <${DEVICES.join("|")}|prepare|preview [device…]|test>`);
}
