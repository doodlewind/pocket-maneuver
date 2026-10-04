#!/usr/bin/env bun
// Pocket Maneuver on PSP: build the PRX with PocketJS's pinned rust-psp
// toolchain, stage it with the pack on a PSPLINK share, start it, steer and
// measure it.
//
//   bun tools/psp.ts build                    # psp/ → dist/psp/{pocket-maneuver.prx,EBOOT.PBP}
//   bun tools/psp.ts serve                    # start usbhostfs_pc (detached, logged) when none is running
//   bun tools/psp.ts run [--no-build]         # build, stage, reset PSPLINK, wait for it to reconnect, start the PRX
//   bun tools/psp.ts status
//   bun tools/psp.ts ctl "auto=1 stats=1"     # host0:/maneuver/control.txt (see Game::control)
//   bun tools/psp.ts capture [--out f.png]    # PSPLINK screenshot
//   bun tools/psp.ts bench [--seconds 60]     # autopilot frame timings → .pocket-build/validation/psp/
//   bun tools/psp.ts package                  # dist/psp/PSP/GAME/PocketManeuver for a Memory Stick
//   bun tools/psp.ts emu [--frames 240] [--ctl "view=..."] [--out f.png] [--standalone]
//                                             # the same PRX in PPSSPPHeadless (software GE): a frame and its status
//
// One usbhostfs_pc owns the PSP's cable. If one is running (in any checkout),
// these commands use its directory; `--share DIR` names another. Device
// commands take PocketJS's `psp:usb` lease; `--take` ends another holder first.

import { $ } from "bun";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { withDeviceLease } from "../vendor/pocketjs/tools/device-lease.ts";
import { encodePng } from "./png.ts";

const ROOT = resolve(import.meta.dir, "..");
const OUT = resolve(ROOT, "dist/psp");
const PACK = resolve(ROOT, ".pocket-build/world/walled-town.psp60.pack");
const argv = process.argv.slice(2);
const cmd = argv[0] ?? "";
const opt = (key: string, fallback: string) => {
  const at = argv.indexOf(key);
  return at < 0 ? fallback : (argv[at + 1] ?? fallback);
};
const port = opt("--port", "10000");

/** The directory the running usbhostfs_pc serves, if there is one. */
async function runningShare(): Promise<string | undefined> {
  const pid = (await $`pgrep -x usbhostfs_pc`.nothrow().quiet().text()).trim().split("\n")[0];
  if (!pid) return undefined;
  const args = (await $`ps -o args= -p ${pid}`.nothrow().quiet().text()).trim().split(/\s+/);
  return args.at(-1);
}
const share = resolve(opt("--share", process.env.MANEUVER_PSP_SHARE ?? (await runningShare()) ?? `${ROOT}/.pocket-build/psp/host0`));
const app = `${share}/maneuver`;

const HOST_LOG = `${ROOT}/.pocket-build/psp/usbhostfs.log`;

/** How many times the usbhostfs_pc started by `serve` has connected to the PSP; undefined without its log. */
function connections(): number | undefined {
  if (!existsSync(HOST_LOG)) return undefined;
  return (readFileSync(HOST_LOG, "utf8").match(/Connected to device/g) ?? []).length;
}

async function build() {
  // Loaded by path at run time: PocketJS's toolchain module resolves its manifest through its own tsconfig.
  const toolchain: string = `${ROOT}/vendor/pocketjs/tools/psp-toolchain.ts`;
  const tc = (await import(toolchain)).resolvePspBuildToolchain();
  await $`${tc.rustup} run ${tc.manifest.rust.toolchain} cargo psp --release`.cwd(`${ROOT}/psp`).env({
    ...tc.environment,
    RUST_PSP_ABORT_ONLY: "1",
    RUST_PSP_TARGET: `${ROOT}/vendor/pocketjs/hosts/psp/targets/mipsel-sony-psp.json`,
  });
  const from = `${ROOT}/psp/target/mipsel-sony-psp/release`;
  mkdirSync(OUT, { recursive: true });
  cpSync(`${from}/pocket-maneuver-psp.prx`, `${OUT}/pocket-maneuver.prx`);
  cpSync(`${from}/EBOOT.PBP`, `${OUT}/EBOOT.PBP`);
  console.log(`psp: ${OUT}/pocket-maneuver.prx ${(readFileSync(`${OUT}/pocket-maneuver.prx`).length / 1024).toFixed(0)} KiB`);
}

function sha(path: string): string {
  return new Bun.CryptoHasher("sha256").update(readFileSync(path)).digest("hex");
}

function stage() {
  if (!existsSync(PACK)) throw new Error(`no pack at ${PACK}: run \`bun tools/maneuver.ts cook --profile psp60\` first`);
  mkdirSync(app, { recursive: true });
  if (!existsSync(`${app}/world.pack`) || sha(`${app}/world.pack`) !== sha(PACK)) cpSync(PACK, `${app}/world.pack`);
  cpSync(`${OUT}/pocket-maneuver.prx`, `${share}/pocket-maneuver.prx`);
  writeFileSync(`${app}/build.json`, JSON.stringify({ prxSha256: sha(`${OUT}/pocket-maneuver.prx`), packSha256: sha(PACK) }, null, 1));
}

async function pspsh(text: string): Promise<string> {
  const p = Bun.spawn(["pspsh", "-p", port, "-e", text], { stdout: "pipe", stderr: "pipe" });
  const timer = setTimeout(() => p.kill(), 15000);
  const [out, err] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text(), p.exited]);
  clearTimeout(timer);
  if (/Error|Could not|failed|connect:/i.test(out + err)) throw new Error(`pspsh ${text}: ${(out + err).trim()}`);
  return out + err;
}

function readStatus(): any {
  // The device rewrites the file in place; a read can land in the middle.
  for (let i = 0; ; i++) {
    try {
      return JSON.parse(readFileSync(`${app}/status.json`, "utf8"));
    } catch (e) {
      if (i >= 40) throw new Error(`no status at ${app}/status.json: is the game running?`);
      Bun.sleepSync(25);
    }
  }
}

async function waitFor(test: (s: any) => boolean, seconds: number): Promise<any> {
  const end = Date.now() + seconds * 1000;
  let last: any;
  while (Date.now() < end) {
    try {
      last = readStatus();
      if (last.stage === "failed") throw new Error(`the device reports: ${last.error}`);
      if (test(last)) return last;
    } catch (e) {
      if (String(e).includes("the device reports")) throw e;
    }
    await Bun.sleep(300);
  }
  throw new Error(`the PSP did not get there in ${seconds} s (last status: ${JSON.stringify(last)})`);
}

function ctl(text: string) {
  mkdirSync(app, { recursive: true });
  // The nonce makes two equal commands differ.
  writeFileSync(`${app}/control.txt`, `${text} nonce=${Date.now()}\n`);
}

async function capture(out: string) {
  const name = `capture-${Date.now()}.bmp`;
  await pspsh(`scrshot host0:/${name}`);
  const path = `${share}/${name}`;
  for (let i = 0; i < 40 && !existsSync(path); i++) await Bun.sleep(100);
  const bmp = readFileSync(path);
  rmSync(path, { force: true });
  // BITMAPINFOHEADER, 24 or 32 bits, rows bottom-up unless the height is negative.
  const view = new DataView(bmp.buffer, bmp.byteOffset, bmp.byteLength);
  const at = view.getUint32(10, true);
  const w = view.getInt32(18, true);
  const hRaw = view.getInt32(22, true);
  const h = Math.abs(hRaw);
  const bpp = view.getUint16(28, true) / 8;
  const stride = (w * bpp + 3) & ~3;
  const rgba = new Uint8Array(w * h * 4);
  for (let y = 0; y < h; y++) {
    const row = at + (hRaw > 0 ? h - 1 - y : y) * stride;
    for (let x = 0; x < w; x++) {
      const s = row + x * bpp;
      rgba.set([bmp[s + 2]!, bmp[s + 1]!, bmp[s]!, 255], (y * w + x) * 4);
    }
  }
  mkdirSync(resolve(out, ".."), { recursive: true });
  writeFileSync(out, encodePng(rgba, w, h));
  console.log(out);
}

async function bench(seconds: number, extra: string) {
  ctl(`auto=1 reset=1 view=off ${extra}`);
  await Bun.sleep(3000);
  const first = readStatus();
  const build = JSON.parse(readFileSync(`${app}/build.json`, "utf8"));
  const samples: any[] = [];
  const start = Date.now();
  let prev = first;
  while (Date.now() - start < seconds * 1000) {
    await Bun.sleep(1000);
    const s = readStatus();
    if (s.frames === prev.frames) continue;
    samples.push({ t: (Date.now() - start) / 1000, frameMs: s.frameMs, worstMs: s.worstMs, late: s.late, frames: s.frames, cpuMs: s.cpuMs, gpuMs: s.gpuMs, draws: s.draws, tris: s.tris, clip: s.clip, cells: s.cells, player: s.player });
    prev = s;
  }
  if (samples.length < 2) throw new Error("no samples: is the game on screen?");
  const last = samples.at(-1)!;
  const frames = last.frames - first.frames;
  const late = last.late - first.late;
  const avg = samples.reduce((n, s) => n + s.frameMs, 0) / samples.length;
  const summary = {
    seconds,
    frames,
    lateFrames: late,
    lateShare: late / Math.max(frames, 1),
    averageFrameMs: avg,
    fps: 1000 / avg,
    worstFrameMs: Math.max(...samples.map((s) => s.worstMs)),
    maxTriangles: Math.max(...samples.map((s) => s.tris)),
    maxDraws: Math.max(...samples.map((s) => s.draws)),
    control: extra,
  };
  const dir = resolve(ROOT, `.pocket-build/validation/psp/bench-${new Date().toISOString().replace(/[:.]/g, "-")}`);
  mkdirSync(dir, { recursive: true });
  writeFileSync(`${dir}/device.json`, JSON.stringify({ summary, identity: { device: "psp:usb", ...build }, memory: last && readStatus().memory, samples }, null, 1));
  console.log(`${dir}/device.json`);
  console.log(JSON.stringify(summary, null, 1));
}

/** Runs `f` holding the PSP's lease; `--take` ends another holder first. */
async function device<T>(f: () => Promise<T>): Promise<T> {
  for (let attempt = 0; ; attempt++) {
    try {
      return await withDeviceLease("psp:usb", f);
    } catch (e) {
      const m = /owner pid=(\d+)/.exec(String(e));
      if (!m || !argv.includes("--take") || attempt > 0) throw e;
      console.log(`psp: ending the holder of psp:usb (pid ${m[1]})`);
      process.kill(Number(m[1]), "SIGTERM");
      await Bun.sleep(800);
    }
  }
}

switch (cmd) {
  case "build":
    await build();
    break;
  case "run":
    if (!argv.includes("--no-build")) await build();
    await device(async () => {
      stage();
      rmSync(`${app}/status.json`, { force: true });
      // A reset restarts PSPLINK from the Memory Stick and drops the cable for several seconds. A command sent
      // before the cable is back leaves PSPLINK half reset (the shell answers, storage does not) until someone
      // restarts it on the console. So: reset, then nothing until usbhostfs_pc logs a new connection.
      // PSPLINK lists its own modules last: with nothing after USBHostFS, no program is loaded and no reset is needed.
      const names = (await pspsh("modlist").catch(() => "")).split("\n").filter((l) => l.includes("Name:")).map((l) => l.split("Name:")[1]!.trim());
      const idle = names.length > 0 && (names.at(-1) === "USBHostFS" || names.at(-1) === "PSPLINK");
      const before = connections();
      if (idle) {
        console.log("psp: PSPLINK is idle; loading without a reset");
      } else {
        console.log(`psp: resetting PSPLINK (${names.at(-1) ?? "module list unreadable"})`);
        await pspsh("reset").catch(() => "");
      }
      if (idle) {
        // Nothing to wait for.
      } else if (before === undefined) {
        console.log("psp: no usbhostfs_pc log (start the host with `bun tools/psp.ts serve`); waiting 15 s for PSPLINK");
        await Bun.sleep(15000);
      } else {
        const end = Date.now() + 40000;
        while ((connections() ?? 0) <= before) {
          if (Date.now() > end) throw new Error("PSPLINK did not reconnect after the reset: restart PSPLINK on the console");
          await Bun.sleep(250);
        }
        await Bun.sleep(1000);
      }
      if (!/world\.pack/.test(await pspsh("ls host0:/maneuver"))) throw new Error("PSPLINK does not serve host0: restart PSPLINK on the console");
      await pspsh("ldstart host0:/pocket-maneuver.prx");
      const s = await waitFor((s) => s.stage === "running", 180);
      console.log(JSON.stringify(s, null, 1));
    });
    break;
  case "serve": {
    // One usbhostfs_pc owns the cable. This one outlives the command and logs where `run` can count its connections.
    if ((await $`pgrep -x usbhostfs_pc`.nothrow().quiet()).exitCode === 0) throw new Error("a usbhostfs_pc is already running; stop it first, or pass its directory with --share");
    mkdirSync(share, { recursive: true });
    mkdirSync(resolve(HOST_LOG, ".."), { recursive: true });
    const { spawn } = await import("node:child_process");
    const { openSync } = await import("node:fs");
    const log = openSync(HOST_LOG, "w");
    spawn("usbhostfs_pc", ["-b", port, share], { cwd: share, detached: true, stdio: ["ignore", log, log] }).unref();
    console.log(`psp: usbhostfs_pc serves ${share} (log ${HOST_LOG})`);
    break;
  }
  case "status":
    console.log(JSON.stringify(readStatus(), null, 1));
    break;
  case "ctl":
    ctl(argv[1] ?? "");
    break;
  case "capture":
    await device(() => capture(resolve(opt("--out", `${ROOT}/.pocket-build/validation/psp/capture-${Date.now()}.png`))));
    break;
  case "bench":
    await device(() => bench(Number(opt("--seconds", "60")), opt("--ctl", "")));
    break;
  case "emu": {
    // PPSSPP mounts `--root` as host0:. Its software renderer follows the GE's clipping rules.
    const headless = process.env.PPSSPP_HEADLESS ?? `${process.env.HOME}/ppsspp-src/build/PPSSPPHeadless`;
    if (!existsSync(headless)) throw new Error(`no PPSSPPHeadless at ${headless} (set PPSSPP_HEADLESS)`);
    const root = `${ROOT}/.pocket-build/psp/emu`;
    mkdirSync(`${root}/maneuver`, { recursive: true });
    // `--standalone` puts the pack beside the EBOOT, as on a Memory Stick, instead of on the share.
    const standalone = argv.includes("--standalone");
    const packAt = standalone ? `${root}/world.pack` : `${root}/maneuver/world.pack`;
    rmSync(standalone ? `${root}/maneuver/world.pack` : `${root}/world.pack`, { force: true });
    if (!existsSync(packAt) || sha(packAt) !== sha(PACK)) cpSync(PACK, packAt);
    cpSync(`${OUT}/EBOOT.PBP`, `${root}/EBOOT.PBP`);
    const frames = Number(opt("--frames", "240"));
    for (const f of ["status.json", "shot.raw"]) rmSync(`${root}/maneuver/${f}`, { force: true });
    writeFileSync(`${root}/maneuver/boot.txt`, `stats=1 ${opt("--ctl", "")} shot=${frames} exit=${frames + 3}\n`);
    const run = await $`${headless} --root ${root} --graphics=${opt("--graphics", "software")} --timeout=${opt("--timeout", "240")} ${root}/EBOOT.PBP`.nothrow().quiet();
    const log = (run.stdout.toString() + run.stderr.toString()).trim();
    if (log) console.log(log.split("\n").slice(-12).join("\n"));
    if (existsSync(`${root}/maneuver/status.json`)) console.log(readFileSync(`${root}/maneuver/status.json`, "utf8"));
    if (!existsSync(`${root}/maneuver/shot.raw`)) throw new Error("the emulator run wrote no frame");
    const raw = readFileSync(`${root}/maneuver/shot.raw`);
    const rgba = new Uint8Array(480 * 272 * 4);
    // The frame buffer is 16-bit: red in the low five bits, then six of green, five of blue.
    for (let i = 0; i < 480 * 272; i++) {
        const p = raw[i * 2]! | (raw[i * 2 + 1]! << 8);
        rgba.set([((p & 31) * 255) / 31, (((p >> 5) & 63) * 255) / 63, ((p >> 11) * 255) / 31, 255], i * 4);
    }
    const out = resolve(opt("--out", `${ROOT}/.pocket-build/psp/emu/frame.png`));
    writeFileSync(out, encodePng(rgba, 480, 272));
    console.log(out);
    break;
  }
  case "package": {
    const dir = `${OUT}/PSP/GAME/PocketManeuver`;
    mkdirSync(dir, { recursive: true });
    cpSync(`${OUT}/EBOOT.PBP`, `${dir}/EBOOT.PBP`);
    cpSync(PACK, `${dir}/world.pack`);
    console.log(`psp: copy ${OUT}/PSP to the root of a Memory Stick`);
    break;
  }
  default:
    console.log("usage: bun tools/psp.ts <build|serve|run|status|ctl|capture|bench|emu|package> [--share DIR] [--take]");
    process.exit(cmd ? 1 : 0);
}
