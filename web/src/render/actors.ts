// Everything that moves: the player, the giants, the cloak, the wires and
// the gas. All of it reads the simulation's snapshot.

import * as THREE from "three";
import { CLOAK_H, CLOAK_W, HOOK, ROPE_N, SNAP } from "../sim/abi.gen";
import { Sim } from "../sim/sim";
import { Entities } from "../world/city";
import { buildScout, CLOAK_COLOR } from "../world/scout";
import { SkinModel } from "../world/sdf";
import { buildTitan } from "../world/titan";
import { Skinned } from "./skinned";

const PUFFS = 96;
/** A giant nearer than this draws its detailed mesh. */
const TITAN_NEAR = 140;
const TITAN_FAR = 520;

export class Actors {
  readonly group = new THREE.Group();
  private scout: Skinned;
  private material = new THREE.MeshLambertMaterial({ vertexColors: true });
  private titanModels: { near: SkinModel; far: SkinModel }[] = [];
  private titans: { near?: Skinned; far?: Skinned }[] = [];
  private cloak: THREE.Mesh;
  private cloakPos = new Float32Array(CLOAK_W * CLOAK_H * 3);
  private cloakNrm = new Float32Array(CLOAK_W * CLOAK_H * 3);
  private wires: THREE.Mesh;
  private wirePos = new Float32Array(2 * ROPE_N * 2 * 3);
  private puffs: THREE.Points;
  private puffPos = new Float32Array(PUFFS * 3);
  private puffVel = new Float32Array(PUFFS * 3);
  private puffAge = new Float32Array(PUFFS).fill(9);
  private puffNext = 0;

  constructor(
    private sim: Sim,
    private entities: Entities,
  ) {
    this.scout = new Skinned(buildScout(sim.bind(0)), this.material);
    this.group.add(this.scout.mesh);
    for (let v = 0; v < 3; v++) {
      const bind = sim.bind(1 + v);
      this.titanModels.push({ near: buildTitan(v, bind, 1 / 100), far: buildTitan(v, bind, 1 / 36) });
    }
    this.titans = entities.dummies.map(() => ({}));

    // Cloak: a grid, both sides lit.
    const cg = new THREE.BufferGeometry();
    cg.setAttribute("position", new THREE.BufferAttribute(this.cloakPos, 3));
    cg.setAttribute("normal", new THREE.BufferAttribute(this.cloakNrm, 3));
    const ci: number[] = [];
    for (let r = 0; r + 1 < CLOAK_H; r++) {
      for (let c = 0; c + 1 < CLOAK_W; c++) {
        const a = r * CLOAK_W + c;
        ci.push(a, a + CLOAK_W, a + 1, a + 1, a + CLOAK_W, a + CLOAK_W + 1);
      }
    }
    cg.setIndex(ci);
    this.cloak = new THREE.Mesh(cg, new THREE.MeshLambertMaterial({ color: new THREE.Color().setRGB(CLOAK_COLOR[0], CLOAK_COLOR[1], CLOAK_COLOR[2], THREE.SRGBColorSpace), side: THREE.DoubleSide }));
    this.cloak.frustumCulled = false;
    this.cloak.castShadow = true;
    this.group.add(this.cloak);

    // Wires: a ribbon per rope, turned to the camera.
    const wg = new THREE.BufferGeometry();
    wg.setAttribute("position", new THREE.BufferAttribute(this.wirePos, 3));
    const wi: number[] = [];
    for (let r = 0; r < 2; r++) {
      for (let k = 0; k + 1 < ROPE_N; k++) {
        const a = (r * ROPE_N + k) * 2;
        wi.push(a, a + 1, a + 2, a + 2, a + 1, a + 3);
      }
    }
    wg.setIndex(wi);
    this.wires = new THREE.Mesh(wg, new THREE.MeshBasicMaterial({ color: 0x1b1b1e, side: THREE.DoubleSide }));
    this.wires.frustumCulled = false;
    this.group.add(this.wires);

    const pg = new THREE.BufferGeometry();
    pg.setAttribute("position", new THREE.BufferAttribute(this.puffPos, 3));
    this.puffs = new THREE.Points(pg, new THREE.PointsMaterial({ color: 0xf2f4f6, size: 0.55, transparent: true, opacity: 0.5, depthWrite: false, sizeAttenuation: true }));
    this.puffs.frustumCulled = false;
    this.group.add(this.puffs);
  }

  /** Triangles drawn for the player and the giants in the last update. */
  triangles = 0;

  /** Applies a snapshot. `dummies` holds, per giant, alive and ticks since its cut. */
  update(s: Float32Array, dummies: Float32Array, dt: number, eye: THREE.Vector3) {
    const tooClose = Math.hypot(s[SNAP.CAM_POS] - s[SNAP.POS], s[SNAP.CAM_POS + 1] - s[SNAP.POS + 1], s[SNAP.CAM_POS + 2] - s[SNAP.POS + 2]) < 1.7;
    this.scout.mesh.visible = !tooClose;
    this.cloak.visible = !tooClose;
    this.scout.skin(s.subarray(SNAP.SKIN, SNAP.SKIN + 19 * 12));
    this.triangles = this.scout.triangles;
    for (let i = 0; i < CLOAK_W * CLOAK_H; i++) {
      for (let k = 0; k < 3; k++) {
        this.cloakPos[i * 3 + k] = s[SNAP.CLOAK + i * 6 + k];
        this.cloakNrm[i * 3 + k] = s[SNAP.CLOAK + i * 6 + 3 + k];
      }
    }
    this.cloak.geometry.getAttribute("position").needsUpdate = true;
    this.cloak.geometry.getAttribute("normal").needsUpdate = true;

    // Wires: each point becomes two, offset across the line of sight; at least a pixel and a half wide.
    const w = this.wirePos;
    for (const [r, state, at] of [
      [0, SNAP.HOOK_L_STATE, SNAP.ROPE_L],
      [1, SNAP.HOOK_R_STATE, SNAP.ROPE_R],
    ] as const) {
      const out = s[state] !== HOOK.IDLE;
      for (let k = 0; k < ROPE_N; k++) {
        const p = at + k * 3;
        const a = at + Math.max(k - 1, 0) * 3;
        const b = at + Math.min(k + 1, ROPE_N - 1) * 3;
        const tx = s[b] - s[a];
        const ty = s[b + 1] - s[a + 1];
        const tz = s[b + 2] - s[a + 2];
        const ex = s[p] - eye.x;
        const ey = s[p + 1] - eye.y;
        const ez = s[p + 2] - eye.z;
        let sx = ty * ez - tz * ey;
        let sy = tz * ex - tx * ez;
        let sz = tx * ey - ty * ex;
        const l = Math.hypot(sx, sy, sz) || 1;
        const half = out ? Math.max(0.012, Math.hypot(ex, ey, ez) * 0.0011) : 0;
        sx = (sx / l) * half;
        sy = (sy / l) * half;
        sz = (sz / l) * half;
        const o = (r * ROPE_N + k) * 6;
        w[o] = s[p] - sx;
        w[o + 1] = s[p + 1] - sy;
        w[o + 2] = s[p + 2] - sz;
        w[o + 3] = s[p] + sx;
        w[o + 4] = s[p + 1] + sy;
        w[o + 5] = s[p + 2] + sz;
      }
    }
    this.wires.geometry.getAttribute("position").needsUpdate = true;

    // Giants: skinned while in range; the detailed mesh up close.
    this.entities.dummies.forEach((d, i) => {
      const t = this.titans[i];
      const alive = dummies[i * 2] > 0;
      const since = dummies[i * 2 + 1] / 60;
      const dist = Math.hypot(d.pos[0] - eye.x, d.pos[1] + d.height * 0.5 - eye.y, d.pos[2] - eye.z);
      const shown = dist < TITAN_FAR && (alive || since < 9);
      const near = shown && dist < TITAN_NEAR;
      if (t.near) t.near.mesh.visible = near;
      if (t.far) t.far.mesh.visible = shown && !near;
      if (!shown) return;
      const models = this.titanModels[i % 3];
      let m = near ? t.near : t.far;
      if (!m) {
        m = new Skinned(near ? models.near : models.far, this.material);
        this.group.add(m.mesh);
        if (near) t.near = m;
        else t.far = m;
      }
      m.skin(this.sim.titan(i));
      // A fallen giant sinks away.
      m.mesh.position.y = alive ? 0 : -Math.max(since - 5, 0) * d.height * 0.08;
      m.mesh.updateMatrix();
      this.triangles += m.triangles;
    });

    // Gas: puffs from the hips while thrusting or reeling.
    if (s[SNAP.THRUST] > 0 || s[SNAP.REEL] > 0) {
      for (const hip of [SNAP.HIP_L, SNAP.HIP_R]) {
        const i = this.puffNext++ % PUFFS;
        for (let k = 0; k < 3; k++) {
          this.puffPos[i * 3 + k] = s[hip + k];
          this.puffVel[i * 3 + k] = s[SNAP.VEL + k] * 0.35 + ((Math.sin(i * 12.9898 + k * 78.233) * 43758.5453) % 1) * 1.5;
        }
        this.puffAge[i] = 0;
      }
    }
    for (let i = 0; i < PUFFS; i++) {
      this.puffAge[i] += dt;
      if (this.puffAge[i] > 0.6) {
        this.puffPos[i * 3 + 1] = -1000;
        continue;
      }
      for (let k = 0; k < 3; k++) this.puffPos[i * 3 + k] += this.puffVel[i * 3 + k] * dt;
    }
    this.puffs.geometry.getAttribute("position").needsUpdate = true;
  }
}
