// A touch panel and nothing else (the iPod touch, held sideways): every verb
// is a control under a thumb. In play a stick stands in the lower left to
// move, the keys in the lower right fire the wires, burn gas and cut, and a
// finger anywhere else turns the view. Lists are buttons a finger presses.
import { createEffect, createMemo, createSignal, For, on, Show, type JSX } from "solid-js";
import { Text, View } from "@pocketjs/framework/components";
import { createGesture } from "@pocketjs/framework/gesture";
import { onFrame } from "@pocketjs/framework/lifecycle";
import type { NodeMirror } from "@pocketjs/framework/renderer";
import { clock, createGame, createPulse, type Game, type Row } from "../game.ts";
import { connectHost, type Host } from "../host.ts";
import { Button, Chip, createMenu, Fade, Gauge, Heading, Keep, Loading, type Menu, Note, Panel, Result, Rows, Speed, Stats, Targets, Touchable, TownMap, Wordmark } from "../parts.tsx";
import { PLAY } from "../protocol.ts";
import { AMBER, DIM, HAIRLINE, INK, NIGHT, tint } from "../theme.ts";

const W = 480, H = 320;
/** A fingertip on this panel (Pocket HIG, touch modality). */
const TARGET = 44;

export default function TouchScreen() {
  const host = connectHost();
  const game = createGame(host, true);
  const menu = createMenu(game);
  return (
    <View class="relative w-full h-full">
      <Show when={host.mode() === "loading" || host.mode() === "error"}><Loading host={host} width={W} height={H} /></Show>
      <Keep when={host.mode() === "title"} eager width={W} height={H}><Title game={game} menu={menu} /></Keep>
      {/* The controls stay built under the pause list: a pause and its end build nothing. */}
      <Keep when={host.mode() === "play" || host.mode() === "paused"} eager width={W} height={H}><Play game={game} /></Keep>
      <Keep when={host.mode() === "paused"} eager width={W} height={H}><Paused game={game} menu={menu} /></Keep>
      <Keep when={host.mode() === "results"} width={W} height={H}><Results game={game} menu={menu} /></Keep>
    </View>
  );
}

/** The list that is up: its heading with the way back, and its rows under whatever stands between. */
function Sheet(props: { game: Game; menu: Menu; width: number; active: () => boolean; children?: JSX.Element }) {
  return (
    <View class="flex-col">
      <Heading text={props.game.heading()} width={props.width} height={TARGET} back={props.game.sheet() !== "menu" ? "Back" : props.game.mode() === "paused" ? "Resume" : undefined} onBack={props.game.back} />
      {props.children}
      <Rows game={props.game} menu={props.menu} width={props.width} rowHeight={TARGET} infoHeight={30} active={props.active} />
    </View>
  );
}

function Title(props: { game: Game; menu: Menu }) {
  const game = props.game;
  // The title's own rows as buttons; kept as they are while another list is up.
  const rows = createMemo<Row[]>((before) => (game.mode() === "title" && game.sheet() === "menu" ? game.rows() : before), []);
  return (
    <View class="relative w-full h-full">
      <View class="absolute bg-gradient-to-r from-[#000000c0] to-[#00000000]" style={{ insetL: 0, insetT: 0, width: 340, height: H }} />
      <View class="absolute" style={{ insetL: 28, insetT: 30 }}><Wordmark large /></View>
      <Text class="absolute text-xs" style={{ insetL: 28, insetT: 96, textColor: DIM }}>Two wires, a tank of gas, a walled town.</Text>
      <Show when={game.best()}>
        <View class="absolute" style={{ insetR: 14, insetB: 14 }}><Chip text={`BEST ${clock(game.best())}`} /></View>
      </Show>
      <Show
        when={game.sheet() === "menu"}
        fallback={<Panel width={W} height={H} panelWidth={360} panelHeight={296}><Sheet game={game} menu={props.menu} width={360} active={() => game.mode() === "title"} /></Panel>}
      >
        <View class="absolute flex-col gap-2" style={{ insetL: 28, insetT: 132 }}>
          <For each={rows()}>
            {(row, index) => <Button label={row.label} width={200} height={TARGET} strong={index() === 0} onPress={() => props.menu.press(index())} />}
          </For>
        </View>
      </Show>
    </View>
  );
}

const MAP = 200;

function Paused(props: { game: Game; menu: Menu }) {
  return (
    <Panel width={W} height={H} panelWidth={460} panelHeight={296}>
      <View class="items-center justify-center" style={{ width: MAP + 20, height: 296 }}><TownMap host={props.game.host} size={MAP} /></View>
      <Sheet game={props.game} menu={props.menu} width={460 - MAP - 20} active={() => props.game.mode() === "paused"} />
    </Panel>
  );
}

function Results(props: { game: Game; menu: Menu }) {
  return (
    <Panel width={W} height={H} panelWidth={340} panelHeight={props.game.sheet() === "menu" ? 232 : 296}>
      <Sheet game={props.game} menu={props.menu} width={340} active={() => props.game.mode() === "results"}>
        <Show when={props.game.sheet() === "menu"}>
          <View style={{ paddingT: 10, paddingB: 8 }}><Result game={props.game} width={340} /></View>
        </Show>
      </Sheet>
    </Panel>
  );
}

/** A stick's ring, how far around its centre a thumb still takes hold of it,
 *  and the play at its centre that does nothing. */
const STICK = 46, KNOB = 44, REACH = 84, SLACK = 0.18;

/** A stick that stays where it is drawn: a thumb landing on or near it
 *  pushes it toward where it landed. */
function Stick(props: { at: { x: number; y: number }; onChange: (x: number, y: number) => void }) {
  let area: NodeMirror | undefined;
  const [knob, setKnob] = createSignal({ x: 0, y: 0 });
  const [held, setHeld] = createSignal(false);
  const push = (c: { x: number; y: number }) => {
    let x = (c.x - props.at.x) / STICK, y = (c.y - props.at.y) / STICK;
    const length = Math.hypot(x, y);
    if (length > 1) {
      x /= length;
      y /= length;
    }
    setKnob({ x, y });
    // Past the slack the push grows from nothing, so the first step is a small one.
    const drive = length <= SLACK ? 0 : (Math.min(1, length) - SLACK) / (1 - SLACK) / Math.min(1, length);
    props.onChange(x * drive, y * drive);
  };
  const rest = () => {
    setHeld(false);
    setKnob({ x: 0, y: 0 });
    props.onChange(0, 0);
  };
  createGesture({
    region: { node: () => area },
    tapSlop: 9999,
    onDown: (c) => {
      setHeld(true);
      push(c);
    },
    onMove: push,
    onUp: rest,
    onCancel: rest,
  });
  const alpha = () => (held() ? 0.5 : 0.26);
  return (
    <View ref={area} class="absolute" style={{ insetL: props.at.x - REACH, insetT: props.at.y - REACH, width: REACH * 2, height: REACH * 2 }}>
      <View class="absolute rounded-full w-[92] h-[92]" style={{ insetL: REACH - STICK, insetT: REACH - STICK, bgColor: tint("#ffffff", alpha() * 0.35), borderWidth: 2, borderColor: tint("#ffffff", alpha()) }} />
      <View class="absolute rounded-full w-[44] h-[44]" style={{ insetL: REACH - KNOB / 2, insetT: REACH - KNOB / 2, translateX: knob().x * STICK, translateY: knob().y * STICK, bgColor: tint("#ffffff", alpha() + 0.15) }} />
    </View>
  );
}

/** A tap shorter than this keeps a latching key held. */
const TAP_SECONDS = 0.25;

/**
 * A round key under a thumb, held while a finger is on it. A key that
 * latches (a wire) stays held after a short tap, until it is tapped again:
 * one thumb can keep both wires and still reach the gas.
 */
function Key(props: { label: string; x: number; y: number; size: number; latch?: boolean; strong?: boolean; seconds: () => number; onChange: (held: boolean) => void }) {
  let node: NodeMirror | undefined;
  const [held, setHeld] = createSignal(false);
  let kept = false, since = 0, letGo = false;
  const set = (on: boolean) => {
    setHeld(on);
    props.onChange(on);
  };
  createGesture({
    region: { node: () => node },
    tapSlop: 9999,
    onDown: () => {
      letGo = kept;
      kept = false;
      since = props.seconds();
      set(!letGo);
    },
    onUp: () => {
      if (letGo) return;
      kept = !!props.latch && props.seconds() - since < TAP_SECONDS;
      if (!kept) set(false);
    },
    onCancel: () => {
      kept = false;
      set(false);
    },
  });
  return (
    <View ref={node} class="absolute items-center justify-center" style={{ insetL: props.x - props.size / 2, insetT: props.y - props.size / 2, width: props.size, height: props.size, radius: props.size / 2, bgColor: held() ? tint(AMBER, 0.8) : "#0c101680", borderWidth: 2, borderColor: held() || props.strong ? AMBER : "#ffffff70" }}>
      <Text class="text-xs font-bold" style={{ textColor: held() ? NIGHT : INK }}>{props.label}</Text>
    </View>
  );
}

function Play(props: { game: Game }) {
  const host: Host = props.game.host;
  // The stick and the keys, sent when they change; move y is forward.
  const stick = { x: 0, y: 0 };
  let keys = 0, sent = "0,0,0", seconds = 0;
  const key = (bit: number) => (on: boolean) => (keys = on ? keys | bit : keys & ~bit);
  onFrame(() => {
    seconds += 1 / 60;
    const now = [Math.round(stick.x * 100), Math.round(-stick.y * 100), keys];
    const line = now.join(",");
    if (line === sent) return;
    sent = line;
    host.send({ type: "drive", mx: now[0], my: now[1], lx: 0, ly: 0, b: keys });
  });
  // A finger on the scene turns the view by what it travels.
  let scene: NodeMirror | undefined;
  createGesture({
    region: { node: () => scene },
    tapSlop: 9999,
    onMove: (c) => {
      if (c.fdx || c.fdy) host.send({ type: "look", dx: c.fdx, dy: c.fdy });
    },
  });
  const [hint, showHint] = createPulse(6);
  createEffect(on(host.mode, (mode, before) => mode === "play" && before !== "paused" && showHint()));
  const clockNow = () => seconds;
  return (
    <View class="relative w-full h-full">
      {/* The gauges are children of the scene: a finger that lands on one still turns the view. */}
      <View ref={scene} class="absolute" style={{ insetL: 0, insetT: 0, width: W, height: H }}>
        <View class="absolute" style={{ insetL: 66, insetT: 18 }}><Gauge host={host} width={128} /></View>
        <View class="absolute" style={{ insetR: 14, insetT: 12 }}><Targets host={host} /></View>
        <View class="absolute" style={{ insetL: W / 2 - 28, insetT: 8 }}><Speed host={host} /></View>
        <View class="absolute" style={{ insetL: 0, insetT: 76 }}><Note game={props.game} width={W} /></View>
        <View class="absolute" style={{ insetL: 66, insetT: 50 }}><Stats host={host} /></View>
        <View class="absolute items-center justify-center" style={{ insetL: 0, insetT: 104, width: W }}>
          <Fade shown={hint()}><Chip text="Tap L or R to fire a wire and keep it · drag to look" /></Fade>
        </View>
      </View>
      <Touchable class="absolute rounded-lg items-center justify-center" style={{ insetL: 10, insetT: 10, width: TARGET, height: TARGET, bgColor: "#0c1016b0", borderWidth: 1, borderColor: HAIRLINE }} onTap={props.game.pause}>
        <View class="flex-row gap-1">
          <View style={{ width: 4, height: 14, bgColor: INK }} />
          <View style={{ width: 4, height: 14, bgColor: INK }} />
        </View>
      </Touchable>
      <Stick at={{ x: 86, y: H - 86 }} onChange={(x, y) => { stick.x = x; stick.y = y; }} />
      <Key label="L" x={W - 138} y={H - 142} size={60} latch seconds={clockNow} onChange={key(PLAY.wireLeft)} />
      <Key label="R" x={W - 60} y={H - 142} size={60} latch seconds={clockNow} onChange={key(PLAY.wireRight)} />
      <Key label="GAS" x={W - 98} y={H - 58} size={76} strong seconds={clockNow} onChange={key(PLAY.gas)} />
      <Key label="CUT" x={W - 184} y={H - 52} size={56} seconds={clockNow} onChange={key(PLAY.cut)} />
      <Key label="AIM" x={W - 204} y={H - 126} size={48} latch seconds={clockNow} onChange={key(PLAY.aim)} />
      <Key label="DROP" x={W - 34} y={H - 212} size={48} seconds={clockNow} onChange={key(PLAY.drop)} />
    </View>
  );
}
