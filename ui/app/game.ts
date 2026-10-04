// The game as the interface sees it, on any device: which sheet is up, what
// its rows do, the note in the middle of the screen, and what is kept between
// runs (the best time and the settings).
import { createEffect, createMemo, createSignal, on, onCleanup, untrack, type Accessor } from "solid-js";
import { after } from "@pocketjs/framework/clock";
import { glyph, modality, surfaceHasTouch } from "@pocketjs/framework/modality";
import type { Host } from "./host.ts";
import type { Mode, Setting } from "./protocol.ts";

/** What the settings a renderer may offer are called. */
const LABELS: Record<string, string> = {
  invert: "Invert camera",
  sound: "Sound",
  bloom: "Bloom",
  rays: "Light shafts",
  blur: "Speed blur",
  govern: "Steady frame rate",
  stats: "Statistics",
};

/** A row of a sheet. */
export interface Row {
  label: string;
  /** A switch's state, or undefined for a row with a named value or none. */
  on?: boolean;
  /** The named value of a choice, or what stands at the right of the row. */
  value?: string;
  /** A row that only informs takes no press and no focus mark. */
  press?: () => void;
}

/** The sheet over the game: the mode's own list, or one opened from it. */
export type Sheet = "menu" | "settings" | "controls";

export interface Game {
  host: Host;
  mode: Accessor<Mode>;
  /** True while a list is up and the pad is the interface's. */
  listing: Accessor<boolean>;
  sheet: Accessor<Sheet>;
  /** The sheet's heading and rows. */
  heading: Accessor<string>;
  rows: Accessor<Row[]>;
  /** One level up: a sub-sheet closes; a paused game resumes. */
  back(): void;
  /** Whether `back` does anything here, and what it is called. */
  backLabel: Accessor<string>;
  pause(): void;
  /** The line for the middle of the screen, "" when none. */
  note: Accessor<string>;
  /** The best finished run, tenths of a second; 0 when none. */
  best: Accessor<number>;
  /** The finished run beat the best one. */
  record: Accessor<boolean>;
  /** Whether the renderer's `key` switch is on. */
  on(key: string): boolean;
}

/** True for `seconds` after each `show()`. */
export function createPulse(seconds: number): [Accessor<boolean>, () => void] {
  const [shown, setShown] = createSignal(false);
  let cancel: (() => void) | undefined;
  onCleanup(() => cancel?.());
  return [shown, () => {
    cancel?.();
    setShown(true);
    cancel = after(seconds, () => setShown(false));
  }];
}

/** "1:05.2" */
export function clock(tenths: number): string {
  const seconds = Math.floor(tenths / 10) % 60;
  return `${Math.floor(tenths / 600)}:${seconds < 10 ? "0" : ""}${seconds}.${tenths % 10}`;
}

/** What plays the game on this device, each control with what it does. */
export function controls(touch: boolean): [string, string][] {
  if (touch) {
    return [
      ["Stick", "Move"],
      ["Drag", "Turn the view"],
      ["L  R", "Wires; a tap keeps one"],
      ["GAS", "Jump, reel, thrust"],
      ["CUT", "Cut at the nape"],
      ["AIM", "Aimed pair of wires"],
      ["DROP", "Let go, dive"],
    ];
  }
  // A Vita has a second stick; a 3DS may have a C-Stick; a PSP turns the view with its direction pad.
  const view = modality.screens.length > 1 ? "D-pad, C-Stick" : surfaceHasTouch() ? "Right stick" : "D-pad";
  return [
    [`${glyph("ltrigger")} / ${glyph("rtrigger")}`, "Fire a wire, hold to keep"],
    [glyph("cross"), "Gas: jump, reel, thrust"],
    [glyph("square"), "Cut at the nape"],
    [glyph("triangle"), "Aimed pair of wires"],
    [glyph("circle"), "Let go, dive"],
    ["Stick", "Move"],
    [view, "Turn the view"],
    [glyph("start"), "Pause"],
  ];
}

interface Prefs {
  best?: number;
  options?: Record<string, number>;
}

export function createGame(host: Host, touch: boolean): Game {
  const [sheet, setSheet] = createSignal<Sheet>("menu");
  const [note, setNote] = createSignal("");
  const [prefs, setPrefs] = createSignal<Prefs>({});
  const [record, setRecord] = createSignal(false);
  const mode = host.mode;
  const listing = createMemo(() => mode() === "title" || mode() === "paused" || mode() === "results");

  // Each mode opens on its own list.
  createEffect(on(mode, () => setSheet("menu")));

  // A note stands for a moment, then leaves.
  let cancel: (() => void) | undefined;
  onCleanup(() => cancel?.());
  createEffect(on(host.noteId, () => {
    cancel?.();
    setNote(host.note());
    cancel = after(1.6, () => setNote(""));
  }, { defer: true }));

  // What the renderer stored for us last time: the best run, and the
  // settings, which it is told again once it has listed them.
  let restored = false;
  createEffect(() => {
    if (!host.ready() || restored || !host.options().length) return;
    restored = true;
    let stored: Prefs = {};
    try {
      stored = JSON.parse(untrack(host.prefs) || "{}") as Prefs;
    } catch {
      // A damaged file starts over.
    }
    setPrefs(stored);
    for (const setting of untrack(host.options)) {
      const value = stored.options?.[setting.key];
      if (typeof value === "number" && value !== setting.value) host.send({ type: "option", key: setting.key, value });
    }
  });
  const keep = (change: (prefs: Prefs) => Prefs) => {
    const next = change(prefs());
    setPrefs(next);
    host.send({ type: "prefs", value: JSON.stringify(next) });
  };

  // A finished run is the best one when it is the first or the fastest.
  createEffect(on(mode, (now) => {
    if (now !== "results") return;
    const time = untrack(host.result)[0];
    const best = untrack(prefs).best ?? 0;
    setRecord(time > 0 && (!best || time < best));
    if (untrack(record)) keep((p) => ({ ...p, best: time }));
  }, { defer: true }));

  const set = (setting: Setting, value: number) => {
    host.send({ type: "option", key: setting.key, value });
    keep((p) => ({ ...p, options: { ...p.options, [setting.key]: value } }));
  };
  const settings = (): Row[] =>
    host.options().map((setting): Row => {
      const label = LABELS[setting.key] ?? setting.key;
      const choices = setting.choices;
      if (!choices) return { label, on: !!setting.value, press: () => set(setting, setting.value ? 0 : 1) };
      return { label, value: choices[setting.value] ?? "", press: () => set(setting, (setting.value + 1) % choices.length) };
    });

  const rows = createMemo<Row[]>(() => {
    if (sheet() === "settings") return settings();
    if (sheet() === "controls") return controls(touch).map(([label, value]) => ({ label, value }));
    const more: Row[] = [
      { label: touch ? "How to play" : "Controls", press: () => setSheet("controls") },
      { label: "Settings", press: () => setSheet("settings") },
    ];
    if (mode() === "title") return [{ label: "Play", press: () => host.send({ type: "start" }) }, ...more];
    if (mode() === "paused") {
      return [
        { label: "Resume", press: () => host.send({ type: "pause", on: false }) },
        { label: "Restart", press: () => host.send({ type: "restart" }) },
        ...more,
        { label: "Quit to title", press: () => host.send({ type: "title" }) },
      ];
    }
    return [
      { label: "Again", press: () => host.send({ type: "restart" }) },
      { label: "Title", press: () => host.send({ type: "title" }) },
    ];
  });

  return {
    host, mode, listing, sheet, rows, note, record,
    heading: () => {
      if (sheet() === "settings") return "SETTINGS";
      if (sheet() === "controls") return touch ? "HOW TO PLAY" : "CONTROLS";
      return mode() === "paused" ? "PAUSED" : mode() === "results" ? "RUN COMPLETE" : "";
    },
    back() {
      if (sheet() !== "menu") setSheet("menu");
      else if (mode() === "paused") host.send({ type: "pause", on: false });
    },
    backLabel: () => (sheet() !== "menu" ? "back" : mode() === "paused" ? "resume" : ""),
    pause: () => host.send({ type: "pause", on: true }),
    best: () => prefs().best ?? 0,
    on: (key) => !!host.options().find((setting) => setting.key === key)?.value,
  };
}
