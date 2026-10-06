// Pocket Maneuver on PS Vita: build the native runtime, replace the running
// development container's binary over PocketJS's wired debug transport
// (vendor/pocketjs), sync the pack and the interface, steer and measure.
//
//   bun tools/maneuver.ts build  [--title P3B1D7273] [--debug]
//   bun tools/maneuver.ts sync                       # pack and compiled interface → host0:maneuver/
//   bun tools/maneuver.ts native                     # sync + build + USB SELF replacement
//   bun tools/maneuver.ts serve                      # USB host (keep running)
//   bun tools/maneuver.ts status | capture [--out f.png]
//   bun tools/maneuver.ts ctl '{"ui":"start"}'       # host0:maneuver/control.json
//   bun tools/maneuver.ts bench [--seconds 60]       # autopilot frame timings → device.json
//   bun tools/maneuver.ts vpk                        # standalone PKMV00001 package: pack, interface and programs inside
//
// Control messages: `"ui": "start" | "pause" | "resume" | "restart" | "title"`
// asks as the interface would; `"press": 8` (a mask of pad bits) or
// `"press": ["down", "circle"]` presses buttons on the interface;
// `{"mode": "play", "auto": true}` is play flown by the autopilot; the
// renderer's switches (`hud stats sound world actors cullCw profile`,
// `lodNear lodMid repeat`, `post`, `view`, `reset`) are as before.
//   bun tools/maneuver.ts programs                   # the console compiles every program from this checkout: the set a package carries
//   bun tools/maneuver.ts push-vpk [file.vpk]        # → ux0:data/pocket-maneuver/ via the development build
//   bun tools/maneuver.ts hold [--take|--release]    # keep the console for this repository across commands
//
// `--share DIR` uses an already-running USB host's root directory instead of
// this repository's `.pocket-build/vita-usb/share`.
//
// The default title is Pocket Devkit (PocketJS apps/devkit), the development
// container installed on the console: its native slots accept replacement
// SELFs, and reopening its LiveArea bubble returns to it.

import { $ } from "bun";
import { createHash, randomBytes } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { POCKET3D_ICON } from "../vendor/pocketjs/tools/pocket3d-icon.ts";
import { packageVitaVpk } from "../vendor/pocketjs/tools/vita-package.ts";
import { prepareVitaUsb } from "../vendor/pocketjs/tools/vita-usb.ts";
import { compileInterface } from "./ui.ts";

const ROOT = resolve(import.meta.dir, "..");
const POCKETJS = resolve(ROOT, "vendor/pocketjs");
const APP_DIR = resolve(ROOT, "vita");
const OUT_DIR = resolve(ROOT, "dist/vita");
const home = process.env.HOME ?? "";
const vitasdk = process.env.VITASDK || `${home}/vitasdk`;
const rustup = Bun.which("rustup") ?? `${home}/.cargo/bin/rustup`;
const BIN = "pocket-maneuver-vita";

/** The Devkit installed on the development console: vitaTitleId("dev.pocket-stack.devkit"). Builds of PocketJS after the pocket-nexus rename install as P25BFE5E2. */
const DEVKIT = "P3B1D7273";
export const PACK = resolve(ROOT, ".pocket-build/world/walled-town.vita60.pack");
/** The compiled interface (`bun tools/ui.ts vita`), which the device reads beside the pack. */
const INTERFACE = ["maneuver.js", "maneuver.pak"];

export interface VitaOptions {
  argv: string[];
}

function value(argv: string[], flag: string, fallback: string): string {
  const i = argv.indexOf(flag);
  return i >= 0 && argv[i + 1] ? argv[i + 1]! : fallback;
}

export function context(argv: string[]) {
  const standalone = argv.includes("--standalone");
  const title = value(argv, "--title", standalone ? "PKMV00001" : DEVKIT);
  const share = resolve(value(argv, "--share", process.env.MANEUVER_SHARE ?? resolve(ROOT, ".pocket-build/vita-usb/share")));
  return { title, share, appShare: resolve(share, "maneuver"), output: `pocket-maneuver-${title}`, release: !argv.includes("--debug"), standalone };
}

/**
 * The id a build carries. A development build takes a fresh one, which the dev host tells two builds apart by.
 * `tools/release.ts` names a release build's instead (POCKET_RELEASE_BUILD: 32 hex digits from the commit and
 * the hashes of what the package is built from), so two builds of one commit are the same bytes.
 */
function buildId(): string {
  const named = process.env.POCKET_RELEASE_BUILD;
  if (named === undefined) return randomBytes(16).toString("hex");
  if (!/^[0-9a-f]{32}$/.test(named)) throw new Error("POCKET_RELEASE_BUILD is not 32 hex digits");
  return named;
}

export async function build(argv: string[], assets?: string): Promise<string> {
  const c = context(argv);
  if (!existsSync(`${vitasdk}/bin/vita-pack-vpk`)) throw new Error(`VitaSDK not found at ${vitasdk}`);
  const usb = c.standalone ? undefined : await prepareVitaUsb();
  const nativeBuild = buildId();
  const env = {
    ...process.env,
    PATH: `${vitasdk}/bin:${home}/.cargo/bin:${process.env.PATH ?? ""}`,
    VITASDK: vitasdk,
    VITA_DEFAULT_TITLE_ID: c.title,
    POCKETJS_VITA_TITLE_ID: c.title,
    POCKETJS_NATIVE_BUILD: nativeBuild,
    POCKETJS_EMBED_APP: "0",
    TARGET_AR: "arm-vita-eabi-ar",
    AR_armv7_sony_vita_newlibeabihf: "arm-vita-eabi-ar",
    TARGET_CC: "arm-vita-eabi-gcc",
    CC_armv7_sony_vita_newlibeabihf: "arm-vita-eabi-gcc",
    TARGET_CXX: "arm-vita-eabi-g++",
    CXX_armv7_sony_vita_newlibeabihf: "arm-vita-eabi-g++",
  };
  const cargoArgs = [...(c.release ? ["--release"] : []), ...(c.standalone ? ["--no-default-features"] : [])];
  console.log(`maneuver: cargo vita build vpk (title ${c.title}, ${c.release ? "release" : "debug"})`);
  await $`${rustup} run nightly-2026-05-28 cargo vita build vpk -- ${cargoArgs}`.cwd(APP_DIR).env(env);

  const target = `${APP_DIR}/target/armv7-sony-vita-newlibeabihf/${c.release ? "release" : "debug"}`;
  const eboot = `${target}/${BIN}.self`;
  const sfo = `${target}/${BIN}.sfo`;
  const vpk = `${target}/${BIN}.vpk`;
  // Unsafe-homebrew SELF: loading the USB driver and writing the inactive native slot need the standard homebrew permissions.
  await $`${vitasdk}/bin/vita-make-fself ${target}/${BIN}.velf ${eboot}`;
  await $`${vitasdk}/bin/vita-mksfoex -d ATTRIBUTE2=12 -s TITLE_ID=${c.title} ${"Pocket Maneuver"} ${sfo}`;
  // The bubble's icon is the Pocket3D icon from PocketJS; the asset tree holds the LiveArea pictures, which are the game's own.
  await packageVitaVpk({ tool: `${vitasdk}/bin/vita-pack-vpk`, sfo, eboot, output: vpk, usbDriver: usb?.driver, applicationAssets: assets ?? `${APP_DIR}/assets`, icon: POCKET3D_ICON.vita });

  mkdirSync(OUT_DIR, { recursive: true });
  cpSync(vpk, `${OUT_DIR}/${c.output}.vpk`);
  cpSync(eboot, `${OUT_DIR}/${c.output}.self`);
  const selfSha256 = createHash("sha256").update(readFileSync(eboot)).digest("hex");
  const runtime = `${OUT_DIR}/${c.output}.runtime.json`;
  await Bun.write(
    runtime,
    JSON.stringify({ version: 1, titleId: c.title, applicationId: "dev.pocket-nexus.maneuver", output: c.output, nativeBuild, plan: null, self: `${c.output}.self`, usbDebug: !c.standalone, usbDriver: usb?.fingerprint ?? null, selfSha256 }, null, 2) + "\n",
  );
  console.log(`maneuver: ${OUT_DIR}/${c.output}.self (native build ${nativeBuild})`);
  return runtime;
}

const LEASE = `${home}/.pocketjs/device-leases/${createHash("sha256").update("vita:usb").digest("hex")}.json`;

function leaseOwner(): { pid: number; cwd: string; token: string; active: boolean } | null {
  try {
    const o = JSON.parse(readFileSync(LEASE, "utf8"));
    process.kill(o.pid, 0);
    return o.active ? o : null;
  } catch {
    return null;
  }
}

/**
 * Holds the console for this repository across commands: a detached owner of
 * PocketJS's `vita:usb` lease. Other worktrees' device commands then fail with
 * "Device busy" and name this holder. `--take` ends another holder first;
 * `--release` ends this one.
 */
export async function hold(argv: string[]): Promise<void> {
  const o = leaseOwner();
  if (argv.includes("--release")) {
    if (o?.cwd === ROOT) process.kill(o.pid, "SIGTERM");
    console.log(o?.cwd === ROOT ? "maneuver: released vita:usb" : "maneuver: vita:usb is not held by this repository");
    return;
  }
  if (o) {
    if (o.cwd === ROOT) return console.log(`maneuver: vita:usb already held (pid ${o.pid})`);
    if (!argv.includes("--take")) throw new Error(`vita:usb is held by pid ${o.pid} (${o.cwd}); pass --take to end that holder`);
    process.kill(o.pid, "SIGTERM");
    await Bun.sleep(600);
  }
  const { spawn } = await import("node:child_process");
  spawn("bun", [`${POCKETJS}/tools/device-lease.ts`, "vita:usb", "--", "sleep", "86400"], { cwd: ROOT, detached: true, stdio: "ignore" }).unref();
  await Bun.sleep(800);
  const now = leaseOwner();
  if (now?.cwd !== ROOT) throw new Error("could not take the vita:usb lease");
  console.log(`maneuver: holding vita:usb (pid ${now.pid})`);
}

/** The holder's lease, for child commands. */
function leaseEnv(): Record<string, string> {
  const o = leaseOwner();
  return o?.cwd === ROOT ? { POCKET_DEVICE_LEASES: JSON.stringify({ "vita:usb": { token: o.token, path: LEASE } }) } : {};
}

/** PocketJS's wired debug tool, pointed at the chosen USB share. */
export async function dev(argv: string[], ...args: string[]): Promise<void> {
  const c = context(argv);
  mkdirSync(c.share, { recursive: true });
  await $`bun ${POCKETJS}/tools/vita-dev.ts ${args} --runtime ${OUT_DIR}/${c.output}.runtime.json --title ${c.title} --dir ${c.share}`.cwd(POCKETJS).env({ ...process.env, ...leaseEnv() });
}

/** Compiles the interface, then copies it and the pack to the share: each file unless the same bytes are already there. */
export async function sync(argv: string[]): Promise<void> {
  const c = context(argv);
  // The device never creates directories on host0:; every directory it writes into exists up front.
  for (const dir of ["", "gxp"]) mkdirSync(`${c.appShare}/${dir}`, { recursive: true });
  if (!existsSync(PACK)) throw new Error(`no pack at ${PACK}: run \`bun tools/maneuver.ts cook\` first`);
  const sha = (p: string) => createHash("sha256").update(readFileSync(p)).digest("hex");
  const copy = (src: string, name: string) => {
    const dst = `${c.appShare}/${name}`;
    if (existsSync(dst) && sha(dst) === sha(src)) return;
    cpSync(src, dst);
    console.log(`maneuver: synced ${name} to ${c.appShare}`);
  };
  copy(PACK, "world.pack");
  const ui = await compileInterface("vita");
  for (const name of INTERFACE) copy(`${ui.directory}/${name}`, name);
}

/**
 * The standalone package: the pack, the interface and the programs the device
 * compiled go inside the VPK, and the build carries no USB debug driver.
 */
export async function vpk(argv: string[]): Promise<void> {
  const c = context(argv);
  // `--gxp DIR` names the programs (tools/release.ts passes a checked set); without it, what the last development run left on the share.
  const programDirectory = resolve(value(argv, "--gxp", `${c.appShare}/gxp`));
  const manifest = `${programDirectory}/manifest.txt`;
  if (!existsSync(manifest)) throw new Error(`${manifest} missing: run the development build on the device first`);
  if (!existsSync(PACK)) throw new Error(`no pack at ${PACK}: run \`bun tools/maneuver.ts cook\` first`);
  const stage = resolve(ROOT, ".pocket-build/vpk");
  rmSync(stage, { recursive: true, force: true });
  mkdirSync(`${stage}/gxp`, { recursive: true });
  cpSync(`${APP_DIR}/assets`, stage, { recursive: true });
  const hashes = readFileSync(manifest, "utf8").split("\n").filter(Boolean);
  for (const h of hashes) {
    const gxp = `${programDirectory}/${h}.gxp`;
    if (!existsSync(gxp)) throw new Error(`${gxp} missing: the device has not compiled this build's programs`);
    cpSync(gxp, `${stage}/gxp/${h}.gxp`);
  }
  cpSync(PACK, `${stage}/world.pack`);
  const ui = await compileInterface("vita");
  for (const name of INTERFACE) cpSync(`${ui.directory}/${name}`, `${stage}/${name}`);
  console.log(`maneuver: staged ${hashes.length} programs, the pack and the interface in ${stage}`);
  await build([...argv.filter((a) => a !== "--standalone"), "--standalone"], stage);
}

/** Where `programs` leaves the console's programs, their list and `coverage.json`. No development run writes here. */
export const PROGRAMS_DIR = resolve(ROOT, ".pocket-build/vita-programs");

/** `vita/shaders` as one hash over each file's name and bytes, in the order of the names. */
export function shaderSources(): string {
  const hash = createHash("sha256");
  for (const name of readdirSync(`${APP_DIR}/shaders`).sort()) hash.update(name).update(readFileSync(`${APP_DIR}/shaders/${name}`));
  return hash.digest("hex");
}

/** How a set of programs was collected: what `programs` writes and `tools/release.ts` checks. */
export interface Coverage {
  schema: 1;
  at: string;
  /** The checkout the development build was built from. */
  commit: string | null;
  dirty: boolean;
  /** The development build that compiled the programs, as the console reported it. */
  nativeBuild: string;
  /** The pack the console read: the programs' numeric defines come from its scene record. */
  packSha256: string;
  /** `shaderSources()` when the build was made. */
  shaderSources: string;
  /** What the console reported: how many programs the build asked for, and how many it compiled in this run. */
  requested: number;
  compiled: number;
  /** The programs' names, each once, in the order the build asked for them. */
  programs: string[];
}

/**
 * Collects the programs a package carries. SceShaccCg runs on a console, so a package holds what a console
 * compiled. The runtime asks for every program it has while it loads (`Gpu::program` in vita/src/main.rs and
 * vita/src/post.rs, before `Gpu::finish`); no setting adds one later, and the multisampling choice changes how
 * a program is patched, not its source. One start therefore asks for the whole set.
 *
 * This builds the development build from this checkout, starts it with `{"programs": "fresh"}` in `boot.json`
 * (it then reads no program from the card or the package and compiles each one), waits until it runs, and
 * copies the console's list and its `.gxp` files to `.pocket-build/vita-programs/` with `coverage.json`.
 */
export async function programs(argv: string[]): Promise<void> {
  const c = context(argv);
  if (c.standalone) throw new Error("the pass runs the development build: drop --standalone");
  const git = (...args: string[]) => {
    const done = Bun.spawnSync(["git", ...args], { cwd: ROOT, stdout: "pipe", stderr: "pipe" });
    return done.exitCode === 0 ? done.stdout.toString().trim() : null;
  };
  const sha = (p: string) => createHash("sha256").update(readFileSync(p)).digest("hex");
  await sync(argv);
  const gxp = `${c.appShare}/gxp`;
  // Only what this run compiles may be in the share's folder when it is read.
  rmSync(gxp, { recursive: true, force: true });
  mkdirSync(gxp, { recursive: true });
  const boot = `${c.appShare}/boot.json`;
  const before = existsSync(boot) ? readFileSync(boot) : null;
  const sources = shaderSources();
  let engine: any;
  let nativeBuild: string;
  try {
    writeFileSync(boot, JSON.stringify({ title: false, programs: "fresh" }) + "\n");
    nativeBuild = JSON.parse(readFileSync(await build(argv), "utf8")).nativeBuild as string;
    if (shaderSources() !== sources) throw new Error("vita/shaders changed while the build ran");
    await dev(argv, "native");
    const end = Date.now() + 240_000;
    for (;;) {
      await Bun.sleep(1000);
      let s: any;
      try {
        s = status(argv);
      } catch {
        s = null;
      }
      if (s?.nativeBuild === nativeBuild && s.engine?.stage === "error") throw new Error(`the build stopped while loading: ${s.engine.message ?? JSON.stringify(s.engine)}`);
      if (s?.nativeBuild === nativeBuild && s.engine?.stage === "running") {
        engine = s.engine;
        break;
      }
      if (Date.now() > end) throw new Error(`the console did not report this build running within 240 s (it reports build ${s?.nativeBuild ?? "none"}, stage ${s?.engine?.stage ?? "none"})`);
    }
  } finally {
    if (before) writeFileSync(boot, before);
    else rmSync(boot, { force: true });
  }
  const ran = engine.programs ?? {};
  if (ran.fresh !== true || ran.cached !== 0 || ran.compiled !== ran.requested || !(ran.requested > 0)) {
    throw new Error(`the console did not compile every program in this run: ${JSON.stringify(ran)}`);
  }
  if (engine.pack?.sha256 !== sha(PACK)) throw new Error(`the console read a pack that is not ${PACK}: run \`bun tools/maneuver.ts cook --profile vita60\` and the pass again`);
  const listed = readFileSync(`${gxp}/manifest.txt`, "utf8").split("\n").filter(Boolean);
  if (listed.length !== ran.requested) throw new Error(`the console's list names ${listed.length} programs and the build asked for ${ran.requested}`);
  const names = [...new Set(listed)];
  rmSync(PROGRAMS_DIR, { recursive: true, force: true });
  mkdirSync(PROGRAMS_DIR, { recursive: true });
  for (const name of names) {
    if (!/^[0-9a-f]{16}$/.test(name)) throw new Error(`"${name}" in the console's list is not a program's name`);
    const from = `${gxp}/${name}.gxp`;
    if (!existsSync(from) || readFileSync(from).length <= 16) throw new Error(`${from} is missing: the console listed a program it did not send`);
    cpSync(from, `${PROGRAMS_DIR}/${name}.gxp`);
  }
  writeFileSync(`${PROGRAMS_DIR}/manifest.txt`, names.join("\n"));
  const coverage: Coverage = {
    schema: 1, at: new Date().toISOString(), commit: git("rev-parse", "HEAD"), dirty: git("status", "--porcelain") !== "",
    nativeBuild, packSha256: engine.pack.sha256, shaderSources: sources, requested: ran.requested, compiled: ran.compiled, programs: names,
  };
  writeFileSync(`${PROGRAMS_DIR}/coverage.json`, JSON.stringify(coverage, null, 2) + "\n");
  console.log(`maneuver: ${names.length} programs from ${ran.requested} requests, compiled by build ${nativeBuild} → ${PROGRAMS_DIR}`);
}

/**
 * Why a directory is not the programs of this checkout's package, or null when it is: `coverage.json` must be
 * there, name this checkout's `vita/shaders` and the pack being packaged, and list the programs the directory
 * holds, each compiled in the pass.
 */
export function coverageFault(directory: string, packSha256: string): string | null {
  const record = `${directory}/coverage.json`;
  const again = "run `bun tools/maneuver.ts programs` with the console on the development host";
  if (!existsSync(record)) return `${record} is missing: the programs were not collected by a pass (${again})`;
  const c = JSON.parse(readFileSync(record, "utf8")) as Coverage;
  if (c.schema !== 1) return `${record} is not a coverage record this tool reads`;
  if (c.shaderSources !== shaderSources()) return `the programs in ${directory} were compiled from other shader sources (${c.shaderSources.slice(0, 12)}) than this checkout's vita/shaders (${shaderSources().slice(0, 12)}): ${again}`;
  if (c.packSha256 !== packSha256) return `the programs in ${directory} were compiled for another pack (${c.packSha256.slice(0, 12)}) than the one being packaged (${packSha256.slice(0, 12)}): ${again}`;
  if (c.compiled !== c.requested || !(c.requested > 0)) return `${record}: the console compiled ${c.compiled} of ${c.requested} programs in the pass`;
  const listed = existsSync(`${directory}/manifest.txt`) ? readFileSync(`${directory}/manifest.txt`, "utf8").split("\n").filter(Boolean) : [];
  if (listed.join("\n") !== c.programs.join("\n")) return `${directory}/manifest.txt is not the list ${record} records`;
  for (const name of c.programs) if (!existsSync(`${directory}/${name}.gxp`)) return `${directory}/${name}.gxp is missing`;
  return null;
}

/** Sends a packaged VPK to `ux0:data/pocket-maneuver/` through the running development build, for VitaShell to install. */
export async function pushVpk(argv: string[]): Promise<void> {
  const c = context(argv);
  const file = resolve(argv.find((a) => a.endsWith(".vpk")) ?? `${OUT_DIR}/pocket-maneuver-PKMV00001.vpk`);
  const name = file.split("/").pop()!;
  mkdirSync(`${c.appShare}/outbox`, { recursive: true });
  rmSync(`${c.appShare}/outbox/${name}.done`, { force: true });
  cpSync(file, `${c.appShare}/outbox/${name}`);
  ctl(argv, JSON.stringify({ fetch: name, nonce: Date.now() }));
  for (let i = 0; i < 240; i++) {
    await Bun.sleep(500);
    if (existsSync(`${c.appShare}/outbox/${name}.done`)) {
      console.log(`maneuver: ${readFileSync(`${c.appShare}/outbox/${name}.done`, "utf8")}`);
      return;
    }
  }
  throw new Error("the device did not confirm the copy");
}

export function ctl(argv: string[], json: string): void {
  const c = context(argv);
  mkdirSync(c.appShare, { recursive: true });
  JSON.parse(json);
  writeFileSync(`${c.appShare}/control.json`, json);
}

/** The running process's status receipt, straight from the share. */
export function status(argv: string[]): any {
  const c = context(argv);
  // The device replaces the file every few frames; a read can land between the old one and the new one.
  for (let i = 0; ; i++) {
    try {
      return JSON.parse(readFileSync(`${c.share}/pocket-vita/${c.title}/status.json`, "utf8"));
    } catch (e) {
      if (i >= 20) throw e;
      Bun.sleepSync(25);
    }
  }
}
