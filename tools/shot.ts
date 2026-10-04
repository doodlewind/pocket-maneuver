// Headless captures of the web reference.
//
//   bun tools/maneuver.ts shot --out a.png [--ticks N] [--auto] [--view px,py,pz,tx,ty,tz,fov] [--w 960 --h 544] [--query "k=v&..."]

import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { chromium } from "playwright-core";

const ROOT = resolve(import.meta.dir, "..");
const PORT = 5273;

async function up(): Promise<boolean> {
  try {
    return (await fetch(`http://127.0.0.1:${PORT}/`)).ok;
  } catch {
    return false;
  }
}

/** Starts the Vite server unless one already answers on the port. It keeps running for later shots. */
export async function serveWeb() {
  if (await up()) return;
  const child = spawn("bunx", ["vite", "--port", String(PORT), "--strictPort"], { cwd: join(ROOT, "web"), detached: true, stdio: "ignore" });
  child.unref();
  for (let i = 0; i < 100; i++) {
    if (await up()) return;
    await Bun.sleep(200);
  }
  throw new Error("vite did not start");
}

export async function shot(argv: string[]) {
  const arg = (name: string) => {
    const i = argv.indexOf(`--${name}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const out = resolve(arg("out") ?? join(ROOT, ".pocket-build/preview/shot.png"));
  const w = Number(arg("w") ?? 960);
  const h = Number(arg("h") ?? 544);
  const query = new URLSearchParams(arg("query") ?? "");
  query.set("shot", "1");
  query.set("w", String(w));
  query.set("h", String(h));
  if (arg("ticks")) query.set("ticks", arg("ticks")!);
  if (arg("view")) query.set("view", arg("view")!);
  if (argv.includes("--auto")) query.set("auto", "1");
  await serveWeb();
  const browser = await chromium.launch({ channel: "chrome", headless: true, args: ["--use-angle=metal", "--enable-gpu", "--ignore-gpu-blocklist"] });
  try {
    const page = await browser.newPage({ viewport: { width: w, height: h }, deviceScaleFactor: 1 });
    const errors: string[] = [];
    page.on("pageerror", (e: Error) => errors.push(String(e)));
    page.on("console", (m: { type(): string; text(): string }) => m.type() === "error" && errors.push(m.text()));
    await page.goto(`http://127.0.0.1:${PORT}/?${query}`);
    try {
      await page.waitForFunction(() => document.title.startsWith("shot-ready"), undefined, { timeout: 120_000 });
    } catch (e) {
      throw new Error(`no frame: ${errors.join(" | ") || e}`);
    }
    await mkdir(dirname(out), { recursive: true });
    await page.locator("#view").screenshot({ path: out });
    console.log(`${out}  ${(await page.title()).slice("shot-ready ".length)}`);
    if (errors.length) console.log("page errors:", errors.join(" | "));
  } finally {
    await browser.close();
  }
}
