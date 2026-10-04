#ifndef MANEUVER_RENDER_H
#define MANEUVER_RENDER_H

#include <3ds.h>
#include <citro3d.h>
#include <stdbool.h>

#include "core.h"

/* The pack's drawing data. Vertex, index and model buffers are in linear
 * memory and stay for the life of the program; the texture buffers may be
 * freed after render_init. */
typedef struct {
  const MhScene *scene;
  const MhMesh *meshes;
  const uint8_t *vtx;
  const uint16_t *idx;
  const uint8_t *models;
  /* The atlas: after the header, every page's levels, each on a 16-byte boundary. */
  const uint8_t *tex;
  uint32_t tex_width, tex_height, tex_mips, tex_format;
  const uint8_t *font_texels;
  uint32_t font_width, font_height;
} RenderData;

typedef struct {
  uint32_t draws, tris;
} RenderStats;

extern RenderStats render_stats;
/* Uniform registers of the skinning program, for the status record. */
extern int render_debug[4];

bool render_init(const RenderData *data, char *error, size_t n);
/* Draws one frame between C3D_FrameBegin and C3D_FrameEnd. */
void render_frame(C3D_RenderTarget *target, const MhView *view, uint32_t ticks, const MhPerf *perf);

#endif
