// The renderer's state as signals, and the way to command it.
import { createSignal, type Accessor } from "solid-js";
import { connectOverlay } from "@pocketjs/framework/overlay-host";
import type { Command, HostState } from "./protocol.ts";

/** Everything but the numbers that change in flight. */
type Slow = Omit<HostState, "t">;
/** One signal per member: a statistics line once a second must not re-run
 *  what reads the settings. */
type Signals = { [K in keyof Slow]: Accessor<Slow[K]> };

const initial: Slow = {
  mode: "loading", message: "", kills: 0, total: 0, alive: "", note: "", noteId: 0, result: [0, 0],
  options: [], stats: "", giants: "", prefs: "",
};

function same(a: unknown, b: unknown): boolean {
  return typeof a === "object" ? JSON.stringify(a) === JSON.stringify(b) : a === b;
}

export interface Host extends Signals {
  send(command: Command): void;
  /** True once the renderer has reported its state. */
  ready: Accessor<boolean>;
  /** The numbers in flight as they last arrived (`T` indexes them). */
  t: number[];
  /** Calls `listener` in the turn new numbers arrive. They change up to 30
   *  times a second, so a listener writes straight to its nodes
   *  (`@pocketjs/framework/hot`) instead of through a signal. */
  onNumbers(listener: (t: number[]) => void): void;
}

export function connectHost(): Host {
  const [ready, setReady] = createSignal(false);
  const set = {} as { [K in keyof Slow]: (value: Slow[K]) => void };
  const listeners: ((t: number[]) => void)[] = [];
  const host = { ready, t: [0, 100, 0, 0, 0, 0, 0], onNumbers: (listener) => void listeners.push(listener) } as Host;
  const last = { ...initial };
  for (const key of Object.keys(initial) as (keyof Slow)[]) {
    const [get, put] = createSignal<unknown>(initial[key], { equals: false });
    (host as unknown as Record<string, unknown>)[key] = get;
    (set as unknown as Record<string, unknown>)[key] = put;
  }
  const overlay = connectOverlay<Partial<HostState>, Command>((state) => {
    for (const key of Object.keys(state) as (keyof HostState)[]) {
      if (key === "t") {
        host.t = state.t!;
        for (const listener of listeners) listener(host.t);
      } else if (key in initial && !same(last[key], state[key])) {
        (last as Record<string, unknown>)[key] = state[key];
        (set[key] as (value: unknown) => void)(state[key]);
      }
    }
    setReady(true);
  });
  host.send = overlay.send;
  return host;
}
