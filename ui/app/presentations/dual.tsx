// Two screens (the 3DS): the scene and its gauges on the top screen, the
// town from above on the touch screen below. A list (the title, a pause, a
// finished run, the settings) takes the lower screen, where a finger or the
// d-pad walks it, and the top screen says what it is for.
import { createEffect, on, Show } from "solid-js";
import { AuxiliarySurface, Text, View } from "@pocketjs/framework/components";
import { glyph } from "@pocketjs/framework/modality";
import { clock, createGame, createPulse, type Game } from "../game.ts";
import { connectHost } from "../host.ts";
import { Button, Chip, createMenu, Fade, Gauge, Heading, Keep, Legend, Loading, type Menu, Note, playHint, Result, Rows, Speed, Stats, Targets, TownMap, Wordmark } from "../parts.tsx";
import { DIM, HAIRLINE, INK, NIGHT } from "../theme.ts";

const TOP = { w: 400, h: 240 };
const LOW = { w: 320, h: 240 };
const BAR = 36, FOOTER = 24;

export default function DualScreen() {
  const host = connectHost();
  const game = createGame(host, false);
  const menu = createMenu(game);
  return (
    <>
      <View class="relative" style={{ width: TOP.w, height: TOP.h }}>
        <Show when={host.mode() === "loading" || host.mode() === "error"}><Loading host={host} width={TOP.w} height={TOP.h} /></Show>
        <Keep when={host.mode() === "title"} eager width={TOP.w} height={TOP.h}><TitleTop game={game} /></Keep>
        <Keep when={host.mode() === "play" || host.mode() === "paused" || host.mode() === "results"} eager width={TOP.w} height={TOP.h}><PlayTop game={game} /></Keep>
        <Show when={host.mode() === "paused"}>
          <View class="absolute items-center justify-center" style={{ insetL: 0, insetT: 0, width: TOP.w, height: TOP.h, bgColor: "#00000080" }}>
            <Text class="text-2xl font-bold tracking-wide" style={{ textColor: INK }}>PAUSED</Text>
          </View>
        </Show>
        <Show when={host.mode() === "results"}>
          <View class="absolute items-center justify-center" style={{ insetL: 0, insetT: 0, width: TOP.w, height: TOP.h, bgColor: "#00000099" }}>
            <Result game={game} width={TOP.w} />
          </View>
        </Show>
      </View>
      <AuxiliarySurface>
        {() => (
          <View class="relative" style={{ width: LOW.w, height: LOW.h, bgColor: NIGHT }}>
            <Show when={host.mode() === "loading" || host.mode() === "error"}>
              <View class="items-center justify-center" style={{ width: LOW.w, height: LOW.h }}>
                <Text class="text-sm" style={{ textColor: DIM }}>{host.mode() === "error" ? "The game could not start" : "Loading"}</Text>
              </View>
            </Show>
            {/* The map stays built under the lists: a pause and its end build nothing. */}
            <Keep when={host.mode() === "play"} eager width={LOW.w} height={LOW.h}><PlayLow game={game} /></Keep>
            <Keep when={game.listing()} eager width={LOW.w} height={LOW.h}><ListLow game={game} menu={menu} /></Keep>
          </View>
        )}
      </AuxiliarySurface>
    </>
  );
}

function TitleTop(props: { game: Game }) {
  return (
    <View class="relative" style={{ width: TOP.w, height: TOP.h }}>
      <View class="absolute bg-gradient-to-b from-[#00000000] to-[#000000c0]" style={{ insetL: 0, insetB: 0, width: TOP.w, height: 130 }} />
      <View class="absolute" style={{ insetL: 20, insetB: 34 }}><Wordmark large /></View>
      <Text class="absolute text-xs" style={{ insetL: 20, insetB: 14, textColor: DIM }}>Two wires, a tank of gas, a walled town.</Text>
      <Show when={props.game.best()}>
        <View class="absolute" style={{ insetR: 12, insetB: 12 }}><Chip text={`BEST ${clock(props.game.best())}`} /></View>
      </Show>
    </View>
  );
}

function PlayTop(props: { game: Game }) {
  const host = props.game.host;
  const [hint, showHint] = createPulse(6);
  createEffect(on(host.mode, (mode, before) => mode === "play" && before !== "paused" && showHint()));
  return (
    <View class="relative" style={{ width: TOP.w, height: TOP.h }}>
      <View class="absolute" style={{ insetL: 12, insetB: 12 }}><Gauge host={host} width={112} /></View>
      <View class="absolute" style={{ insetR: 12, insetB: 6 }}><Speed host={host} /></View>
      <View class="absolute" style={{ insetL: 0, insetT: 58 }}><Note game={props.game} width={TOP.w} /></View>
      <View class="absolute" style={{ insetL: 8, insetT: 8 }}><Stats host={host} /></View>
      <View class="absolute items-center justify-center" style={{ insetL: 0, insetT: 30, width: TOP.w }}>
        <Fade shown={hint()}><Chip text={playHint()} /></Fade>
      </View>
    </View>
  );
}

const SIDE = LOW.w - LOW.h;

/** In play: the town at the left, the run in numbers and the way to a pause beside it. */
function PlayLow(props: { game: Game }) {
  return (
    <View class="relative" style={{ width: LOW.w, height: LOW.h }}>
      <TownMap host={props.game.host} size={LOW.h} />
      <View class="absolute" style={{ insetL: LOW.h, insetT: 0, width: 1, height: LOW.h, bgColor: HAIRLINE }} />
      <View class="absolute" style={{ insetL: LOW.h + 8, insetT: 10 }}><Targets host={props.game.host} /></View>
      <View class="absolute" style={{ insetL: LOW.h + 6, insetB: 8 }}>
        <Button label="Pause" width={SIDE - 12} height={40} surface="auxiliary" onPress={props.game.pause} />
      </View>
    </View>
  );
}

/** A list on the lower screen: its heading, its rows, what the buttons do. */
function ListLow(props: { game: Game; menu: Menu }) {
  const game = props.game;
  return (
    <View class="relative flex-col" style={{ width: LOW.w, height: LOW.h, bgColor: NIGHT }}>
      <View style={{ width: LOW.w, height: BAR, bgColor: "#161a24" }}>
        <Heading text={game.heading() || "POCKET MANEUVER"} width={LOW.w} height={BAR} back={game.sheet() !== "menu" ? "Back" : undefined} onBack={game.back} surface="auxiliary" />
      </View>
      <Rows game={game} menu={props.menu} width={LOW.w} rowHeight={34} surface="auxiliary" />
      <View class="absolute" style={{ insetL: 0, insetB: 0 }}>
        <Legend width={LOW.w} legend={props.menu.actions.legend()} left={game.mode() === "play" ? `${glyph("start")} pause` : ""} />
      </View>
    </View>
  );
}

export { FOOTER };
