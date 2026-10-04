// The static world in three.js: one mesh per bucket, shown by distance the
// way the device shows them (see `selectCells`).

import * as THREE from "three";
import { ATLAS_H, ATLAS_W } from "../world/atlas";
import { Bucket, CELL, Geo, Layer, Meshes, STRIDE, SUPER } from "../world/geo";
import { SCENE } from "../world/scene";

export function atlasTexture(rgba: Uint8ClampedArray): THREE.DataTexture {
  const t = new THREE.DataTexture(new Uint8Array(rgba.buffer), ATLAS_W, ATLAS_H, THREE.RGBAFormat);
  t.colorSpace = THREE.SRGBColorSpace;
  t.wrapS = THREE.RepeatWrapping;
  t.wrapT = THREE.ClampToEdgeWrapping;
  t.magFilter = THREE.LinearFilter;
  t.minFilter = THREE.LinearMipmapLinearFilter;
  t.generateMipmaps = true;
  t.anisotropy = 8;
  t.needsUpdate = true;
  return t;
}

const toLinear = (c: number) => Math.pow(c, 2.2);

/** A three.js geometry from a builder mesh; tints are sRGB multipliers. */
export function toGeometry(geo: Geo): THREE.BufferGeometry {
  const v = geo.vertices();
  const n = geo.nv;
  const pos = new Float32Array(n * 3);
  const nrm = new Float32Array(n * 3);
  const uv = new Float32Array(n * 2);
  const col = new Float32Array(n * 3);
  for (let i = 0; i < n; i++) {
    const o = i * STRIDE;
    pos.set(v.subarray(o, o + 3), i * 3);
    nrm.set(v.subarray(o + 3, o + 6), i * 3);
    uv[i * 2] = v[o + 6];
    uv[i * 2 + 1] = v[o + 7];
    col[i * 3] = toLinear(v[o + 8]);
    col[i * 3 + 1] = toLinear(v[o + 9]);
    col[i * 3 + 2] = toLinear(v[o + 10]);
  }
  const g = new THREE.BufferGeometry();
  g.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  g.setAttribute("normal", new THREE.BufferAttribute(nrm, 3));
  g.setAttribute("uv", new THREE.BufferAttribute(uv, 2));
  g.setAttribute("color", new THREE.BufferAttribute(col, 3));
  g.setIndex(new THREE.BufferAttribute(geo.indices().slice(), 1));
  g.computeBoundingBox();
  g.computeBoundingSphere();
  return g;
}

interface Cell {
  box: THREE.Box3;
  base?: THREE.Mesh;
  near?: THREE.Mesh;
  mid?: THREE.Mesh;
}
interface Super {
  box: THREE.Box3;
  far?: THREE.Mesh;
  cells: Cell[];
}

export interface Selection {
  draws: number;
  triangles: number;
}

export class WorldView {
  readonly group = new THREE.Group();
  readonly material: THREE.MeshLambertMaterial;
  private supers: Super[] = [];
  private backdrop: THREE.Mesh[] = [];

  constructor(meshes: Meshes, atlas: THREE.Texture) {
    this.material = new THREE.MeshLambertMaterial({ map: atlas, vertexColors: true });
    const cells = new Map<string, Cell>();
    const supers = new Map<string, Super>();
    const superOf = (x: number, z: number) => {
      const key = `${x}:${z}`;
      let s = supers.get(key);
      if (!s) {
        s = { box: new THREE.Box3(), cells: [] };
        supers.set(key, s);
      }
      return s;
    };
    const mesh = (b: Bucket) => {
      const m = new THREE.Mesh(toGeometry(b.geo), this.material);
      m.castShadow = b.layer !== Layer.Backdrop;
      m.receiveShadow = true;
      m.matrixAutoUpdate = false;
      this.group.add(m);
      return m;
    };
    for (const b of meshes.sorted()) {
      if (b.geo.ni === 0) continue;
      const m = mesh(b);
      if (b.layer === Layer.Backdrop) {
        m.frustumCulled = false;
        this.backdrop.push(m);
        continue;
      }
      if (b.layer === Layer.Far) {
        const s = superOf(b.cx, b.cz);
        s.far = m;
        s.box.union(m.geometry.boundingBox!);
        continue;
      }
      const key = `${b.cx}:${b.cz}`;
      let c = cells.get(key);
      if (!c) {
        c = { box: new THREE.Box3() };
        cells.set(key, c);
        const per = SUPER / CELL;
        superOf(Math.floor(b.cx / per), Math.floor(b.cz / per)).cells.push(c);
      }
      c.box.union(m.geometry.boundingBox!);
      if (b.layer === Layer.Base) c.base = m;
      else if (b.layer === Layer.Near) c.near = m;
      else c.mid = m;
    }
    for (const s of supers.values()) for (const c of s.cells) s.box.union(c.box);
    this.supers = [...supers.values()];
  }

  /**
   * Chooses what to draw from `eye`: far meshes for super-cells beyond the
   * middle distance, otherwise each cell's base with its near or middle mesh.
   */
  select(eye: THREE.Vector3, frustum: THREE.Frustum): Selection {
    const out = { draws: 0, triangles: 0 };
    const show = (m: THREE.Mesh | undefined, on: boolean, box: THREE.Box3) => {
      if (!m) return;
      m.visible = on;
      if (on && frustum.intersectsBox(box)) {
        out.draws++;
        out.triangles += m.geometry.index!.count / 3;
      }
    };
    for (const s of this.supers) {
      const far = s.box.distanceToPoint(eye) > SCENE.lod.mid;
      show(s.far, far, s.box);
      for (const c of s.cells) {
        const near = !far && c.box.distanceToPoint(eye) < SCENE.lod.near;
        show(c.base, !far, c.box);
        show(c.near, !far && near, c.box);
        show(c.mid, !far && !near, c.box);
      }
    }
    for (const m of this.backdrop) {
      out.draws++;
      out.triangles += m.geometry.index!.count / 3;
    }
    return out;
  }
}
