/* The interface: one PocketJS guest (ui/, shared with the other devices)
 * drawn over the upper screen's scene and on the whole touch screen, through
 * PocketJS's 3DS pieces: its UI core, its QuickJS driver (qjs.c) and its PICA
 * DrawList backend (gfx.c). What the guest is shown and what it asks for go
 * through the core library (core.h): the guest's service channel is answered
 * there, in the process. */
#ifndef MANEUVER_GUEST_H
#define MANEUVER_GUEST_H

#include <citro3d.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* After C3D_Init and romfsInit. Reads romfs:/maneuver.js and maneuver.pak. */
bool guest_boot(char *error, size_t capacity);
void guest_shutdown(void);
bool guest_running(void);
/* Why the interface is not running, or "". */
const char *guest_error(void);

/* Once a frame, after hidScanInput: the pad and the stylus are noted, and
 * when `dt` more seconds make a turn due the guest takes it with everything
 * noted since its last one, then lays out both screens. */
void guest_turn(float dt);
/* A turn now, whatever is owed: for the frames shown while the pack loads. */
void guest_turn_now(void);
/* Milliseconds a turn takes, smoothed. */
float guest_turn_ms(void);

/* After C3D_FrameBegin. Builds both screens' vertices when a turn ran since
 * the last call, and says whether it did; otherwise the last ones stand. */
bool guest_prepare(void);
/* Over the upper target as bound. */
void guest_draw_top(void);
/* Clears the lower target and draws the touch screen. */
void guest_draw_bottom(C3D_RenderTarget *bottom);

/* From a control message: buttons pressed one after another (PocketJS BTN
 * bits), and a stylus held on the touch screen until it is lifted. */
void guest_press(uint32_t buttons);
void guest_touch(bool down, int x, int y);

#endif
