// The app icon on every console is the Pocket3D icon from PocketJS
// (vendor/pocketjs/engine/pocket3d/icon/). This repository holds no icon file,
// and every build names the PocketJS one.
//   bun test ./tools/icon.test.ts          (part of `bun run test`)
import { expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { POCKET3D_ICON } from "../vendor/pocketjs/tools/pocket3d-icon.ts";

const ROOT = resolve(import.meta.dir, "..");
const read = (file: string) => readFileSync(resolve(ROOT, file), "utf8");

/** File names a launcher reads its icon from, or that carry one. */
const LAUNCHER_ICONS = [
  /(^|\/)icon0\.png$/i, // PSP ICON0.PNG, PS Vita sce_sys/icon0.png
  /^n3ds\/(.*\/)?icon[^/]*\.png$/i, // the two sizes smdhtool takes
  /\.smdh$/i, // a built SMDH holds both
  /^ipod\/(.*\/)?icon[^/]*\.png$/i, // Icon.png and Icon@2x.png of the bundle
];

test("the repository holds no launcher icon outside vendor/", () => {
  // Tracked files and new ones Git would add; ignored build output (the iPod bundle's copies) is not listed.
  const listed = Bun.spawnSync(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], { cwd: ROOT });
  expect(listed.exitCode).toBe(0);
  const files = listed.stdout.toString().split("\0").filter((file) => file && !file.startsWith("vendor/"));
  expect(files.length).toBeGreaterThan(0);
  expect(files.filter((file) => LAUNCHER_ICONS.some((name) => name.test(file)))).toEqual([]);
});

test("PocketJS holds the icon of every console this game ships for", () => {
  for (const file of Object.values(POCKET3D_ICON)) expect(existsSync(file), file).toBe(true);
});

test("PSP: Psp.toml and the EBOOT packer name PocketJS's ICON0.PNG", () => {
  const icon = read("psp/Psp.toml").match(/^xmb_icon_png\s*=\s*"([^"]+)"/m)?.[1];
  expect(icon).toBeDefined();
  expect(resolve(ROOT, "psp", icon!)).toBe(POCKET3D_ICON.psp);
  // pack-pbp's third argument is ICON0.PNG.
  expect(read("tools/psp.ts")).toMatch(/pack-pbp \$\{out\} \$\{sfo\} \$\{POCKET3D_ICON\.psp\} /);
});

test("PS Vita: the VPK packager is given PocketJS's icon0.png", () => {
  const calls = read("tools/vita.ts").match(/packageVitaVpk\(\{.*\}\)/g) ?? [];
  expect(calls.length).toBeGreaterThan(0);
  for (const call of calls) expect(call).toMatch(/\bicon: POCKET3D_ICON\.vita\b/);
});

test("Nintendo 3DS: smdhtool is given both of PocketJS's sizes", () => {
  const makefile = read("n3ds/Makefile");
  expect(makefile).toMatch(/^ICON := \$\(ROOT\)\/vendor\/pocketjs\/engine\/pocket3d\/icon\/3ds\/icon\.png$/m);
  expect(makefile).toMatch(/^SMALL_ICON := \$\(ROOT\)\/vendor\/pocketjs\/engine\/pocket3d\/icon\/3ds\/icon-small\.png$/m);
  // The large icon, the output, then the small icon.
  expect(makefile).toMatch(/smdhtool --create .* \$\(ICON\) \$@ \$\(SMALL_ICON\)$/m);
});

test("iPod touch: the bundle gets PocketJS's two files, and SpringBoard adds no gloss", () => {
  const tool = read("tools/ipod.ts");
  expect(tool).toContain(`cpSync(POCKET3D_ICON.ios, join(bundle, "Icon.png"))`);
  expect(tool).toContain(`cpSync(POCKET3D_ICON.ios2x, join(bundle, "Icon@2x.png"))`);
  expect(tool).toContain(`UIPrerenderedIcon: "<true/>"`);
});
