// A skinned model in three.js, deformed on the CPU from the simulation's
// skin matrices: two bones per vertex, as the device does in its vertex program.

import * as THREE from "three";
import { SKIN_STRIDE, SkinModel } from "../world/sdf";

const toLinear = (c: number) => Math.pow(c, 2.2);

export class Skinned {
  readonly mesh: THREE.Mesh;
  private pos: Float32Array;
  private nrm: Float32Array;

  constructor(
    private model: SkinModel,
    material: THREE.Material,
  ) {
    const n = model.v.length / SKIN_STRIDE;
    this.pos = new Float32Array(n * 3);
    this.nrm = new Float32Array(n * 3);
    const col = new Float32Array(n * 3);
    for (let i = 0; i < n; i++) {
      const o = i * SKIN_STRIDE;
      this.pos.set(model.v.subarray(o, o + 3), i * 3);
      this.nrm.set(model.v.subarray(o + 3, o + 6), i * 3);
      for (let c = 0; c < 3; c++) col[i * 3 + c] = toLinear(model.v[o + 6 + c]);
    }
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(this.pos, 3));
    g.setAttribute("normal", new THREE.BufferAttribute(this.nrm, 3));
    g.setAttribute("color", new THREE.BufferAttribute(col, 3));
    g.setIndex(new THREE.BufferAttribute(model.i, 1));
    this.mesh = new THREE.Mesh(g, material);
    this.mesh.frustumCulled = false;
    this.mesh.castShadow = true;
    this.mesh.matrixAutoUpdate = false;
  }

  get triangles(): number {
    return this.model.i.length / 3;
  }

  /** Places every vertex with `mats`: twelve floats per bone (three rotation columns, then translation). */
  skin(mats: Float32Array) {
    const v = this.model.v;
    const n = v.length / SKIN_STRIDE;
    const p = this.pos;
    const q = this.nrm;
    for (let i = 0; i < n; i++) {
      const o = i * SKIN_STRIDE;
      const x = v[o];
      const y = v[o + 1];
      const z = v[o + 2];
      const nx = v[o + 3];
      const ny = v[o + 4];
      const nz = v[o + 5];
      const a = v[o + 9] * 12;
      const b = v[o + 10] * 12;
      const wa = v[o + 11];
      const wb = 1 - wa;
      let px = (mats[a] * x + mats[a + 3] * y + mats[a + 6] * z + mats[a + 9]) * wa;
      let py = (mats[a + 1] * x + mats[a + 4] * y + mats[a + 7] * z + mats[a + 10]) * wa;
      let pz = (mats[a + 2] * x + mats[a + 5] * y + mats[a + 8] * z + mats[a + 11]) * wa;
      let mx = (mats[a] * nx + mats[a + 3] * ny + mats[a + 6] * nz) * wa;
      let my = (mats[a + 1] * nx + mats[a + 4] * ny + mats[a + 7] * nz) * wa;
      let mz = (mats[a + 2] * nx + mats[a + 5] * ny + mats[a + 8] * nz) * wa;
      if (wb > 0) {
        px += (mats[b] * x + mats[b + 3] * y + mats[b + 6] * z + mats[b + 9]) * wb;
        py += (mats[b + 1] * x + mats[b + 4] * y + mats[b + 7] * z + mats[b + 10]) * wb;
        pz += (mats[b + 2] * x + mats[b + 5] * y + mats[b + 8] * z + mats[b + 11]) * wb;
        mx += (mats[b] * nx + mats[b + 3] * ny + mats[b + 6] * nz) * wb;
        my += (mats[b + 1] * nx + mats[b + 4] * ny + mats[b + 7] * nz) * wb;
        mz += (mats[b + 2] * nx + mats[b + 5] * ny + mats[b + 8] * nz) * wb;
      }
      const l = Math.hypot(mx, my, mz) || 1;
      p[i * 3] = px;
      p[i * 3 + 1] = py;
      p[i * 3 + 2] = pz;
      q[i * 3] = mx / l;
      q[i * 3 + 1] = my / l;
      q[i * 3 + 2] = mz / l;
    }
    const g = this.mesh.geometry;
    g.getAttribute("position").needsUpdate = true;
    g.getAttribute("normal").needsUpdate = true;
  }
}
