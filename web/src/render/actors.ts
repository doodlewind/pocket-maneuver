// Everything that moves: the character's parts, the targets, the wires and
// the gas. All of it reads the simulation's snapshot.

import * as THREE from "three";
import { HOOK, PARTS, SNAP } from "../sim/abi.gen";
import { Entities } from "../world/city";
import { characterParts, targetModel } from "../world/models";
import { toGeometry } from "./worldview";

const PUFFS = 96;

export class Actors {
  readonly group = new THREE.Group();
  private parts: THREE.Mesh[] = [];
  private bodies: THREE.Mesh[] = [];
  private napes: THREE.Mesh[] = [];
  private wires: THREE.LineSegments;
  private wirePos = new Float32Array(12);
  private puffs: THREE.Points;
  private puffPos = new Float32Array(PUFFS * 3);
  private puffVel = new Float32Array(PUFFS * 3);
  private puffAge = new Float32Array(PUFFS).fill(9);
  private puffNext = 0;
  private fall = new Map<number, THREE.Vector3>();

  constructor(material: THREE.Material, entities: Entities) {
    for (const g of characterParts()) {
      const m = new THREE.Mesh(toGeometry(g), material);
      m.matrixAutoUpdate = false;
      m.castShadow = true;
      m.frustumCulled = false;
      this.parts.push(m);
      this.group.add(m);
    }
    const t = targetModel();
    const body = toGeometry(t.body);
    const nape = toGeometry(t.nape);
    for (const d of entities.dummies) {
      for (const [geo, list] of [
        [body, this.bodies],
        [nape, this.napes],
      ] as const) {
        const m = new THREE.Mesh(geo, material);
        m.position.set(d.pos[0], d.pos[1], d.pos[2]);
        m.rotation.y = d.yaw;
        m.scale.setScalar(d.height);
        m.castShadow = true;
        list.push(m);
        this.group.add(m);
      }
    }
    const wg = new THREE.BufferGeometry();
    wg.setAttribute("position", new THREE.BufferAttribute(this.wirePos, 3));
    this.wires = new THREE.LineSegments(wg, new THREE.LineBasicMaterial({ color: 0x1c1c1c }));
    this.wires.frustumCulled = false;
    this.group.add(this.wires);
    const pg = new THREE.BufferGeometry();
    pg.setAttribute("position", new THREE.BufferAttribute(this.puffPos, 3));
    this.puffs = new THREE.Points(pg, new THREE.PointsMaterial({ color: 0xf2f4f6, size: 0.55, transparent: true, opacity: 0.5, depthWrite: false, sizeAttenuation: true }));
    this.puffs.frustumCulled = false;
    this.group.add(this.puffs);
  }

  /** Applies a snapshot. `dummies` holds, per target, alive and ticks since its cut. */
  update(s: Float32Array, dummies: Float32Array, dt: number) {
    const e = new THREE.Matrix4();
    for (let i = 0; i < PARTS; i++) {
      const o = SNAP.PARTS_AT + i * 12;
      e.set(s[o], s[o + 3], s[o + 6], s[o + 9], s[o + 1], s[o + 4], s[o + 7], s[o + 10], s[o + 2], s[o + 5], s[o + 8], s[o + 11], 0, 0, 0, 1);
      this.parts[i].matrix.copy(e);
    }
    // Wires: hip to hook tip while a hook is out.
    const w = this.wirePos;
    for (const [i, st, tip, hip] of [
      [0, SNAP.HOOK_L_STATE, SNAP.HOOK_L_TIP, SNAP.HIP_L],
      [1, SNAP.HOOK_R_STATE, SNAP.HOOK_R_TIP, SNAP.HIP_R],
    ] as const) {
      const out = s[st] !== HOOK.IDLE;
      for (let k = 0; k < 3; k++) {
        w[i * 6 + k] = s[hip + k];
        w[i * 6 + 3 + k] = out ? s[tip + k] : s[hip + k];
      }
    }
    this.wires.geometry.getAttribute("position").needsUpdate = true;
    // Targets: a cut nape drops away.
    for (let i = 0; i < this.napes.length; i++) {
      const alive = dummies[i * 2] > 0;
      const since = dummies[i * 2 + 1];
      const n = this.napes[i];
      if (alive) {
        this.fall.delete(i);
        n.visible = true;
        n.position.copy(this.bodies[i].position);
      } else {
        const t = since / 60;
        n.visible = t < 2.5;
        n.position.copy(this.bodies[i].position);
        n.position.y -= 4.9 * t * t;
        n.rotation.x = t * 4;
      }
    }
    // Gas: puffs from the hips while thrusting or reeling.
    if (s[SNAP.THRUST] > 0 || s[SNAP.REEL] > 0) {
      for (const hip of [SNAP.HIP_L, SNAP.HIP_R]) {
        const i = this.puffNext++ % PUFFS;
        for (let k = 0; k < 3; k++) {
          this.puffPos[i * 3 + k] = s[hip + k];
          this.puffVel[i * 3 + k] = s[SNAP.VEL + k] * 0.35 + (Math.sin(i * 12.9898 + k * 78.233) * 43758.5453 % 1) * 1.5;
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
