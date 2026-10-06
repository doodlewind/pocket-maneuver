// Pocket Maneuver in a browser tab: the wgpu renderer (../src, built to pkg/)
// and the game's own interface (../../ui, one bundle a device, compiled by
// tools/ui.ts), which a realm of the page runs on PocketJS's UI core and the
// renderer lays over the scene. The page itself is the Pocket3D player of
// PocketJS's browser kernel (vendor/pocketjs/devices/web/pocket-web-wgpu,
// staged beside this file): the bar, the device's shell with its keys, the
// way to Pocket Studio. This file says what the game is and draws into the
// player's canvas.
//
// The page shows one of the handhelds the game runs on: its shell, its
// screens, its presentation of the interface, its buttons. The Pocket3D title
// card plays first; the interface and the pack are read while it plays.
//
// The address chooses the device and the pack:
//
//   ?device=vita|psp|3ds|ipod   the handheld (without it: an iPod touch for a finger, a PS Vita otherwise)
//   ?size=960x544               the first device's screen, changed
//   ?pack=URL                   the pack (the PS Vita's): its file, on a server that answers byte ranges,
//                               or the manifest (.json) of one cut into pieces. Without it, the page's
//                               own (<meta name="pocket-pack">)
//   ?words=mode=play+auto=1     what a development host would send the game (App::control)
//   ?interface=off              the scene alone, played by the device's pad: no guest is started
//
// `window.pocketManeuver` is the running game, for a console and for tools/wgpu.ts.
import { playTitle } from "./pocket3d-title.js";
import { frames, hasWebGPU, titleCard } from "./pocket3d-shell.js";
import { openInterface, screens } from "./pocket3d-interface.js";
import { createPlayer } from "./pocket3d-player.js";
import init, { Maneuver, shapes } from "./pkg/maneuver_wgpu.js";

// The handhelds: each one's screen is the renderer's shape of the same name, its interface the bundle
// under ui/<id>/, and `sticks` says how it is played (README, "Controls"): two sticks, one stick with
// the direction pad for the camera, or the interface's own stick and keys on a touch panel. `note` is
// what the player says beside the device's name: how this picture differs from the one that device's
// own build draws (README, the table of devices and "Compile").
const DEVICES = [
  { id: "vita", label: "PS Vita", sticks: 2, note: { en: "This page draws the PS Vita build's own world and passes: its houses and models, its bloom, light shafts and grade. A PS Vita compiles its own programs for them.", ja: "このページは PS Vita 版そのものの世界と描画パスで描いています。家やモデル、ブルーム、光の筋、色調補正もそのままです。PS Vita はそのためのシェーダーを本体でコンパイルします。" } },
  { id: "psp", label: "PSP", sticks: 1, note: { en: "On a PSP the picture has 16-bit colour with dither, the models have about an eighth of the triangles, and there is no bloom and no light shafts. This page draws the PS Vita build's world at the PSP's size.", ja: "PSP では画面がディザ付きの 16 ビットカラーになり、モデルのポリゴン数は約 8 分の 1 で、ブルームと光の筋はありません。このページは PS Vita 版の世界を PSP の画面サイズで描いています。" } },
  { id: "3ds", label: "Nintendo 3DS", sticks: 1, note: { en: "On a 3DS the models have about a sixth of the triangles, and there is no bloom and no light shafts. This page draws the PS Vita build's world at the 3DS's size.", ja: "3DS ではモデルのポリゴン数が約 6 分の 1 になり、ブルームと光の筋はありません。このページは PS Vita 版の世界を 3DS の画面サイズで描いています。" } },
  { id: "ipod", label: "iPod touch", sticks: 0, note: { en: "On an iPod touch 4 the models have about an eighth of the triangles, and there is no bloom and no light shafts. This page draws the PS Vita build's world at the iPod touch's size.", ja: "iPod touch 4 ではモデルのポリゴン数が約 8 分の 1 になり、ブルームと光の筋はありません。このページは PS Vita 版の世界を iPod touch の画面サイズで描いています。" } },
];
// Where the interface's best time and settings are kept between visits (a device keeps them in a file).
const KEPT = "pocket-maneuver.interface";

const query = new URLSearchParams(location.search);
const beside = (name) => new URL(name, import.meta.url).href;
const message = (error) => String(error?.message ?? error);
const coarse = matchMedia("(pointer: coarse)").matches;
const started = performance.now();

// The page: PocketJS's player, with the device the address asks for, or an iPod touch under a finger.
const wanted = query.get("device") ?? query.get("shape");
let device = DEVICES.find((d) => d.id === wanted) ?? DEVICES.find((d) => d.id === (coarse ? "ipod" : "vita"));
let present = () => {};
const player = createPlayer({
  title: "Pocket Maneuver",
  tagline: { en: "Two wires, a tank of gas, a walled town.", ja: "ワイヤー二本と、ガスのタンクと、城壁に囲まれた町。" },
  devices: DEVICES,
  device: device.id,
  // (the targets `bun tools/release.ts` builds a package for)
  runsOn: ["psp", "vita", "3ds", "ipod-touch"],
  pick: (id) => present(DEVICES.find((d) => d.id === id)),
});
const { canvas, stage, controls } = player;
const say = (text) => player.say(text);

function kept() {
  try {
    return localStorage.getItem(KEPT) ?? "";
  } catch {
    return "";
  }
}

async function start() {
  // The card is the first picture of every launch, and covers the page while the world is read. No frame
  // is drawn while it plays.
  const title = titleCard(playTitle);
  if (!hasWebGPU()) {
    await title;
    say({ en: "This browser has no WebGPU, which Pocket Maneuver draws with.", ja: "このブラウザは WebGPU に対応していません。Pocket Maneuver の描画には WebGPU が必要です。" });
    return;
  }
  await init();
  const all = JSON.parse(shapes());

  // The shell, on the first device's screen. It draws before there is a world: the interface says how
  // much of the pack has arrived.
  const [width, height] = (query.get("size") ?? "").split("x").map((n) => Number.parseInt(n, 10) || 0);
  const first = all.find((s) => s.name === device.id);
  stage.show({ device: device.id, width: width || first.width, height: height || first.height });
  const maneuver = await Maneuver.open(canvas, first.name, kept());
  let shape = JSON.parse(maneuver.reshape(first.name, canvas.width, canvas.height));
  // (the flow waits at the title for a guest: one is on its way)
  maneuver.interface_opened();

  // What a frame's parts cost, in milliseconds summed since the start: the guest's turns, the redraws of
  // its picture (`redrawMs`: the UI core's drawing, `drawMs` of it, then the upload), and the second
  // screen's redraws.
  const timing = { turns: 0, turnMs: 0, redraws: 0, redrawMs: 0, drawMs: 0, lowers: 0, lowerMs: 0 };
  // (milliseconds from the page's start: the pack read, the interface up, the first frame, the first of the world)
  const report = { maneuver, device: () => device.id, shape: () => shape, ready: 0, interfaceReady: 0, firstFrame: 0, firstWorld: 0, frames: 0, failure: "", timing };
  window.pocketManeuver = report;

  // The world, read beside everything else.
  const pack = query.get("pack") ?? document.querySelector('meta[name="pocket-pack"]').content;
  maneuver.reader().read(new URL(pack, location.href).href).then((world) => {
    maneuver.fly(world);
    if (query.get("words")) maneuver.control(query.get("words"));
    report.ready = performance.now() - started;
  }).catch((error) => {
    report.failure = message(error);
    maneuver.fail(report.failure);
    if (!ui) say(report.failure);
  });

  // A device on the page: its screens, its controls, and its presentation of the interface in a new realm.
  let ui = null;
  let lower = null;
  present = async (next, sized) => {
    device = next;
    const plan = await (await fetch(beside(`ui/${next.id}/plan.json`))).json();
    if (device !== next) return;
    const of = screens(plan);
    const to = sized ?? all.find((s) => s.name === next.id);
    // The device's shell with its screens in it, its keys as the controls, and the screen that takes touch.
    player.show(next.id, { width: to.width, height: to.height, lower: of.auxiliary, sticks: next.sticks, glyphs: of.glyphs, touch: of.touch, viewport: of.viewport });
    shape = JSON.parse(maneuver.reshape(to.name, to.width, to.height));
    lower = of.auxiliary ? new ImageData(of.auxiliary[0], of.auxiliary[1]) : null;
    // The guest of the device before goes with its realm; the new one is told the whole state on its first turn.
    ui?.close();
    ui = null;
    maneuver.overlay_hide();
    if (query.get("interface") === "off") return maneuver.interface_closed();
    try {
      // (`simHz`: the turns a second the device's own host gives its interface)
      const opened = await openInterface({ realm: beside("app-instance.html"), wasm: beside("pocketjs.wasm"), bundle: beside(`ui/${next.id}/maneuver.js`), pak: beside(`ui/${next.id}/maneuver.pak`), plan, simHz: shape.turns });
      // (another device was chosen while this one's interface was read)
      if (device !== next) return opened.close();
      maneuver.interface_opened();
      ui = opened;
      report.interfaceReady ||= performance.now() - started;
    } catch (error) {
      // Without its interface the game is played by the pad alone: START and SELECT keep the flow.
      report.failure = message(error);
      maneuver.interface_closed();
    }
  };
  const presented = present(device, { ...shape });

  // Sound starts on the first key or pointer: a browser asks for a gesture. The synthesizer is the
  // simulation crate's, as on every device; the page pulls it a block at a time.
  let sound = null;
  const startSound = () => {
    if (sound || query.get("sound") === "off") return;
    try {
      sound = new AudioContext();
      const node = sound.createScriptProcessor(1024, 0, 2);
      node.onaudioprocess = (event) => {
        const pcm = maneuver.audio(1024, sound.sampleRate);
        const [left, right] = [event.outputBuffer.getChannelData(0), event.outputBuffer.getChannelData(1)];
        for (let i = 0; i < 1024; i++) {
          left[i] = pcm[i * 2] / 32768;
          right[i] = pcm[i * 2 + 1] / 32768;
        }
      };
      node.connect(sound.destination);
    } catch {
      // A browser without an audio output plays the game without sound.
    }
  };
  for (const type of ["keydown", "pointerdown"]) addEventListener(type, startSound, { once: true, capture: true });

  // One frame: the simulation, the guest's turn when it is worth one, its picture when that has changed, the scene.
  const frame = (now) => {
    const held = controls.read();
    maneuver.step(now, held.buttons, held.left[0], held.left[1], held.right[0], held.right[1]);
    // (a turn is offered as often as the device offers one, and is that many sixtieths of a second)
    const ticks = ui !== null ? maneuver.guest_due(held.touching) : 0;
    const turned = ticks > 0;
    if (turned) {
      const from = performance.now();
      const line = maneuver.heard();
      if (line) ui.send(line);
      ui.turn(maneuver.guest_buttons(), held.contacts, ticks);
      for (const said of ui.drain()) maneuver.say(said);
      const turnedAt = performance.now();
      if (ui.changed()) {
        // One drawing by the UI core, with its alpha, and the upload.
        const picture = ui.picture();
        const drew = performance.now();
        maneuver.overlay(picture.pixels, picture.width, picture.height);
        timing.redraws++;
        timing.drawMs += drew - turnedAt;
        timing.redrawMs += performance.now() - turnedAt;
      }
      const drawn = performance.now();
      if (lower && ui.lowerChanged()) {
        // The second screen is the interface's alone: its pixels go to its canvas as they are.
        lower.data.set(ui.lower());
        stage.lower.putImageData(lower, 0, 0);
        timing.lowers++;
        timing.lowerMs += performance.now() - drawn;
      }
      timing.turns++;
      timing.turnMs += turnedAt - from;
    }
    controls.next(turned);
    const settings = maneuver.prefs_take();
    if (settings) {
      try {
        localStorage.setItem(KEPT, settings);
      } catch {
        // A browser that keeps nothing starts from the defaults next time.
      }
    }
    maneuver.draw();
    report.frames++;
    report.firstFrame ||= performance.now() - started;
    if (!report.firstWorld && maneuver.flies()) {
      report.firstWorld = performance.now() - started;
      // (the world is on the screen: the player may read what it kept back)
      player.ready();
    }
  };

  await title;
  canvas.hidden = false;
  stage.fit();
  const loop = frames(() => 60, (now) => {
    try {
      frame(now);
    } catch (error) {
      // A frame the canvas had no texture for is skipped; anything else stops the game and says why.
      report.failure = message(error);
      if (!/Outdated|Lost|Timeout/.test(report.failure)) {
        loop.stop();
        say(report.failure);
      }
    }
  });

  // For a console and for tools/wgpu.ts.
  report.presented = () => presented;
  // Another device while the game runs: `pocketManeuver.present("psp")`.
  report.present = (id) => present(DEVICES.find((d) => d.id === id));
  report.interface = () => ui;
  report.sound = () => sound?.state ?? "none";
  // The frame as the canvases hold it, one pixel of the scene to a pixel: PNGs as data URLs.
  report.capture = () => {
    frame(performance.now());
    return { upper: canvas.toDataURL("image/png"), lower: stage.second.hidden ? null : stage.second.toDataURL("image/png") };
  };
  // A held game for a picture (tools/listing.ts): the display's loop stops, `words` go to the game
  // (App::control), `count` frames of a sixtieth of a second are made one after another, and the screens
  // come back as the last of them left them.
  report.take = (words, count) => {
    loop.stop();
    maneuver.control(words);
    const from = performance.now();
    for (let i = 0; i < count; i++) frame(from + (i * 1000) / 60);
    return { upper: canvas.toDataURL("image/png"), lower: stage.second.hidden ? null : stage.second.toDataURL("image/png") };
  };
  // Milliseconds a frame costs the processor and the GPU together, over `count` frames made without
  // waiting for the display: the simulation, the guest's turns and redraws, the scene.
  report.burst = async (count = 300) => {
    const gpu = canvas.getContext("webgpu").getConfiguration().device;
    const from = performance.now();
    for (let i = 0; i < count; i++) frame(from + ((i + 1) * 1000) / 60);
    await gpu.queue.onSubmittedWorkDone();
    return (performance.now() - from) / count;
  };
}

start().catch((error) => {
  window.pocketManeuver = { failure: message(error) };
  say(window.pocketManeuver.failure);
});
