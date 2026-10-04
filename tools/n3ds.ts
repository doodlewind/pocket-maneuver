#!/usr/bin/env bun
// Pocket Maneuver on Nintendo 3DS: build the Rust core, the interface (ui/) and
// the C host into a .3dsx (devkitARM in PocketJS's pinned container), install
// and start it over PocketJS's paired LAN wire, steer and measure it.
//
//   bun tools/n3ds.ts build [--no-ui]           # dist/3ds/pocket-maneuver.3dsx, the pack and the interface in its ROMFS
//                                               # (--no-ui keeps the interface as last compiled)
//   bun tools/n3ds.ts install [--no-build]      # build, send, start, wait for the game to report
//   bun tools/n3ds.ts status
//   bun tools/n3ds.ts ctl "mode=play stats=1"   # Game::control words, and the host's own:
//                                               #   press=MASK (PocketJS BTN bits on the interface)
//                                               #   touch=X,Y | touch=off (a stylus on the lower screen)
//                                               #   ui=start|pause|resume|restart|title
//   bun tools/n3ds.ts capture [--out f.png] [--surface top|auxiliary]
//   bun tools/n3ds.ts bench [--seconds 60]      # play flown by the autopilot: frame timings → .pocket-build/validation/3ds/
//   bun tools/n3ds.ts emu [--ctl "…"] [--seconds 6] [--out f.png] [--keep]
//                                               # the built .3dsx in Azahar (a window opens), with its own SD card and
//                                               # pairing key under .pocket-build/3ds/azahar: status and both screens
//
// `--host ADDRESS` (default 192.168.8.159, or POCKET_3DS_HOST). The console
// must run a Pocket Runtime build with the paired wire: this program itself
// once installed, or another Pocket Nexus .3dsx. Installs are .3dsx only.
// `--host 127.0.0.1` is the emulator `emu --keep` left running: status, ctl
// and capture reach it with its own key and do not claim the console.

import { $ } from "bun";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { pocketRuntimeDeviceId } from "../vendor/pocketjs/contracts/spec/pocket-runtime-wire.ts";
import { extractHostBuildInputs } from "../vendor/pocketjs/framework/src/manifest/index.ts";
import { withDeviceLease } from "../vendor/pocketjs/tools/device-lease.ts";
import { ensureQuickJs, runContainer, THREE_DS_CONTAINER_IMAGE } from "../vendor/pocketjs/tools/3ds-toolchain.ts";
import { compileInterface } from "./ui.ts";

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
const host = cmd === "emu" ? "127.0.0.1" : opt("--host", process.env.POCKET_3DS_HOST ?? "192.168.8.159");
/** Azahar's home for `emu`: its settings, its SD card and the pairing key this tool wrote there. */
const EMULATOR = join(DIR, "azahar");
const emulated = host === "127.0.0.1";

// Loaded by path at run time: PocketJS's client module is checked by PocketJS's own compiler settings.
const clientModule: string = join(POCKETJS, "tools/3ds-runtime-client.ts");
const { discoverPocketRuntimes, parsePocketRuntimeToken, PocketRuntimeClient } = await import(clientModule);
type Client = any;

const sha = (path: string) => new Bun.CryptoHasher("sha256").update(readFileSync(path)).digest("hex");

/** PocketJS's UI core for the 3DS: a Rust static library, built on the host with the compiler its crate pins. */
async function interfaceCore(): Promise<string> {
  const { RUSTUP_TOOLCHAIN: _toolchain, RUSTC: _rustc, ...environment } = process.env;
  await $`cargo build --release --locked`.cwd(join(POCKETJS, "hosts/3ds/core")).env({ ...environment, CARGO_TARGET_DIR: join(DIR, "ui-core") });
  return join(DIR, "ui-core/armv6k-nintendo-3ds/release/libpocketjs_3ds_core.a");
}

async function build() {
  if (!existsSync(PACK)) throw new Error(`no pack at ${PACK}: run \`bun tools/maneuver.ts cook --profile n3ds60\` first`);
  // The interface: its bundle and pak for this device, and the plan they were resolved against.
  const uiDirectory = join(ROOT, ".pocket-build/ui/3ds");
  const ui = argv.includes("--no-ui") ? extractHostBuildInputs(JSON.parse(readFileSync(join(uiDirectory, "plan.json"), "utf8"))) : (await compileInterface("3ds")).inputs;
  const aux = ui.surfaces?.auxiliary.logical;
  if (ui.viewport.logical.join("x") !== "400x240" || aux?.join("x") !== "320x240") throw new Error("the interface's 3DS presentation is not 400x240 over 320x240");
  await $`rustup run nightly-2026-07-02 cargo build --release`.cwd(join(ROOT, "n3ds/core"));
  const core = "n3ds/core/target/armv6k-nintendo-3ds/release/libmaneuver_n3ds_core.a";
  const uiCore = await interfaceCore();
  const mounts = [{ hostPath: ROOT, containerPath: "/maneuver" }];
  await ensureQuickJs(join(DIR, "quickjs"), THREE_DS_CONTAINER_IMAGE, mounts);
  const romfs = join(DIR, "romfs");
  mkdirSync(romfs, { recursive: true });
  mkdirSync(join(DIR, "build"), { recursive: true });
  const packSha = sha(PACK);
  if (!existsSync(join(romfs, "world.pack")) || sha(join(romfs, "world.pack")) !== packSha) cpSync(PACK, join(romfs, "world.pack"));
  // The build's identity: the sources, both Rust libraries, the interface and the pack.
  const id = new Bun.CryptoHasher("sha256");
  for (const f of readdirSync(join(ROOT, "n3ds/src")).sort()) id.update(readFileSync(join(ROOT, "n3ds/src", f)));
  for (const f of [join(ROOT, core), uiCore, join(uiDirectory, "maneuver.js"), join(uiDirectory, "maneuver.pak")]) id.update(readFileSync(f));
  id.update(packSha);
  const buildId = id.digest("hex").slice(0, 12);
  // The guest refuses a bundle resolved for another target or host ABI: both come from the interface's plan.
  const header = `#define POCKETJS_HOST_ABI ${ui.hostAbi}\n#define POCKETJS_TARGET_ID "${ui.target}"\n#define MANEUVER_BUILD_ID "${buildId}"\n`;
  const config = join(DIR, "build/config.h");
  if (!existsSync(config) || readFileSync(config, "utf8") !== header) writeFileSync(config, header);
  // Compile a snapshot on the container's own filesystem: the shared mount can show a stale size for a file
  // that was just rewritten, on either side. The snapshot's name is new each build. It holds everything this
  // build wrote: the sources, both Rust libraries and the interface. The pack is copied through the mount
  // and checked against its hash; PocketJS's sources and QuickJS are read through the mount as they are.
  const snapshot = `source-${buildId}-${Date.now()}.tar`;
  for (const f of readdirSync(DIR).filter((f) => f.startsWith("source-"))) rmSync(join(DIR, f));
  const uiCoreInTar = ".pocket-build/3ds/ui-core/armv6k-nintendo-3ds/release/libpocketjs_3ds_core.a";
  await $`tar --no-xattrs -cf ${join(DIR, snapshot)} n3ds/src n3ds/Makefile n3ds/icon.png .pocket-build/3ds/build/config.h ${core} ${uiCoreInTar} .pocket-build/ui/3ds/maneuver.js .pocket-build/ui/3ds/maneuver.pak`.cwd(ROOT);
  await runContainer(
    `mkdir -p /tmp/source /tmp/build /tmp/romfs && tar -xf /maneuver/.pocket-build/3ds/${snapshot} -C /tmp/source
cp /tmp/source/.pocket-build/3ds/build/config.h /tmp/build/config.h
cp /maneuver/.pocket-build/3ds/romfs/world.pack /tmp/source/.pocket-build/ui/3ds/maneuver.js /tmp/source/.pocket-build/ui/3ds/maneuver.pak /tmp/romfs/
echo "${packSha}  /tmp/romfs/world.pack" | sha256sum -c -
make -f /tmp/source/n3ds/Makefile -j8 BUILD=/tmp/build SOURCE=/tmp/source/n3ds/src ROMFS=/tmp/romfs CORE=/tmp/source/${core} UI_CORE=/tmp/source/${uiCoreInTar}
cp /tmp/build/maneuver.elf /tmp/build/maneuver.map /maneuver/.pocket-build/3ds/build/`,
    mounts,
    "/maneuver",
    {},
    "Pocket Maneuver build",
  );
  const bytes = readFileSync(ARTIFACT).length;
  if (bytes > 32 * 1024 * 1024) throw new Error(`the .3dsx is ${bytes} bytes; the wire installs at most 32 MiB`);
  mkdirSync(RECEIPTS, { recursive: true });
  const receipt = {
    target: "3ds", buildId, bytes, sha256: sha(ARTIFACT), packSha256: packSha,
    interface: { target: ui.target, hostAbi: ui.hostAbi, js: sha(join(uiDirectory, "maneuver.js")), pak: sha(join(uiDirectory, "maneuver.pak")) },
  };
  writeFileSync(join(RECEIPTS, "build.json"), JSON.stringify(receipt, null, 1) + "\n");
  console.log(`3ds: ${ARTIFACT} ${(bytes / 1e6).toFixed(1)} MB, build ${buildId}`);
  return receipt;
}

async function connect(): Promise<Client> {
  const keys = opt("--keys", emulated ? join(EMULATOR, "keys") : join(POCKETJS, ".pocket/3ds/devices"));
  let devices: any[] = await discoverPocketRuntimes({ addresses: [host] });
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

async function status(c: Client, text = ""): Promise<any> {
  const reply = c.waitForCtrl((m: any) => m.t === "maneuver.status", 8000);
  await c.sendCtrl({ t: "maneuver.control", ...(text ? { text } : {}) });
  return await reply;
}

/** Connects, retrying: right after another client leaves, the first connect can time out. */
async function session<T>(f: (c: Client) => Promise<T>): Promise<T> {
  let last: unknown;
  for (let attempt = 0; attempt < 4; attempt++) {
    let c: Client | undefined;
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
  return await withDeviceLease(emulated ? "3ds:azahar" : "3ds:wire", f);
}

/**
 * Starts the built .3dsx in Azahar. The emulator has no headless mode and takes its whole user
 * directory from $HOME, so it runs with a home of its own: the user's settings and system files
 * copied, an empty SD card, and a pairing key so the dev wire listens on 127.0.0.1:8131.
 */
async function emulate() {
  const app = process.env.AZAHAR || "/Applications/Azahar.app";
  const binary = join(app, "Contents/MacOS/azahar");
  const source = join(homedir(), "Library/Application Support/Azahar");
  if (!existsSync(binary) || !existsSync(join(source, "config/qt-config.ini"))) throw new Error(`Azahar and its settings are needed (${app})`);
  if (!existsSync(ARTIFACT)) throw new Error("build first: bun tools/n3ds.ts build");
  const user = join(EMULATOR, "home/Library/Application Support/Azahar");
  // Only the emulator this tool started: its command line names this checkout's artifact.
  const stop = () => Bun.spawnSync(["pkill", "-9", "-f", `${binary} ${ARTIFACT}`]);
  stop();
  rmSync(EMULATOR, { recursive: true, force: true });
  mkdirSync(join(user, "config"), { recursive: true });
  mkdirSync(join(user, "sdmc/pocketjs/runtime"), { recursive: true });
  mkdirSync(join(EMULATOR, "keys"), { recursive: true });
  for (const directory of ["nand", "sysdata"]) if (existsSync(join(source, directory))) cpSync(join(source, directory), join(user, directory), { recursive: true });
  const settings = readFileSync(join(source, "config/qt-config.ini"), "utf8").replace(/^check_for_update_on_start=.*$/m, "check_for_update_on_start=false");
  writeFileSync(join(user, "config/qt-config.ini"), settings);
  const token = Buffer.from(Uint8Array.from({ length: 32 }, (_, i) => i * 7 + 3)).toString("hex");
  writeFileSync(join(user, "sdmc/pocketjs/runtime/dev.key"), token + "\n");
  writeFileSync(join(EMULATOR, "keys/azahar.key"), token + "\n");
  const log = join(EMULATOR, "console.log");
  await $`open -n -a ${app} --env HOME=${join(EMULATOR, "home")} --stdout ${log} --stderr ${log} --args ${ARTIFACT}`;
  try {
    const end = Date.now() + Number(opt("--timeout", "90")) * 1000;
    let last: any;
    while (Date.now() < end) {
      await Bun.sleep(1500);
      try {
        last = await session((c) => status(c));
        if (last.stage === "running" || last.phase === "load-error") break;
      } catch (e) {
        last = { error: String(e) };
      }
    }
    if (last?.stage !== "running") throw new Error(`the emulator did not reach the game: ${JSON.stringify(last)}`);
    const words = opt("--ctl", "");
    if (words) await session((c) => status(c, words));
    await Bun.sleep(Number(opt("--seconds", "6")) * 1000);
    const out = resolve(opt("--out", join(RECEIPTS, `emu-${Date.now()}.png`)));
    mkdirSync(resolve(out, ".."), { recursive: true });
    const [report, shot] = await session(async (c) => {
      const report = await status(c);
      const pending = c.waitForScreenshot();
      await c.sendCtrl({ t: "screenshot" });
      return [report, await pending];
    });
    await Bun.write(out, shot.png);
    console.log(JSON.stringify(report, null, 1));
    console.log(out);
  } finally {
    if (!argv.includes("--keep")) stop();
  }
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
        // Play flown by the autopilot: the interface shows what it shows a player.
        const first = await status(c, `mode=play auto=1 reset=1 view=off ${extra}`);
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
  case "emu":
    await emulate();
    break;
  default:
    console.log("usage: bun tools/n3ds.ts <build|install|status|ctl|capture|bench|emu> [--host ADDRESS]");
    process.exit(cmd ? 1 : 0);
}
