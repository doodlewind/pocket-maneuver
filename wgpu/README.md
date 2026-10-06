# Pocket Maneuver in a browser tab

The game of the handheld builds, drawn with [wgpu](https://wgpu.rs) 25: over WebGPU in a tab (wasm32), and over Metal on the machine that builds it, where frames go to files. It reads **the PS Vita's pack** (`profiles/vita60.json`) as it is and draws **the PS Vita's passes**. The simulation is `crates/maneuver-sim` and the game's flow is `crates/maneuver-interface`, as every device links them. The game's own interface (`ui/`) runs over the scene, and the page shows the game as one of four handhelds (PS Vita, PSP, Nintendo 3DS, iPod touch): its screens, its presentation of the interface, its buttons. `web/` is something else: the three.js reference the world is generated in.

| Part | What it is |
| --- | --- |
| `src/app.rs` | The shell: a frame as `step`, the guest's turn, `draw`; the flow (`Session`), the settings, the screen's shape and each handheld's pad; the sound's blocks. |
| `src/render.rs`, `src/shaders/` | The renderer: the PS Vita's Cg programs (`vita/shaders`) in WGSL, the buffers, and the passes of a frame. |
| `src/world.rs`, `src/actors.rs`, `src/marks.rs`, `src/scene.rs`, `src/mat.rs` | What a frame draws, as `vita/src` decides it: the cells' levels of detail, the skinned models, the cloak, the wires and the gas as vertices, the marks on the world, the scene's constants. |
| `src/pack.rs` | The pack read over HTTP with its progress, and BC1 blocks decoded to texels. |
| `src/web.rs`, `page/` | The tab: what the page calls, and the page itself. |
| `src/bin/shot.rs` | Frames on this machine's GPU: one as a PNG, or a run as rows of RGBA for an encoder; two pictures compared. |
| `vendor/pocketjs/devices/web/pocket-web-wgpu` | PocketJS's browser kernel: what is not this game's. The device and the screens, the overlay pass, ranges of a pack, and the page itself: the title card first and the frame loop, the interface's guest in its realm, and **the Pocket3D player** (the bar, each handheld's shell with its keys as the controls, the way to Pocket Studio). |
| `tools/wgpu.ts` | cook, build, serve, dist, shot, check. |
| `tools/listing.ts`, `listing/listing.json` | The game's listing on Pocket Studio: its words, and the clips and stills recorded from this renderer. |

```
bun tools/wgpu.ts cook               # the PS Vita's pack → .pocket-build/world/walled-town.vita60.pack
bun tools/wgpu.ts serve              # build when there is no site, then http://127.0.0.1:8801/
bun tools/wgpu.ts shot --frames 600 --words "mode=play auto=1" --out a.png
bun tools/wgpu.ts check              # Chrome: every device from the title into play, by keys and pointer
bun tools/wgpu.ts dist               # the directory a static host serves
bun tools/wgpu.ts check --dist       # the same check of that directory
bun tools/listing.ts                 # the listing's pictures → dist/listing/
```

The build needs the `wasm32-unknown-unknown` target and `wasm-bindgen` 0.2.126 on the path (the version in `Cargo.lock`; `build` refuses another). It compiles the interface for the four devices as `bun tools/ui.ts` does, so it needs what that needs: `bun install` in `vendor/pocketjs`, and the exported world under `.pocket-build/world/ir` for the map, which `cook` writes. The kernel's page modules, the title card, the realm and PocketJS's UI core are staged by PocketJS's own tool (`stagePocket3dWeb` of `vendor/pocketjs/tools/pocket3d-web.ts`).

## The launch

The page plays the Pocket3D title card first (`playTitle()` of PocketJS's `pocket3d-title`, copied into the site as it is): 144 ticks, 2.4 s, over the whole page. While it plays the page opens the renderer on the canvas, starts the interface's guest and reads the pack. When the card has ended the canvas is shown. While the pack is on its way the frames show the interface alone, which says `Reading the world · 12.3 of 30.7 MB` (`Mode::Loading`), as a device's does; a start that fails is said the same way. A browser without WebGPU is told so in one sentence after the card; there is no other renderer.

## The pack over HTTP

**The first frame of the world needs the whole pack, 30.7 MB**: every section is read before the game starts, as the PS Vita reads it. Reads are `Source::range(offset, size)` of the kernel, 8 MiB at a time, so the loading screen counts.

The source has two forms:

- **The pack's file**, on a server that answers byte ranges. `serve` does this.
- **The pack cut into pieces of one size** with a manifest that lists them (`<meta name="pocket-pack">` names a `.json`). A read fetches the pieces it lies in, whole and with plain requests, all at once. `dist` writes this form.

`bun tools/wgpu.ts dist` writes `.pocket-build/wgpu/dist` for a host that limits a file to 32 MiB and keeps a file ten minutes in a browser's cache (Pocket Studio's site deployments): `index.html` and `icon.png`; everything else of the site under `app/<build>/`, named by a hash of its contents; `pack/<hash>.json` and the pieces, each named by its own hash. Only the page is asked for again at every visit. With pieces of 2 MiB the directory is **60 files and 38.1 MB**: 15 pieces and their manifest (30.7 MB), 42 files of the site (7.4 MB), the page and the icon. `dist` refuses a directory the host would (a file over 32 MiB, more than 4 000 files or 1 GiB, a top-level `play/` or `runtime/`). A checkout that `pocket-studio register` has linked (`.pocket-studio.json`, which Git ignores) gets the project's id and the Studio's origin written into the page (`<meta name="pocket-app">`, `<meta name="pocket-studio">`): the player's door then leads to the game's card. Nothing uploads the directory.

## The interface

The interface is the game's own: `ui/`, compiled for a device by `tools/ui.ts`, the bundle and its pak as that device loads them. The page runs it as a guest in a realm of its own: PocketJS's AppInstance on the UI core built for wasm, started without the text worker. The guest opens the service it opens on a device, `pocket.overlay`, and the lines are carried in the page:

1. `step`: what the guest asked for on its last turn (`start`, `pause`, `restart`, `title`, `option`, `prefs`, `drive`, `look`) goes to `Session::command`; then the simulation's ticks; then `Session::publish` writes the state the interface is shown.
2. The guest's turn. **A turn is offered as often as the device's own host offers one**: 30 times a second on the PS Vita, the PSP and the 3DS, 60 on the iPod touch (`turns` in `SHAPES`). The realm is told that rate before the bundle runs (`simHz`). A turn is taken on the frames `maneuver_interface::Pace` says it is worth one: the line of state it has not seen goes in (`heard`), with the contacts and the buttons held at any moment since a turn was last offered, and what it sends comes back (`say`).
3. When the guest's draw hash has changed, its picture is drawn again and handed to the renderer.
4. `draw`: the scene, then the picture over it in a pass of its own, premultiplied.

A device's second screen is the interface's alone: the 3DS's lower screen (the town from above, the run in numbers, the lists) is drawn into a canvas of its own from the core's opaque pixels, when its own draw hash changes.

What the interface keeps between runs (the best time and the settings, `prefs`) is in the browser's `localStorage`, where a device has a file.

## The devices

The page shows one handheld at a time. A device is the renderer's shape (`SHAPES` in `src/app.rs`: the size, the interface's turns a second, how often the numbers in flight reach it, and whether the camera has a stick of its own), the interface's bundle for it, and the build plan PocketJS wrote for that bundle, from which the page reads the screens, the raster density and which surface takes touch.

| | Scene | Interface | Turns of the interface a second | Camera | Touch |
| --- | --- | --- | --- | --- | --- |
| PS Vita | 960 × 544 | `single`, 480 × 272 at two samples a pixel | 30 | the right stick | the screen takes taps on a list |
| PSP | 480 × 272 | `single`, 480 × 272 | 30 | the direction pad | none |
| Nintendo 3DS | 400 × 240 | `dual`: 400 × 240 over it, and a 320 × 240 screen under it | 30 | the direction pad | the lower screen |
| iPod touch | 480 × 320 | `touch`, 480 × 320 | 60 | a finger on the scene | the screen; no buttons |

**The page is PocketJS's Pocket3D player** (`createPlayer` of the kernel): `page/index.html` has an empty body, and `main.js` says what the game is and draws into the player's canvas. Picking another device in play changes the shell and the screen's shape, starts that device's guest in a new realm and leaves the run as it is: the new guest is told the whole state on its first turn. The first device is the iPod touch for a browser whose pointer is a finger and the PS Vita otherwise (`?device=`).

**Whatever the device, the pack and the passes are the PS Vita's.** A PSP, a 3DS or an iPod touch draws its own pack with fewer triangles a model, no bloom and no light shafts; the page says so beside the device's name (`note` in `DEVICES` of `main.js`).

What the buttons do is the game's, the same on every device with a pad (`Shape::pad`, from `session_pad` in `vita/src/main.rs` and `read_pad` in `psp/src/main.rs` and `n3ds/src/main.c`):

| | Keys |
| --- | --- |
| Move: the stick (the left one of two) | W A S D |
| Camera: the right stick of a PS Vita; the direction pad elsewhere | I J K L; the arrows |
| Left and right wire: L, R | Q, E |
| Gas, cut, aimed pair, let go: the face buttons at the bottom, the left, the top and the right | X, C, V, Z |
| Pause: START | Space (or Escape) |
| Lists: move, choose, back | the arrows, Z (or Enter), X (or Backspace) |

**Sound** is the simulation crate's synthesizer, as on every device. It starts on the first key or pointer (a browser asks for a gesture): the page pulls blocks of 1 024 frames at its output's own rate through a `ScriptProcessorNode`. The `sound` switch of the settings and a pause silence it.

## The frame

1. `step`: what the interface asked for, then one tick of the simulation per sixtieth of a second since the last frame, with the pad the device's buttons make. The synthesizer hears each tick.
2. The guest's turn and, when what it shows has changed, its picture.
3. The camera (the simulation's, with its shake), the cells' levels of detail against it, this frame's vertices of the cloak, the wires and the gas, the skinned models' records, the marks on the world.
4. The scene into a target with **four samples a pixel**: the sky's dome and the sun, the world (one program, one picture, one matrix a draw), the player and the giants in range, the cloak and the wires, the ground shadow and the gas.
5. A quarter-size chain: what is brighter than 0.86 luma, blurred across and down, and smeared away from the sun when the sun is near the frame.
6. One pass onto the canvas: the scene plus bloom and light shafts, graded (contrast, saturation, split tone, vignette), blurred toward the centre above 26 m/s; then the marks on the world. Then the interface over it.

## What differs from the PS Vita's picture

- **The atlas is texels, not blocks.** The pack stores BC1; the module decodes it to `rgba8unorm` at load, so a GPU without BC textures (a phone's) samples the same picture. A level's two interpolated colours are rounded here to the nearest 255th.
- **A vertex's packed numbers are read as integers and scaled in the program**: a position as four 16-bit numbers (WebGPU has no format of three), a texture coordinate in 32 767ths, a normal in 127ths, two bones and their weights as four bytes.
- **Colours are computed in 32-bit floats** where the Cg fragment programs compute in `half`.
- **A draw's matrix is a record of one buffer**, written once a frame and picked by an offset at each draw.
- **The composite is one program.** The PS Vita has three (plain, with glow, with glow and the blur at speed); here a gain of zero leaves a part out.
- **On a screen that is not 960 × 544** the scene is drawn at that screen's size with the same four samples a pixel, the quarter-size chain follows it, and the marks on the world follow its width. The PS Vita's own build draws one size.
- **While the game is paused the gas stands still.** On the PS Vita the puffs go on drifting under the pause list.
- **The frame is paced by the display's refresh and the browser's clock.** A display that does not refresh 60 times a second gets whole ticks when its interval is a multiple of a sixtieth of a second, and owes the rest to the next frame otherwise.
- **The levels of detail are held at the profile's distances.** No governor changes them: the PS Vita's build has none either, and the handhelds' governor is not here.

## Measured

Chrome 154 (headless, WebGPU on the Apple GPU, Metal 3) on an M3 Max, the pack `vita60`, served from the same machine. Each device is played by the autopilot with its interface over the scene.

| | PS Vita | PSP | Nintendo 3DS | iPod touch |
| --- | --- | --- | --- | --- |
| Frames a second over 5 s | 59.98 | 59.98 | 59.98 | 59.98 |
| A frame's cost without the display, over 300 frames | 0.74 ms | 0.25 ms | 0.34 ms | 0.48 ms |
| A redraw of the interface's picture | 1.32 ms | 0.38 ms | 0.30 ms | 1.40 ms |
| Redraws a second | 25.0 | 11.2 | 13.2 | 16.6 |
| Turns the guest took a second | 29.6 of 30 | 18.4 of 30 | 24.8 of 30 | 41.6 of 60 |
| A turn of the guest | 0.09 ms | 0.07 ms | 0.06 ms | 0.04 ms |
| The second screen | | | 0.55 ms a redraw, 13.6 a second | |

- **A frame draws what the PS Vita's draws**: on the autopilot's route the most triangles in a frame are 251 873 (the PS Vita's bench reports 251 115 over its 90 s), in 150 to 212 draws of the world.
- **The tab's scene beside this machine's own** of one held view: a mean difference of 0.07 of 255.
- **The first frame of the world on a line of 16 Mbit/s**: the interface is up at 4.0 s and says how much has arrived; the world is there at 17.7 s, after 35.5 MB. On this machine's own loopback: 2.4 s, the title card's length.
- **The site**: the module is 724 387 bytes (254 369 gzipped); 7.4 MB with PocketJS's UI core (365 482 bytes), the four devices' interfaces (the PS Vita's pak is 2.7 MB) and the player's shells and font.

`check` drives the real page with real input: the title card before any other picture; the tab's frame of a held view beside this machine's; on each device with a pad the title's list, a run along the wall on the stick, both wires fired among the houses, the camera turned, the pause list opened and closed by a key (and by a tap on the PS Vita's panel), the sound's output running; the 3DS's two screens; the iPod touch's title under a finger, a drag over the scene and the drawn stick; another device picked in play; **the player in three windows** (1440 × 900 at two display pixels a point, 1280 × 800 at one, and 390 × 844 at three with a finger for a pointer), each device in each: the shell whole between the bar and the dock, nothing over a screen, the mark's sentences, the door's address, a key of the shell held; the first frame on a slow line; a browser without WebGPU.

Not measured: another browser, another GPU, a phone itself, a network that is not this machine's (the line of 16 Mbit/s is Chrome's own throttle). Nobody has judged the sound in a tab by ear.
