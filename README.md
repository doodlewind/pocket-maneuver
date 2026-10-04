# Pocket Maneuver

A traversal game for the PS Vita: two wire hooks, a tank of compressed gas and a walled town of about 5 400 houses, drawn at **960 × 544 with 4× MSAA at 60 frames per second**.

The player fires a wire from each hip into a wall or a roof, is pulled along it, lets go and fires the next. Gas reels a wire in faster, or thrusts when no wire holds. Thirty-two plank targets stand in the streets and among the trees outside the wall; a target counts when the player cuts the pad at the back of its neck at speed.

The repository holds the whole path from authoring to hardware:

- **`web/`** is the reference: a three.js app that generates the world from one seed at load time and runs the game in a browser.
- **`crates/maneuver-sim`** is the game: collision, wire physics, camera, procedural animation, sound and the autopilot. The reference runs it as wasm; the Vita links it natively. There is one implementation of every rule.
- **`crates/maneuver-cook`** compiles the world for the device: it bakes lighting into vertex colours, merges geometry into cells with three levels of detail, compresses the atlas and writes one pack with a compile receipt.
- **`vita/`** draws the pack through GXM and runs the simulation at one tick per display refresh.

PocketJS (pinned in `vendor/pocketjs`) supplies the Vita dev host, the wired debug transport, VPK packaging and the GXM device kernel.

## The world

`web/src/world/city.ts` lays the town out on a radial plan: a central square, **eight bands of blocks** between ring streets, four avenues to the gates, a canal ring with eight bridges, and a **50 m wall** with a walk on top. Row houses line every block around a yard: two to four storeys, timber-framed or stone-dressed, gable to the street or eaves to the street, with jetties and chimneys in the detailed level. Landmarks take whole blocks: a town hall with a clock tower, a cathedral with two 74 m spires, a keep, a market hall, six watch towers. Outside the wall are fields, farmsteads and a stand of sixty-four trees 56–86 m tall with limbs to perch on.

Every static surface samples **one 1024 × 2048 atlas** (`web/src/world/atlas.ts`), painted by a software rasterizer that runs the same in the browser and in Bun. The atlas is a stack of strips that repeat along U. A facade style stores its storeys as adjacent rows, so one quad spanning three rows shows three storeys. Tints (plaster, roof clay, slate) are vertex colours. One texture for the whole world makes a cell one draw.

The generator writes each surface twice: into a render bucket (a 64 m cell and a layer) and into the collision soup the simulation and the bake share.

## The simulation

`maneuver_sim::Sim::tick` advances 1/60 s. The player is a 0.5 m sphere.

- **Wires**: a shoulder button searches a fan of 42 rays on its side for an anchor (distance near `24 + 0.55 × speed` metres, elevation near 30°) and fires a hook at 190 m/s. An attached wire is a rope that only shortens (the constraint removes outward velocity) and pulls at 17 m/s²; with gas it pulls at 47 m/s². A wire that meets a corner re-anchors there. △ fires both wires at the surface under the screen centre.
- **Gas**: ✕ jumps from the ground, reels while a wire holds, and thrusts at 30 m/s² otherwise; a press in the air adds a 9 m/s burst. Gas returns at 2.5 units/s after 1.2 s without use, 10 units/s on the ground, and in full at a depot.
- **Contacts**: the centre is swept along its path, then pushed out of every triangle it touches. A hit at more than 12 m/s turns the velocity along the surface and keeps from 100 % of the speed (grazing) down to 40 % (head-on), so roofs and walls are skimmed.
- **Camera**: third person, 5.2 m back at rest and 6.8 m at speed, field of view 62° to 80°, pulled in by a ray against the world.
- **Pose**: thirteen rigid parts on a small skeleton; target joint angles come from the action (run, slide, fly, reel, thrust, slash) and ease toward them.
- **Autopilot**: plays through the same inputs as a player. It flies the route for the attract mode and gives a repeatable load for measurements.

Transcendentals go through `libm`, so wasm, the host and the Vita compute the same values.

## Compile

```
generator (TypeScript)  →  WorldIR  →  maneuver-cook  →  walled-town.vita60.pack + .compile.json
```

`bun tools/maneuver.ts cook` exports WorldIR (float geometry, the RGBA atlas, the simulation's world file, scene constants, each file's SHA-256 in a manifest) and runs the compiler with `profiles/vita60.json`. The passes, in order:

1. **bake-lighting**: per vertex, `tint × (sun × N·L × visibility + hemisphere(N) × openness)`, with 4 rays toward the sun and 16 cosine-weighted rays for openness, against the collision world. The result is sRGB-encoded at half scale, so a shader multiplies by 2. 1.15 million vertices bake in 2–5 s.
2. **merge-cells**: a 64 m cell gets a near mesh (base + detailed buckets) and a middle mesh (base + simple buckets); a 256 m super-cell gets a far mesh.
3. **quantize**: 16-byte vertices: position `u16 × 3` over the mesh's bounds, texture coordinates `i16 × 2`, colour `u8 × 4`.
4. **atlas-mips-bc1**: eight mip levels, each filtered inside its strip so strips never mix, compressed to BC1.
5. **interface-font**: Inter Display Bold at 18, 26 and 44 px into an 8-bit coverage image.
6. **structural-budgets**: at most 65 535 vertices per mesh, at most 64 MiB per pack.

The current pack is **28.6 MB**: 1 493 meshes, 1.15 million vertices, 549 000 triangles across the three levels, 135 600 collision triangles.

## On the Vita

One scene per frame goes straight into the display surface: the sky dome and the sun, the world, the moving models, the interface.

- **One program for the world**: atlas texel × baked light, then haze. Haze is a function of view depth computed per vertex, and its colour is a constant in the shader source, so the fragment program has no uniforms and a draw uploads one matrix (the mesh's bounds are folded into it).
- **Level of detail**: a super-cell beyond 560 m draws its far mesh; inside that, each cell draws its near mesh within 160 m and its middle mesh beyond. Bounds are tested against the frustum first.
- **Moving geometry** is lit and placed on the CPU into a two-frame ring: about 700 vertices for the character, the targets' nape pads, wire ribbons, and soft discs for gas and the ground shadow.
- **Programs** compile on the device through SceShaccCg on first run and are cached by source hash; the packaged build ships the six GXPs.
- **Sound** is synthesized at 22.05 kHz from the simulation's state and events.

### Measured

PS Vita (PCH-2000, CPU 444 MHz, GPU 222 MHz), development build in Pocket Devkit, 960 × 544, 4× MSAA, autopilot flying the route (`bun tools/maneuver.ts bench`):

| Window | Frames | Late frames | Average frame | Worst frame | Most triangles | Most draws |
| --- | --- | --- | --- | --- | --- | --- |
| 180 s | 10 820 | 0 | 16.683 ms | 16.80 ms | 105 618 | 213 |

CPU time per frame: simulation 0.5–1.0 ms, moving geometry and interface 1.0–1.4 ms, world submission 0.6 ms.

Headroom, measured by drawing the world several times per frame from the heaviest fixed view (on the south gatehouse, the whole town in the frustum, 86 500 triangles): three passes hold 60 fps with 4× MSAA and four passes miss it; without MSAA four passes hold and six miss.

The pack reads from the USB share at **8.6 MB/s**; a start takes 6 s including the first compile of the programs.

## Controls

| | Vita | Keyboard | Gamepad |
| --- | --- | --- | --- |
| Left and right wire (hold) | L, R | Q, E or mouse buttons | L1, R1 |
| Gas: jump, reel, thrust, burst | ✕ | Space | A / ✕ |
| Cut | □ | F | X / □ |
| Aimed pair of wires (hold) | △ | R | Y / △ |
| Let go and dive | ○ | C | B / ○ |
| Move, camera | sticks | WASD, mouse or arrows | sticks |
| Autopilot on or off | START | `?auto` in the URL | |
| Start again | SELECT | Backspace (respawn) | Back |

## Commands

```
bun run setup                          # submodule and dependencies
bun run dev                            # build the wasm simulation, serve the reference on :5273

bun tools/maneuver.ts sim              # wasm + web/src/sim/abi.gen.ts
bun tools/maneuver.ts shot --out a.png [--auto --ticks 900] [--view px,py,pz,tx,ty,tz,fov]
bun tools/maneuver.ts cook             # WorldIR, then the pack and its receipt

bun tools/maneuver.ts serve            # USB host for the console (keep running)
bun tools/maneuver.ts native           # sync the pack, build, replace the binary in Pocket Devkit
bun tools/maneuver.ts status | capture --out f.png
bun tools/maneuver.ts ctl '{"auto":false,"view":{"pos":[0,64,612],"target":[0,20,0],"fov":62}}'
bun tools/maneuver.ts bench --seconds 120
bun tools/maneuver.ts vpk              # standalone PKMV00001 package
bun tools/maneuver.ts push-vpk         # → ux0:data/pocket-maneuver/ for VitaShell

cargo test --workspace
cargo run --release -p maneuver-sim --bin harness -- .pocket-build/world/ir/world.mvsw 600 [--wav out.wav]
```

`ctl` keys: `auto`, `reset`, `view {pos, target, fov}`, `lodNear`, `lodMid`, `repeat` (world passes per frame), `profile`, `world`, `actors`, `hud`, `stats`, `cullCw`, `fetch`. `host0:maneuver/boot.json` takes `{"msaa": 0 | 2 | 4}` at start. `--share DIR` (or `MANEUVER_SHARE`) points the device commands at a USB host that is already running.

The Vita toolchain is VitaSDK, `cargo-vita` and Rust `nightly-2026-05-28` with `rust-src`, as for PocketJS's Vita host.

## Layout

| Path | Contents |
| --- | --- |
| `web/src/world` | generator: plan, houses, landmarks, wall, canal, forest (`city.ts`), building blocks (`build.ts`), atlas painter, moving models, scene constants |
| `web/src/render`, `web/src/game`, `web/src/main.ts` | reference renderer, input, interface |
| `web/scripts/export-world.ts` | WorldIR export |
| `crates/maneuver-sim` | simulation core, wasm interface, snapshot layout, harness |
| `crates/maneuver-pack` | pack container, mesh table and vertex layouts |
| `crates/maneuver-cook` | world compiler |
| `vita/` | Vita app and its Cg programs; LiveArea art under `vita/assets` |
| `profiles/` | compile profiles |
| `tools/` | `maneuver.ts` (commands), `vita.ts` (build and device), `bench.ts`, `shot.ts` |

## Not done

- The numbers above come from the autopilot. Wire pull, gas economy, reach and camera rates are set from simulated runs and have not been tuned by hand on the console.
- The sound has been checked for level (peak 58 % of full scale over a minute of flight), not by ear.
- The reference lights with a shadow map; the device shows the bake. The web app does not draw the cooked pack yet, so comparing the two is by eye.
- The town has no townsfolk, carts or birds.

## License

MIT
