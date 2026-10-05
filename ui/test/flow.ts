// Drives one device's bundle through a run against the mock renderer and
// checks what it asked for.
//   bun ui/test/flow.ts <psp|vita|3ds|ipod>     (after `bun tools/ui.ts <device>`)
import assert from "node:assert/strict";
import { BTN } from "../../vendor/pocketjs/contracts/spec/spec.ts";
import { PLAY, type Command } from "../app/protocol.ts";
import { boot, type Device } from "./harness.ts";

const device = (process.argv[2] ?? "psp") as Device;
const rig = await boot(device);
const mock = rig.mock;
const touch = device === "ipod";
/** The commands since the last call, without the touch panel's streams. */
let read = 0;
const asked = (): Command[] => {
  const all = mock.log.slice(read).filter((command) => command.type !== "drive" && command.type !== "look" && command.type !== "idle");
  read = mock.log.length;
  return all;
};

// What was stored comes back: the settings are told again, the best run is kept.
mock.state.prefs = JSON.stringify({ best: 900, options: { invert: 1 } });
rig.step(10);
assert.deepEqual(asked(), [{ type: "option", key: "invert", value: 1 }]);
assert.equal(mock.state.options[0].value, 1);

// The title's first row starts the run.
if (touch) rig.tap(128, 154);
else rig.press(BTN.CIRCLE);
rig.step(4);
assert.deepEqual(asked(), [{ type: "start" }]);
assert.equal(mock.state.mode, "play");
// The hint is a timer pending; once it has left, nothing is scheduled.
assert.equal(mock.idle, false);
rig.step(400);
assert.equal(mock.idle, true);
mock.cut(0, 150);
rig.step(2);
assert.equal(mock.idle, false);
rig.step(120);
assert.equal(mock.idle, true);
mock.state.alive = "1".repeat(32);
mock.state.kills = 0;

if (touch) {
  // A thumb on the stick and one on the gas: the stick forward, the gas held.
  for (let i = 0; i < 8; i++) rig.step(1, { touch: [{ id: 1, x: 86, y: 234 - i * 6 }, { id: 2, x: 382, y: 262 }] });
  assert.equal(mock.drive.b, PLAY.gas);
  assert.ok(mock.drive.my > 60 && Math.abs(mock.drive.mx) < 10, `stick ${mock.drive.mx},${mock.drive.my}`);
  rig.step(2);
  assert.deepEqual(mock.drive, { mx: 0, my: 0, lx: 0, ly: 0, b: 0 });
  // A tap on a wire keeps it; the next tap lets it go.
  rig.tap(342, 178);
  assert.equal(mock.drive.b, PLAY.wireLeft);
  rig.tap(342, 178);
  assert.equal(mock.drive.b, 0);
  // A finger on the scene turns the view by what it travels.
  for (let i = 0; i < 6; i++) rig.step(1, { touch: [{ id: 3, x: 200 + i * 10, y: 120 }] });
  rig.step(2);
  assert.ok(mock.looked.dx >= 40, `looked ${mock.looked.dx}`);
  rig.tap(32, 32);
} else {
  // A button held through play is no press when the list comes up.
  rig.step(3, { buttons: BTN.CIRCLE });
  rig.step(2, { buttons: BTN.CIRCLE | BTN.START });
  rig.step(3, { buttons: BTN.CIRCLE });
  rig.step(2);
}
assert.deepEqual(asked(), [{ type: "pause", on: true }]);
assert.equal(mock.state.mode, "paused");

// Settings from the pause list: the second switch twice (the focus stays on
// it when it changes), then back twice resumes.
if (touch) {
  rig.tap(340, 210);
  rig.step(4);
  rig.tap(340, 122);
  rig.step(2);
  rig.tap(340, 122);
  rig.step(2);
  rig.tap(270, 34);
  rig.step(4);
  rig.tap(270, 34);
} else {
  for (let i = 0; i < 3; i++) rig.press(BTN.DOWN);
  rig.press(BTN.CIRCLE);
  rig.step(4);
  rig.press(BTN.DOWN);
  rig.press(BTN.CIRCLE);
  rig.press(BTN.CIRCLE);
  rig.press(BTN.CROSS);
  rig.step(4);
  rig.press(BTN.CROSS);
}
rig.step(4);
assert.deepEqual(asked(), [
  { type: "option", key: "sound", value: 0 },
  { type: "prefs", value: JSON.stringify({ best: 900, options: { invert: 1, sound: 0 } }) },
  { type: "option", key: "sound", value: 1 },
  { type: "prefs", value: JSON.stringify({ best: 900, options: { invert: 1, sound: 1 } }) },
  { type: "pause", on: false },
]);

// Every giant cut in 40.2 s beats the stored 1:30.0.
mock.fly(151, 40, 402, 0, 0, 0);
for (let i = 0; i < 32; i++) mock.cut(i, 151);
rig.step(6);
assert.equal(mock.state.mode, "results");
assert.deepEqual(asked(), [{ type: "prefs", value: JSON.stringify({ best: 402, options: { invert: 1, sound: 1 } }) }]);
if (touch) rig.tap(240, 250);
else {
  rig.press(BTN.DOWN);
  rig.press(BTN.CIRCLE);
}
rig.step(4);
assert.deepEqual(asked(), [{ type: "title" }]);
console.log(`${device}: the interface asked for what it should`);
