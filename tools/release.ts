#!/usr/bin/env bun
// Pocket Maneuver's packages for Pocket Studio: one file per device, built from
// the checked-out commit by the commands a developer runs (tools/maneuver.ts,
// psp.ts, n3ds.ts, ipod.ts).
//
//   bun tools/release.ts [--targets vita,psp,3ds,ipod-touch] [--out dist/release]
//                        [--vita-gxp DIR] [--no-build] [--upload]
//
// It builds the simulation, exports the world from its seed, and for each
// target compiles the profile's pack with this commit's compiler, builds the
// program and writes one file to --out:
//
//   vita        pocket-maneuver-<version>.vpk        the pack, the interface and the console's programs inside
//   psp         pocket-maneuver-<version>-psp.zip    PSP/GAME/PocketManeuver/, for the root of a Memory Stick
//   3ds         pocket-maneuver-<version>.3dsx       the pack and the interface in its ROMFS
//   ipod-touch  pocket-maneuver-<version>-ipod.ipa   Payload/PocketManeuver.app
//
// and release.json beside them: the commit, the version (ui/pocket.json),
// each file's size and SHA-256, the inputs and the toolchains. A target that
// fails is reported, the others still build, and the exit status is 1.
//
// --vita-gxp DIR  The programs a console compiled, with the record of the pass
//                 that collected them: `manifest.txt`, the `<hash>.gxp` files
//                 and `coverage.json`, as `bun tools/maneuver.ts programs`
//                 leaves them (default .pocket-build/vita-programs).
//                 SceShaccCg runs on the console only, so the package carries
//                 what it compiled. The set is refused without its coverage
//                 record, and when the record names other shader sources or
//                 another pack than this build's.
// --no-build      Take the packages release.json lists; their hashes are checked.
// --upload        Send each package to Pocket Studio with `pocket-studio package`,
//                 run here, where `pocket-studio register` wrote .pocket-studio.json.
//                 POCKET_STUDIO_CLI names the command when it is not on PATH.
//
// No package goes to Git or to a GitHub release (AGENTS.md).

import { closeSync, cpSync, existsSync, mkdirSync, openSync, readdirSync, readFileSync, readSync, rmSync, statSync, writeFileSync, writeSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { deflateRawSync } from "node:zlib";
import { PROGRAMS_DIR, coverageFault, type Coverage } from "./vita.ts";

const ROOT = resolve(import.meta.dir, "..");
const POCKETJS = join(ROOT, "vendor/pocketjs");
const WORK = join(ROOT, ".pocket-build/release");
const WORLD = join(ROOT, ".pocket-build/world");
const NAME = "pocket-maneuver";
const TITLE = "Pocket Maneuver";

const argv = process.argv.slice(2);
const option = (key: string, fallback: string) => {
  const at = argv.indexOf(key);
  return at < 0 ? fallback : (argv[at + 1] ?? fallback);
};
const OUT = resolve(option("--out", join(ROOT, "dist/release")));

/** A package as release.json lists it. */
interface Package {
  target: Target;
  filename: string;
  bytes: number;
  sha256: string;
}

/** Pocket Studio's ids for the devices this repository builds for. */
const TARGETS = ["vita", "psp", "3ds", "ipod-touch"] as const;
type Target = (typeof TARGETS)[number];

/** What each target compiles and what its package is called. */
const BUILDS: Record<Target, { profile: string; filename: (version: string) => string; build: (log: string, output: string) => Promise<void> }> = {
  vita: { profile: "vita60", filename: (v) => `${NAME}-${v}.vpk`, build: vita },
  psp: { profile: "psp60", filename: (v) => `${NAME}-${v}-psp.zip`, build: psp },
  "3ds": { profile: "n3ds60", filename: (v) => `${NAME}-${v}.3dsx`, build: n3ds },
  "ipod-touch": { profile: "ipod60", filename: (v) => `${NAME}-${v}-ipod.ipa`, build: ipod },
};

const sha256 = (bytes: Uint8Array | string) => new Bun.CryptoHasher("sha256").update(bytes).digest("hex");
const fileSha256 = (path: string) => sha256(readFileSync(path));
const pack = (profile: string) => join(WORLD, `walled-town.${profile}.pack`);

/** The first line a command prints, or null when it cannot run. */
function line(command: string[], cwd = ROOT): string | null {
  try {
    const done = Bun.spawnSync(command, { cwd, stdout: "pipe", stderr: "pipe" });
    return done.exitCode === 0 ? (done.stdout.toString().trim().split("\n")[0] ?? "") : null;
  } catch {
    return null;
  }
}

/** Runs a build command with its output in `log`; a failure carries the log's last lines. */
async function run(log: string, command: string[], env: Record<string, string> = {}): Promise<void> {
  const fd = openSync(log, "a");
  writeSync(fd, `\n$ ${command.join(" ")}\n`);
  const code = await Bun.spawn(command, { cwd: ROOT, stdin: "ignore", stdout: fd, stderr: fd, env: { ...process.env, ...env } }).exited;
  closeSync(fd);
  if (code !== 0) throw new Error(`\`${command.join(" ")}\` exited ${code}; ${log} ends:\n${readFileSync(log, "utf8").trimEnd().split("\n").slice(-20).join("\n")}`);
}

// ---------------------------------------------------------------- archives

/** Every file and directory under `directory`, named from `prefix`. A directory's name ends in a slash and it has no path. */
function tree(directory: string, prefix: string): { name: string; path?: string }[] {
  return [
    { name: `${prefix}/` },
    ...readdirSync(directory, { withFileTypes: true }).flatMap((entry) =>
      entry.isDirectory() ? tree(join(directory, entry.name), `${prefix}/${entry.name}`) : [{ name: `${prefix}/${entry.name}`, path: join(directory, entry.name) }],
    ),
  ];
}

/**
 * Writes a zip whose bytes follow from its entries alone: the entries named
 * in `first`, then the others in the order of their names, every date
 * 1980-01-01 00:00, modes 0644, or 0755 for a directory and for a file its
 * owner may execute, no extra fields. An entry is deflated at level 6 unless
 * that makes it longer.
 */
function writeZip(output: string, entries: { name: string; path?: string }[], first: string[] = []): void {
  const rank = (name: string) => (first.includes(name) ? first.indexOf(name) : first.length);
  const fd = openSync(output, "w");
  const directory: Buffer[] = [];
  let at = 0;
  for (const entry of [...entries].sort((a, b) => rank(a.name) - rank(b.name) || (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
    const data = entry.path === undefined ? Buffer.alloc(0) : readFileSync(entry.path);
    const deflated = data.length ? deflateRawSync(data, { level: 6 }) : data;
    const body = deflated.length < data.length ? deflated : data;
    const method = body === data ? 0 : 8;
    const mode = entry.path === undefined ? 0o040755 : statSync(entry.path).mode & 0o100 ? 0o100755 : 0o100644;
    const name = Buffer.from(entry.name);
    const crc = Bun.hash.crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(method, 8);
    local.writeUInt16LE(0x0021, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(body.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(name.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    // Made on Unix, so the upper half of the external attributes is a file mode.
    central.writeUInt16LE((3 << 8) | 20, 4);
    local.copy(central, 6, 4, 30);
    central.writeUInt32LE(((mode << 16) | (entry.path === undefined ? 0x10 : 0)) >>> 0, 38);
    central.writeUInt32LE(at, 42);
    directory.push(central, name);
    for (const part of [local, name, body]) writeSync(fd, part);
    at += local.length + name.length + body.length;
  }
  const list = Buffer.concat(directory);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(directory.length / 2, 8);
  end.writeUInt16LE(directory.length / 2, 10);
  end.writeUInt32LE(list.length, 12);
  end.writeUInt32LE(at, 16);
  writeSync(fd, list);
  writeSync(fd, end);
  closeSync(fd);
}

/**
 * Writes a VPK again from what `vita-pack-vpk` packed. Its archive dates every entry with the time of the
 * build; this one holds the same files, `sce_sys/param.sfo` and `eboot.bin` first as it had them.
 */
function repackVpk(source: string, output: string): void {
  const unpacked = join(WORK, "vpk");
  rmSync(unpacked, { recursive: true, force: true });
  mkdirSync(unpacked, { recursive: true });
  const done = Bun.spawnSync(["unzip", "-q", "-o", source, "-d", unpacked], { stdout: "pipe", stderr: "pipe" });
  if (done.exitCode !== 0) throw new Error(`unzip ${source}: ${done.stderr.toString().trim()}`);
  const files = tree(unpacked, "").filter((entry) => entry.path !== undefined).map((entry) => ({ name: entry.name.slice(1), path: entry.path }));
  writeZip(output, files, ["sce_sys/param.sfo", "eboot.bin"]);
}

/** The id each release build carried, by target (release.json). */
const buildIds: Partial<Record<Target, string>> = {};

/**
 * The id a release build carries where a development build takes a random one: 32 hex digits from the
 * commit and the hashes of what the package is built from. The device tool reads it as POCKET_RELEASE_BUILD.
 */
function releaseBuild(target: Target, inputs: unknown): Record<string, string> {
  buildIds[target] = sha256(JSON.stringify([commit, dirty, target, inputs])).slice(0, 32);
  return { POCKET_RELEASE_BUILD: buildIds[target]! };
}

// ---------------------------------------------------------------- targets

let programs: { count: number; manifestSha256: string; sourcesSha256: string; compiledBy: string; collectedAt: string } | undefined;

/**
 * The standalone VPK (`tools/vita.ts` `vpk`) with the programs of a pass, written again with fixed dates.
 * A package starts on a console without SceShaccCg only when it holds every program the runtime asks for.
 * The runtime asks for all of them while it loads, and `bun tools/maneuver.ts programs` records what one
 * such start compiled; `coverageFault` refuses a directory whose record is missing or names other shader
 * sources or another pack than this build's.
 */
async function vita(log: string, output: string): Promise<void> {
  const source = resolve(option("--vita-gxp", PROGRAMS_DIR));
  const packSha256 = fileSha256(pack("vita60"));
  const fault = coverageFault(source, packSha256);
  if (fault) throw new Error(`no programs for the Vita: ${fault}`);
  const pass = JSON.parse(readFileSync(join(source, "coverage.json"), "utf8")) as Coverage;
  // The package takes the programs the record lists, and nothing else the directory may hold.
  const checked = join(WORK, "vita-gxp");
  rmSync(checked, { recursive: true, force: true });
  mkdirSync(checked, { recursive: true });
  writeFileSync(join(checked, "manifest.txt"), pass.programs.join("\n"));
  for (const name of pass.programs) cpSync(join(source, `${name}.gxp`), join(checked, `${name}.gxp`));
  programs = { count: pass.programs.length, manifestSha256: sha256(pass.programs.join("\n")), sourcesSha256: pass.shaderSources, compiledBy: pass.nativeBuild, collectedAt: pass.at };
  await run(log, ["bun", "tools/maneuver.ts", "vpk", "--gxp", checked], releaseBuild("vita", [packSha256, programs.manifestSha256]));
  repackVpk(join(ROOT, "dist/vita/pocket-maneuver-PKMV00001.vpk"), output);
}

/** The Memory Stick folder (`tools/psp.ts` `package`), zipped from the card's root. */
async function psp(log: string, output: string): Promise<void> {
  rmSync(join(ROOT, "dist/psp/PSP"), { recursive: true, force: true });
  await run(log, ["bun", "tools/psp.ts", "build"]);
  await run(log, ["bun", "tools/psp.ts", "package"]);
  writeZip(output, [{ name: "PSP/" }, { name: "PSP/GAME/" }, ...tree(join(ROOT, "dist/psp/PSP/GAME/PocketManeuver"), "PSP/GAME/PocketManeuver")]);
}

/** The `.3dsx` (`tools/n3ds.ts` `build`, which refuses one over the 32 MiB the wire installs). Never a CIA. */
async function n3ds(log: string, output: string): Promise<void> {
  await run(log, ["bun", "tools/n3ds.ts", "build"]);
  cpSync(join(ROOT, "dist/3ds/pocket-maneuver.3dsx"), output);
}

/** The app bundle (`tools/ipod.ts` `package`), zipped as an `.ipa`. */
async function ipod(log: string, output: string): Promise<void> {
  await run(log, ["bun", "tools/ipod.ts", "package"]);
  writeZip(output, tree(join(ROOT, ".pocket-build/ipod/Payload"), "Payload"));
}

/** What Pocket Studio accepts as a package: its name, its first bytes and its size. */
function accept(path: string): void {
  const name = basename(path);
  const bytes = statSync(path).size;
  if (!/^[A-Za-z0-9._-]{1,80}$/.test(name)) throw new Error(`${name}: a package's name is 1 to 80 letters, digits, dots, underscores and hyphens`);
  if (bytes > 512 * 1024 * 1024) throw new Error(`${name} is ${bytes} bytes: a package is at most 512 MiB`);
  const head = Buffer.alloc(4);
  const fd = openSync(path, "r");
  readSync(fd, head, 0, 4, 0);
  closeSync(fd);
  const magic = name.endsWith(".3dsx") ? "3DSX" : "PK\x03\x04";
  if (head.toString("latin1") !== magic) throw new Error(`${name} does not start with the bytes of its format`);
}

// ---------------------------------------------------------------- what the build came from

/** The exported world the packs are compiled from: one hash over each file's name and bytes. */
function worldIr() {
  const directory = join(WORLD, "ir");
  const manifest = JSON.parse(readFileSync(join(directory, "manifest.json"), "utf8"));
  const names = readdirSync(directory).sort();
  const hash = new Bun.CryptoHasher("sha256");
  for (const name of names) hash.update(name).update(readFileSync(join(directory, name)));
  return { name: (manifest.name ?? null) as string | null, seed: (manifest.seed ?? null) as number | null, files: names.length, sha256: hash.digest("hex") };
}

/** The toolchain a tool names in `rustup run <toolchain> cargo`. */
function named(tool: string): string | null {
  return / run (\S+) cargo /.exec(readFileSync(join(ROOT, tool), "utf8"))?.[1] ?? null;
}

function toolchains() {
  const rustc = (toolchain: string | null) => (toolchain ? `${toolchain}: ${line(["rustup", "run", toolchain, "rustc", "-V"]) ?? "not installed"}` : null);
  const pinned = (file: string) => JSON.parse(readFileSync(join(POCKETJS, "tools/cli", file), "utf8"));
  const vitasdk = process.env.VITASDK || `${process.env.HOME}/vitasdk`;
  const container = /"(devkitpro\/devkitarm@sha256:[0-9a-f]{64})"/.exec(readFileSync(join(POCKETJS, "tools/3ds-toolchain.ts"), "utf8"))?.[1] ?? null;
  return {
    pocketjs: line(["git", "-C", POCKETJS, "rev-parse", "HEAD"]),
    bun: Bun.version,
    rustc: {
      cook: line(["rustc", "-V"]),
      vita: rustc(named("tools/vita.ts")),
      psp: rustc(pinned("psp-toolchain.json").rust.toolchain),
      "3ds": rustc(named("tools/n3ds.ts")),
      "ipod-touch": rustc(pinned("iphone4s-toolchain.json").compiler.rustToolchain),
    },
    vitasdk: {
      gcc: line([`${vitasdk}/bin/arm-vita-eabi-gcc`, "--version"]),
      versionInfoSha256: existsSync(`${vitasdk}/version_info.txt`) ? fileSha256(`${vitasdk}/version_info.txt`) : null,
      cargoVita: line(["cargo", "vita", "--version"]),
    },
    pspSdk: pinned("psp-toolchain.json").sdk.sha256 as string,
    devkitarm: container,
    clang: line(["xcrun", "clang", "--version"]),
  };
}

// ---------------------------------------------------------------- Pocket Studio

/** Sends the packages with the Studio's own CLI, from this directory's project link. */
async function upload(packages: Package[], version: string): Promise<boolean> {
  const link = join(ROOT, ".pocket-studio.json");
  if (!existsSync(link)) {
    console.error(
      `release: ${link} is missing, so this checkout names no Pocket Studio project. Once, in ${ROOT}:\n` +
        `  pocket-studio link <CODE>                       # the link code from the room, when this computer is not linked to an account\n` +
        `  pocket-studio register --title "${TITLE}"   # writes .pocket-studio.json, which Git ignores\n` +
        `then: bun tools/release.ts --no-build --upload`,
    );
    return false;
  }
  const project = JSON.parse(readFileSync(link, "utf8"));
  if (project.kind !== "site") throw new Error(`${link} links a device session, not a registered game: run \`pocket-studio register --title "${TITLE}"\` in a directory without one`);
  const cli = process.env.POCKET_STUDIO_CLI?.trim().split(/\s+/) ?? (Bun.which("pocket-studio") ? ["pocket-studio"] : null);
  if (!cli) throw new Error("no pocket-studio on PATH: install it from the Studio, or set POCKET_STUDIO_CLI to its command");
  console.log(`release: uploading ${packages.length} package(s) to ${project.server} (${project.app})`);
  for (const pkg of packages) {
    const command = [...cli, "package", join(OUT, pkg.filename), "--target", pkg.target, "--version", version];
    console.log(`$ ${command.join(" ")}`);
    const code = await Bun.spawn(command, { cwd: ROOT, stdin: "ignore", stdout: "inherit", stderr: "inherit" }).exited;
    if (code !== 0) throw new Error(`pocket-studio package exited ${code} for ${pkg.filename}`);
  }
  return true;
}

// ---------------------------------------------------------------- main

const asked = option("--targets", TARGETS.join(",")).split(",").filter(Boolean);
const unknown = asked.filter((t) => !(TARGETS as readonly string[]).includes(t));
if (unknown.length || argv.includes("--help")) {
  if (unknown.length) console.error(`release: no build for ${unknown.join(", ")}: the targets are ${TARGETS.join(", ")}`);
  console.log("usage: bun tools/release.ts [--targets vita,psp,3ds,ipod-touch] [--out dist/release] [--vita-gxp DIR] [--no-build] [--upload]");
  process.exit(unknown.length ? 1 : 0);
}
const targets = TARGETS.filter((t) => asked.includes(t));
const version = JSON.parse(readFileSync(join(ROOT, "ui/pocket.json"), "utf8")).version as string;
const commit = line(["git", "rev-parse", "HEAD"]);
const dirty = Bun.spawnSync(["git", "status", "--porcelain"], { cwd: ROOT }).stdout.toString().trim() !== "";
const record = join(OUT, "release.json");
if (argv.includes("--upload") && dirty) throw new Error("the checkout has uncommitted changes: a package names the commit it was built from, so commit before --upload");
let packages: Package[] = [];
const failed: { target: Target; error: string }[] = [];

if (argv.includes("--no-build")) {
  if (!existsSync(record)) throw new Error(`${record} is missing: run without --no-build first`);
  const built = JSON.parse(readFileSync(record, "utf8"));
  if (built.commit !== commit) throw new Error(`${record} was built from ${built.commit}; the checkout is at ${commit}`);
  packages = (built.packages as Package[]).filter((p) => targets.includes(p.target));
  for (const pkg of packages) {
    if (fileSha256(join(OUT, pkg.filename)) !== pkg.sha256) throw new Error(`${pkg.filename} is not the file release.json lists`);
    console.log(`release: ${pkg.target.padEnd(10)} ${pkg.filename}  ${pkg.bytes} bytes  sha256 ${pkg.sha256}`);
  }
} else {
  mkdirSync(OUT, { recursive: true });
  mkdirSync(join(WORK, "logs"), { recursive: true });
  console.log(`release: ${TITLE} ${version} at ${commit}${dirty ? " with uncommitted changes" : ""}`);
  // The world comes from its seed: the generator runs the simulation's wasm build, so that is built first.
  const prepare = join(WORK, "logs", "world.log");
  rmSync(prepare, { force: true });
  console.log(`release: building the simulation and exporting the world (log: ${prepare})`);
  await run(prepare, ["bun", "tools/maneuver.ts", "sim"]);
  await run(prepare, ["bun", "tools/maneuver.ts", "export"]);
  const ir = worldIr();
  const packs: Record<string, { bytes: number; sha256: string }> = {};
  const seconds: Partial<Record<Target, number>> = {};
  // One at a time: each device's interface is compiled into the same generated files under ui/.
  for (const target of targets) {
    const { profile, filename, build } = BUILDS[target];
    const log = join(WORK, "logs", `${target}.log`);
    const output = join(OUT, filename(version));
    const start = performance.now();
    rmSync(log, { force: true });
    rmSync(output, { force: true });
    try {
      console.log(`release: ${target}: compiling ${profile}, building (log: ${log})`);
      await run(log, ["bun", "tools/maneuver.ts", "cook", "--no-export", "--profile", profile]);
      packs[profile] = { bytes: statSync(pack(profile)).size, sha256: fileSha256(pack(profile)) };
      await build(log, output);
      accept(output);
      packages.push({ target, filename: filename(version), bytes: statSync(output).size, sha256: fileSha256(output) });
    } catch (error) {
      rmSync(output, { force: true });
      failed.push({ target, error: error instanceof Error ? error.message : String(error) });
      console.error(`release: ${target} failed: ${failed.at(-1)!.error}`);
    }
    seconds[target] = Math.round((performance.now() - start) / 100) / 10;
  }
  writeFileSync(
    record,
    JSON.stringify(
      {
        schema: 1,
        name: NAME,
        title: TITLE,
        version,
        commit,
        dirty,
        packages,
        failed: failed.map(({ target, error }) => ({ target, error: error.split("\n")[0] })),
        inputs: { worldIr: ir, packs, vitaPrograms: programs ?? null, buildIds },
        toolchains: toolchains(),
      },
      null,
      2,
    ) + "\n",
  );
  for (const pkg of packages) console.log(`release: ${pkg.target.padEnd(10)} ${pkg.filename}  ${pkg.bytes} bytes  sha256 ${pkg.sha256}  ${seconds[pkg.target]} s`);
  for (const { target } of failed) console.log(`release: ${target.padEnd(10)} not built  ${seconds[target]} s`);
  console.log(`release: ${record}`);
}

if (argv.includes("--upload")) {
  if (failed.length) throw new Error(`not uploading: ${failed.map((f) => f.target).join(", ")} did not build`);
  if (!(await upload(packages, version))) process.exit(1);
}
process.exit(failed.length ? 1 : 0);
