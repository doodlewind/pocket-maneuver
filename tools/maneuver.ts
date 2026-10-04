#!/usr/bin/env bun
// Pocket Maneuver's command line: build the simulation for the web, export and
// compile the world, build and drive the Vita app.
//
//   bun tools/maneuver.ts sim              wasm build of crates/maneuver-sim + web/src/sim/abi.gen.ts
//   bun tools/maneuver.ts shot [...]       headless capture of the web reference
//   bun tools/maneuver.ts export           WorldIR from the generator → .pocket-build/world/ir
//   bun tools/maneuver.ts cook [--profile vita60|psp60|n3ds60] [--no-export]   export, then compile a device pack and its receipt
//   bun tools/maneuver.ts build|sync|native|serve|status|capture|ctl|bench   the Vita loop (tools/vita.ts)

import { $ } from "bun";
import { mkdir } from "node:fs/promises";
import { join, resolve } from "node:path";

export const ROOT = resolve(import.meta.dir, "..");
export const BUILD = join(ROOT, ".pocket-build");

async function sim() {
  await $`cargo build --release -p maneuver-sim --target wasm32-unknown-unknown --lib`.cwd(ROOT);
  await mkdir(join(ROOT, "web/public/sim"), { recursive: true });
  await Bun.write(join(ROOT, "web/public/sim/maneuver_sim.wasm"), Bun.file(join(ROOT, "target/wasm32-unknown-unknown/release/maneuver_sim.wasm")));
  const abi = await $`cargo run --release -q -p maneuver-sim --bin abi`.cwd(ROOT).text();
  await Bun.write(join(ROOT, "web/src/sim/abi.gen.ts"), abi);
  console.log(`sim: wasm ${(Bun.file(join(ROOT, "web/public/sim/maneuver_sim.wasm")).size / 1024).toFixed(0)} KiB, abi.gen.ts written`);
}

async function exportWorld(rest: string[]) {
  await $`bun web/scripts/export-world.ts ${rest}`.cwd(ROOT);
}

/** Compiles the pack of one device profile (`--profile vita60|psp60|n3ds60`, default vita60). */
async function cook(rest: string[]) {
  const at = rest.indexOf("--profile");
  const profile = at >= 0 ? rest[at + 1] : "vita60";
  await $`cargo run --release -q -p maneuver-cook -- --in .pocket-build/world/ir --out .pocket-build/world/walled-town.${profile}.pack --profile profiles/${profile}.json --font vendor/pocketjs/assets/fonts/InterDisplay-Bold.ttf`.cwd(ROOT);
}

const [cmd, ...rest] = process.argv.slice(2);
switch (cmd) {
  case "sim":
    await sim();
    break;
  case "export":
    await exportWorld(rest);
    break;
  case "cook":
    if (!rest.includes("--no-export")) await exportWorld([]);
    await cook(rest);
    break;
  case "build":
  case "sync":
  case "native":
  case "push":
  case "serve":
  case "status":
  case "capture":
  case "ctl":
  case "hold":
  case "vpk":
  case "push-vpk":
  case "bench": {
    const vita = await import("./vita.ts");
    if (cmd === "build") await vita.build(rest);
    else if (cmd === "sync") vita.sync(rest);
    else if (cmd === "native") {
      vita.sync(rest);
      await vita.build(rest);
      await vita.dev(rest, "native");
    } else if (cmd === "push") {
      // The build already in dist/, without rebuilding it.
      vita.sync(rest);
      await vita.dev(rest, "native");
    } else if (cmd === "serve") await vita.dev(rest, "serve");
    else if (cmd === "status") console.log(JSON.stringify(vita.status(rest), null, 1));
    else if (cmd === "capture") await vita.dev(rest, "capture", ...(rest.includes("--out") ? ["--out", resolve(rest[rest.indexOf("--out") + 1])] : []));
    else if (cmd === "ctl") vita.ctl(rest, rest.find((a) => a.startsWith("{")) ?? "{}");
    else if (cmd === "hold") await vita.hold(rest);
    else if (cmd === "vpk") await vita.vpk(rest);
    else if (cmd === "push-vpk") await vita.pushVpk(rest);
    else {
      const { bench } = await import("./bench.ts");
      await bench(rest);
    }
    break;
  }
  case "shot": {
    const { shot } = await import("./shot.ts");
    await shot(rest);
    break;
  }
  default:
    console.log("usage: bun tools/maneuver.ts <sim|export|cook|shot|build|sync|native|serve|status|capture|ctl|bench>");
    process.exit(cmd ? 1 : 0);
}
