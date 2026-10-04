// Writes pictures of the interface on one device to .pocket-build/ui/preview/.
//   bun ui/test/preview.ts <psp|vita|3ds|ipod>     (after `bun tools/ui.ts <device>`)
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { BTN } from "../../vendor/pocketjs/contracts/spec/spec.ts";
import { boot, type Device } from "./harness.ts";

const device = (process.argv[2] ?? "psp") as Device;
const build = resolve(import.meta.dir, "../../.pocket-build/ui");
const out = join(build, "preview");
mkdirSync(out, { recursive: true });
const rig = await boot(device);
const scene = join(build, "backdrops/scene.png");
const names: string[] = [];
const save = async (name: string) => {
  const file = join(out, `${device}-${name}.png`);
  writeFileSync(file, (await rig.shot(scene)).toBuffer("image/png"));
  names.push(file);
};
const touch = device === "ipod";
/** Chooses row `index` of the list that is up: the d-pad and confirm, or a finger. */
const choose = (index: number, at: { x: number; y: number }) => {
  if (touch) return rig.tap(at.x, at.y);
  for (let i = 0; i < index; i++) rig.press(BTN.DOWN);
  rig.press(BTN.CIRCLE);
};

rig.mock.state.prefs = JSON.stringify({ best: 1843 });
rig.step(20);
await save("title");
choose(2, { x: 128, y: 258 });
rig.step(10);
await save("settings");
if (touch) rig.tap(100, 34);
else rig.press(BTN.CROSS);
rig.step(4);
choose(0, { x: 128, y: 154 });
rig.step(20);
await save("play-start");
rig.mock.fly(142, 64, 315, -120, 210, 300, 1);
rig.step(420);
await save("play");
rig.mock.fly(187, 12, 402, -60, 150, 320, 3);
rig.mock.cut(3, 187);
rig.step(12);
await save("play-cut");
if (touch) {
  // Both thumbs down: run forward, hold the gas.
  for (let i = 0; i < 12; i++) rig.step(1, { touch: [{ id: 1, x: 86, y: 234 - i * 3 }, { id: 2, x: 382, y: 262 }] });
  await save("play-thumbs");
  rig.step(4);
  rig.tap(32, 32);
} else rig.press(BTN.START);
rig.step(12);
await save("paused");
choose(2, { x: 340, y: 166 });
rig.step(10);
await save("controls");
if (touch) rig.tap(270, 34);
else rig.press(BTN.CROSS);
rig.step(4);
if (touch) rig.tap(270, 34);
else rig.press(BTN.CROSS);
rig.step(4);
for (let i = 0; i < 32; i++) if (rig.mock.state.alive[i] === "1") rig.mock.cut(i, 151);
rig.step(12);
await save("results");

const sheet = join(out, `sheet-${device}.png`);
Bun.spawnSync(["magick", "montage", ...names, "-tile", "4x", "-geometry", `${device === "vita" ? "50%x50%+4+4" : "+4+4"}`, "-background", "#333333", sheet]);
console.log(sheet);
console.log(JSON.stringify(rig.mock.log.filter((command) => command.type !== "drive" && command.type !== "look")));
