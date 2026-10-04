// Pocket Maneuver, three.js reference.
//
//   /                      play (keyboard, mouse or a gamepad)
//   /?auto                 the autopilot flies the route
//   /?shot&ticks=N[&auto]  run N ticks, draw one frame, then set the title to `shot-ready {...}`
//   /?view=px,py,pz,tx,ty,tz,fov   a fixed camera (with `shot`)

import * as THREE from "three";
import { Hud } from "./game/hud";
import { Input } from "./game/input";
import { Actors } from "./render/actors";
import { makeSky } from "./render/sky";
import { atlasTexture, WorldView } from "./render/worldview";
import { EV, SNAP } from "./sim/abi.gen";
import { Sim } from "./sim/sim";
import { paintAtlas } from "./world/atlas";
import { generate } from "./world/city";
import { SCENE } from "./world/scene";
import { writeWorldFile } from "./world/worldfile";

const q = new URLSearchParams(location.search);
const seed = Number(q.get("seed") ?? 2026);
const shot = q.has("shot");
const auto = q.has("auto");
const fixed = q.get("view")?.split(",").map(Number);

const canvas = document.querySelector<HTMLCanvasElement>("#view")!;
const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, preserveDrawingBuffer: shot });
renderer.outputColorSpace = THREE.SRGBColorSpace;
renderer.shadowMap.enabled = true;
renderer.shadowMap.type = THREE.PCFSoftShadowMap;

const t0 = performance.now();
const gen = generate(seed);
const atlas = atlasTexture(paintAtlas(seed));
const world = writeWorldFile(gen.world.col, gen.entities);
const sim = await Sim.load(await fetch("/sim/maneuver_sim.wasm"), world, gen.entities.dummies.length);
const buildMs = performance.now() - t0;

const scene = new THREE.Scene();
scene.fog = new THREE.FogExp2(new THREE.Color().setRGB(...SCENE.fog), SCENE.fogDensity);
const view = new WorldView(gen.world.meshes, atlas);
scene.add(view.group);
const actors = new Actors(view.material, gen.entities);
scene.add(actors.group);
const sky = makeSky();
scene.add(sky);

const sun = new THREE.DirectionalLight(new THREE.Color().setRGB(...SCENE.sun), Math.PI);
sun.castShadow = true;
sun.shadow.mapSize.set(4096, 4096);
sun.shadow.bias = -0.0004;
sun.shadow.normalBias = 0.25;
const sc = sun.shadow.camera;
sc.left = sc.bottom = -210;
sc.right = sc.top = 210;
sc.near = 1;
sc.far = 1200;
scene.add(sun, sun.target);
scene.add(new THREE.HemisphereLight(new THREE.Color().setRGB(...SCENE.sky), new THREE.Color().setRGB(...SCENE.bounce), Math.PI));

const camera = new THREE.PerspectiveCamera(62, 16 / 9, SCENE.clip.near, SCENE.clip.far);
const hud = new Hud(document.querySelector("#hud")!);
const input = new Input(canvas);

function resize() {
  const w = shot ? Number(q.get("w") ?? 960) : innerWidth;
  const h = shot ? Number(q.get("h") ?? 544) : innerHeight;
  renderer.setPixelRatio(shot ? 1 : Math.min(devicePixelRatio, 2));
  renderer.setSize(w, h, !shot);
  camera.aspect = w / h;
}
window.addEventListener("resize", resize);
resize();

const frustum = new THREE.Frustum();
const pv = new THREE.Matrix4();
const sunDir = new THREE.Vector3(...SCENE.sunDir);
let stats = { draws: 0, triangles: 0 };

function events(s: Float32Array) {
  const e = s[SNAP.EVENTS];
  if (e & EV.SLASH_HIT) hud.say(`CUT  ${Math.round(s[SNAP.LAST_CUT_SPEED] * 3.6)} km/h`);
  else if (e & EV.SLASH_WEAK) hud.say("TOO SLOW", 1);
  if (e & EV.REFILL) hud.say("GAS REFILLED", 1.2);
  if (e & EV.RUN_DONE) hud.say("ALL TARGETS CUT", 5);
}

function draw(dt: number) {
  const s = sim.snapshot();
  if (fixed && fixed.length >= 6) {
    camera.position.set(fixed[0], fixed[1], fixed[2]);
    camera.up.set(0, 1, 0);
    camera.lookAt(fixed[3], fixed[4], fixed[5]);
    camera.fov = fixed[6] ?? 62;
  } else {
    const shake = s[SNAP.CAM_SHAKE] * 0.25;
    const t = s[SNAP.TICK];
    camera.position.set(s[SNAP.CAM_POS] + Math.sin(t * 1.7) * shake, s[SNAP.CAM_POS + 1] + Math.sin(t * 2.3) * shake, s[SNAP.CAM_POS + 2] + Math.cos(t * 1.9) * shake);
    camera.up.set(0, 1, 0);
    camera.lookAt(camera.position.x + s[SNAP.CAM_LOOK], camera.position.y + s[SNAP.CAM_LOOK + 1], camera.position.z + s[SNAP.CAM_LOOK + 2]);
    camera.rotateZ(s[SNAP.CAM_ROLL]);
    camera.fov = s[SNAP.CAM_FOV];
  }
  camera.updateProjectionMatrix();
  camera.updateMatrixWorld();
  pv.multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse);
  frustum.setFromProjectionMatrix(pv);
  stats = view.select(camera.position, frustum);
  actors.update(s, sim.dummies(), dt);
  sky.position.copy(camera.position);
  // The shadow map follows the eye in steps, so its texels do not swim.
  const step = 420 / 4096;
  const cx = Math.round(camera.position.x / step / 64) * step * 64;
  const cz = Math.round(camera.position.z / step / 64) * step * 64;
  sun.target.position.set(cx, 0, cz);
  sun.position.copy(sun.target.position).addScaledVector(sunDir, 600);
  renderer.render(scene, camera);
  hud.update(s, camera);
  events(s);
}

function tick() {
  if (auto) sim.tickAuto();
  else {
    const p = input.read();
    sim.tick(p.buttons, p.lx, p.ly, p.rx, p.ry);
  }
}

/** Sound starts on the first key, click or pad press: browsers require a gesture. */
function startAudio() {
  const ctx = new AudioContext();
  const node = ctx.createScriptProcessor(1024, 0, 2);
  node.onaudioprocess = (e) => {
    const pcm = sim.audio(1024, ctx.sampleRate);
    const l = e.outputBuffer.getChannelData(0);
    const r = e.outputBuffer.getChannelData(1);
    for (let i = 0; i < 1024; i++) {
      l[i] = pcm[i * 2] / 32768;
      r[i] = pcm[i * 2 + 1] / 32768;
    }
  };
  node.connect(ctx.destination);
}

if (shot) {
  document.body.classList.add("shot");
  const n = Number(q.get("ticks") ?? 1);
  for (let i = 0; i < n; i++) tick();
  draw(1 / 60);
  draw(1 / 60);
  const s = sim.snapshot();
  document.title = `shot-ready ${JSON.stringify({ ...stats, calls: renderer.info.render.calls, buildMs: Math.round(buildMs), pos: [...s.subarray(SNAP.POS, SNAP.POS + 3)].map((v) => Math.round(v)), speed: Math.round(s[SNAP.SPEED]) })}`;
} else {
  for (const type of ["keydown", "pointerdown"]) window.addEventListener(type, startAudio, { once: true });
  let last = performance.now();
  let acc = 0;
  const frame = (now: number) => {
    const dt = Math.min((now - last) / 1000, 0.1);
    last = now;
    acc += dt;
    while (acc >= 1 / 60) {
      tick();
      acc -= 1 / 60;
    }
    draw(dt);
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}
