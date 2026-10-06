# Pocket Maneuver on the iPod touch 4

480 × 320, OpenGL ES 2 on the SGX535, iOS 6. The device has a touch panel and no pad, so the game is played through the interface's touch presentation (`ui/app/presentations/touch.tsx`): a stick, round keys and a finger on the scene to turn the view.

| Part | What it is |
| --- | --- |
| `core/` | A Cargo manifest only. Its `[lib] path` is the 3DS core's source (`n3ds/core/src/lib.rs`), built for `armv7-apple-ios`: the simulation, `maneuver-handheld` and the interface's channel behind the `mh_*` functions of `n3ds/src/core.h`. |
| `src/render.c` | The renderer: the 3DS's passes in OpenGL ES 2, from a pack lowered the same way with texels in row order (`profiles/ipod60.json`, target `ipod`). |
| `src/main.c` | The shell: UIKit through the Objective-C runtime, the render thread, the guest's turn, touches, sound, preferences, the control and status files. |
| `tools/ipod.ts` | cook, build, package, deploy, launch, drive, capture, bench. |

## The pack

`bun tools/ipod.ts cook` (or `bun tools/maneuver.ts cook --profile ipod60`) lowers the world as for the 3DS (one position grid, runs of meshes on shared vertex bases, two bones per skinned vertex) and stores every texture as 16-bit texels in row order, the image's first row first: `UNSIGNED_SHORT_5_6_5` for the atlas pages and `UNSIGNED_SHORT_4_4_4_4` for the font. The profile takes the coarser model set (the player at 4 528 triangles, giants at 1 668 – 2 028 and 1 038 – 1 148) and draws at most six giants within 300 m; its detail distances are 64 m, 150 m and 1 000 m.

## The launch

The Pocket3D title card plays first, before the interface's guest starts and before the pack is read: 144 ticks, 2.4 s. PocketJS's drawers write a console's frame buffer, and this device has none a CPU writes, so the core draws each tick's frame into memory (`mh_title`, over `pocket3d_title::draw`) and the shell uploads it to a texture and shows it with the pass that lays the interface over a frame. A frame that differs from the last is uploaded; the held frames in the middle are not. The card follows the clock, so a slow frame skips a tick and the card keeps its length. `bun tools/ipod.ts title --out a.png` launches the app and brings back the held frame as the device presented it.

## The frame

1. `mh_step`: what the interface asked for since the last frame, then one simulation tick per display refresh.
2. The guest's turn, every frame: the contacts go in (eight slots through PocketJS's contact latch), the game's state is read, commands are left for the next `mh_step`.
3. The interface is drawn into a texture when what it shows has changed, on every other frame at most. There are two textures, used in turn.
4. The scene: sky, world, skinned models, cloak, wires, discs, marks. The drawable is the portrait screen, untransformed and opaque, and every matrix ends with a quarter turn.
5. The interface's texture over the frame, then the present.

What limits the frame on this device:

- **A vertex costs what its shader's instructions do.** A skinned vertex was about four times a world vertex: four more draws of the player (18 000 triangles) took the route from 60 to 35 frames per second, where drawing the whole world twice (33 000 more triangles) took it to 49. The skin shader blends the two bones' rows before one transform, takes one bone for a giant beyond the near distance, and adds three precomputed sRGB light terms instead of encoding per vertex.
- **Laying out and drawing the interface costs about 6 ms** of the one core (PocketJS's draw list is built once for the hash and once for the draw). A frame that pays it twice in a row misses its refresh, so the texture follows the interface at 30 Hz while the guest turns at 60.
- **Asking the device anything over SSH costs frames.** `bench` sends `mark=SECONDS` and the app measures the window itself; the host reads the result when it is done.

Measured on an iPod touch 4 (A4, SGX535, iOS 6.1.6), play flown by the autopilot from the start of the route, the governor on (`bun tools/ipod.ts bench --seconds 60`):

| Window | Frames | Late frames | Average frame | Worst frame | Most triangles | Most draws |
| --- | --- | --- | --- | --- | --- | --- |
| 60 s | 3 570 | 31 (0.9 %) | 16.81 ms | 38.5 ms | 36 549 | 69 |

CPU time per frame: simulation 2.2 ms, the guest's turn 0.6 ms, the interface's redraw 2.5 ms (about 6 ms on a frame that redraws), the scene's commands 2.4 ms. The first half minute, over the town from the wall, is the heaviest stretch: with the governor off it has 3 % of its frames late. Shorter world distances do not change that (the governor's lever does little here); fewer skinned vertices do.

## Development loop

```
bun tools/ipod.ts cook
bun tools/ipod.ts deploy            # first install (MobileInstallation)
bun tools/ipod.ts native [--pack]   # replace the executable and the interface (and the pack)
bun tools/ipod.ts launch
bun tools/ipod.ts ctl "mode=play auto=1"
bun tools/ipod.ts capture --out a.png
bun tools/ipod.ts bench --seconds 60
```

`ctl` words go to `Game::control` (`mode= auto= view= lodNear= lodMid= lodFar= govern= stats= world= actors= reset=`) and to the shell: `touch=X,Y[;X,Y]` holds fingers on the panel in the interface's pixels and `touch=off` lifts them, `tap=X,Y` is one finger for a few turns, `ui=start|pause|resume|restart|title` asks what the interface would, `screen=1` writes the presented frame, `mark=SECONDS` measures.

`launch` and `bench` wait while another tool drives the device: control files or captures written in another app's container in the last three minutes.

Sound leaves through an audio queue that drains a ring the render thread fills from the synthesizer at 22.05 kHz. The pinned sysroot's AudioToolbox cannot be linked against, so its four calls are looked up on the device when sound starts.
