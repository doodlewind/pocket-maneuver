// The reference's heads-up display, in DOM. The device draws the same
// elements from its own font atlas.

import * as THREE from "three";
import { SNAP, TUNE } from "../sim/abi.gen";

export class Hud {
  private gas: HTMLElement;
  private speed: HTMLElement;
  private score: HTMLElement;
  private time: HTMLElement;
  private note: HTMLElement;
  private marks: HTMLElement[] = [];
  private noteUntil = 0;

  constructor(private root: HTMLElement) {
    root.innerHTML = `
      <div class="gauge"><div class="label">GAS</div><div class="bar"><div class="fill" id="gas"></div></div></div>
      <div class="speed"><span id="speed">0</span><small>km/h</small></div>
      <div class="score"><span id="score">0 / 0</span><small>targets</small><span id="time">0:00.0</span></div>
      <div class="note" id="note"></div>
      <div class="mark l"></div><div class="mark r"></div><div class="mark z"></div>
      <div class="help">L1/Q · R1/E wires &nbsp; ✕/Space gas &nbsp; □/F cut &nbsp; △/R aimed wires &nbsp; ○/C let go</div>`;
    this.gas = root.querySelector("#gas")!;
    this.speed = root.querySelector("#speed")!;
    this.score = root.querySelector("#score")!;
    this.time = root.querySelector("#time")!;
    this.note = root.querySelector("#note")!;
    this.marks = [...root.querySelectorAll<HTMLElement>(".mark")];
  }

  say(text: string, seconds = 1.6) {
    this.note.textContent = text;
    this.note.classList.add("on");
    this.noteUntil = performance.now() + seconds * 1000;
  }

  update(s: Float32Array, camera: THREE.Camera) {
    this.gas.style.width = `${(s[SNAP.GAS] / TUNE.GAS_MAX) * 100}%`;
    this.gas.classList.toggle("low", s[SNAP.GAS] < 20);
    this.speed.textContent = String(Math.round(s[SNAP.SPEED] * 3.6));
    this.score.textContent = `${s[SNAP.RUN_KILLS]} / ${s[SNAP.RUN_TOTAL]}`;
    const t = s[SNAP.RUN_TICKS] / 60;
    this.time.textContent = `${Math.floor(t / 60)}:${(t % 60).toFixed(1).padStart(4, "0")}`;
    if (performance.now() > this.noteUntil) this.note.classList.remove("on");
    const v = new THREE.Vector3();
    [SNAP.RET_L, SNAP.RET_R, SNAP.RET_ZIP].forEach((at, i) => {
      const m = this.marks[i];
      v.set(s[at + 1], s[at + 2], s[at + 3]).project(camera);
      const on = s[at] > 0 && v.z < 1 && Math.abs(v.x) < 1 && Math.abs(v.y) < 1;
      m.style.display = on ? "block" : "none";
      if (on) {
        m.style.left = `${(v.x * 0.5 + 0.5) * this.root.clientWidth}px`;
        m.style.top = `${(-v.y * 0.5 + 0.5) * this.root.clientHeight}px`;
      }
    });
  }
}
