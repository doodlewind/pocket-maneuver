// The pieces every presentation is built from. A device chooses where they
// go and how large they are; what the gas gauge, a list row or the map looks
// like is decided here once.
//
// The numbers in flight (speed, gas, the clock, the player on the map) change
// up to 30 times a second. Each is written straight to its node
// (`@pocketjs/framework/hot`) in a cell of fixed size, so a new value costs
// one native call and no layout. Everything else is a signal.
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import { useActions, type ActionsHandle } from "@pocketjs/framework/actions";
import { Image, Text, View } from "@pocketjs/framework/components";
import type { SurfaceId } from "@pocketjs/framework/display";
import { createGesture } from "@pocketjs/framework/gesture";
import { prop as hotProp, text as hotText } from "@pocketjs/framework/hot";
import { BTN } from "@pocketjs/framework/input";
import { onFrame } from "@pocketjs/framework/lifecycle";
import { glyph, modality } from "@pocketjs/framework/modality";
import type { NodeMirror } from "@pocketjs/framework/renderer";
import { clock, type Game, type Row } from "./game.ts";
import { MAP_EXTENT, MAP_IMAGE } from "./generated/map.ts";
import type { Host } from "./host.ts";
import { T } from "./protocol.ts";
import { ALERT, AMBER, DIM, FAINT, GLASS, HAIRLINE, INK, NIGHT, PANEL, TARGET, tint, WASH } from "./theme.ts";

/**
 * A view a finger can tap, on the surface it is drawn on. It stays out of
 * the focus order: a pad has its own button for the same verb.
 */
export function Touchable(props: { surface?: SurfaceId; onTap?: () => void; class?: string; style?: Record<string, number | string>; children?: JSX.Element }) {
  let node: NodeMirror | undefined;
  const [down, setDown] = createSignal(false);
  createGesture({
    surface: props.surface,
    region: { node: () => node },
    onDown: () => setDown(!!props.onTap),
    onUp: () => setDown(false),
    onCancel: () => setDown(false),
    onTap: () => props.onTap?.(),
  });
  return <View ref={node} class={props.class} style={{ ...props.style, opacity: down() ? 0.55 : 1 }}>{props.children}</View>;
}

/** A button for a finger: at least as tall as a fingertip on its surface. */
export function Button(props: { label: string; width: number; height: number; strong?: boolean; surface?: SurfaceId; onPress: () => void }) {
  return (
    <Touchable surface={props.surface} class="rounded-lg items-center justify-center" style={{ width: props.width, height: props.height, bgColor: props.strong ? AMBER : "#0c1016b0", borderWidth: 1, borderColor: props.strong ? AMBER : HAIRLINE }} onTap={props.onPress}>
      <Text class="text-sm font-bold" style={{ textColor: props.strong ? NIGHT : INK }}>{props.label}</Text>
    </Touchable>
  );
}

/** A pill of text over the scene. */
export function Chip(props: { text: string; color?: string }) {
  return (
    <View class="rounded px-2 py-1" style={{ bgColor: "#0c1016b0" }}>
      <Text class="text-xs font-bold" style={{ textColor: props.color ?? INK }}>{props.text}</Text>
    </View>
  );
}

/** The game's name, as large as the surface has room for. */
export function Wordmark(props: { large?: boolean }) {
  return (
    <View class="flex-col">
      <Text class="text-sm font-bold tracking-wide" style={{ textColor: AMBER }}>POCKET</Text>
      <Show when={props.large} fallback={<Text class="text-2xl font-bold" style={{ textColor: INK }}>MANEUVER</Text>}>
        <Text class="text-4xl font-bold" style={{ textColor: INK }}>MANEUVER</Text>
      </Show>
    </View>
  );
}

/** Dark glass a little larger than a gauge, behind it: white type stays
 *  legible over a bright sky or a plaster wall. */
function Plate(props: { width: number; height: number }) {
  return <View class="absolute rounded" style={{ insetL: -6, insetT: -4, width: props.width + 12, height: props.height + 8, bgColor: "#0a0e1459" }} />;
}

/** The tank: a bar that empties to the left, orange under a fifth, and a lamp
 *  for each wire that holds. */
export function Gauge(props: { host: Host; width: number }) {
  let fill: NodeMirror | undefined, left: NodeMirror | undefined, right: NodeMirror | undefined;
  const [low, setLow] = createSignal(false);
  onMount(() => {
    let gas = -1, wires = -1;
    onCleanup(props.host.onNumbers((t) => {
      if (t[T.gas] !== gas) {
        gas = t[T.gas];
        hotProp(fill, "scaleX", gas / 100);
        if (gas < 20 !== low()) setLow(gas < 20);
      }
      if (t[T.wires] !== wires) {
        wires = t[T.wires];
        hotProp(left, "opacity", wires & 1 ? 1 : 0.25);
        hotProp(right, "opacity", wires & 2 ? 1 : 0.25);
      }
    }));
  });
  return (
    <View class="relative" style={{ width: props.width, height: 26 }}>
      <Plate width={props.width} height={26} />
      <Text class="absolute text-xs font-bold tracking-wide" style={{ insetL: 0, insetT: 0, textColor: DIM }}>GAS</Text>
      <View ref={left} class="absolute" style={{ insetR: 12, insetT: 4, width: 7, height: 7, bgColor: AMBER, opacity: 0.25 }} />
      <View ref={right} class="absolute" style={{ insetR: 0, insetT: 4, width: 7, height: 7, bgColor: AMBER, opacity: 0.25 }} />
      <View class="absolute overflow-hidden" style={{ insetL: 0, insetB: 0, width: props.width, height: 8, bgColor: "#0a0e148c", borderWidth: 1, borderColor: "#ffffff82" }}>
        <View ref={fill} class="absolute" style={{ insetL: 2, insetT: 2, width: props.width - 4, height: 4, originX: -0.5, bgColor: low() ? ALERT : INK }} />
      </View>
    </View>
  );
}

/**
 * Speed in km/h: large digits on a wide surface, smaller where `compact`.
 * The digits stand at the left of a cell of fixed size with the unit over
 * them: a number written to its node keeps the place layout gave the last
 * one, so text that is centred or set to the right would drift.
 */
export function Speed(props: { host: Host; compact?: boolean }) {
  let digits: NodeMirror | undefined;
  let shown = -1;
  onMount(() => onCleanup(props.host.onNumbers((t) => {
    if (t[T.speed] === shown) return;
    shown = t[T.speed];
    hotText(digits, shown);
  })));
  return (
    <View class="relative flex-col" style={{ width: props.compact ? 58 : 84 }}>
      <Plate width={props.compact ? 58 : 84} height={props.compact ? 45 : 60} />
      <Text class="text-xs font-bold tracking-wide" style={{ textColor: DIM }}>KM/H</Text>
      <Text ref={digits} class={props.compact ? "text-2xl font-bold" : "text-4xl font-bold"} style={{ width: props.compact ? 58 : 84, height: props.compact ? 29 : 44, textColor: INK }}>0</Text>
    </View>
  );
}

/** Giants cut of all, and the run's clock under it. */
export function Targets(props: { host: Host }) {
  let time: NodeMirror | undefined;
  let shown = -1;
  onMount(() => onCleanup(props.host.onNumbers((t) => {
    if (t[T.tenths] === shown) return;
    shown = t[T.tenths];
    hotText(time, clock(shown));
  })));
  return (
    <View class="relative flex-col" style={{ width: 72 }}>
      <Plate width={72} height={54} />
      <Text class="text-xs font-bold tracking-wide" style={{ textColor: DIM }}>CUT</Text>
      <Text class="text-lg font-bold" style={{ textColor: INK }}>{`${props.host.kills()} / ${props.host.total()}`}</Text>
      <Text ref={time} class="text-xs" style={{ width: 72, height: 16, textColor: DIM }}>0:00.0</Text>
    </View>
  );
}

/** The line in the middle of the screen: a cut and its speed, a refill. */
export function Note(props: { game: Game; width: number }) {
  return (
    <View class={props.game.note() ? "items-center justify-center opacity-100 transition-opacity duration-200" : "items-center justify-center opacity-0 transition-opacity duration-200"} style={{ width: props.width }}>
      <View class="rounded px-3 py-1" style={{ bgColor: "#0a0e1480" }}>
        <Text class="text-lg font-bold" style={{ textColor: INK }}>{props.game.note()}</Text>
      </View>
    </View>
  );
}

/** What the play buttons do, in this device's own labels. */
export function playHint(): string {
  return `${glyph("ltrigger")}/${glyph("rtrigger")} wires · ${glyph("cross")} gas · ${glyph("square")} cut · ${glyph("triangle")} aim · ${glyph("circle")} let go`;
}

/** The strip along the bottom of a screen with buttons: a notice at the left,
 *  what the buttons do at the right. */
export function Legend(props: { width: number; left?: string; legend: string }) {
  return (
    <View class="relative" style={{ width: props.width, height: 24, bgColor: GLASS }}>
      <View class="absolute" style={{ insetL: 0, insetT: 0, width: props.width, height: 1, bgColor: HAIRLINE }} />
      <Text class="absolute text-xs" style={{ insetL: 10, insetT: 5, textColor: DIM }}>{props.left ?? ""}</Text>
      <Text class="absolute text-xs" style={{ insetR: 10, insetT: 5, textColor: INK }}>{props.legend}</Text>
    </View>
  );
}

/** The renderer's statistics line, while its setting is on. */
export function Stats(props: { host: Host }) {
  return (
    <Show when={props.host.stats()}>
      <Chip text={props.host.stats()} color={DIM} />
    </Show>
  );
}

/** A list's focus, and the buttons that drive it. One for the life of a
 *  presentation: it watches the pad through play too, so a button held when
 *  a list opens is not taken for a press. */
export interface Menu {
  focus: () => number;
  press(index: number): void;
  actions: ActionsHandle;
}

export function createMenu(game: Game): Menu {
  const [focus, setFocus] = createSignal(0);
  const pressable = (row: Row | undefined) => !!row?.press;
  // A list starts on its first row; a switch that changes under the focus keeps it.
  createEffect(on(() => `${game.mode()} ${game.sheet()}`, () => setFocus(0)));
  const press = (index: number) => {
    const row = game.rows()[index];
    if (!pressable(row)) return;
    setFocus(index);
    row!.press!();
  };
  const step = (by: number) => {
    const rows = game.rows();
    for (let at = focus() + by; at >= 0 && at < rows.length; at += by) {
      if (pressable(rows[at])) return setFocus(at);
    }
  };
  let previous = ~0;
  onFrame((buttons) => {
    const pressed = buttons & ~previous;
    previous = buttons;
    if (!game.listing()) return;
    if (pressed & BTN.UP) step(-1);
    if (pressed & BTN.DOWN) step(1);
  });
  // The bindings are read every turn: built again only when the mode or the sheet changes.
  const actions = useActions(createMemo(() => ({
    confirm: { label: "select", run: () => press(focus()), when: () => game.listing() && pressable(game.rows()[focus()]) },
    back: { label: game.backLabel(), run: game.back, when: () => game.listing() && !!game.backLabel() },
    // START pauses, and resumes from the pause list.
    media: { label: game.mode() === "play" ? "pause" : undefined, run: () => (game.mode() === "paused" ? game.back() : game.pause()), when: () => game.mode() === "play" || (game.mode() === "paused" && game.sheet() === "menu") },
  })));
  return { focus, press, actions };
}

/** A switch: its knob at the right when on. */
function Switch(props: { on: boolean }) {
  return (
    <View class="relative rounded-full w-[34] h-[18]" style={{ bgColor: props.on ? AMBER : "#ffffff30" }}>
      <View class="absolute rounded-full w-[14] h-[14] transition-transform duration-150" style={{ insetL: 2, insetT: 2, translateX: props.on ? 16 : 0, bgColor: props.on ? NIGHT : INK }} />
    </View>
  );
}

/**
 * The rows of the sheet that is up. A row that acts takes a tap and, where
 * the device has buttons, the focus mark: one bar that slides to the row,
 * so moving the focus changes one node. A row that only informs is a label
 * and what it means, in a shorter line.
 */
export function Rows(props: { game: Game; menu: Menu; width: number; rowHeight: number; infoHeight?: number; surface?: SurfaceId; active?: () => boolean }) {
  const info = () => props.infoHeight ?? 22;
  // A list that is kept but hidden (`active` false) holds its rows and its mark as they were.
  const live = () => props.active?.() ?? true;
  const rows = createMemo<Row[]>((before) => (live() ? props.game.rows() : before), []);
  const focus = createMemo<number>((before) => (live() ? props.menu.focus() : before), 0);
  return (
    <View class="relative flex-col" style={{ width: props.width }}>
      <Show when={modality.buttons && rows().some((row) => row.press)}>
        <View class="absolute transition-transform duration-100 ease-out" style={{ insetL: 0, insetT: 0, width: props.width, height: props.rowHeight, translateY: focus() * props.rowHeight, bgColor: WASH }}>
          <View class="absolute" style={{ insetL: 0, insetT: 0, width: 3, height: props.rowHeight, bgColor: AMBER }} />
        </View>
      </Show>
      <For each={rows()}>
        {(row, index) => (
          <Show
            when={row.press}
            fallback={
              <View class="relative" style={{ width: props.width, height: info() }}>
                <Text class="absolute text-xs font-bold" style={{ insetL: 14, insetT: info() / 2 - 8, textColor: AMBER }}>{row.label}</Text>
                <Text class="absolute text-xs" style={{ insetL: 14 + Math.min(96, props.width * 0.3), insetT: info() / 2 - 8, textColor: DIM }}>{row.value ?? ""}</Text>
              </View>
            }
          >
            <Touchable surface={props.surface} class="relative" style={{ width: props.width, height: props.rowHeight }} onTap={() => props.menu.press(index())}>
              <Text class="absolute text-sm font-bold" style={{ insetL: 14, insetT: props.rowHeight / 2 - 9, textColor: INK }}>{row.label}</Text>
              <Show when={row.on !== undefined}>
                <View class="absolute" style={{ insetR: 14, insetT: props.rowHeight / 2 - 9 }}><Switch on={!!row.on} /></View>
              </Show>
              <Show when={row.value !== undefined}>
                <Text class="absolute text-sm" style={{ insetR: 14, insetT: props.rowHeight / 2 - 9, textColor: DIM }}>{`${row.value} ›`}</Text>
              </Show>
              <View class="absolute" style={{ insetL: 0, insetB: 0, width: props.width, height: 1, bgColor: HAIRLINE }} />
            </Touchable>
          </Show>
        )}
      </For>
    </View>
  );
}

/**
 * A screen that is built the first time `when` holds (at once with `eager`)
 * and then stays, shown or hidden. Showing it again builds nothing: on the
 * PSP a screen takes tenths of a second to build, which a pause can spend and
 * the moment play resumes cannot.
 */
export function Keep(props: { when: boolean; eager?: boolean; width: number; height: number; children: JSX.Element }) {
  const [built, setBuilt] = createSignal(!!props.eager);
  createEffect(() => {
    if (props.when) setBuilt(true);
  });
  return (
    <View class="absolute" style={{ insetL: 0, insetT: 0, width: props.width, height: props.height, display: props.when ? 0 : 1, hitPass: 1 }}>
      <Show when={built()}>{props.children}</Show>
    </View>
  );
}

/** A sheet's heading over a rule. On a surface a finger reaches, `back`
 *  stands at its left as the way out of the sheet. */
export function Heading(props: { text: string; width: number; height: number; back?: string; onBack?: () => void; surface?: SurfaceId }) {
  const way = () => (props.back ? 88 : 0);
  return (
    <View class="relative" style={{ width: props.width, height: props.height }}>
      <Show when={props.back}>
        <Touchable surface={props.surface} class="absolute flex-row items-center" style={{ insetL: 0, insetT: 0, width: 84, height: props.height }} onTap={props.onBack}>
          <Text class="text-sm font-bold" style={{ marginL: 14, textColor: AMBER }}>{`‹ ${props.back}`}</Text>
        </Touchable>
      </Show>
      <Text class="absolute text-xs font-bold tracking-wide" style={{ insetL: 14 + way(), insetT: props.height / 2 - 8, textColor: DIM }}>{props.text}</Text>
      <View class="absolute" style={{ insetL: 0, insetB: 0, width: props.width, height: 1, bgColor: HAIRLINE }} />
    </View>
  );
}

/** A view that fades with `shown`. */
export function Fade(props: { shown: boolean; children: JSX.Element }) {
  return <View class={props.shown ? "opacity-100 transition-opacity duration-300" : "opacity-0 transition-opacity duration-300"}>{props.children}</View>;
}

/** A panel over a scrim, in the middle of a surface. */
export function Panel(props: { width: number; height: number; panelWidth: number; panelHeight: number; children: JSX.Element }) {
  return (
    <View class="absolute" style={{ insetL: 0, insetT: 0, width: props.width, height: props.height, bgColor: "#00000099" }}>
      <View class="absolute rounded-lg overflow-hidden flex-row" style={{ insetL: (props.width - props.panelWidth) / 2, insetT: Math.max(4, (props.height - props.panelHeight) / 2), width: props.panelWidth, height: props.panelHeight, bgColor: PANEL, borderWidth: 1, borderColor: HAIRLINE }}>
        {props.children}
      </View>
    </View>
  );
}

/** The finished run: its time, its top speed, and how it stands to the best one. */
export function Result(props: { game: Game; width: number }) {
  const host = props.game.host;
  return (
    <View class="flex-col items-center" style={{ width: props.width }}>
      <Text class="text-4xl font-bold" style={{ textColor: INK }}>{clock(host.result()[0] ?? 0)}</Text>
      <Text class="text-xs" style={{ textColor: DIM }}>{`${host.kills()} of ${host.total()} cut · top speed ${host.result()[1] ?? 0} km/h`}</Text>
      <Text class="text-xs font-bold tracking-wide" style={{ marginT: 4, textColor: props.game.record() ? AMBER : FAINT }}>
        {props.game.record() ? "NEW BEST" : props.game.best() ? `BEST ${clock(props.game.best())}` : ""}
      </Text>
    </View>
  );
}

/**
 * The town from above with every giant and the player on it. North is up;
 * the image shows the square of world `MAP_EXTENT` metres to each side. A
 * giant's mark dims when it is cut; the player's mark moves and turns with
 * the numbers in flight.
 */
export function TownMap(props: { host: Host; size: number }) {
  const host = props.host;
  const scale = () => props.size / (2 * MAP_EXTENT);
  const giants = () => host.giants().split(";").filter(Boolean).map((pair) => pair.split(",").map(Number) as [number, number]);
  const marks: (NodeMirror | undefined)[] = [];
  let player: NodeMirror | undefined;
  // One effect for all the marks: a cut dims one node.
  createEffect(() => {
    const alive = host.alive();
    for (let i = 0; i < marks.length; i++) hotProp(marks[i], "opacity", alive[i] === "0" ? 0.25 : 1);
  });
  onMount(() => {
    onCleanup(host.onNumbers((t) => {
      hotProp(player, "translateX", Math.round((t[T.x] + MAP_EXTENT) * scale()));
      hotProp(player, "translateY", Math.round((t[T.z] + MAP_EXTENT) * scale()));
      hotProp(player, "rotate", t[T.heading]);
    }));
  });
  return (
    <View class="relative overflow-hidden" style={{ width: props.size, height: props.size, bgColor: "#3a4030" }}>
      <Image src={MAP_IMAGE} class="absolute" style={{ insetL: 0, insetT: 0, width: props.size, height: props.size }} />
      <For each={giants()}>
        {([x, z], index) => (
          <View ref={(node: NodeMirror) => (marks[index()] = node)} class="absolute" style={{ insetL: (x + MAP_EXTENT) * scale() - 2, insetT: (z + MAP_EXTENT) * scale() - 2, width: 5, height: 5, bgColor: TARGET, borderWidth: 1, borderColor: NIGHT }} />
        )}
      </For>
      {/* The player: a square with a line toward where the view faces. */}
      <View ref={player} class="absolute" style={{ insetL: -5, insetT: -5, width: 10, height: 10 }}>
        <View class="absolute" style={{ insetL: 4, insetT: -6, width: 2, height: 8, bgColor: INK }} />
        <View class="absolute" style={{ insetL: 2, insetT: 2, width: 6, height: 6, bgColor: "#ffe000", borderWidth: 1, borderColor: NIGHT }} />
      </View>
    </View>
  );
}

/** The screen while the world loads, and when it could not. */
export function Loading(props: { host: Host; width: number; height: number }) {
  return (
    <View class="items-center justify-center flex-col gap-3" style={{ width: props.width, height: props.height, bgColor: NIGHT }}>
      <Wordmark />
      <Text class="text-xs" style={{ textColor: props.host.mode() === "error" ? ALERT : DIM }}>{props.host.message() || "Reading the world"}</Text>
    </View>
  );
}

export { tint };
