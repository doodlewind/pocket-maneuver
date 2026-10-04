// Keyboard and gamepad to the simulation's input: a button mask and two sticks.

import { BTN } from "../sim/abi.gen";

export interface Pad {
  buttons: number;
  lx: number;
  ly: number;
  rx: number;
  ry: number;
}

const KEYS: Record<string, number> = {
  KeyQ: BTN.HOOK_L,
  KeyE: BTN.HOOK_R,
  Space: BTN.GAS,
  KeyF: BTN.SLASH,
  KeyR: BTN.ZIP,
  KeyC: BTN.DROP,
  Backspace: BTN.RESET,
};

export class Input {
  private down = new Set<string>();
  private mouse = [0, 0];
  private mouseButtons = 0;

  constructor(canvas: HTMLCanvasElement) {
    window.addEventListener("keydown", (e: KeyboardEvent) => {
      this.down.add(e.code);
      if (e.code === "Space" || e.code.startsWith("Arrow")) e.preventDefault();
    });
    window.addEventListener("keyup", (e: KeyboardEvent) => this.down.delete(e.code));
    window.addEventListener("blur", () => this.down.clear());
    canvas.addEventListener("click", () => canvas.requestPointerLock?.());
    window.addEventListener("mousemove", (e: MouseEvent) => {
      if (document.pointerLockElement === canvas) {
        this.mouse[0] += e.movementX;
        this.mouse[1] += e.movementY;
      }
    });
    window.addEventListener("mousedown", (e: MouseEvent) => {
      if (document.pointerLockElement === canvas) this.mouseButtons |= e.button === 0 ? BTN.HOOK_L : e.button === 2 ? BTN.HOOK_R : 0;
    });
    window.addEventListener("mouseup", (e: MouseEvent) => {
      this.mouseButtons &= ~(e.button === 0 ? BTN.HOOK_L : e.button === 2 ? BTN.HOOK_R : 0);
    });
    window.addEventListener("contextmenu", (e: Event) => e.preventDefault());
  }

  /** One tick's input. Mouse motion since the last read becomes right-stick deflection. */
  read(): Pad {
    const k = (c: string) => (this.down.has(c) ? 1 : 0);
    let buttons = this.mouseButtons;
    for (const [code, bit] of Object.entries(KEYS)) if (this.down.has(code)) buttons |= bit;
    let lx = k("KeyD") - k("KeyA");
    let ly = k("KeyW") - k("KeyS");
    let rx = k("ArrowRight") - k("ArrowLeft") + this.mouse[0] * 0.09;
    let ry = k("ArrowUp") - k("ArrowDown") - this.mouse[1] * 0.09;
    this.mouse[0] = this.mouse[1] = 0;
    const pad = navigator.getGamepads?.().find((p) => p && p.mapping === "standard");
    if (pad) {
      const b = (i: number) => pad.buttons[i]?.pressed;
      if (b(4) || b(6)) buttons |= BTN.HOOK_L;
      if (b(5) || b(7)) buttons |= BTN.HOOK_R;
      if (b(0)) buttons |= BTN.GAS;
      if (b(2)) buttons |= BTN.SLASH;
      if (b(3)) buttons |= BTN.ZIP;
      if (b(1)) buttons |= BTN.DROP;
      if (b(8)) buttons |= BTN.RESET;
      lx += pad.axes[0];
      ly -= pad.axes[1];
      rx += pad.axes[2];
      ry -= pad.axes[3];
    }
    const clamp = (v: number) => Math.max(-1, Math.min(1, v));
    return { buttons, lx: clamp(lx), ly: clamp(ly), rx: clamp(rx), ry: clamp(ry) };
  }
}
