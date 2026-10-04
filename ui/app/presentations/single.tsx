// One 480×272 screen with a pad: the PSP, and the Vita, whose panel also
// takes taps. In play the scene has the screen and the interface is the
// gauges at its edges; a list comes up over it for the title, a pause and a
// finished run.
import { createEffect, on, Show } from "solid-js";
import { Text, View } from "@pocketjs/framework/components";
import { glyph } from "@pocketjs/framework/modality";
import { clock, createGame, createPulse, type Game } from "../game.ts";
import { connectHost } from "../host.ts";
import { Chip, createMenu, Fade, Gauge, Heading, Keep, Legend, Loading, type Menu, Note, Panel, playHint, Result, Rows, Speed, Stats, Targets, TownMap, Wordmark } from "../parts.tsx";
import { DIM } from "../theme.ts";

const W = 480, H = 272;
const FOOTER = 24;

export default function SingleScreen() {
  const host = connectHost();
  const game = createGame(host, false);
  const menu = createMenu(game);
  return (
    <View class="relative w-full h-full">
      <Show when={host.mode() === "loading" || host.mode() === "error"}><Loading host={host} width={W} height={H} /></Show>
      <Keep when={host.mode() === "title"} eager width={W} height={H}><Title game={game} menu={menu} /></Keep>
      <Keep when={host.mode() === "play" || host.mode() === "paused"} eager width={W} height={H}><Play game={game} /></Keep>
      <Keep when={host.mode() === "paused"} eager width={W} height={H}><Paused game={game} menu={menu} /></Keep>
      <Keep when={host.mode() === "results"} width={W} height={H}><Results game={game} menu={menu} /></Keep>
      <Show when={game.listing()}>
        <View class="absolute" style={{ insetL: 0, insetB: 0 }}>
          <Legend width={W} left={game.mode() === "title" && game.best() ? `Best ${clock(game.best())}` : ""} legend={menu.actions.legend()} />
        </View>
      </Show>
    </View>
  );
}

function Title(props: { game: Game; menu: Menu }) {
  const game = props.game;
  return (
    <View class="relative w-full h-full">
      <View class="absolute bg-gradient-to-r from-[#000000c0] to-[#00000000]" style={{ insetL: 0, insetT: 0, width: 320, height: H }} />
      <View class="absolute" style={{ insetL: 24, insetT: 26 }}><Wordmark large /></View>
      <Text class="absolute text-xs" style={{ insetL: 24, insetT: 92, textColor: DIM }}>Two wires, a tank of gas, a walled town.</Text>
      <Show
        when={game.sheet() === "menu"}
        fallback={
          <Panel width={W} height={H - FOOTER} panelWidth={320} panelHeight={212}>
            <View class="flex-col">
              <Heading text={game.heading()} width={320} height={28} />
              <Rows game={game} menu={props.menu} width={320} rowHeight={30} active={() => game.mode() === "title"} />
            </View>
          </Panel>
        }
      >
        <View class="absolute" style={{ insetL: 10, insetT: 124 }}><Rows game={game} menu={props.menu} width={190} rowHeight={30} active={() => game.mode() === "title"} /></View>
      </Show>
    </View>
  );
}

function Play(props: { game: Game }) {
  const host = props.game.host;
  // What the buttons do shows when a run begins, then leaves the view clear.
  const [hint, showHint] = createPulse(6);
  createEffect(on(host.mode, (mode, before) => mode === "play" && before !== "paused" && showHint()));
  return (
    <View class="relative w-full h-full">
      <View class="absolute" style={{ insetL: 16, insetB: 14 }}><Gauge host={host} width={128} /></View>
      <View class="absolute" style={{ insetR: 16, insetB: 6 }}><Speed host={host} /></View>
      <View class="absolute" style={{ insetR: 16, insetT: 12 }}><Targets host={host} /></View>
      <View class="absolute" style={{ insetL: 0, insetT: 66 }}><Note game={props.game} width={W} /></View>
      <View class="absolute" style={{ insetL: 10, insetT: 10 }}><Stats host={host} /></View>
      <View class="absolute items-center justify-center" style={{ insetL: 0, insetT: 100, width: W }}>
        <Fade shown={hint()}><Chip text={`${playHint()} · ${glyph("start")} pause`} /></Fade>
      </View>
    </View>
  );
}

const MAP = 176;

function Paused(props: { game: Game; menu: Menu }) {
  const list = 440 - MAP - 24;
  return (
    <Panel width={W} height={H - FOOTER} panelWidth={440} panelHeight={216}>
      <View class="items-center justify-center" style={{ width: MAP + 24, height: 216 }}><TownMap host={props.game.host} size={MAP} /></View>
      <View class="flex-col">
        <Heading text={props.game.heading()} width={list} height={28} />
        <Rows game={props.game} menu={props.menu} width={list} rowHeight={30} active={() => props.game.mode() === "paused"} />
      </View>
    </Panel>
  );
}

function Results(props: { game: Game; menu: Menu }) {
  return (
    <Panel width={W} height={H - FOOTER} panelWidth={320} panelHeight={206}>
      <View class="flex-col">
        <Heading text={props.game.heading()} width={320} height={28} />
        <Show when={props.game.sheet() === "menu"}>
          <View style={{ paddingT: 8, paddingB: 8 }}><Result game={props.game} width={320} /></View>
        </Show>
        <Rows game={props.game} menu={props.menu} width={320} rowHeight={30} active={() => props.game.mode() === "results"} />
      </View>
    </Panel>
  );
}
