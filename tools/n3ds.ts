#!/usr/bin/env bun
// Pocket Maneuver on Nintendo 3DS: build the Rust core and the C host into a
// .3dsx (devkitARM in PocketJS's pinned container), install and start it over
// PocketJS's paired LAN wire, steer and measure it.
//
//   bun tools/n3ds.ts build                     # dist/3ds/pocket-maneuver.3dsx, the pack in its ROMFS
//   bun tools/n3ds.ts install [--no-build]      # build, send, start, wait for the game to report
//   bun tools/n3ds.ts status
//   bun tools/n3ds.ts ctl "auto=1 stats=1"      # Game::control words
//   bun tools/n3ds.ts capture [--out f.png] [--surface top|auxiliary]
//   bun tools/n3ds.ts bench [--seconds 60]      # autopilot frame timings → .pocket-build/validation/3ds/
//
// `--host ADDRESS` (default 192.168.8.159, or POCKET_3DS_HOST). The console
// must run a Pocket Runtime build with the paired wire: this program itself
// once installed, or another Pocket Nexus .3dsx. Installs are .3dsx only.

import { $ } from "bun";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pocketRuntimeDeviceId } from "../vendor/pocketjs/contracts/spec/pocket-runtime-wire.ts";
import { withDeviceLease } from "../vendor/pocketjs/tools/device-lease.ts";
import { THREE_DS_DEV_HOST_ABI, THREE_DS_DEV_TARGET_ID } from "../vendor/pocketjs/tools/3ds-profile.ts";
import { discoverPocketRuntimes, parsePocketRuntimeToken, PocketRuntimeClient } from "../vendor/pocketjs/tools/3ds-runtime-client.ts";
import { runContainer } from "../vendor/pocketjs/tools/3ds-toolchain.ts";

const ROOT = resolve(import.meta.dir, "..");
const POCKETJS = join(ROOT, "vendor/pocketjs");
const DIR = join(ROOT, ".pocket-build/3ds");
const PACK = join(ROOT, ".pocket-build/world/walled-town.n3ds60.pack");
const ARTIFACT = join(ROOT, "dist/3ds/pocket-maneuver.3dsx");
const NAME = "pocket-maneuver.3dsx";
const RECEIPTS = join(ROOT, ".pocket-build/validation/3ds");
const argv = process.argv.slice(2);
const cmd = argv[0] ?? "";
const opt = (key: string, fallback: string) => {
  const at = argv.indexOf(key);
  return at < 0 ? fallback : (argv[at + 1] ?? fallback);
};
const host = opt("--host", process.env.POCKET_3DS_HOST ?? "192.168.8.159");

const sha = (path: string) => new Bun.CryptoHasher("sha256").update(readFileSync(path)).digest("hex");

async function build() {
  if (!existsSync(PACK)) throw new Error(`no pack at ${PACK}: run \`bun tools/maneuver.ts cook --profile n3ds60\` first`);
  await $`rustup run nightly-2026-07-02 cargo build --release`.cwd(join(ROOT, "n3ds/core"));
  const romfs = join(DIR, "romfs");
  mkdirSync(romfs, { recursive: true });
  mkdirSync(join(DIR, "build"), { recursive: true });
  if (!existsSync(join(romfs, "world.pack")) || sha(join(romfs, "world.pack")) !== sha(PACK)) cpSync(PACK, join(romfs, "world.pack"));
  // The build's identity: the sources, the core library and the pack.
  const id = new Bun.CryptoHasher("sha256");
  for (const f of readdirSync(join(ROOT, "n3ds/src")).sort()) id.update(readFileSync(join(ROOT, "n3ds/src", f)));
  id.update(readFileSync(join(ROOT, "n3ds/core/target/armv6k-nintendo-3ds/release/libmaneuver_n3ds_core.a")));
  id.update(sha(PACK));
  const buildId = id.digest("hex").slice(0, 12);
  const header = `#define POCKETJS_HOST_ABI ${THREE_DS_DEV_HOST_ABI}\n#define POCKETJS_TARGET_ID "${THREE_DS_DEV_TARGET_ID}"\n#define MANEUVER_BUILD_ID "${buildId}"\n`;
  const config = join(DIR, "build/config.h");
  if (!existsSync(config) || readFileSync(config, "utf8") !== header) writeFileSync(config, header);
  // Compile a snapshot on the container's own filesystem: the shared mount can show a stale size for a file
  // that was just rewritten, on either side. The snapshot's name is new each build.
  const snapshot = `source-${buildId}-${Date.now()}.tar`;
  for (const f of readdirSync(DIR).filter((f) => f.startsWith("source-"))) rmSync(join(DIR, f));
  await $`tar --no-xattrs -cf ${join(DIR, snapshot)} n3ds/src n3ds/Makefile n3ds/icon.png .pocket-build/3ds/build/config.h`.cwd(ROOT);
  await runContainer(
    `mkdir -p /tmp/source /tmp/build && tar -xf /maneuver/.pocket-build/3ds/${snapshot} -C /tmp/source
cp /tmp/source/.pocket-build/3ds/build/config.h /tmp/build/config.h
make -f /tmp/source/n3ds/Makefile -j8 BUILD=/tmp/build SOURCE=/tmp/source/n3ds/src
cp /tmp/build/maneuver.elf /tmp/build/maneuver.map /maneuver/.pocket-build/3ds/build/`,
    [{ hostPath: ROOT, containerPath: "/maneuver" }],
    "/maneuver",
    {},
    "Pocket Maneuver build",
  );
  const bytes = readFileSync(ARTIFACT).length;
  if (bytes > 32 * 1024 * 1024) throw new Error(`the .3dsx is ${bytes} bytes; the wire installs at most 32 MiB`);
  mkdirSync(RECEIPTS, { recursive: true });
  const receipt = { target: "3ds", buildId, bytes, sha256: sha(ARTIFACT), packSha256: sha(PACK) };
  writeFileSync(join(RECEIPTS, "build.json"), JSON.stringify(receipt, null, 1) + "\n");
  console.log(`3ds: ${ARTIFACT} ${(bytes / 1e6).toFixed(1)} MB, build ${buildId}`);
  return receipt;
}

async function connect(): Promise<PocketRuntimeClient> {
  const keys = opt("--keys", join(POCKETJS, ".pocket/3ds/devices"));
  let devices = await discoverPocketRuntimes({ addresses: [host] });
  for (let attempt = 0; attempt < 3 && !devices.some((d) => d.address === host); attempt++) {
    await Bun.sleep(400);
    devices = await discoverPocketRuntimes({ addresses: [host] });
  }
  const device = devices.find((d) => d.address === host);
  if (!device) throw new Error(`no Pocket Runtime answers at ${host}:8131`);
  let token: Uint8Array | undefined;
  for (const name of existsSync(keys) ? readdirSync(keys).filter((n) => n.endsWith(".key")) : []) {
    const t = parsePocketRuntimeToken(readFileSync(join(keys, name), "utf8"));
    if (pocketRuntimeDeviceId(t) === device.deviceId) token = t;
  }
  if (!token) throw new Error(`no pairing key in ${keys} matches the console; pass --keys DIR`);
  const client = new PocketRuntimeClient({ host, port: device.port, token, timeoutMs: 20000, heartbeatTimeoutMs: 30000 });
  client.on("ctrl", (m: any) => {
    if (m.t === "log" || m.t === "runtime.native") console.log(JSON.stringify(m));
  });
  try {
    await client.connect();
    return client;
  } catch (e) {
    client.close();
    throw e;
  }
}

async function status(c: PocketRuntimeClient, text = ""): Promise<any> {
  const reply = c.waitForCtrl((m: any) => m.t === "maneuver.status", 8000);
  await c.sendCtrl({ t: "maneuver.control", ...(text ? { text } : {}) });
  return await reply;
}

/** Connects, retrying: right after another client leaves, the first connect can time out. */
async function session<T>(f: (c: PocketRuntimeClient) => Promise<T>): Promise<T> {
  let last: unknown;
  for (let attempt = 0; attempt < 4; attempt++) {
    let c: PocketRuntimeClient | undefined;
    try {
      c = await connect();
      return await f(c);
    } catch (e) {
      last = e;
      await Bun.sleep(1200);
    } finally {
      c?.close();
    }
  }
  throw last;
}

async function device<T>(f: () => Promise<T>): Promise<T> {
  return await withDeviceLease("3ds:wire", f);
}

switch (cmd) {
  case "build":
    await build();
    break;
  case "install": {
    const receipt = argv.includes("--no-build") ? JSON.parse(readFileSync(join(RECEIPTS, "build.json"), "utf8")) : await build();
    await $`bun ${join(POCKETJS, "tools/3ds-dev.ts")} install --host ${host} --file ${ARTIFACT} --name ${NAME}`.cwd(POCKETJS);
    await device(async () => {
      const end = Date.now() + 120_000;
      let last: any;
      while (Date.now() < end) {
        await Bun.sleep(1500);
        try {
          last = await session((c) => status(c));
          if (last.build === receipt.buildId && (last.stage === "running" || last.phase === "load-error")) break;
        } catch (e) {
          last = { error: String(e) };
        }
      }
      console.log(JSON.stringify(last, null, 1));
      if (last?.build !== receipt.buildId) throw new Error("the console did not come up with this build");
    });
    break;
  }
  case "status":
    await device(async () => console.log(JSON.stringify(await session((c) => status(c)), null, 1)));
    break;
  case "ctl":
    await device(async () => console.log(JSON.stringify(await session((c) => status(c, argv[1] ?? "")), null, 1)));
    break;
  case "capture":
    await device(async () => {
      const out = resolve(opt("--out", join(RECEIPTS, `capture-${Date.now()}.png`)));
      mkdirSync(resolve(out, ".."), { recursive: true });
      const shot = await session(async (c) => {
        const pending = c.waitForScreenshot();
        await c.sendCtrl({ t: "screenshot", surface: opt("--surface", "top") });
        return await pending;
      });
      await Bun.write(out, shot.png);
      console.log(out);
    });
    break;
  case "bench":
    await device(async () => {
      const seconds = Number(opt("--seconds", "60"));
      const extra = opt("--ctl", "");
      const build = JSON.parse(readFileSync(join(RECEIPTS, "build.json"), "utf8"));
      await session(async (c) => {
        const first = await status(c, `auto=1 reset=1 view=off ${extra}`);
        if (first.build !== build.buildId) throw new Error(`the console runs build ${first.build}, not ${build.buildId}`);
        await Bun.sleep(3000);
        const start = Date.now();
        const begin = await status(c);
        const samples: any[] = [];
        while (Date.now() - start < seconds * 1000) {
          // Each sample costs the console a late frame or so: answering takes it a few milliseconds.
          await Bun.sleep(5000);
          const s = await status(c);
          samples.push({ t: (Date.now() - start) / 1000, frameMs: s.frameMs, worstMs: s.worstMs, late: s.late, frames: s.frames, cpuMs: s.cpuMs, gpuMs: s.gpuMs, draws: s.draws, tris: s.tris, player: s.player });
        }
        const last = samples.at(-1)!;
        const frames = last.frames - begin.frames;
        const late = last.late - begin.late;
        const avg = samples.reduce((n, s) => n + s.frameMs, 0) / samples.length;
        const summary = {
          seconds,
          frames,
          lateFrames: late,
          lateShare: late / Math.max(frames, 1),
          averageFrameMs: avg,
          fps: (frames / (last.t * 1000 - 0)) * 1000,
          worstFrameMs: Math.max(...samples.map((s) => s.worstMs)),
          maxGpuMs: Math.max(...samples.map((s) => s.gpuMs)),
          maxTriangles: Math.max(...samples.map((s) => s.tris)),
          maxDraws: Math.max(...samples.map((s) => s.draws)),
          control: extra,
        };
        const dir = join(RECEIPTS, `bench-${new Date().toISOString().replace(/[:.]/g, "-")}`);
        mkdirSync(dir, { recursive: true });
        writeFileSync(join(dir, "device.json"), JSON.stringify({ summary, identity: { device: `3ds:${host}`, ...build, new3ds: begin.new3ds }, samples }, null, 1));
        console.log(join(dir, "device.json"));
        console.log(JSON.stringify(summary, null, 1));
      });
    });
    break;
  default:
    console.log("usage: bun tools/n3ds.ts <build|install|status|ctl|capture|bench> [--host ADDRESS]");
    process.exit(cmd ? 1 : 0);
}
