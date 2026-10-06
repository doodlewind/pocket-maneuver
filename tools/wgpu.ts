#!/usr/bin/env bun
// Pocket Maneuver drawn with wgpu (wgpu/): in a browser tab over WebGPU, and
// on this machine, where frames go to files.
//
//   bun tools/wgpu.ts cook                         the world for it: the PS Vita's pack (profiles/vita60.json)
//   bun tools/wgpu.ts build                        wasm32 + wasm-bindgen + the page + the interface for the four
//                                                  devices (tools/ui.ts) on PocketJS's UI core → .pocket-build/wgpu/site
//   bun tools/wgpu.ts serve [--port 8801]          the site and the pack, with byte ranges
//   bun tools/wgpu.ts dist [--piece 2]             the directory a static host serves → .pocket-build/wgpu/dist:
//                                                  the page, the module under its build's name, and the pack
//                                                  cut into pieces of that many MiB with their manifest
//   bun tools/wgpu.ts serve --dist                 that directory as such a host serves it: no byte ranges
//   bun tools/wgpu.ts shot [--out f.png] [--shape vita] [--size WxH] [--frames N] [--words "mode=play auto=1"]
//                                                  one frame on this machine's GPU (Metal) → a PNG and the status
//   bun tools/wgpu.ts check [--headed] [--seconds 5] [--dist]   the page in Chrome, driven by keys, pointer and
//                                                  touch: each device from its title into play and its pause
//                                                  list, another device picked in play, what a frame costs,
//                                                  what the first frame needs on a slow line
//                                                  → .pocket-build/validation/web/
//
// Every command takes [--pack PATH] (default: .pocket-build/world/walled-town.vita60.pack).
// `build` compiles the interface as tools/ui.ts does: it needs `bun install` in vendor/pocketjs and the
// exported world (.pocket-build/world/ir), which `cook` writes.
// The pack, the site and the captures stay under the ignored .pocket-build/.

import { $ } from "bun";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { join, resolve } from "node:path";
import { POCKET3D_ICON } from "../vendor/pocketjs/tools/pocket3d-icon.ts";
import { cutPack, stagePocket3dWeb } from "../vendor/pocketjs/tools/pocket3d-web.ts";
import { compileInterface, type Device } from "./ui.ts";

export const ROOT = resolve(import.meta.dir, "..");
const CRATE = join(ROOT, "wgpu");
const BUILD = join(ROOT, ".pocket-build/wgpu");
const SITE = join(BUILD, "site");
const DIST = join(BUILD, "dist");
export const PACK = join(ROOT, ".pocket-build/world/walled-town.vita60.pack");
// The handhelds the page shows (`DEVICES` in wgpu/page/main.js, `SHAPES` in wgpu/src/app.rs).
const SHOWN = ["vita", "psp", "3ds", "ipod"] as const satisfies readonly Device[];
// What the host a build is deployed to allows (Pocket Studio's site deployments): the size of a file, the
// files and the bytes of a deployment, and the top-level names it keeps for itself.
const HOST = { file: 32 << 20, files: 4000, bytes: 1 << 30, reserved: ["play", "runtime"] };

const [command, ...rest] = import.meta.main ? process.argv.slice(2) : [];
const option = (flag: string, fallback = "") => {
  const i = rest.indexOf(flag);
  return i >= 0 && rest[i + 1] ? rest[i + 1]! : fallback;
};
const pack = resolve(option("--pack", PACK));

function needPack() {
  if (!existsSync(pack)) throw new Error(`${pack} is missing: bun tools/wgpu.ts cook`);
}

/** The wasm module, its JavaScript side and the page, as one directory a static server can serve. */
export async function build() {
  // The crate and the command line tool write two halves of one interface: their versions must be the same.
  const lock = readFileSync(join(CRATE, "Cargo.lock"), "utf8").match(/name = "wasm-bindgen"\nversion = "([^"]+)"/)?.[1];
  const tool = (await $`wasm-bindgen --version`.text()).trim().split(" ")[1];
  if (lock !== tool) throw new Error(`wasm-bindgen ${tool} is installed and wgpu/Cargo.lock has ${lock}: cargo install wasm-bindgen-cli --version ${lock}`);
  await $`cargo build --release --lib --target wasm32-unknown-unknown`.cwd(CRATE);
  rmSync(SITE, { recursive: true, force: true });
  mkdirSync(join(SITE, "pkg"), { recursive: true });
  await $`wasm-bindgen --target web --no-typescript --out-dir ${join(SITE, "pkg")} ${join(CRATE, "target/wasm32-unknown-unknown/release/maneuver_wgpu.wasm")}`;
  for (const file of ["index.html", "main.js"]) cpSync(join(CRATE, "page", file), join(SITE, file));
  // What the page loads from PocketJS's browser kernel (vendor/pocketjs/devices/web/pocket-web-wgpu), as
  // PocketJS stages it: the page's modules, the Pocket3D title card, the realm of the interface's guest
  // with the UI core, and the host helpers of the framework. Then the icon of the tab.
  await stagePocket3dWeb(SITE);
  cpSync(POCKET3D_ICON.ios2x, join(SITE, "icon.png"));
  // The game's interface for each device, as its own build compiles it, with the plan PocketJS resolved.
  for (const device of SHOWN) {
    const built = await compileInterface(device);
    mkdirSync(join(SITE, "ui", device), { recursive: true });
    for (const file of ["maneuver.js", "maneuver.pak", "plan.json"]) cpSync(join(built.directory, file), join(SITE, "ui", device, file));
  }

  const sizes: Record<string, { bytes: number; gzip: number }> = {};
  for (const file of files(SITE).sort()) {
    const bytes = readFileSync(join(SITE, file));
    sizes[file] = { bytes: bytes.length, gzip: gzipSync(bytes, { level: 9 }).length };
  }
  writeFileSync(join(BUILD, "site.json"), JSON.stringify({ wasmBindgen: tool, sizes }, null, 1));
  return sizes;
}

const sha256 = (bytes: Uint8Array | string) => new Bun.CryptoHasher("sha256").update(bytes).digest("hex");

/** Every file under a directory, as paths from it. */
function files(directory: string, under = ""): string[] {
  return readdirSync(join(directory, under), { withFileTypes: true }).flatMap((entry) => (entry.isDirectory() ? files(directory, join(under, entry.name)) : [join(under, entry.name)]));
}

/**
 * The directory a static host serves, for a host that limits a file's size and keeps a file for ten minutes
 * in a browser's cache. Only the page is asked for again at every visit, so everything it names has a name of
 * its own contents: the module and its scripts under `app/<build>/`, the pack's manifest by the pack's hash,
 * a piece by its own. A deployment of new code leaves the pack's files as they are.
 */
async function dist(pieceBytes: number) {
  needPack();
  await build();
  rmSync(DIST, { recursive: true, force: true });
  // (everything of the site but the page and its icon: the module, the scripts, the interface)
  const app = files(SITE).filter((file) => !["index.html", "icon.png"].includes(file)).sort().map((file) => [file, readFileSync(join(SITE, file))] as const);
  const id = sha256(Buffer.concat(app.flatMap(([file, bytes]) => [Buffer.from(file), bytes]))).slice(0, 12);
  for (const [file, bytes] of app) {
    mkdirSync(join(DIST, "app", id, file, ".."), { recursive: true });
    writeFileSync(join(DIST, "app", id, file), bytes);
  }
  // The pack in pieces of one size, each named by its hash, and the manifest that lists them
  // (pocket_web_wgpu::source::Manifest), named by the pack's.
  const cut = cutPack(pack, join(DIST, "pack"), pieceBytes);
  const manifest = `pack/${cut.manifest}`;
  // The page names its build and its pack.
  let page = readFileSync(join(SITE, "index.html"), "utf8");
  for (const [from, to] of [[`<meta name="pocket-pack" content="world.pack">`, `<meta name="pocket-pack" content="${manifest}">`], [`src="main.js"`, `src="app/${id}/main.js"`], [`href="pocket3d-stage.css"`, `href="app/${id}/pocket3d-stage.css"`], [`href="pocket3d-player.css"`, `href="app/${id}/pocket3d-player.css"`]] as const) {
    if (!page.includes(from)) throw new Error(`wgpu/page/index.html has no ${from}`);
    page = page.replace(from, to);
  }
  // The game in Pocket Studio, for the player's door to it: the project this checkout is registered as
  // (`pocket-studio register` wrote .pocket-studio.json, which Git ignores). A checkout that is not
  // registered deploys a page whose door is the Studio's front one.
  const link = join(ROOT, ".pocket-studio.json");
  const project = existsSync(link) ? (JSON.parse(readFileSync(link, "utf8")) as { kind?: string; server?: string; app?: string }) : null;
  const registered = project?.kind === "site" && /^[A-Za-z0-9_-]+$/.test(project.app ?? "") && /^https:\/\/[A-Za-z0-9.-]+$/.test(project.server ?? "") ? project : null;
  if (registered) page = page.replace(`<meta name="pocket-pack"`, `<meta name="pocket-app" content="${registered.app}">\n<meta name="pocket-studio" content="${registered.server}">\n<meta name="pocket-pack"`);
  writeFileSync(join(DIST, "index.html"), page);
  cpSync(join(SITE, "icon.png"), join(DIST, "icon.png"));

  // What was written is what the host takes.
  const all = files(DIST).map((file) => ({ file, bytes: statSync(join(DIST, file)).size }));
  const total = all.reduce((sum, f) => sum + f.bytes, 0);
  const largest = all.reduce((a, b) => (b.bytes > a.bytes ? b : a));
  const part = (prefix: string) => all.filter((f) => f.file.startsWith(prefix)).reduce((sum, f) => ({ files: sum.files + 1, bytes: sum.bytes + f.bytes }), { files: 0, bytes: 0 });
  const refused = [
    ...all.filter((f) => f.bytes > HOST.file).map((f) => `${f.file} is ${f.bytes} bytes (a file is at most ${HOST.file})`),
    ...(all.length > HOST.files ? [`${all.length} files (at most ${HOST.files})`] : []),
    ...(total > HOST.bytes ? [`${total} bytes (at most ${HOST.bytes})`] : []),
    ...readdirSync(DIST).filter((name) => HOST.reserved.includes(name)).map((name) => `${name}/ is the host's own`),
  ];
  if (refused.length) throw new Error(`the host would refuse the directory: ${refused.join("; ")}`);
  const report = { directory: DIST, files: all.length, bytes: total, largest, build: id, studio: registered ? { app: registered.app, server: registered.server } : null, page: part("index.html"), app: part("app/"), pack: { ...part("pack/"), manifest, pieces: cut.pieces.length, piece: pieceBytes, sha256: cut.sha256 } };
  writeFileSync(join(BUILD, "dist.json"), JSON.stringify(report, null, 1));
  return report;
}

/** What Chrome is started with for a page that draws with WebGPU: the real GPU, through Metal. */
export const CHROME = ["--use-angle=metal", "--enable-gpu", "--ignore-gpu-blocklist", "--enable-unsafe-webgpu", "--no-proxy-server", "--autoplay-policy=no-user-gesture-required"];

const TYPES: Record<string, string> = { html: "text/html; charset=utf-8", js: "text/javascript; charset=utf-8", css: "text/css; charset=utf-8", wasm: "application/wasm", json: "application/json", png: "image/png", webp: "image/webp", woff2: "font/woff2", txt: "text/plain; charset=utf-8", pak: "application/octet-stream" };

/**
 * What a game's host of Pocket Studio answers at `/app.json`, for the server here: the player reads the
 * game's name and its packages from it, and the address it tells that it opened (`opened`, which the
 * server here answers itself and counts).
 */
const OPENED = "/api/events/player";
const opens: { query: string; mode: string }[] = [];
const APP = {
  kind: "site", id: "local", slug: null, url: null, title: "Pocket Maneuver", author: "local", tagline: "Two wires, a tank of gas, a walled town.", verified: true, status: "published",
  packages: [{ target: "psp", filename: "pocket-maneuver-0.1.0-psp.zip", size: 21_000_000, version: "0.1.0" }, { target: "3ds", filename: "pocket-maneuver-0.1.0.3dsx", size: 30_000_000, version: "0.1.0" }],
};

/** The deployable directory as its host serves it: whole files, the page asked for again at every visit. */
function serveDist(port: number) {
  return Bun.serve({
    port,
    hostname: "127.0.0.1",
    fetch(request) {
      const path = decodeURIComponent(new URL(request.url).pathname);
      // (this host names no address for it: a page that asks is heard, and the check says so)
      if (path === OPENED) opens.push({ query: new URL(request.url).search, mode: request.headers.get("sec-fetch-mode") ?? "" });
      const file = join(DIST, path === "/" ? "index.html" : path);
      if (!file.startsWith(DIST) || !existsSync(file) || !statSync(file).isFile()) return new Response("not found", { status: 404 });
      const type = file.split(".").pop()!;
      return new Response(Bun.file(file), { headers: { "Content-Type": TYPES[type] ?? "application/octet-stream", "Cache-Control": type === "html" ? "no-cache" : "public, max-age=600" } });
    },
  });
}

/** The site and the pack. The pack is answered a range at a time, as a tab asks for it. */
export function serve(port: number) {
  const size = statSync(pack).size;
  return Bun.serve({
    port,
    hostname: "127.0.0.1",
    fetch(request) {
      const path = decodeURIComponent(new URL(request.url).pathname);
      if (path === "/app.json") return Response.json({ ...APP, opened: new URL(OPENED, request.url).href }, { headers: { "Cache-Control": "no-store" } });
      if (path === OPENED) {
        opens.push({ query: new URL(request.url).search, mode: request.headers.get("sec-fetch-mode") ?? "" });
        return new Response(null, { status: 204 });
      }
      if (path === "/world.pack") {
        const head = { "Accept-Ranges": "bytes", "Content-Type": "application/octet-stream", "Cache-Control": "no-store" };
        const range = request.headers.get("range")?.match(/^bytes=(\d+)-(\d*)$/);
        if (!range) return new Response(Bun.file(pack), { headers: head });
        const from = Number(range[1]);
        const to = Math.min(range[2] ? Number(range[2]) : size - 1, size - 1);
        if (from > to) return new Response(null, { status: 416, headers: { "Content-Range": `bytes */${size}` } });
        return new Response(Bun.file(pack).slice(from, to + 1), { status: 206, headers: { ...head, "Content-Range": `bytes ${from}-${to}/${size}`, "Content-Length": String(to - from + 1) } });
      }
      const file = join(SITE, path === "/" ? "index.html" : path);
      if (!file.startsWith(SITE) || !existsSync(file) || !statSync(file).isFile()) return new Response("not found", { status: 404 });
      return new Response(Bun.file(file), { headers: { "Content-Type": TYPES[file.split(".").pop()!] ?? "application/octet-stream", "Cache-Control": "no-store" } });
    },
  });
}

/** The program that draws frames on this machine's GPU (wgpu/src/bin/shot.rs), built. */
export async function shotProgram(): Promise<string> {
  await $`cargo build --release --bin maneuver-shot`.cwd(CRATE).quiet();
  return join(CRATE, "target/release/maneuver-shot");
}

/** One frame on this machine's GPU, from the pack's file or from a manifest of its pieces. Returns the status
 * the run printed. */
async function shot(out: string, extra: string[], from = pack) {
  const program = await shotProgram();
  mkdirSync(resolve(out, ".."), { recursive: true });
  return JSON.parse(await $`${program} --pack ${from} --out ${out} ${extra}`.text());
}

const stamp = () => new Date().toISOString().replace(/[:.]/g, "-");
const validation = (run: string) => {
  const directory = join(ROOT, ".pocket-build/validation/web", run);
  mkdirSync(directory, { recursive: true });
  return directory;
};

if (command === "cook") {
  await $`bun tools/maneuver.ts cook --profile vita60`.cwd(ROOT);
} else if (command === "build") {
  console.log(JSON.stringify(await build(), null, 1));
} else if (command === "dist") {
  console.log(JSON.stringify(await dist(Math.round(Number(option("--piece", "2")) * (1 << 20))), null, 1));
} else if (command === "serve" && rest.includes("--dist")) {
  if (!existsSync(join(DIST, "index.html"))) await dist(2 << 20);
  const server = serveDist(Number(option("--port", "8801")));
  console.log(`http://127.0.0.1:${server.port}/   (${DIST})`);
} else if (command === "serve") {
  needPack();
  if (!existsSync(join(SITE, "pkg/maneuver_wgpu_bg.wasm")) || rest.includes("--build")) await build();
  const server = serve(Number(option("--port", "8801")));
  console.log(`http://127.0.0.1:${server.port}/   (${pack})`);
} else if (command === "shot") {
  needPack();
  const out = resolve(option("--out", join(validation(`shot-${stamp()}`), "frame.png")));
  const passed = ["--shape", "--size", "--frames", "--words", "--status"].flatMap((flag) => (option(flag) ? [flag, option(flag)] : []));
  console.log(JSON.stringify(await shot(out, passed), null, 1));
  console.log(out);
} else if (command === "check") {
  needPack();
  // (--dist: the deployable directory, served whole files only, with the pack in pieces)
  const deployed = rest.includes("--dist") ? await dist(Math.round(Number(option("--piece", "2")) * (1 << 20))) : null;
  const sizes = deployed ? JSON.parse(readFileSync(join(BUILD, "site.json"), "utf8")).sizes : await build();
  const { chromium } = await import("playwright-core");
  const server = deployed ? serveDist(0) : serve(0);
  const directory = validation(`check-${deployed ? "dist-" : ""}${stamp()}`);
  const seconds = Number(option("--seconds", "5"));
  const origin = `http://127.0.0.1:${server.port}`;
  // WebGPU needs the real GPU: headless Chrome is given Metal through ANGLE; --headed opens a window instead.
  const browser = await chromium.launch({ channel: "chrome", headless: !rest.includes("--headed"), args: CHROME });
  const report: Record<string, any> = { chrome: browser.version(), pack, deployed };
  const expect = (what: string, ok: boolean) => {
    if (!ok) throw new Error(`the page: ${what}`);
  };
  type Page = Awaited<ReturnType<typeof browser.newPage>>;
  /** A page on the site, and what a person does to it: keys held for a moment, a pointer down and up at a
   * place of a screen given in that screen's logical pixels, a drag. */
  const visit = async (address: string, context?: Awaited<ReturnType<typeof browser.newContext>>) => {
    const page: Page = await (context ?? browser).newPage({ viewport: { width: 1280, height: 800 } });
    const problems: string[] = [];
    // (the deployable directory's host answers no /app.json: the browser writes that one reply to the
    // console, and the player goes on with the page's own words)
    page.on("console", (message) => message.type() === "error" && !message.location().url.endsWith("/app.json") && problems.push(message.text()));
    page.on("pageerror", (error) => problems.push(String(error)));
    await page.goto(`${origin}/${address}`);
    const status = async () => JSON.parse((await page.evaluate("pocketManeuver.maneuver.status()")) as string);
    const at = async (screen: "upper" | "lower", size: number[], x: number, y: number) => {
      const box = (await page.locator(`[data-pocket-screen=${screen}]`).boundingBox())!;
      return [box.x + (x / size[0]!) * box.width, box.y + (y / size[1]!) * box.height] as const;
    };
    return {
      page,
      problems,
      status,
      /** The world is behind the title, the interface is up, and the card has left. */
      async up() {
        await page.waitForFunction("window.pocketManeuver && ((pocketManeuver.firstWorld && pocketManeuver.interface?.()) || pocketManeuver.failure)", undefined, { timeout: 90_000 });
        const failure = (await page.evaluate("pocketManeuver.failure")) as string;
        if (failure) throw new Error(`${address}: ${failure}`);
      },
      async mode(want: string) {
        await page.waitForFunction(`JSON.parse(pocketManeuver.maneuver.status()).mode === ${JSON.stringify(want)}`, undefined, { timeout: 5_000 }).catch(() => {});
        const now = (await status()).mode;
        expect(`${address}: the flow is "${now}", not "${want}"`, now === want);
      },
      async key(code: string, hold = 90) {
        await page.keyboard.down(code);
        await page.waitForTimeout(hold);
        await page.keyboard.up(code);
        await page.waitForTimeout(200);
      },
      async tap(screen: "upper" | "lower", size: number[], x: number, y: number) {
        const [px, py] = await at(screen, size, x, y);
        await page.mouse.move(px, py);
        await page.mouse.down();
        await page.waitForTimeout(100);
        await page.mouse.up();
        await page.waitForTimeout(250);
      },
      async drag(screen: "upper" | "lower", size: number[], from: number[], to: number[], hold = 0) {
        const [ax, ay] = await at(screen, size, from[0]!, from[1]!);
        const [bx, by] = await at(screen, size, to[0]!, to[1]!);
        await page.mouse.move(ax, ay);
        await page.mouse.down();
        await page.mouse.move(bx, by, { steps: 12 });
        await page.waitForTimeout(hold);
        await page.mouse.up();
        await page.waitForTimeout(250);
      },
      /** The screens as their canvases hold them, a pixel to a pixel: `<name>.png`, and `<name>-lower.png`. */
      async save(name: string) {
        const shot = (await page.evaluate("pocketManeuver.capture()")) as { upper: string; lower: string | null };
        const write = (file: string, url: string) => writeFileSync(join(directory, file), Buffer.from(url.slice(url.indexOf(",") + 1), "base64"));
        write(`${name}.png`, shot.upper);
        if (shot.lower) write(`${name}-lower.png`, shot.lower);
        return join(directory, `${name}.png`);
      },
    };
  };
  const program = await shotProgram();
  const compare = async (a: string, b: string) => JSON.parse(await $`${program} --compare ${a} ${b}`.text());
  const far = (a: number[], b: number[]) => Math.hypot(a[0]! - b[0]!, a[1]! - b[1]!, a[2]! - b[2]!);

  try {
    // The GPU the tab is given.
    const probe = await browser.newPage();
    await probe.goto(`${origin}/index.html?interface=off`);
    report.gpu = await probe.evaluate(`(async () => {
      const adapter = await navigator.gpu?.requestAdapter({ powerPreference: "high-performance" });
      if (!adapter) return null;
      const { vendor, architecture, device, description } = adapter.info ?? {};
      return { vendor, architecture, device, description, fallback: adapter.info?.isFallbackAdapter ?? adapter.isFallbackAdapter ?? false, format: navigator.gpu.getPreferredCanvasFormat() };
    })()`);
    await probe.close();

    // ---- the scene alone: the tab's frame of a held eye beside this machine's own, and the title card first
    const words = "view=60,30,40,107,10,14,62 blur=0";
    {
      const p = await visit(`?device=vita&interface=off&words=${encodeURIComponent(words)}`);
      await p.page.waitForTimeout(1200);
      const during = (await p.page.evaluate("[document.querySelectorAll('[aria-label=Pocket3D]').length, document.querySelector('[data-pocket-screen=upper]').hidden]")) as [number, boolean];
      await p.page.screenshot({ path: join(directory, "title-card.png") });
      expect(`the Pocket3D title card covers the page before the game is shown (${during})`, during[0] === 1 && during[1] === true);
      await p.page.waitForFunction("window.pocketManeuver?.firstWorld > 0", undefined, { timeout: 90_000 });
      await p.page.waitForTimeout(600);
      const afterwards = (await p.page.evaluate("[document.querySelectorAll('[aria-label=Pocket3D]').length, document.querySelector('[data-pocket-screen=upper]').hidden]")) as [number, boolean];
      expect(`the card has left and the game is shown (${afterwards})`, afterwards[0] === 0 && afterwards[1] === false);
      const tab = await p.save("scene-tab");
      // (the giants turn their heads and the autopilot flies behind the houses: both frames hold the same
      // view of the houses, and the tab's is some ticks later)
      await shot(join(directory, "scene-here.png"), ["--shape", "vita", "--frames", "60", "--words", words], deployed ? join(DIST, deployed.pack.manifest) : pack);
      report.sceneTabAgainstHere = await compare(tab, join(directory, "scene-here.png"));
      expect(`the tab's frame is this machine's (${JSON.stringify(report.sceneTabAgainstHere)})`, report.sceneTabAgainstHere.mean < 1.5);
      expect(`no error on the page (${p.problems.join("; ")})`, p.problems.length === 0);
      await p.page.close();
    }

    // ---- each device: from the title into play through its interface, the pause list opened and closed, and
    // what a frame costs with the interface over the scene
    const vita = [480, 272], ipod = [480, 320];
    const measure = async (p: Awaited<ReturnType<typeof visit>>) => {
      const read = `({ frames: pocketManeuver.frames, at: performance.now(), timing: { ...pocketManeuver.timing } })`;
      const from = (await p.page.evaluate(read)) as { frames: number; at: number; timing: Record<string, number> };
      await p.page.waitForTimeout(seconds * 1000);
      const to = (await p.page.evaluate(read)) as typeof from;
      const span = (to.at - from.at) / 1000;
      const d = (key: string) => to.timing[key]! - from.timing[key]!;
      const status = await p.status();
      const frameMs = (await p.page.evaluate("pocketManeuver.burst(300)")) as number;
      const round = (n: number) => +n.toFixed(3);
      return {
        fps: round((to.frames - from.frames) / span),
        // (300 frames made without waiting for the display: the simulation, the guest's turns and redraws, the scene)
        frameMs: round(frameMs),
        turnsPerSecond: round(d("turns") / span),
        turnMs: round(d("turnMs") / Math.max(1, d("turns"))),
        redrawsPerSecond: round(d("redraws") / span),
        // (a redraw: the UI core draws the interface once with its alpha, `drawMs`; the rest is the upload)
        redrawMs: round(d("redrawMs") / Math.max(1, d("redraws"))),
        drawMs: round(d("drawMs") / Math.max(1, d("redraws"))),
        lowersPerSecond: round(d("lowers") / span),
        lowerMs: round(d("lowerMs") / Math.max(1, d("lowers"))),
        world: status.world,
        actors: status.actors,
        late: status.late,
        shape: status.shape,
      };
    };
    /** Play by the keys of a device with a pad: the stick runs, a wire fires, the gas burns, the pause list opens and closes. */
    const played = async (p: Awaited<ReturnType<typeof visit>>, id: string) => {
      await p.up();
      await p.mode("title");
      await p.save(`${id}-title`);
      // The title's first row starts the run (the bottom-right face button chooses: ○ or A).
      await p.key("KeyZ");
      await p.mode("play");
      const before = await p.status();
      expect(`${id}: Start hands the gear over`, before.settings.auto === false && before.player.tick < 120);
      // The stick forward, then gas: the player runs along the wall and jumps.
      await p.page.keyboard.down("KeyW");
      await p.page.waitForTimeout(900);
      await p.page.keyboard.down("KeyX");
      await p.page.waitForTimeout(500);
      await p.page.keyboard.up("KeyX");
      await p.page.keyboard.up("KeyW");
      const ran = await p.status();
      expect(`${id}: the stick moves the player (${far(before.player.pos, ran.player.pos).toFixed(1)} m)`, far(before.player.pos, ran.player.pos) > 4);
      // Among the houses (the autopilot's first 600 ticks of the route, taken at once) both shoulder keys,
      // held: hooks fly from the hips.
      await p.page.evaluate(`pocketManeuver.maneuver.control("skip=600")`);
      await p.page.keyboard.down("KeyQ");
      await p.page.keyboard.down("KeyE");
      let wires = [0, 0];
      for (let i = 0; i < 12 && wires.every((s) => s === 0); i++) {
        await p.page.waitForTimeout(80);
        wires = (await p.status()).player.wires;
      }
      await p.page.waitForTimeout(300);
      await p.save(`${id}-play`);
      for (const code of ["KeyQ", "KeyE"]) await p.page.keyboard.up(code);
      expect(`${id}: the shoulder keys fire wires (${wires})`, wires.some((s: number) => s !== 0));
      // START pauses: no tick runs. The list's first row resumes.
      await p.key("Space");
      await p.mode("paused");
      const paused = await p.status();
      await p.page.waitForTimeout(400);
      expect(`${id}: a pause runs no tick`, (await p.status()).player.tick === paused.player.tick);
      await p.save(`${id}-paused`);
      await p.key("KeyZ");
      await p.mode("play");
    };
    report.devices = {};
    {
      // PS Vita: two sticks; the panel takes taps on a list.
      const p = await visit("?device=vita");
      await played(p, "vita");
      const before = await p.status();
      await p.page.keyboard.down("KeyL");
      await p.page.waitForTimeout(700);
      await p.page.keyboard.up("KeyL");
      const turned = await p.status();
      expect(`vita: the right stick turns the view (${before.camera.yaw.toFixed(2)} to ${turned.camera.yaw.toFixed(2)})`, Math.abs(turned.camera.yaw - before.camera.yaw) > 0.2);
      // The pause list under a finger: its first row (Resume).
      await p.key("Space");
      await p.mode("paused");
      await p.tap("upper", vita, 340, 59);
      await p.mode("play");
      // Sound: the page's output runs after the first key.
      report.sound = await p.page.evaluate("pocketManeuver.sound()");
      expect(`vita: the sound's output runs after a key (${report.sound})`, report.sound === "running");
      await p.page.evaluate(`pocketManeuver.maneuver.control("mode=play auto=1")`);
      report.devices.vita = await measure(p);
      expect(`vita: no error on the page (${p.problems.join("; ")})`, p.problems.length === 0);
      await p.page.close();
    }
    {
      // PSP: one stick; the direction pad turns the camera.
      const p = await visit("?device=psp");
      await played(p, "psp");
      const before = await p.status();
      await p.page.keyboard.down("ArrowRight");
      await p.page.waitForTimeout(700);
      await p.page.keyboard.up("ArrowRight");
      const turned = await p.status();
      expect(`psp: the direction pad turns the view (${before.camera.yaw.toFixed(2)} to ${turned.camera.yaw.toFixed(2)})`, Math.abs(turned.camera.yaw - before.camera.yaw) > 0.2);
      // A setting is kept between visits: the pause list, Settings, the first switch; then the page again.
      await p.key("Space");
      await p.mode("paused");
      await p.save("psp-list");
      report.kept = await p.page.evaluate(`localStorage.getItem("pocket-maneuver.interface")`);
      await p.key("KeyZ");
      await p.page.evaluate(`pocketManeuver.maneuver.control("mode=play auto=1")`);
      report.devices.psp = await measure(p);
      expect(`psp: no error on the page (${p.problems.join("; ")})`, p.problems.length === 0);
      await p.page.close();
    }
    {
      // Nintendo 3DS: the map and the lists are on the lower screen, under the pointer as under a stylus.
      const p = await visit("?device=3ds");
      await played(p, "3ds");
      const sizes = (await p.page.evaluate("[...document.querySelectorAll('[data-pocket-screen]')].map((c) => [c.width, c.height, c.hidden])")) as [number, number, boolean][];
      expect(`3ds: two screens (${JSON.stringify(sizes)})`, JSON.stringify(sizes) === "[[400,240,false],[320,240,false]]");
      await p.page.evaluate(`pocketManeuver.maneuver.control("mode=play auto=1")`);
      report.devices["3ds"] = await measure(p);
      expect(`3ds: no error on the page (${p.problems.join("; ")})`, p.problems.length === 0);
      await p.page.close();
    }
    {
      // iPod touch: no button at all. The pointer is a finger: the title's row, then the stick and a drag on the scene.
      const p = await visit("?device=ipod");
      await p.up();
      await p.mode("title");
      await p.save("ipod-title");
      report.ipodRows = await p.page.evaluate(`pocketManeuver.interface().plan.viewport.logical`);
      // (the rows of the title's list: found by tapping down the left column until the run starts)
      for (const y of [150, 170, 190, 210, 230]) {
        if ((await p.status()).mode !== "title") break;
        await p.tap("upper", ipod, 110, y);
      }
      await p.mode("play");
      await p.page.waitForTimeout(400);
      const before = await p.status();
      await p.save("ipod-play");
      // A finger on the scene turns the view.
      await p.drag("upper", ipod, [240, 110], [330, 110]);
      await p.page.waitForTimeout(500);
      const looked = await p.status();
      expect(`ipod: a finger dragged over the scene turns the view (${before.camera.yaw.toFixed(2)} to ${looked.camera.yaw.toFixed(2)})`, Math.abs(looked.camera.yaw - before.camera.yaw) > 0.15);
      // The stick in the lower left, pushed up: the player runs.
      await p.drag("upper", ipod, [70, 250], [70, 215], 1200);
      const ran = await p.status();
      expect(`ipod: the drawn stick moves the player (${far(looked.player.pos, ran.player.pos).toFixed(1)} m)`, far(looked.player.pos, ran.player.pos) > 2);
      await p.page.evaluate(`pocketManeuver.maneuver.control("mode=play auto=1")`);
      report.devices.ipod = await measure(p);
      expect(`ipod: no error on the page (${p.problems.join("; ")})`, p.problems.length === 0);
      await p.page.close();
    }

    // ---- another device in the middle of play, picked from the page's own text
    {
      const p = await visit("?device=vita");
      await p.up();
      await p.key("KeyZ");
      await p.mode("play");
      await p.page.evaluate(`pocketManeuver.maneuver.control("auto=1")`);
      await p.page.waitForTimeout(1500);
      const before = await p.status();
      await p.page.getByRole("button", { name: "Nintendo 3DS" }).click();
      await p.page.waitForFunction("pocketManeuver.device() === '3ds' && pocketManeuver.interface()", undefined, { timeout: 20_000 });
      await p.page.waitForTimeout(1500);
      const after = await p.status();
      expect(`a device picked in play keeps the run (${after.mode}, tick ${before.player.tick} to ${after.player.tick}) on its own screens (${after.shape.name})`, after.mode === "play" && after.player.tick > before.player.tick && after.shape.name === "3ds" && after.shape.width === 400);
      await p.save("switched-3ds");
      await p.page.screenshot({ path: join(directory, "page-3ds.png") });
      await p.page.getByRole("button", { name: "iPod touch" }).click();
      await p.page.waitForFunction("pocketManeuver.device() === 'ipod' && pocketManeuver.interface()", undefined, { timeout: 20_000 });
      await p.page.waitForTimeout(1200);
      const last = await p.status();
      expect("the next device too", last.mode === "play" && last.shape.name === "ipod");
      await p.save("switched-ipod");
      await p.page.screenshot({ path: join(directory, "page-ipod.png") });
      report.switched = { from: before.shape.name, to: [after.shape.name, last.shape.name], ticks: [before.player.tick, after.player.tick, last.player.tick] };
      expect(`no error on the page (${p.problems.join("; ")})`, p.problems.length === 0);
      await p.page.close();
    }

    // ---- the player around the game, in three windows: every device in its shell, the mark that says the
    // picture is simulated, the door to Pocket Studio. A browser whose pointer is a finger gets the iPod touch first.
    report.player = {};
    const WINDOWS = [
      { name: "1440", options: { viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 }, coarse: false },
      { name: "1280", options: { viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 }, coarse: false },
      { name: "phone", options: { viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, hasTouch: true, isMobile: true }, coarse: true },
    ];
    for (const window of WINDOWS) {
      const context = await browser.newContext(window.options);
      if (window.coarse) {
        const first = await visit("", context);
        await first.up();
        expect("a finger gets the iPod touch first", (await first.page.evaluate("pocketManeuver.device()")) === "ipod");
        await first.page.close();
      }
      for (const id of SHOWN) {
        const heard = opens.length;
        const p = await visit(`?device=${id}`, context);
        await p.up();
        const tag = `${window.name} ${id}`;
        // The host is told once that a player opened, as this device: the host here names the address in
        // /app.json; the deployable directory's names none, and its page tells no one.
        const told = opens.slice(heard);
        expect(`${tag}: the page tells the host once that a player opened (${JSON.stringify(told)})`, deployed ? told.length === 0 : told.length === 1 && told[0]!.query === `?app=local&layout=${id}` && told[0]!.mode === "no-cors");
        const seen = (await p.page.evaluate(`(async () => {
          const box = (selector) => { const el = document.querySelector(selector); if (!el || el.hidden) return null; const r = el.getBoundingClientRect(); return [r.left, r.top, r.right, r.bottom].map((n) => +n.toFixed(2)); };
          const art = document.querySelector("[data-pocket-shell-art]");
          await art.decode().catch(() => {});
          const covered = [...document.querySelectorAll("[data-pocket-screen]:not([hidden])")].map((canvas) => {
            const r = canvas.getBoundingClientRect();
            const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
            return top === canvas ? "" : (top?.outerHTML ?? "").slice(0, 80);
          });
          return { view: [innerWidth, innerHeight], bar: box("[data-pocket-bar]"), stage: box("[data-pocket-stage=root]"), dock: box("[data-pocket-studio]"), shell: box("[data-pocket-shell]"), covered,
            scroll: [document.documentElement.scrollWidth, document.documentElement.scrollHeight] };
        })()`)) as Record<string, any>;
        const { shell, stage, bar, dock } = seen;
        expect(`${tag}: the page is one window, with no scroll (${seen.scroll} in ${seen.view})`, seen.scroll[0] <= seen.view[0] && seen.scroll[1] <= seen.view[1]);
        expect(`${tag}: the shell is whole inside the stage, under the bar and over the dock (${JSON.stringify({ shell, stage, bar, dock })})`,
          shell[0] >= stage[0] - 0.5 && shell[2] <= stage[2] + 0.5 && shell[1] >= bar[3] - 0.5 && shell[3] <= dock[1] + 0.5 && shell[2] - shell[0] > 100);
        expect(`${tag}: nothing lies over a screen (${seen.covered})`, seen.covered.every((over: string) => over === ""));
        await p.page.screenshot({ path: join(directory, `player-${window.name}-${id}.png`) });

        // The mark beside the device's name: a pointer that rests on it, or a finger, shows what it says.
        const mark = (await p.page.locator("[data-pocket-mark]").first().boundingBox())!;
        if (window.coarse) await p.page.touchscreen.tap(mark.x + mark.width / 2, mark.y + mark.height / 2);
        else await p.page.mouse.move(mark.x + mark.width / 2, mark.y + mark.height / 2);
        await p.page.waitForTimeout(300);
        const said = (await p.page.evaluate(`(() => { const tip = document.getElementById("pocket-simulated"); return { hidden: tip.hidden, text: tip.textContent }; })()`)) as Record<string, any>;
        expect(`${tag}: the mark says the picture is simulated, and how this device's differs (${JSON.stringify(said)})`, said.hidden === false && said.text.includes("Your browser draws this picture") && said.text.includes("PS Vita build's"));
        if (window.name === "1440") await p.page.screenshot({ path: join(directory, `player-${id}-simulated.png`) });
        if (window.coarse) await p.page.touchscreen.tap(mark.x + mark.width / 2, mark.y + mark.height / 2);
        else await p.page.mouse.move(4, seen.view[1] / 2);

        // The door to Pocket Studio: the game's card there, and the Studio's own front door.
        const door = (await p.page.evaluate(`({ get: document.querySelector('[data-pocket-action=get]').href, make: document.querySelector('[data-pocket-action=make]').href, words: document.querySelector('[data-pocket-pitch]').textContent })`)) as Record<string, any>;
        report.player.door ??= door;
        const card = deployed ? (deployed.studio ? `${deployed.studio.server}/studio/?app=${deployed.studio.app}&from=player` : "https://studio.pocket.nexus/?from=player") : "https://studio.pocket.nexus/studio/?app=local&from=player";
        expect(`${tag}: the door leads to the game in Pocket Studio (${JSON.stringify(door)}, wanted ${card})`, door.get === card && door.words.includes("Pocket Maneuver is built for PSP, PS Vita, Nintendo 3DS and iPod touch."));

        // From the title into play by the shell's own key, which goes down while it is held; an iPod touch
        // has none: its screen is under the finger.
        if (id !== "ipod" && !window.coarse) {
          const key = (await p.page.locator("[data-pocket-control=circle]").first().boundingBox())!;
          await p.page.mouse.move(key.x + key.width / 2, key.y + key.height / 2);
          await p.page.mouse.down();
          await p.page.waitForTimeout(160);
          const down = (await p.page.evaluate(`document.querySelectorAll("[data-pocket-part][data-held]").length`)) as number;
          await p.page.mouse.up();
          expect(`${tag}: the shell's key goes down under the pointer (${down})`, down >= 1);
          await p.mode("play");
        }
        expect(`${tag}: no error on the page (${p.problems.join("; ")})`, p.problems.length === 0);
        await p.page.close();
      }
      await context.close();
    }

    // ---- what the first frame of the world needs: the page on a line of 16 Mbit/s
    {
      const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
      const cdp = await page.context().newCDPSession(page);
      await cdp.send("Network.enable");
      await cdp.send("Network.emulateNetworkConditions", { offline: false, latency: 20, downloadThroughput: 2_000_000, uploadThroughput: 1_000_000 });
      let received = 0;
      let counting = true;
      cdp.on("Network.dataReceived", (event: { encodedDataLength: number; dataLength: number }) => {
        if (counting) received += event.encodedDataLength || event.dataLength;
      });
      await page.goto(`${origin}/?device=vita`);
      await page.waitForFunction("window.pocketManeuver?.firstFrame > 0", undefined, { timeout: 60_000 });
      await page.waitForTimeout(3500);
      await page.screenshot({ path: join(directory, "reading-the-world.png") });
      await page.waitForFunction("window.pocketManeuver && (pocketManeuver.firstWorld || pocketManeuver.failure)", undefined, { timeout: 180_000 });
      counting = false;
      const first = (await page.evaluate("({ ready: pocketManeuver.ready, interfaceReady: pocketManeuver.interfaceReady, firstFrame: pocketManeuver.firstFrame, firstWorld: pocketManeuver.firstWorld, failure: pocketManeuver.failure })")) as Record<string, any>;
      await page.screenshot({ path: join(directory, "first-world-frame.png") });
      report.firstFrame = { bytesPerSecond: 2_000_000, bytesReceived: received, interfaceMs: Math.round(first.interfaceReady), firstFrameMs: Math.round(first.firstFrame), firstWorldFrameMs: Math.round(first.firstWorld), failure: first.failure };
      await page.close();
    }

    // ---- a browser without WebGPU is told so in one sentence, after the card
    {
      const without = await browser.newPage();
      await without.addInitScript("Object.defineProperty(Navigator.prototype, 'gpu', { get: undefined, configurable: true }); delete Navigator.prototype.gpu;");
      await without.goto(`${origin}/`);
      await without.waitForFunction("document.querySelector('[data-pocket-say]')?.textContent !== ''", undefined, { timeout: 20_000 });
      report.withoutWebGPU = await without.locator("[data-pocket-say]").textContent();
      expect(`a browser without WebGPU is told so ("${report.withoutWebGPU}")`, report.withoutWebGPU === "This browser has no WebGPU, which Pocket Maneuver draws with.");
      await without.close();
    }
  } finally {
    await browser.close();
    server.stop(true);
    report.sizes = sizes;
    report.opened = { reports: opens.length, last: opens.at(-1) ?? null };
    writeFileSync(join(directory, "report.json"), JSON.stringify(report, null, 1));
  }
  const { sizes: _, ...brief } = report;
  console.log(JSON.stringify({ ...brief, module: sizes["pkg/maneuver_wgpu_bg.wasm"], uiCore: sizes["pocketjs.wasm"] }, null, 1));
  console.log(directory);
} else if (import.meta.main) throw new Error("usage: cook | build | serve [--port N] [--dist] [--build] | dist [--piece MiB] | shot [--out PNG] [--shape NAME] [--size WxH] [--frames N] [--words WORDS] | check [--headed] [--seconds N] [--dist]");
