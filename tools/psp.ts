#!/usr/bin/env bun
// Pocket Maneuver on PSP: compile the interface (ui/) for the PSP, build the
// PRX with PocketJS's pinned rust-psp toolchain, stage both with the pack on a
// PSPLINK share, start it, steer and measure it.
//
//   bun tools/psp.ts build                    # ui/ + psp/ → dist/psp/{pocket-maneuver.prx,EBOOT.PBP,maneuver.js,maneuver.pak}
//   bun tools/psp.ts serve                    # start usbhostfs_pc (detached, logged) when none is running
//   bun tools/psp.ts run [--no-build]         # build, stage, reset PSPLINK, wait for it to reconnect, start the PRX
//   bun tools/psp.ts status
//   bun tools/psp.ts ctl "mode=play stats=1"  # host0:/maneuver/control.txt (see Game::control; `press=<mask>`,
//                                             # `rest=<turns>` and `ui=start|pause|resume|restart|title` reach the interface)
//   bun tools/psp.ts capture [--out f.png]    # PSPLINK screenshot
//   bun tools/psp.ts bench [--seconds 60]     # autopilot frame timings → .pocket-build/validation/psp/
//   bun tools/psp.ts package                  # dist/psp/PSP/GAME/PocketManeuver for a Memory Stick
//   bun tools/psp.ts emu [--frames 240] [--ctl "view=..."] [--out f.png] [--standalone] [--small] [--keep] [--no-interface] [--pack FILE]
//                                             # the same PRX in PPSSPPHeadless (software GE): a frame and its status;
//                                             # --small runs it with the 24 MB of a PSP-1000, where the world leaves
//                                             # no room for the interface; --keep leaves interface.json from the last run
//
// One usbhostfs_pc owns the PSP's cable. If one is running (in any checkout),
// these commands use its directory; `--share DIR` names another. Device
// commands take PocketJS's `psp:usb` lease; `--take` ends another holder first.

import { $ } from "bun";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { extractHostBuildInputs, hostBuildEnvironment } from "../vendor/pocketjs/framework/src/manifest/index.ts";
import { withDeviceLease } from "../vendor/pocketjs/tools/device-lease.ts";
import { POCKET3D_ICON } from "../vendor/pocketjs/tools/pocket3d-icon.ts";
import { encodePng } from "./png.ts";
import { compileInterface, type Interface } from "./ui.ts";

const ROOT = resolve(import.meta.dir, "..");
const OUT = resolve(ROOT, "dist/psp");
const argv = process.argv.slice(2);
const cmd = argv[0] ?? "";
const opt = (key: string, fallback: string) => {
  const at = argv.indexOf(key);
  return at < 0 ? fallback : (argv[at + 1] ?? fallback);
};
const PACK = resolve(opt("--pack", `${ROOT}/.pocket-build/world/walled-town.psp60.pack`));
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

/** The interface's bundle and its pak: the program reads them beside the pack. */
const UI_FILES = ["maneuver.js", "maneuver.pak"];

/** The interface compiled for the PSP; the last compiled one when `ui/` does not compile just now. */
async function interfaceBundle(): Promise<Interface> {
  try {
    return await compileInterface("psp");
  } catch (e) {
    const directory = `${ROOT}/.pocket-build/ui/psp`;
    if (!UI_FILES.every((f) => existsSync(`${directory}/${f}`)) || !existsSync(`${directory}/plan.json`)) throw e;
    console.warn(`psp: ui/ did not compile (${String(e).split("\n")[0]}); using the bundle already in ${directory}`);
    const plan = JSON.parse(readFileSync(`${directory}/plan.json`, "utf8"));
    return { directory, plan, inputs: extractHostBuildInputs(plan) };
  }
}

/** A PARAM.SFO: keys in order, 32-bit integers and NUL-terminated strings padded to four bytes. */
function paramSfo(values: Record<string, number | string>): Buffer {
  const keys = Object.keys(values).sort();
  const data = keys.map((key) => {
    const value = values[key]!;
    if (typeof value === "number") {
      const bytes = Buffer.alloc(4);
      bytes.writeUInt32LE(value);
      return { format: 0x0404, used: 4, bytes };
    }
    const text = Buffer.from(value + "\0");
    return { format: 0x0204, used: text.length, bytes: Buffer.concat([text, Buffer.alloc((4 - (text.length % 4)) % 4)]) };
  });
  const names = Buffer.from(keys.map((key) => key + "\0").join(""));
  const keyTable = 20 + keys.length * 16;
  const dataTable = keyTable + Math.ceil(names.length / 4) * 4;
  const out = Buffer.alloc(dataTable + data.reduce((sum, d) => sum + d.bytes.length, 0));
  out.write("\0PSF");
  out.writeUInt32LE(0x101, 4);
  out.writeUInt32LE(keyTable, 8);
  out.writeUInt32LE(dataTable, 12);
  out.writeUInt32LE(keys.length, 16);
  let nameAt = 0;
  let dataAt = 0;
  keys.forEach((key, i) => {
    const at = 20 + i * 16;
    const d = data[i]!;
    out.writeUInt16LE(nameAt, at);
    out.writeUInt16LE(d.format, at + 2);
    out.writeUInt32LE(d.used, at + 4);
    out.writeUInt32LE(d.bytes.length, at + 8);
    out.writeUInt32LE(dataAt, at + 12);
    d.bytes.copy(out, dataTable + dataAt);
    nameAt += key.length + 1;
    dataAt += d.bytes.length;
  });
  names.copy(out, keyTable);
  return out;
}

/**
 * Packs the PRX as an EBOOT. `large` asks for the 52 MB of a PSP-2000 or later (cargo-psp has no
 * setting for it): the world and the interface together need more than a PSP-1000's 24 MB, where
 * the program runs without the interface. ICON0.PNG is the Pocket3D icon from PocketJS; PIC1.PNG is
 * the game's own capture.
 */
async function pbp(out: string, prx: string, large: boolean) {
  const sfo = `${out}.SFO`;
  writeFileSync(sfo, paramSfo({ BOOTABLE: 1, CATEGORY: "MG", DISC_VERSION: "1.00", ...(large ? { MEMSIZE: 1 } : {}), PARENTAL_LEVEL: 1, PSP_SYSTEM_VER: "1.00", REGION: 0x8000, TITLE: "Pocket Maneuver" }));
  await $`pack-pbp ${out} ${sfo} ${POCKET3D_ICON.psp} NULL NULL ${ROOT}/psp/assets/pic1.png NULL ${prx} NULL`.quiet();
  rmSync(sfo, { force: true });
}

async function build() {
  const ui = await interfaceBundle();
  // Loaded by path at run time: PocketJS's toolchain module resolves its manifest through its own tsconfig.
  const toolchain: string = `${ROOT}/vendor/pocketjs/tools/psp-toolchain.ts`;
  const tc = (await import(toolchain)).resolvePspBuildToolchain();
  // The interface's runtime (PocketJS's PSP host library) builds QuickJS from C for the same target, with
  // PocketJS's own flags for it, and checks the target it was compiled for against the interface's plan.
  await $`${tc.rustup} run ${tc.manifest.rust.toolchain} cargo psp --release`.cwd(`${ROOT}/psp`).env({
    ...tc.environment,
    RUSTFLAGS: "-A linker-messages -A unexpected-cfgs -A unstable-name-collisions",
    CRATE_CC_NO_DEFAULTS: "1",
    TARGET_CC: "clang",
    TARGET_AR: `${tc.llvmBin}/llvm-ar`,
    TARGET_CFLAGS:
      `-target mipsel-sony-psp -mcpu=mips2 -msingle-float -mlittle-endian -mno-abicalls -fno-pic -G0 -mno-check-zero-division ` +
      `-fno-stack-protector -O2 -I${tc.sdk.path}/psp/include -I${tc.sdk.path}/psp/sdk/include`,
    AR_mipsel_sony_psp: `${tc.llvmBin}/llvm-ar`,
    RANLIB_mipsel_sony_psp: `${tc.llvmBin}/llvm-ranlib`,
    ...hostBuildEnvironment(ui.inputs, { outputDirectory: ui.directory, embedApp: false }),
    POCKETJS_OFFLOAD_SLOT: "",
    RUST_PSP_ABORT_ONLY: "1",
    RUST_PSP_TARGET: `${ROOT}/vendor/pocketjs/hosts/psp/targets/mipsel-sony-psp.json`,
  });
  const from = `${ROOT}/psp/target/mipsel-sony-psp/release`;
  mkdirSync(OUT, { recursive: true });
  cpSync(`${from}/pocket-maneuver-psp.prx`, `${OUT}/pocket-maneuver.prx`);
  await pbp(`${OUT}/EBOOT.PBP`, `${OUT}/pocket-maneuver.prx`, true);
  for (const f of UI_FILES) cpSync(`${ui.directory}/${f}`, `${OUT}/${f}`);
  console.log(`psp: ${OUT}/pocket-maneuver.prx ${(readFileSync(`${OUT}/pocket-maneuver.prx`).length / 1024).toFixed(0)} KiB, interface ${UI_FILES.map((f) => `${f} ${(readFileSync(`${OUT}/${f}`).length / 1024).toFixed(0)} KiB`).join(", ")}`);
}

/** Puts the interface's files in `dir` when they differ from the built ones. */
function stageInterface(dir: string) {
  for (const f of UI_FILES) {
    if (!existsSync(`${OUT}/${f}`)) throw new Error(`no ${OUT}/${f}: run \`bun tools/psp.ts build\` first`);
    if (!existsSync(`${dir}/${f}`) || sha(`${dir}/${f}`) !== sha(`${OUT}/${f}`)) cpSync(`${OUT}/${f}`, `${dir}/${f}`);
  }
}

function sha(path: string): string {
  return new Bun.CryptoHasher("sha256").update(readFileSync(path)).digest("hex");
}

function stage() {
  if (!existsSync(PACK)) throw new Error(`no pack at ${PACK}: run \`bun tools/maneuver.ts cook --profile psp60\` first`);
  mkdirSync(app, { recursive: true });
  if (!existsSync(`${app}/world.pack`) || sha(`${app}/world.pack`) !== sha(PACK)) cpSync(PACK, `${app}/world.pack`);
  cpSync(`${OUT}/pocket-maneuver.prx`, `${share}/pocket-maneuver.prx`);
  stageInterface(app);
  writeFileSync(`${app}/build.json`, JSON.stringify({ prxSha256: sha(`${OUT}/pocket-maneuver.prx`), packSha256: sha(PACK), interfaceSha256: Object.fromEntries(UI_FILES.map((f) => [f, sha(`${OUT}/${f}`)])) }, null, 1));
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
  // Play flown by the autopilot: the load of the route, with the interface showing what a player sees.
  ctl(`mode=play auto=1 reset=1 view=off ${extra}`);
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
    samples.push({ t: (Date.now() - start) / 1000, frameMs: s.frameMs, worstMs: s.worstMs, late: s.late, frames: s.frames, cpuMs: s.cpuMs, gpuMs: s.gpuMs, draws: s.draws, tris: s.tris, clip: s.clip, cells: s.cells, interface: s.interface, phaseMs: s.phaseMs, player: s.player });
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
    // The interface's files and what it kept go where the pack is.
    // `--keep` leaves what the interface kept in the last run (`interface.json`) for this one to read.
    for (const f of [...UI_FILES, ...(argv.includes("--keep") ? [] : ["interface.json"])]) for (const dir of [root, `${root}/maneuver`]) rmSync(`${dir}/${f}`, { force: true });
    if (!argv.includes("--no-interface")) stageInterface(standalone ? root : `${root}/maneuver`);
    // `--small`: without the request for large memory the emulator gives the program a PSP-1000's 24 MB.
    if (argv.includes("--small")) await pbp(`${root}/EBOOT.PBP`, `${OUT}/pocket-maneuver.prx`, false);
    else cpSync(`${OUT}/EBOOT.PBP`, `${root}/EBOOT.PBP`);
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
    stageInterface(dir);
    console.log(`psp: copy ${OUT}/PSP to the root of a Memory Stick`);
    break;
  }
  default:
    console.log("usage: bun tools/psp.ts <build|serve|run|status|ctl|capture|bench|emu|package> [--share DIR] [--take]");
    process.exit(cmd ? 1 : 0);
}
