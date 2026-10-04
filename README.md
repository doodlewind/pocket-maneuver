# Pocket Maneuver

A traversal game for the PS Vita, the PSP and the Nintendo 3DS: two wire hooks, a tank of compressed gas and a walled town of about 5 400 houses, at **60 frames per second**.

The player fires a wire from each hip into a wall or a roof, is pulled along it, lets go and fires the next. Gas reels a wire in faster, or thrusts when no wire holds. Thirty-two giants, 10 to 16 m tall, stand in the streets and among the trees outside the wall; a giant falls when the player cuts the nape of its neck at speed.

| | Screen | Renderer | Measured |
| --- | --- | --- | --- |
| PS Vita | 960 × 544, 4× MSAA, bloom, light shafts, graded composite | GXM, programs compiled on the device | 90 s: 5 420 frames, **0 late**, up to 251 000 triangles |
| Nintendo 3DS | 400 × 240, town map on the lower screen | PICA200 through citro3d | Old 3DS, 60 s: 3 652 frames, **8 late** (0.2 %), up to 52 600 triangles |
| PSP | 480 × 272, 16-bit with dither | GE, fixed function | PSP 2000, 60 s: 3 540 frames, **64 late** (1.8 %), up to 23 500 triangles |

The repository holds the whole path from authoring to hardware:

- **`web/`** is the reference: a three.js app that generates the world from one seed at load time and runs the game in a browser.
- **`crates/maneuver-sim`** is the game: collision, wire physics, camera, procedural animation, sound and the autopilot. The reference runs it as wasm; every device links it natively. There is one implementation of every rule.
- **`crates/maneuver-cook`** compiles the world for a device profile: it bakes lighting into vertex colours, merges geometry into cells with levels of detail, encodes the atlas and the models in the device's formats and writes one pack with a compile receipt.
- **`vita/`**, **`psp/`** and **`n3ds/`** draw their pack and run the simulation at one tick per display refresh. **`crates/maneuver-handheld`** is the half of the PSP and 3DS runtimes that does not touch a GPU.

PocketJS (pinned in `vendor/pocketjs`) supplies the device toolchains, the Vita dev host and GXM kernel, the 3DS dev wire and VPK packaging.

## The world

`web/src/world/city.ts` lays the town out on a radial plan: a central square, **eight bands of blocks** between ring streets, four avenues to the gates, a canal ring with eight bridges, and a **50 m wall** with a walk on top. Row houses line every block around a yard: two to four storeys, timber-framed or stone-dressed, gable to the street or eaves to the street, with jetties and chimneys in the detailed level. Landmarks take whole blocks: a town hall with a clock tower, a cathedral with two 74 m spires, a keep, a market hall, six watch towers. Outside the wall are fields, farmsteads and a stand of sixty-four trees 56–86 m tall with limbs to perch on.

Every static surface samples **one 1024 × 2048 atlas** (`web/src/world/atlas.ts`), painted by a software rasterizer that runs the same in the browser and in Bun. The atlas is a stack of strips that repeat along U. A facade style stores its storeys as adjacent rows, so one quad spanning three rows shows three storeys. Tints (plaster, roof clay, slate) are vertex colours.

The generator writes each surface into render buckets (a cell and a layer) and into the collision soup the simulation and the bake share. The layers are levels of detail: `Near` and `Mid` per 64 m cell, `Far` per 256 m, and `Horizon` per 128 m for the handhelds, where **each run of houses is one roofed mass of 10 triangles**.

## The simulation

`maneuver_sim::Sim::tick` advances 1/60 s. The player is a 0.5 m sphere.

- **Wires**: a shoulder button searches a fan of 42 rays on its side for an anchor (distance near `24 + 0.55 × speed` metres, elevation near 30°) and fires a hook at 190 m/s. An attached wire is a rope that only shortens and pulls at 17 m/s²; with gas it pulls at 47 m/s². Past its length it stretches 5 % against a spring. A wire that meets a corner re-anchors there. The aimed pair fires both wires at the surface under the screen centre.
- **Gas**: jumps from the ground, reels while a wire holds, and thrusts at 30 m/s² otherwise; a press in the air adds a 9 m/s burst. Gas returns at 2.5 units/s after 1.2 s without use, 10 units/s on the ground, and in full at a depot.
- **Rays** walk a 16 m grid over the collision triangles. A cell whose triangles lie wholly above or below the ray's span in it is skipped, and a triangle is tested only if the ray meets its plane inside the cell: two dot products before the full intersection. An anchor search is 42 rays of up to 95 m; this halved the cost of a tick (29.4 to 13.9 µs on the host) with the same results over 180 000 ticks of autopilot.
- **Contacts**: the centre is swept along its path, then pushed out of every triangle it touches, the giants' bodies included. A hit at more than 12 m/s turns the velocity along the surface and keeps from 100 % of the speed (grazing) down to 40 % (head-on).
- **Camera**: third person, 5.2 m back at rest and 6.8 m at speed, field of view 62° to 80°, pulled in by a ray against the world. With no camera input for 0.55 s it turns to the direction of travel, which is how a machine with one stick steers it.
- **Pose**: a **19-bone skeleton**. Key poses (stand, run, slide, air, fly, hang, reel, thrust) blend as quaternions; springs carry pitch, roll and drag; firing a wire swings that arm to the anchor; the legs are placed by two-bone IK on feet whose stride length follows the speed, so a foot stays where it lands. The cloak is a 7 × 8 cloth and each wire a 14-point rope, both integrated per tick.
- **Autopilot**: plays through the same inputs as a player. It flies the route for the attract mode and gives a repeatable load for measurements.

Transcendentals go through `libm`, so wasm, the host, the Vita and the 3DS compute the same values. The PSP's FPU has no doubles; its build swaps in single-precision kernels (`fastmath.rs`, within 4 × 10⁻⁶ of `libm`) and repeats against itself only.

## Models

`web/src/world/sdf.ts` builds the player and the three giant builds as signed-distance bodies on the simulation's bind pose and polygonizes them with surface nets. Skin weights come from the primitives a vertex is nearest to. The mesh cell size is the level of detail: the exporter writes the player at three densities and each giant at four, and a device profile names the ones it packs.

| | Player | Giant, near | Giant, far |
| --- | --- | --- | --- |
| PS Vita | 35 066 triangles | 19 216 – 28 276 | 2 908 – 4 064 |
| Nintendo 3DS | 6 328 | 2 908 – 4 064 | 1 668 – 2 028 |
| PSP | 4 528 | 1 668 – 2 028 | 1 038 – 1 148 |

## Compile

```
generator (TypeScript)  →  WorldIR  →  maneuver-cook --profile …  →  walled-town.<profile>.pack + .compile.json
```

`bun tools/maneuver.ts cook --profile vita60|psp60|n3ds60` exports WorldIR (float geometry, the RGBA atlas, the models, the simulation's world file, scene constants, each file's SHA-256 in a manifest) and runs the compiler. The passes, in order:

1. **bake-lighting**: per vertex, `tint × (sun × N·L × visibility + hemisphere(N) × openness)`, with 4 rays toward the sun and 16 cosine-weighted rays for openness, against the collision world. The result is sRGB-encoded at half scale, so the device multiplies by 2.
2. **merge-cells**: a 64 m cell gets a near mesh (base + detailed buckets) and a middle mesh (base + simple buckets); a far cell gets a far mesh.
3. **quantize**: into the device's vertex layout.
4. **atlas-mips**: mip levels filtered inside each strip so strips never mix, then the device's texture format.
5. **interface-font**: Inter Display Bold at the profile's pixel sizes.
6. **structural-budgets**: vertices per mesh, bytes per pack.

What differs by profile:

| | `vita60` | `psp60` | `n3ds60` |
| --- | --- | --- | --- |
| Atlas | 1024 × 2048 BC1, 8 levels | two 512 × 512 pages, 8-bit indices and a 256-colour palette each, swizzled | two 512 × 512 pages, RGB565 in 8 × 8 Morton tiles |
| Static vertex | 16 bytes, position over the mesh's bounds | 12 bytes in GE component order, colour 5650 | 16 bytes, position on one grid of 1/24 m for the whole world |
| Detail distances | 160 m, 560 m | 44 m, 100 m, far cells to 480 m | 64 m, 170 m, far cells to 1 200 m |
| Skinned models | two bones per vertex, one draw | draws of at most four bones, the GE's blend | two bones per vertex, one draw |
| Collision | world file, grid built at load | grid stored built, read into place | world file, grid built at load |
| Extra | | detailed cells in a section read on demand; large triangles in groups with a clip distance each | meshes grouped on shared vertex bases; the town from above |
| Pack | 30.7 MB | 26.8 MB (17.1 MB read at start) | 27.3 MB |

## The handheld loop

`crates/maneuver-handheld` is what the PSP and the 3DS share: which meshes a frame draws, the cloak, wires and soft discs as vertices, the interface as quads, the status record, and `Game::step` around the simulation.

**Hidden giants** are not drawn. Each frame one giant beyond the near distance is tested against the town with three rays from the eye (head, chest, hip); a giant whose three are blocked stays undrawn until a later test sees it. In the town most of the giants in the frustum are behind houses.

**The governor** counts late frames over each 60 frames. More than three pulls the middle and far distances in by 8 %; three windows in a row without one let them out by 4 %, between 55 % and 100 % of the profile's values. The near distance stays, so what is beside the player does not change. `govern=0` switches it off for a measurement.

## On the PS Vita

- **One program for the world**: atlas texel × baked light, then haze. Haze is a function of view depth computed per vertex, and its colour is a constant in the shader source, so a draw uploads one matrix.
- **Skinning on the GPU**: 57 uniform rows (three per bone); the vertex program blends two bones and lights with the scene's sun and hemisphere.
- **Post-processing**: the scene renders to a 960 × 544 target with 4× MSAA. A quarter-size pass keeps what is brighter than 0.86 luma, two passes blur it, one pass smears it away from the sun for light shafts, and the composite adds them, grades (contrast, saturation, split tone, vignette) and blurs toward the screen centre above 26 m/s. Every texture coordinate is computed in a vertex program.
- **Programs** compile on the device through SceShaccCg on first run and are cached by source hash; the packaged build ships them.
- **Sound** is synthesized at 22.05 kHz from the simulation's state and events.

Measured on a PS Vita (PCH-2000, CPU 444 MHz, GPU 222 MHz), development build in Pocket Devkit, autopilot flying the route (`bun tools/maneuver.ts bench`):

| Window | Frames | Late frames | Average frame | Worst frame | Most triangles | Most draws |
| --- | --- | --- | --- | --- | --- | --- |
| 90 s | 5 420 | 0 | 16.683 ms | 16.785 ms | 251 115 | 225 |

From the heaviest fixed view, the world can be drawn twice per frame at 60 fps; three times takes 17.75 ms.

## On the PSP

The GE has no programmable stage, 2 MB of video memory, a 16-bit depth buffer, textures up to 512 × 512, and the base console has 24 MB of memory. `psp/` (Rust on rust-psp, `no_std`) is built around those:

- **Fixed function**: atlas texel × vertex colour × 2 (the GE's colour doubling), linear haze from 70 m to 500 m. Skinned models take the sun and the sky as two directional lights.
- **Palette textures and a 16-bit frame buffer**: each atlas page is 8-bit indices into its own 256 colours (median cut), swizzled, in video memory; the frame buffer is 5650 with ordered dither. The GE reads a texel before it tests depth, so every covered pixel pays for its texture read: with DXT1 pages and a 32-bit buffer a street view's 13 000 world triangles took **16 ms** of GE time, with this pair **7 ms**.
- **Two depth ranges**: the cells around the eye, the character and the near giants draw with a frustum from 0.4 m to 224 m and three quarters of the depth buffer; everything beyond draws with a frustum from 35 m and the last quarter; the sky draws last, at the far end, where nothing else was drawn.
- **Clipping on the CPU**: the GE clips against the near plane only and drops a triangle with a vertex outside its 4096-pixel coordinate space, so the ground under the camera disappears. The compiler puts each mesh's triangles with an edge over 3 m at the end of its index list, in groups of neighbours (at most 4 × 4 over the mesh), the largest first inside a group, with a distance per triangle. The runtime measures its distance to each group and looks at the prefix that could reach the guard band from there: depth along the view decides most, the rest get their clip coordinates tested and are cut against a frustum twice the view's size (`maneuver_handheld::clip`). A frame looks at about 1 200 triangles and cuts under twenty.
- **Cells on demand**: the detailed meshes (9.7 MB) stay in the pack. A second thread, lower in priority than the frame's, reads a cell (at most 48 KiB) into one of 16 buffers when the eye comes within 70 m; it runs while the frame waits for the GE. Until a cell arrives, its simple mesh draws.
- **The frame overlaps the GE**: a frame builds its display list while the GE draws the previous one, then waits for it and swaps on the display refresh.
- **Memory**: allocations of 64 KiB or more are kernel blocks of their exact size; smaller ones share one 2 MiB block. The program holds 18.5 MB.
- **Sound** at 11.025 kHz: the synthesizer costs about 3 µs a frame of sound on this CPU.

`bun tools/psp.ts emu` runs the same PRX in PPSSPPHeadless with its software GE, which follows the console's clipping rule: with `option=4` (no CPU clipping) the ground under a low camera is missing there too.

Measured on a PSP (2000 series, 333 MHz) over PSPLINK, autopilot flying the route (`bun tools/psp.ts bench --seconds 60`):

| Window | Frames | Late frames | Average frame | Worst frame | Most triangles | Most draws |
| --- | --- | --- | --- | --- | --- | --- |
| 60 s | 3 540 | 64 (1.8 %) | 17.0 ms | 33.3 ms | 23 525 | 201 |

CPU time per frame: simulation 2.8 ms, sound 0.7 ms, display list 5.9 ms (of it the detailed cells with their clip tests 2.8 ms). The pack's resident part loads in 0.9 s.

How it got there, from a first run at 25 to 28 ms a frame:

| Change | Effect on the console |
| --- | --- |
| Palette pages and a 16-bit buffer instead of DXT1 and 32-bit | GE time for the world 16 ms → 7 ms |
| Giants hidden by the town are not drawn (see the handheld loop) | 10 ms of GE time in a street with six hidden giants |
| Rays skip cells and planes (see the simulation) | simulation 5.6–8.7 ms → 2.8 ms |
| Large triangles in groups, a depth test before any transform | clip tests 5.7–7.9 ms → 2.8 ms |
| Integer number formatting, the statistics line every 20 frames | interface 2.2–2.9 ms → 0.7 ms |
| Sound at half the rate | 1.7–3.5 ms → 0.7 ms |

Drawing front to back did not help: the GE does not skip the texture read of a pixel that fails the depth test.

## On the Nintendo 3DS

`n3ds/` is a C host and citro3d renderer over a Rust static library (`n3ds/core`, `no_std`) that wraps the simulation and `maneuver-handheld`:

- **Fragment stage**: one texture environment stage (atlas texel × vertex colour, scaled by 2) and the PICA's fog table, filled with the reference's `1 − exp(−(depth × density)²)`.
- **Runs of meshes in one draw**: every static vertex is on one position grid, so no draw needs its own transform. The compiler gives the meshes of a 4 × 4 block of cells a shared vertex base and lays their indices end to end; a run of adjacent visible meshes is one call. That took a frame from 250 draws to 60 and the command time from 7.5 ms to 3.0 ms.
- **Skinning on the GPU**: 57 uniform rows, two bones per vertex; a vertex stores each bone's first row, so the shader's address register takes it as it is. The square root stands in for the 1/2.2 power.
- **The lower screen** shows the town from above (240 × 240, drawn by the compiler from the collision triangles), a mark for each giant, the player's position and heading, and the run in numbers. An update restores the patch under the player's last mark, a few hundred texels.
- **Sound**: ndsp when the console has its DSP firmware dumped; otherwise a looping CSND buffer that the synthesizer writes ahead of the play position.
- **The dev wire** is PocketJS's (`vendor/pocketjs/hosts/3ds`), compiled in: a `.3dsx` install replaces the running program, `maneuver.control` steers it, a screenshot request captures both screens.

Measured on an Old 3DS (268 MHz), autopilot flying the route (`bun tools/n3ds.ts bench --seconds 60`):

| Window | Frames | Late frames | Average frame | Worst frame | Most triangles | Most draws | Longest GPU time |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 60 s | 3 652 | 8 (0.2 %) | 16.70 ms | 24.3 ms | 52 619 | 60 | 8.8 ms |

CPU time per frame: simulation, sound and mesh selection 2–4 ms, commands and moving geometry 2.4–3.0 ms. Before the ray work in the simulation (see above) a 90 s run had 69 late frames: the ticks that search for an anchor.

## Controls

| | Vita | PSP | 3DS | Keyboard |
| --- | --- | --- | --- | --- |
| Left and right wire (hold) | L, R | L, R | L, R (or ZL, ZR) | Q, E or mouse buttons |
| Gas: jump, reel, thrust, burst | ✕ | ✕ | B | Space |
| Cut | □ | □ | Y | F |
| Aimed pair of wires (hold) | △ | △ | X | R |
| Let go and dive | ○ | ○ | A | C |
| Move | left stick | analog stick | Circle Pad | WASD |
| Camera | right stick | direction pad | +Control Pad or C-Stick | mouse or arrows |
| Autopilot on or off | START | START | START | `?auto` in the URL |
| Start again | SELECT | SELECT | SELECT | Backspace |

## Commands

```
bun run setup                          # submodule and dependencies
bun run dev                            # build the wasm simulation, serve the reference on :5273

bun tools/maneuver.ts sim              # wasm + web/src/sim/abi.gen.ts
bun tools/maneuver.ts shot --out a.png [--auto --ticks 900] [--view px,py,pz,tx,ty,tz,fov]
bun tools/maneuver.ts cook [--profile vita60|psp60|n3ds60] [--no-export]

# PS Vita
bun tools/maneuver.ts serve            # USB host for the console (keep running)
bun tools/maneuver.ts native           # sync the pack, build, replace the binary in Pocket Devkit
bun tools/maneuver.ts status | capture --out f.png | bench --seconds 120
bun tools/maneuver.ts ctl '{"auto":false,"view":{"pos":[0,64,612],"target":[0,20,0],"fov":62}}'
bun tools/maneuver.ts vpk | push-vpk   # standalone PKMV00001 package; send it to ux0:data/pocket-maneuver/

# PSP (PSPLINK and usbhostfs_pc running)
bun tools/psp.ts build | run | status | capture | bench --seconds 60 | package
bun tools/psp.ts ctl "auto=0 view=6,2.2,300,0,4,200,62"
bun tools/psp.ts emu --frames 240 [--ctl "…"] [--out f.png] [--standalone]   # PPSSPPHeadless

# Nintendo 3DS (a Pocket Runtime .3dsx running and paired; installs are .3dsx)
bun tools/n3ds.ts build | install | status | capture | bench --seconds 90
bun tools/n3ds.ts ctl "auto=0 stats=1"

cargo test --workspace
cargo run --release -p maneuver-sim --bin harness -- .pocket-build/world/ir/world.mvsw 600 [--wav out.wav]
```

Vita `ctl` keys: `auto`, `reset`, `view {pos, target, fov}`, `lodNear`, `lodMid`, `repeat`, `profile`, `world`, `actors`, `hud`, `stats`, `cullCw`, `post {…}`, `fetch`. The handhelds take words: `auto hud stats world actors govern` (0 or 1), `lodNear lodMid lodFar repeat option` (a number), `reset=1`, `view=px,py,pz,tx,ty,tz,fov` and `view=off`.

Toolchains: VitaSDK and `cargo-vita`; rust-psp's `cargo psp` (PocketJS's pinned SDK); devkitARM in PocketJS's pinned container for the 3DS C code, and `armv6k-nintendo-3ds` with `build-std` for its Rust core.

## Layout

| Path | Contents |
| --- | --- |
| `web/src/world` | generator: plan, houses, landmarks, wall, canal, forest (`city.ts`), building blocks (`build.ts`), atlas painter, model builders (`sdf.ts`, `scout.ts`, `titan.ts`), scene constants |
| `web/src/render`, `web/src/game`, `web/src/main.ts` | reference renderer, input, interface |
| `web/scripts/export-world.ts` | WorldIR export |
| `crates/maneuver-sim` | simulation core, wasm interface, snapshot layout, harness |
| `crates/maneuver-pack` | pack container, mesh tables and vertex layouts |
| `crates/maneuver-cook` | world compiler; `handheld.rs` lowers for the PSP and the 3DS |
| `crates/maneuver-handheld` | mesh selection, moving geometry, interface, the loop around the simulation, guard-band clipping |
| `vita/` | Vita app and its Cg programs; LiveArea art under `vita/assets` |
| `psp/` | PSP program: GE renderer, cell reader, allocator, sound |
| `n3ds/` | 3DS program: C host and renderer, PICA shaders, the Rust core |
| `profiles/` | compile profiles |
| `tools/` | `maneuver.ts`, `vita.ts`, `psp.ts`, `n3ds.ts`, `bench.ts`, `shot.ts` |

## Not done

- The PSP misses about one frame in fifty on the autopilot's route, in the densest streets.
- The 3DS's CSND sound path and the PSP's sound have not been heard by a person; the Vita's sound has been checked for level, not by ear.
- The numbers come from the autopilot. Wire pull, gas economy, reach and camera rates are set from simulated runs and have not been tuned by hand on a console.
- No stereoscopic 3D on the 3DS: a second eye doubles the 8 to 10 ms of GPU time per frame.
- The reference lights with a shadow map; the devices show the bake. The web app does not draw a cooked pack yet, so comparing the two is by eye.
- The town has no townsfolk, carts or birds.

## License

MIT
