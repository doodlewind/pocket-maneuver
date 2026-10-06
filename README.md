# Pocket Maneuver

A traversal game for the PS Vita, the PSP, the Nintendo 3DS and the iPod touch 4: two wire hooks, a tank of compressed gas and a walled town of about 5 400 houses, at **60 frames per second**. A browser tab plays it too: the same simulation compiled to wasm32, with a wgpu renderer that draws the PS Vita's pack.

The player fires a wire from each hip into a wall or a roof, is pulled along it, lets go and fires the next. Gas reels a wire in faster, or thrusts when no wire holds. Thirty-two giants, 10 to 16 m tall, stand in the streets and among the trees outside the wall; a giant falls when the player cuts the nape of its neck at speed.

| | Screen | Renderer | Measured |
| --- | --- | --- | --- |
| PS Vita | 960 × 544, 4× MSAA, bloom, light shafts, graded composite | GXM, programs compiled on the device | 90 s: 5 420 frames, **0 late**, up to 251 000 triangles |
| Nintendo 3DS | 400 × 240, town map on the lower screen | PICA200 through citro3d | Old 3DS, 60 s: 3 652 frames, **8 late** (0.2 %), up to 52 600 triangles |
| PSP | 480 × 272, 16-bit with dither | GE, fixed function | PSP 2000, 60 s: 3 510 frames, **93 late** (2.6 %), up to 22 300 triangles |
| iPod touch 4 | 480 × 320, played by touch | OpenGL ES 2 on the SGX535 | 60 s: 3 570 frames, **31 late** (0.9 %), up to 36 500 triangles |
| Browser tab | the screen of the handheld the page shows, 4× MSAA, the PS Vita's bloom, light shafts and graded composite | wgpu over WebGPU, the PS Vita's pack and passes | Chrome 154 on an M3 Max: 60 frames a second, 0.25 to 0.74 ms a frame, up to 251 900 triangles |

The repository holds the whole path from authoring to hardware:

- **`web/`** is the reference: a three.js app that generates the world from one seed at load time and runs the game in a browser.
- **`crates/maneuver-sim`** is the game: collision, wire physics, camera, procedural animation, sound and the autopilot. The reference runs it as wasm; every device links it natively. There is one implementation of every rule.
- **`crates/maneuver-cook`** compiles the world for a device profile: it bakes lighting into vertex colours, merges geometry into cells with levels of detail, encodes the atlas and the models in the device's formats and writes one pack with a compile receipt.
- **`vita/`**, **`psp/`**, **`n3ds/`** and **`ipod/`** draw their pack and run the simulation at one tick per display refresh. **`crates/maneuver-handheld`** is the half of the PSP, 3DS and iPod runtimes that does not touch a GPU.
- **`wgpu/`** draws the PS Vita's pack with wgpu: in a browser tab over WebGPU, inside PocketJS's Pocket3D player with the handhelds' shells, and on the build machine, where frames go to files. `wgpu/README.md` has the frame, what differs from the PS Vita's picture and the measurements.
- **`ui/`** is the interface: one PocketJS app, compiled for each device and drawn over the scene by every runtime. **`crates/maneuver-interface`** is the renderer's side of it and the game's flow (title, play, pause, the finished run).

PocketJS (pinned in `vendor/pocketjs`) supplies the device toolchains, the Vita dev host and GXM kernel, the 3DS dev wire and VPK packaging.

It also supplies the app icon. Every console's launcher shows the **Pocket3D icon** from `vendor/pocketjs/engine/pocket3d/icon/`, and this repository holds no icon file: `psp/Psp.toml` and `tools/psp.ts` name `psp/ICON0.PNG` (144 × 80), `tools/vita.ts` gives the VPK packager `vita/icon0.png` (128 × 128, indexed), `n3ds/Makefile` gives `smdhtool` `3ds/icon.png` and `3ds/icon-small.png` (48 × 48 and 24 × 24), and `tools/ipod.ts` copies `ios/Icon.png` and `ios/Icon@2x.png` (57 × 57 and 114 × 114) into the bundle. The picture behind the icon on the XMB (`psp/assets/pic1.png`) and the LiveArea pictures (`vita/assets`) are captures of the game.

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
| PSP, iPod touch 4 | 4 528 | 1 668 – 2 028 | 1 038 – 1 148 |

## Compile

```
generator (TypeScript)  →  WorldIR  →  maneuver-cook --profile …  →  walled-town.<profile>.pack + .compile.json
```

`bun tools/maneuver.ts cook --profile vita60|psp60|n3ds60|ipod60` exports WorldIR (float geometry, the RGBA atlas, the models, the simulation's world file, scene constants, each file's SHA-256 in a manifest) and runs the compiler. The passes, in order:

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

`ipod60` is the 3DS lowering with every texture as 16-bit texels in row order (what OpenGL ES takes as it is), the PSP's model set, detail distances of 64 m, 150 m and 1 000 m, and no map section: 27.0 MB.

## The handheld loop

`crates/maneuver-handheld` is what the PSP, the 3DS and the iPod touch share: which meshes a frame draws, the cloak, wires and soft discs as vertices, the interface as quads, the status record, and `Game::step` around the simulation.

**Hidden giants** are not drawn. Each frame one giant beyond the near distance is tested against the town with three rays from the eye (head, chest, hip); a giant whose three are blocked stays undrawn until a later test sees it. In the town most of the giants in the frustum are behind houses.

**The governor** counts late frames over each 60 frames. More than three pulls the middle and far distances in by 8 %; three windows in a row without one let them out by 4 %, between 55 % and 100 % of the profile's values. The near distance stays, so what is beside the player does not change. `govern=0` switches it off for a measurement.

## The interface

Every 2D pixel that is not anchored to the world comes from one PocketJS app, `ui/`. `ui/pocket.json` declares three presentations, and PocketJS picks one at build time from the device's modality (screens, touch, buttons):

| Presentation | Devices | Screen | Input |
| --- | --- | --- | --- |
| `presentations/single.tsx` | PSP, PS Vita | 480 × 272 logical; the Vita rasters it at 2× | pad; the Vita's panel also takes taps on a list |
| `presentations/dual.tsx` | Nintendo 3DS | 400 × 240 over 320 × 240 | pad, and touch on the lower screen |
| `presentations/touch.tsx` | iPod touch 4 | 480 × 320 | touch only |

A presentation decides where things go and how large they are. What the gas gauge, a list row or the map looks like is in `ui/app/parts.tsx` once, and what the lists hold (the title, a pause, the settings, the controls, the finished run) is in `ui/app/game.ts` once.

- **The split**: the renderer owns the simulation, the scene and the marks anchored to the world (where each wire would bite, the nearest target with its distance, the streaks of speed), which it batches itself from the pack's font. The interface owns the gauges, the lists, the map and, on a touch panel, the controls.
- **The protocol** (`ui/app/protocol.ts`, `crates/maneuver-interface`) is JSON lines over PocketJS's `pocket.overlay` service, answered in the process: the QuickJS API on the Vita and the PSP, the `svcwire` symbols of PocketJS's C hosts on the 3DS. The renderer sends the members of its state that changed since the last line; the interface sends `start`, `pause`, `restart`, `title`, `option`, `prefs`, and from a touch panel `drive` (the stick and the held keys) and `look` (pixels a finger dragged).
- **The flow** is `maneuver_interface::Session`, one implementation for every device: behind the title the autopilot flies the route; play takes the pad; a pause runs no tick and plays no sound; a finished run shows its time against the best one. With no guest on the screen (its files are missing, or it threw) START and SELECT keep the flow as before.
- **The numbers in flight** (speed, gas, the clock, the player on the map, the wires that hold) travel as one array, `t`, refreshed 10 times a second on the PSP, 15 on the 3DS and the iPod touch, 30 on the Vita. Each is written straight to its node (`@pocketjs/framework/hot`) in a cell of fixed size with the text at its left: one native call and no layout. A value set through a Solid signal costs milliseconds per update on the PSP.
- **The guest is offered a turn 30 times a second** (two UI-core ticks per turn) and takes it when it is worth its cost (`maneuver_interface::Pace`). A turn runs the whole framework's frame, 4 to 6 ms on a PSP however little changed. The interface says when it has nothing scheduled (`idle`: no note standing, no hint fading); from then a turn is taken when the renderer has news, when a button the interface listens to changes (in play, START alone), while a finger is down, and for a few turns after any of those. Buttons pressed between two turns are latched, so a press shorter than a turn still arrives.
- **Screens stay built.** The title, the gauges and the pause list are built while the world loads and then shown or hidden: a screen takes tenths of a second to build on the PSP, which the loading screen can spend and the moment play resumes cannot.
- **The map** is the town from above, drawn by the world compiler from the collision triangles (`maneuver-cook --map`) and compiled into the app as an image; `tools/ui.ts` regenerates it when the exported world changes. Each giant is a mark that dims when it is cut; the player's mark moves and turns with `t`. The 3DS shows it on the lower screen during play; the other devices show it beside the pause list.
- **A touch panel** gets a stick in the lower left, round keys in the lower right and the rest of the screen to turn the view. A wire key latches on a short tap and lets go on the next one, so one thumb keeps both wires and still reaches the gas.
- **What is kept between runs** (the best time and the settings) is one JSON text the interface hands to the renderer, which stores it as `interface.json` and hands it back at the start.

`bun tools/ui.ts preview` runs each device's compiled bundle on PocketJS's UI core built for wasm, against a renderer that exists only as state (`ui/test/harness.ts`), and writes every screen as a picture to `.pocket-build/ui/preview/`. `bun tools/ui.ts test` drives the same rig through a run and checks what the interface asked for.

## On the PS Vita

- **One program for the world**: atlas texel × baked light, then haze. Haze is a function of view depth computed per vertex, and its colour is a constant in the shader source, so a draw uploads one matrix.
- **Skinning on the GPU**: 57 uniform rows (three per bone); the vertex program blends two bones and lights with the scene's sun and hemisphere.
- **Post-processing**: the scene renders to a 960 × 544 target with 4× MSAA. A quarter-size pass keeps what is brighter than 0.86 luma, two passes blur it, one pass smears it away from the sun for light shafts, and the composite adds them, grades (contrast, saturation, split tone, vignette) and blurs toward the screen centre above 26 m/s. Every texture coordinate is computed in a vertex program.
- **Programs** compile on the device through SceShaccCg on first run and are cached by source hash; the packaged build ships them.
- **Sound** is synthesized at 22.05 kHz from the simulation's state and events.
- **The interface** is drawn by PocketJS's Vita host library into the display scene, after the composite and the world marks: 480 × 272 logical, rastered at 2×. Its vertices come from vita2d's pool, which each frame takes one half of in turn, so a frame's draws stay valid until the GPU has used them.

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
- **Memory**: PocketJS's arena is the allocator: one kernel block in power-of-two classes, which QuickJS's churn recycles in constant time. The world's buffers live for the whole run, so they take their exact size (`psp/src/mem.rs`): first as kernel blocks from the 2 MB the arena leaves the kernel, then from the arena's uncarved tail. The world holds about 18 MB.
- **The interface** runs on PocketJS's PSP host library (QuickJS, the UI core, its GE backend) and takes about 4.3 MB beside the world, so the package asks for the larger user memory of the 2000 and later models (`MEMSIZE=1`: 53 MB free at the start). With 24 MB the guest is not started and the run's numbers are drawn from the pack's font. A turn is two halves on two frames, the script (4 to 6 ms) and then layout with the list of what it shows (2 to 2.7 ms), each beside the GE's work on the previous frame; its draw is the last pass of the list. In play the guest takes about 12 turns a second.
- **Sound** at 11.025 kHz: the synthesizer costs about 3 µs a frame of sound on this CPU.

`bun tools/psp.ts emu` runs the same PRX in PPSSPPHeadless with its software GE, which follows the console's clipping rule: with `option=4` (no CPU clipping) the ground under a low camera is missing there too.

Measured on a PSP (2000 series, 333 MHz) over PSPLINK, play flown by the autopilot (`bun tools/psp.ts bench --seconds 60`):

| Window | Frames | Late frames | Average frame | Worst frame | Most triangles | Most draws |
| --- | --- | --- | --- | --- | --- | --- |
| 60 s | 3 510 | 93 (2.6 %) | 17.1 ms | 33.4 ms | 22 285 | 193 |

CPU time per frame: simulation 2.8 ms, sound 0.7 ms, display list 6.0 ms (of it the detailed cells with their clip tests 2.8 ms), the interface 1.3 ms. The pack's resident part loads in 0.9 s. Before the interface the same route had 64 late frames in 3 540 with the numbers drawn natively. With the guest's turn and its draw both withheld (`option=24`) a 45 s run has 37 late frames, with the turn alone 85, with the draw alone 47: the cost is the turn's CPU time, not the GE's.

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
- **The lower screen** is the interface's second surface, a GPU target like the upper one: the town from above with a mark for each giant and for the player, the run in numbers and a pause key during play, and the lists (title, pause, settings) a finger or the +Control Pad walks.
- **The guest** is PocketJS's 3DS UI core, its citro3d backend and QuickJS, compiled in beside the game's own Rust core. It takes about 2.2 MB of heap and 5 MB of linear memory (a vertex arena, the font and image textures). Its turn runs before `C3D_FrameBegin`, over the previous frame's GPU work; the upper surface is drawn over the scene every frame, the lower one on the frames a turn ran.
- **Sound**: ndsp when the console has its DSP firmware dumped; otherwise a looping CSND buffer that the synthesizer writes ahead of the play position.
- **The dev wire** is PocketJS's (`vendor/pocketjs/hosts/3ds`), compiled in: a `.3dsx` install replaces the running program, `maneuver.control` steers it, a screenshot request captures both screens.

Measured on an Old 3DS (268 MHz), autopilot flying the route (`bun tools/n3ds.ts bench --seconds 60`):

| Window | Frames | Late frames | Average frame | Worst frame | Most triangles | Most draws | Longest GPU time |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 60 s | 3 652 | 8 (0.2 %) | 16.70 ms | 24.3 ms | 52 619 | 60 | 8.8 ms |

CPU time per frame: simulation, sound and mesh selection 2–4 ms, commands and moving geometry 2.4–3.0 ms. Before the ray work in the simulation (see above) a 90 s run had 69 late frames: the ticks that search for an anchor.

## On the iPod touch 4

`ipod/` is a C shell and OpenGL ES 2 renderer over the 3DS's Rust core, built from the same source for `armv7-apple-ios`. The device has no pad: the game is played through the interface's touch presentation, whose stick, keys and drags reach the simulation as `drive` and `look` commands. `ipod/README.md` has the frame, the limits and the loop.

- **The scene** is the 3DS's passes with a leaner skin shader: the two bones' rows are blended before one transform, a giant beyond the near distance takes one bone, and the light is three precomputed terms. On the SGX535 a skinned vertex cost about four times a world vertex, and the frame follows the vertices submitted.
- **The interface** is PocketJS's UI core with its OpenGL ES 2 backend and the portable QuickJS guest driver. The guest turns every frame (0.6 ms here); what it shows is drawn into a texture when it changed, on every other frame at most, and that texture is laid over the scene.
- **Sound** leaves through an audio queue fed from the synthesizer at 22.05 kHz.

Measured on an iPod touch 4 (A4, iOS 6.1.6), play flown by the autopilot (`bun tools/ipod.ts bench --seconds 60`):

| Window | Frames | Late frames | Average frame | Worst frame | Most triangles | Most draws |
| --- | --- | --- | --- | --- | --- | --- |
| 60 s | 3 570 | 31 (0.9 %) | 16.81 ms | 38.5 ms | 36 549 | 69 |

CPU time per frame: simulation 2.2 ms, the guest's turn 0.6 ms, the interface's redraw 2.5 ms (about 6 ms on a frame that redraws), the scene's commands 2.4 ms.

## In a browser tab

`wgpu/` is the game for a tab: **`maneuver-sim` and `maneuver-interface` compiled to wasm32, a wgpu renderer over WebGPU that reads the PS Vita's pack and draws the PS Vita's passes, and PocketJS's Pocket3D player around it** (`vendor/pocketjs/devices/web/pocket-web-wgpu`). The reference in `web/` is where the world is generated; it is not what a visitor is given.

- **The page shows the game as one of four handhelds** (PS Vita, PSP, Nintendo 3DS, iPod touch): that device's shell, its screen's size, its own bundle of `ui/` in a realm of the page, and its buttons from the keyboard, the shell's keys under a pointer, or a finger. The pack and the passes are the PS Vita's whatever the device; the page says beside the device's name how that device's own build differs.
- **The Pocket3D title card plays first**, and the pack (30.7 MB) is read while it plays; the interface's loading screen counts what has arrived.
- **The same renderer writes frames to files** on the build machine (`wgpu/src/bin/shot.rs`), a sixtieth of a second a frame: `bun tools/listing.ts` records the clips and stills of the game's listing on Pocket Studio that way (`listing/listing.json` holds the words and the takes; the pictures go to the ignored `dist/listing/`).
- `bun tools/wgpu.ts dist` writes the directory a static host serves: 60 files, 38.1 MB, the pack in pieces of 2 MiB named by their hashes.

`wgpu/README.md` names what differs from the PS Vita's picture (texels in place of BC1 blocks, packed vertex numbers scaled in the program, one composite program, the gas standing still under a pause).

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
| Pause | START | START | START, or the key on the lower screen | |
| Lists: move, choose, back | direction pad, ○, ✕ (or a tap) | direction pad, ○, ✕ | +Control Pad, A, B (or a tap) | |

The pause list restarts the run and returns to the title; the web reference keeps `?auto` and Backspace. On the iPod touch every control is drawn on the panel: a stick to move, round keys for the wires (a short tap keeps one, the next lets go), the gas, the cut, the aimed pair and the dive, and a finger on the scene to turn the view.

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

# iPod touch 4 (over SSH; PocketJS's pinned iOS 6 toolchain)
bun tools/ipod.ts cook | deploy | native [--pack] | launch | status | capture --out f.png | bench --seconds 60
bun tools/ipod.ts ctl "mode=play auto=1"

# A browser tab (wasm32-unknown-unknown and wasm-bindgen 0.2.126; Chrome for `check`)
bun tools/wgpu.ts cook | build | serve [--port 8801] | dist | check [--dist]
bun tools/wgpu.ts shot --frames 600 --words "mode=play auto=1" --out f.png   # a frame on this machine's GPU
bun tools/listing.ts [--upload]        # the listing's clips and stills → dist/listing/

# The interface
bun tools/ui.ts psp|vita|3ds|ipod      # compile ui/ for one device → .pocket-build/ui/<device>/
bun tools/ui.ts preview [device…]      # every screen as a picture → .pocket-build/ui/preview/
bun tools/ui.ts test                   # the flow against a mock renderer, on every device's bundle

cargo test --workspace
bun test ./tools/icon.test.ts          # no icon file in the repository; every build names PocketJS's
cargo test --manifest-path wgpu/Cargo.toml
bun test ./tools/wgpu.test.ts ./tools/listing.test.ts
cargo run --release -p maneuver-sim --bin harness -- .pocket-build/world/ir/world.mvsw 600 [--wav out.wav]
```

Vita `ctl` keys: `mode`, `auto`, `ui`, `press`, `reset`, `view {pos, target, fov}`, `lodNear`, `lodMid`, `repeat`, `profile`, `world`, `actors`, `hud`, `stats`, `sound`, `cullCw`, `post {…}`, `fetch`. The handhelds take words: `auto hud sound stats world actors govern` (0 or 1), `lodNear lodMid lodFar repeat option` (a number), `reset=1`, `view=px,py,pz,tx,ty,tz,fov` and `view=off`.

On every device `mode=title|play|paused|results` sets the game's flow (`mode=play auto=1` is play flown by the autopilot: what `bench` measures), `ui=start|pause|resume|restart|title` asks what the interface would ask, and `press=<mask>` presses PocketJS buttons on the guest.

Toolchains: VitaSDK and `cargo-vita`; rust-psp's `cargo psp` (PocketJS's pinned SDK); devkitARM in PocketJS's pinned container for the 3DS C code, and `armv6k-nintendo-3ds` with `build-std` for its Rust core; PocketJS's pinned iOS 6 sysroot with Xcode's clang and `ld-classic` for the iPod touch, and `armv7-apple-ios` with `build-std` for its core.

## Layout

| Path | Contents |
| --- | --- |
| `web/src/world` | generator: plan, houses, landmarks, wall, canal, forest (`city.ts`), building blocks (`build.ts`), atlas painter, model builders (`sdf.ts`, `scout.ts`, `titan.ts`), scene constants |
| `web/src/render`, `web/src/game`, `web/src/main.ts` | reference renderer, input, interface |
| `web/scripts/export-world.ts` | WorldIR export |
| `crates/maneuver-sim` | simulation core, wasm interface, snapshot layout, harness |
| `crates/maneuver-pack` | pack container, mesh tables and vertex layouts |
| `crates/maneuver-cook` | world compiler; `handheld.rs` lowers for the PSP, the 3DS and the iPod touch |
| `crates/maneuver-handheld` | mesh selection, moving geometry, the marks on the world, the loop around the simulation, guard-band clipping |
| `crates/maneuver-interface` | the interface's protocol, the game's flow (`Session`), the in-process channel for a PocketJS guest |
| `ui/` | the interface: `pocket.json` (presentations), `app/` (protocol, state, shared parts, one presentation per device shape), `test/` (mock renderer, previews, flow test) |
| `vita/` | Vita app and its Cg programs; LiveArea art under `vita/assets` |
| `psp/` | PSP program: GE renderer, cell reader, exact-size memory, the interface's guest, sound; the XMB background under `psp/assets` |
| `n3ds/` | 3DS program: C host and renderer, PICA shaders, the Rust core |
| `ipod/` | iPod touch program: C shell and OpenGL ES 2 renderer; `core/` builds the 3DS core's source for the device |
| `wgpu/` | the browser's program: the wgpu renderer (the PS Vita's passes in WGSL), the shell around the simulation, the page, and the program that writes frames to files |
| `listing/` | the words of the game's listing on Pocket Studio, and how each of its pictures is recorded |
| `profiles/` | compile profiles |
| `tools/` | `maneuver.ts`, `vita.ts`, `psp.ts`, `n3ds.ts`, `ipod.ts`, `ui.ts`, `wgpu.ts`, `listing.ts`, `bench.ts`, `shot.ts` |

## Not done

- The interface is measured on the PSP. On the 3DS it has run in Azahar and on the Vita it only builds: its cost per frame there, and touch on both, are not measured yet.
- The iPod touch's controls have been driven by remote touches, not by thumbs: where the stick and the keys sit, and how the latching wires feel, are untested by a person. Its sound has not been heard.
- The PSP misses about one frame in forty on the autopilot's route, in the densest streets.
- The 3DS's CSND sound path and the PSP's sound have not been heard by a person; the Vita's sound has been checked for level, not by ear.
- The numbers come from the autopilot. Wire pull, gas economy, reach and camera rates are set from simulated runs and have not been tuned by hand on a console.
- No stereoscopic 3D on the 3DS: a second eye doubles the 8 to 10 ms of GPU time per frame.
- The reference lights with a shadow map; the devices show the bake. `wgpu/` draws the PS Vita's cooked pack on the build machine, but no tool compares its frame with a capture from the console yet.
- The browser tab reads the whole pack (30.7 MB) before its first frame of the world: 17.7 s on a line of 16 Mbit/s. It has been run in Chrome on one Mac; no other browser, no phone and no other GPU has drawn it, and nobody has judged its sound by ear.
- The town has no townsfolk, carts or birds.

## License

MIT
