// The browser build (wgpu/, tools/wgpu.ts): the page, the renderer's shapes and the build tool name the
// same handhelds, the Pocket3D title card plays first, and the tab's icon is PocketJS's.
//   bun test ./tools/wgpu.test.ts          (part of `bun run test`; the page itself is checked in Chrome
//                                            by `bun tools/wgpu.ts check`)
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const ROOT = resolve(import.meta.dir, "..");
const read = (file: string) => readFileSync(resolve(ROOT, file), "utf8");
const page = read("wgpu/page/main.js");

test("the page, the shapes and the build tool name the same handhelds", () => {
  const devices = [...page.matchAll(/\{ id: "([a-z0-9]+)", label:/g)].map((m) => m[1]);
  const shapes = [...read("wgpu/src/app.rs").matchAll(/Shape \{ name: "([a-z0-9]+)", width:/g)].map((m) => m[1]);
  const shown = read("tools/wgpu.ts").match(/const SHOWN = \[([^\]]+)\]/)![1]!.split(",").map((name) => name.trim().replaceAll('"', ""));
  expect(devices).toEqual(["vita", "psp", "3ds", "ipod"]);
  expect(shapes).toEqual(devices);
  expect(shown).toEqual(devices);
});

test("the Pocket3D title card plays before the page shows its canvas", () => {
  const card = page.indexOf("titleCard(playTitle)");
  const shown = page.indexOf("canvas.hidden = false");
  expect(card).toBeGreaterThan(0);
  expect(page.indexOf("await title;", card)).toBeGreaterThan(card);
  expect(shown).toBeGreaterThan(page.lastIndexOf("await title;"));
});

test("the tab's icon is PocketJS's, and the page is the kernel's player", () => {
  expect(read("tools/wgpu.ts")).toContain('cpSync(POCKET3D_ICON.ios2x, join(SITE, "icon.png"))');
  expect(page).toContain('import { createPlayer } from "./pocket3d-player.js"');
  expect(read("wgpu/page/index.html")).toMatch(/<body>\s*<script type="module" src="main\.js"><\/script>\s*<\/body>/);
});

test("the renderer reads the PS Vita's pack and no other", () => {
  expect(read("wgpu/src/app.rs")).toContain('meta["profile"] != "vita60"');
  expect(read("tools/wgpu.ts")).toContain("cook --profile vita60");
});
