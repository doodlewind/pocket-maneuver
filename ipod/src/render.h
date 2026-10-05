// The OpenGL ES 2 renderer of the iPod touch build.
#ifndef MANEUVER_IPOD_RENDER_H
#define MANEUVER_IPOD_RENDER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "../../n3ds/src/core.h"

// The interface's screen (landscape) in pixels; the drawable is the same
// screen on its side, HEIGHT wide and WIDTH high.
enum { WIDTH = 480, HEIGHT = 320 };

typedef struct {
  uint32_t draws, tris;
} RenderStats;
extern RenderStats render_stats;

// Reads the pack, builds the game (mh_init) and uploads what the GPU draws
// from. The GL context is current.
bool render_load(const char *pack, char *error, size_t capacity);
// One frame into the bound framebuffer, a quarter turn round.
void render_frame(const MhView *view, uint32_t ticks, const MhPerf *perf);

#endif
