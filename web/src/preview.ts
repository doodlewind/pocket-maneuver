// Model preview: `/?model=scout|titan0|titan1|titan2&angle=30&pitch=10&dist=3&look=1`
// draws one model in its bind pose on a plain ground, for checking shapes.

import * as THREE from "three";
import { Skinned } from "./render/skinned";
import { BONES } from "./sim/abi.gen";
import { Sim } from "./sim/sim";
import { buildScout } from "./world/scout";
import { buildTitan } from "./world/titan";

export async function preview(q: URLSearchParams, canvas: HTMLCanvasElement) {
  const sim = await Sim.load(await fetch("/sim/maneuver_sim.wasm"), null, 0);
  const name = q.get("model")!;
  const t0 = performance.now();
  const model = name === "scout" ? buildScout(sim.bind(0)) : buildTitan(Number(name.slice(5)), sim.bind(1 + Number(name.slice(5))), Number(q.get("cell") ?? 1 / 100));
  const ms = performance.now() - t0;
  const w = Number(q.get("w") ?? 960);
  const h = Number(q.get("h") ?? 544);
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, preserveDrawingBuffer: true });
  renderer.setPixelRatio(1);
  renderer.setSize(w, h, false);
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x9fb4c8);
  const mat = new THREE.MeshLambertMaterial({ vertexColors: true });
  const skinned = new Skinned(model, mat);
  // The bind pose: every skin matrix is the identity.
  const id = new Float32Array(BONES * 12);
  for (let b = 0; b < BONES; b++) id[b * 12] = id[b * 12 + 4] = id[b * 12 + 8] = 1;
  skinned.skin(id);
  scene.add(skinned.mesh);
  const size = name === "scout" ? 1.74 : 1;
  const ground = new THREE.Mesh(new THREE.CircleGeometry(size * 2, 48), new THREE.MeshLambertMaterial({ color: 0x8c8f8a }));
  ground.rotation.x = -Math.PI / 2;
  ground.receiveShadow = true;
  scene.add(ground);
  const sun = new THREE.DirectionalLight(0xfff0d8, 2.6);
  sun.position.set(-size * 2, size * 3, -size * 2.5);
  sun.castShadow = true;
  sun.shadow.mapSize.set(2048, 2048);
  const sc = sun.shadow.camera;
  sc.left = sc.bottom = -size * 1.5;
  sc.right = sc.top = size * 1.5;
  sc.far = size * 10;
  scene.add(sun, new THREE.HemisphereLight(0xbcd0f0, 0x70645a, 1.6));
  const angle = (Number(q.get("angle") ?? 30) * Math.PI) / 180;
  const pitch = (Number(q.get("pitch") ?? 8) * Math.PI) / 180;
  const dist = Number(q.get("dist") ?? 2.6) * size;
  const look = Number(q.get("look") ?? 0.55) * size;
  const camera = new THREE.PerspectiveCamera(Number(q.get("fov") ?? 32), w / h, 0.02 * size, 100 * size);
  // Angle 0 looks at the figure's front (it faces -Z).
  camera.position.set(Math.sin(angle) * Math.cos(pitch) * dist, look + Math.sin(pitch) * dist, -Math.cos(angle) * Math.cos(pitch) * dist);
  camera.lookAt(0, look, 0);
  renderer.render(scene, camera);
  document.body.classList.add("shot");
  document.title = `shot-ready ${JSON.stringify({ model: name, vertices: model.v.length / 12, triangles: model.i.length / 3, buildMs: Math.round(ms) })}`;
}
