// What crosses between a device's renderer and the interface. The renderer
// owns the simulation, the scene and the marks anchored to the world (where
// a wire would bite, the nearest target); the interface owns every other 2D
// pixel and what a button or a finger means outside play. Both sides speak
// JSON lines over PocketJS's in-process overlay service: the renderer sends
// the members of its state that changed, the interface sends commands.
// `crates/maneuver-interface` is the renderer's side.
//
// The renderers parse commands with a small scanner, so commands are flat
// objects of numbers, booleans and short strings.

export type Mode = "loading" | "title" | "play" | "paused" | "results" | "error";

/** A switch (0 or 1), or with `choices` one of several named values. */
export interface Setting {
  key: string;
  value: number;
  choices?: string[];
}

export interface HostState {
  /** What the screen is for. Behind the title the autopilot flies the route. */
  mode: Mode;
  /** The loading step, or why `mode` is "error". */
  message: string;
  /** Giants cut, and how many there are. */
  kills: number;
  total: number;
  /** One character per giant: "1" while it stands. */
  alive: string;
  /** A line for the middle of the screen, and a number that changes with each one. */
  note: string;
  noteId: number;
  /** The finished run: tenths of a second, and the top speed in km/h. */
  result: number[];
  /** What the device lets the player set, in menu order. */
  options: Setting[];
  /** One line of renderer statistics while the `stats` setting is on. */
  stats: string;
  /** Where each giant stands, "x,z;…" in metres. */
  giants: string;
  /** What the interface last stored with `prefs`. */
  prefs: string;
  /** The numbers that change in flight, at most once a turn (see `T`). */
  t: number[];
}

/** Indices into `HostState.t`. */
export const T = {
  /** km/h */
  speed: 0,
  /** 0…100 */
  gas: 1,
  /** The run's clock, tenths of a second. */
  tenths: 2,
  /** The player on the map, metres. */
  x: 3,
  z: 4,
  /** Degrees clockwise from north. */
  heading: 5,
  /** Bit 0: the left wire holds; bit 1: the right one. */
  wires: 6,
} as const;

/** The simulation's buttons (`maneuver_sim::sim::btn`), for controls drawn on a touch panel. */
export const PLAY = { wireLeft: 1, wireRight: 2, gas: 4, cut: 8, aim: 16, drop: 32 } as const;

export type Command =
  /** Leave the title: the player takes the gear. */
  | { type: "start" }
  | { type: "pause"; on: boolean }
  /** The run again from the start. */
  | { type: "restart" }
  /** Back to the title and the autopilot. */
  | { type: "title" }
  | { type: "option"; key: string; value: number }
  /** Controls drawn on a touch panel: both sticks, -100…100 on each axis (move
   *  y is forward, look y is up), and the held buttons (`PLAY`). */
  | { type: "drive"; mx: number; my: number; lx: number; ly: number; b: number }
  /** A finger turning the view: logical px since the last command. */
  | { type: "look"; dx: number; dy: number }
  /** The interface has the pad (a sheet is open): play takes no input from it. */
  | { type: "hold"; on: boolean }
  | { type: "prefs"; value: string }
  /** The interface shows nothing just now: a renderer that lays it over the
   *  frame as a texture may skip that. */
  | { type: "quiet"; on: boolean };
