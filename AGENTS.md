# Repository instructions

- Pocket Maneuver owns the world generator and the three.js reference (`web/`), the simulation core (`crates/maneuver-sim`), the world compiler and the pack format (`crates/maneuver-cook`, `crates/maneuver-pack`) and the PS Vita runtime (`vita/`). PocketJS (pinned in `vendor/pocketjs`) owns the Vita dev host, the wired debug transport (`tools/vita-dev.ts`, `tools/vita-usb.ts`), VPK packaging and the GXM device kernel (`devices/vita/pocket-vita-gxm`).
- Do not edit the submodule to fix application behavior. Send reusable changes to PocketJS, then update the pinned revision here.
- Use Conventional Commits for commits and pull requests (`type(scope): summary`). Publish validated changes as a Draft PR.
- One rule set, one implementation: gameplay lives in `crates/maneuver-sim` and nowhere else. The reference runs it as wasm (`bun tools/maneuver.ts sim`), the compiler links it for baking rays, the Vita links it natively. Do not re-implement a rule in TypeScript.
- The snapshot layout and the shared constants are generated: after changing `abi.rs`, `sim.rs` constants or the skeleton in `pose.rs`, run `bun tools/maneuver.ts sim` (it rewrites the ignored `web/src/sim/abi.gen.ts`).
- The world derives from one seed. No `Math.random`, wall time or canvas APIs in `web/src/world`: the same code runs in the browser and in Bun for the export.
- Every static surface samples one atlas (`web/src/world/atlas.ts`), so a cell is one draw on the device. A new surface kind is a new strip in that atlas, not a new texture or program.
- A change to the world, the atlas or the bake goes through `bun tools/maneuver.ts cook`; the pack and its compile receipt are outputs under the ignored `.pocket-build/`.
- Keep captures, logs, cooked packs, USB-share contents and build receipts in `.pocket-build/`. Put reproducible commands, results and limits in the PR description instead of committing them.
- Separate evidence kinds when reporting: host build, compile receipt, device frame timing (`bun tools/maneuver.ts bench`), and what a person saw on the screen.
- Shader programs are compiled on the device by SceShaccCg and cached by source hash under `ux0:data/pocket-maneuver/gxp`. Mixing `half` and `float` in `lerp`/`smoothstep` is an overload ambiguity there: cast explicitly.
- The frame budget is 16.7 ms on a PS Vita at 960 × 544. A renderer change reports `bench` before and after; a change that adds a per-pixel texture read or a second pass needs that measurement first.
- The mechanic is a pair of wire hooks and compressed gas; the setting is an original walled town. Do not add characters, emblems or names from any film or series.
