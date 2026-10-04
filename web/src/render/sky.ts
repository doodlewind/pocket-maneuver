// Sky dome: a gradient from the horizon to the zenith with a glow around the
// sun. The device draws the same function per vertex of a coarse dome.

import * as THREE from "three";
import { SCENE } from "../world/scene";

/** Sky radiance (linear RGB) toward the unit direction `d`. */
export function skyColor(d: readonly [number, number, number]): [number, number, number] {
  const h = Math.max(d[1], 0);
  const k = 1 - Math.pow(1 - h, 3.2);
  const s = Math.max(d[0] * SCENE.sunDir[0] + d[1] * SCENE.sunDir[1] + d[2] * SCENE.sunDir[2], 0);
  const glow = 0.42 * Math.pow(s, 6) + 0.9 * Math.pow(s, 90);
  const below = Math.min(Math.max(-d[1] * 6, 0), 1);
  const out: [number, number, number] = [0, 0, 0];
  for (let i = 0; i < 3; i++) {
    const sky = SCENE.horizon[i] + (SCENE.zenith[i] - SCENE.horizon[i]) * k + SCENE.glow[i] * glow;
    out[i] = sky + (SCENE.fog[i] - sky) * below;
  }
  return out;
}

export function makeSky(): THREE.Mesh {
  const geo = new THREE.SphereGeometry(1, 48, 24);
  const pos = geo.getAttribute("position");
  const col = new Float32Array(pos.count * 3);
  for (let i = 0; i < pos.count; i++) col.set(skyColor([pos.getX(i), pos.getY(i), pos.getZ(i)]), i * 3);
  geo.setAttribute("color", new THREE.BufferAttribute(col, 3));
  const mat = new THREE.MeshBasicMaterial({ vertexColors: true, side: THREE.BackSide, depthWrite: false, depthTest: false, fog: false });
  const m = new THREE.Mesh(geo, mat);
  m.renderOrder = -10;
  m.frustumCulled = false;
  m.scale.setScalar(1000);
  return m;
}
