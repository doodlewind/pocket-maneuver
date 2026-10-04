// The simulation core (crates/maneuver-sim) as wasm. The reference never
// re-implements a rule: it feeds inputs and reads the snapshot.

import { ABI_VERSION, BONES, SNAP_LEN } from "./abi.gen";

interface Exports {
  memory: WebAssembly.Memory;
  mv_abi_version(): number;
  mv_alloc(len: number): number;
  mv_free(ptr: number, len: number): void;
  mv_load(ptr: number, len: number): number;
  mv_reset(): void;
  mv_tick(buttons: number, lx: number, ly: number, rx: number, ry: number): void;
  mv_tick_auto(): number;
  mv_snapshot(): number;
  mv_snapshot_len(): number;
  mv_dummies(): number;
  mv_raycast(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number, tmax: number): number;
  mv_ray_hit(): number;
  mv_audio(frames: number, rate: number): number;
  mv_titan(i: number): number;
  mv_bind(kind: number): number;
}

export class Sim {
  private constructor(
    private x: Exports,
    readonly dummyCount: number,
  ) {}

  static async load(wasm: ArrayBuffer | Response, world: Uint8Array | null, dummyCount: number): Promise<Sim> {
    const bytes = wasm instanceof Response ? await wasm.arrayBuffer() : wasm;
    const module = await WebAssembly.compile(bytes);
    const instance = await WebAssembly.instantiate(module, {});
    const x = instance.exports as unknown as Exports;
    if (x.mv_abi_version() !== ABI_VERSION || x.mv_snapshot_len() !== SNAP_LEN) throw new Error("simulation wasm does not match abi.gen.ts; run `bun tools/maneuver.ts sim`");
    if (world) {
      const ptr = x.mv_alloc(world.length);
      new Uint8Array(x.memory.buffer, ptr, world.length).set(world);
      const code = x.mv_load(ptr, world.length);
      x.mv_free(ptr, world.length);
      if (code !== 0) throw new Error(`world file rejected (${code})`);
    }
    return new Sim(x, dummyCount);
  }
  reset() {
    this.x.mv_reset();
  }
  tick(buttons: number, lx: number, ly: number, rx: number, ry: number) {
    this.x.mv_tick(buttons, lx, ly, rx, ry);
  }
  tickAuto(): number {
    return this.x.mv_tick_auto();
  }
  /** The current snapshot; valid until the next call. */
  snapshot(): Float32Array {
    return new Float32Array(this.x.memory.buffer, this.x.mv_snapshot(), SNAP_LEN);
  }
  /** Per target: alive, ticks since its cut. */
  dummies(): Float32Array {
    return new Float32Array(this.x.memory.buffer, this.x.mv_dummies(), this.dummyCount * 2);
  }
  /** Skin matrices of giant `i` at the current tick; valid until the next call. */
  titan(i: number): Float32Array {
    return new Float32Array(this.x.memory.buffer, this.x.mv_titan(i), BONES * 12);
  }
  /** A copy of the bind-pose bone transforms: kind 0 is the player, 1 to 3 the giants. */
  bind(kind: number): Float32Array {
    return new Float32Array(this.x.memory.buffer, this.x.mv_bind(kind), BONES * 12).slice();
  }
  /** Renders `frames` stereo frames of sound; the view is valid until the next call. */
  audio(frames: number, rate: number): Int16Array {
    return new Int16Array(this.x.memory.buffer, this.x.mv_audio(frames, rate), frames * 2);
  }
  raycast(o: readonly number[], d: readonly number[], tmax: number): number {
    return this.x.mv_raycast(o[0], o[1], o[2], d[0], d[1], d[2], tmax);
  }
}
