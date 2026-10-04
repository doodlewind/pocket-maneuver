/* The interface of core/ (Rust): the simulation, the device-independent
 * runtime and the renderer's side of the interface (ui/). core/src/lib.rs
 * defines these functions; mh_sizes lets main.c check that the two sides
 * agree on every layout. The library also defines the svcwire_* functions
 * PocketJS's guest driver calls (svcwire.h): the guest's service channel ends
 * there, in the process. */
#ifndef MANEUVER_CORE_H
#define MANEUVER_CORE_H

#include <stddef.h>
#include <stdint.h>

#define MH_BONES 19
#define MH_CLOAK_VERTS 56
#define MH_CLOAK_INDICES 252
#define MH_ROPE_VERTS 104
#define MH_FAN 8
#define MH_DISCS 49
#define MH_DISC_VERTS (MH_DISCS * (MH_FAN + 1))
#define MH_FAN_INDICES (MH_DISCS * MH_FAN * 3)
#define MH_SKY_VERTS 180
#define MH_SKY_INDICES 960
#define MH_SUN_VERTS (2 * (MH_FAN + 1))

/* Simulation buttons (maneuver_sim::sim::btn), and the two that keep the game's
 * flow while no interface is on the screen. */
#define MH_HOOK_L 1u
#define MH_HOOK_R 2u
#define MH_GAS 4u
#define MH_SLASH 8u
#define MH_ZIP 16u
#define MH_DROP 32u
#define MH_START (1u << 16)
#define MH_SELECT (1u << 17)

/* maneuver_pack::HandScene */
typedef struct {
  float sun_dir[3], fog_density;
  float sun[3], lod_near;
  float sky[3], lod_mid;
  float bounce[3], lod_far;
  float fog[3], clip_near;
  float horizon[3], clip_far;
  float zenith[3], cell;
  float glow[3], super_cell;
  float fog_near, fog_far, titan_near, titan_far;
  float u_range, color_scale, screen[2];
  uint32_t pages, near_streamed, dummies, max_giants;
} MhScene;

/* maneuver_pack::HandMesh */
typedef struct {
  uint32_t kind;
  int32_t cx, cz;
  uint32_t page, vtx_first, vtx_count, idx_first, idx_count, big_first, clip_first;
  float clip_radius, min[3], max[3];
  uint32_t pad;
} MhMesh;

typedef struct {
  uint32_t bones, cloak_verts, cloak_indices, rope_verts, disc_verts, fan, fan_indices, sky_verts, sky_indices, sun_verts, scene_bytes, mesh_bytes, vertex_bytes;
} MhSizes;

typedef struct {
  uint32_t buttons;
  float lx, ly, rx, ry;
} MhPad;

typedef struct {
  float eye[3], fov, look[3], roll, fog[3], rush;
  uint32_t show_character, world, actors, repeat;
  int32_t option;
  uint32_t far_count, near_count, giant_count;
  float vis;
} MhView;

typedef struct {
  uint32_t mesh, cell;
  float dist;
} MhPick;

typedef struct {
  uint32_t index, variant, level;
  float sink, vis, dist;
} MhGiant;

typedef struct {
  uint32_t rope_quads, discs;
} MhFrame;

/* Milliseconds, as the statistics line and the status record show them. */
typedef struct {
  float frame, worst;
  uint32_t late, frames;
  float sim, build, draw, gpu;
  uint32_t draws, tris;
} MhPerf;

/* Colour, then position. */
typedef struct {
  uint8_t color[4];
  float pos[3];
} MhColorVertex;

typedef struct {
  float uv[2];
  uint8_t color[4];
  float pos[3];
} MhHudVertex;

/* mh_stage: what the interface shows while there is no game. */
#define MH_STAGE_LOADING 0u
#define MH_STAGE_ERROR 1u

void mh_sizes(MhSizes *out);
/* Null, or a message. The buffers may be freed afterwards. */
const char *mh_init(const MhScene *scene, const MhMesh *meshes, uint32_t mesh_count, const uint8_t *simw, uint32_t simw_len, const uint8_t *font, uint32_t font_len);
uint32_t mh_font(const uint8_t *font, uint32_t font_len, uint32_t *width, uint32_t *height);
void mh_static(float radius, MhColorVertex *sky_v, uint16_t *sky_i, MhColorVertex *sun_v, uint16_t *cloak_i, uint16_t *quad_i, uint32_t quads, uint16_t *fan_i);
void mh_control(const char *text, uint32_t len);
uint32_t mh_step(const MhPad *pad, uint32_t ticks);
void mh_view(MhView *out);
const MhPick *mh_picks(uint32_t list);
const MhGiant *mh_giants(void);
void mh_giant_pose(uint32_t index, float sink, float vis, float *rows, float *light);
void mh_scout_pose(float *rows, float *light);
void mh_actors(uint32_t ticks, MhColorVertex *cloak, MhColorVertex *rope, MhColorVertex *disc, MhFrame *out);
/* The marks on the world (where a wire would bite, the nearest target, the streaks of speed), four vertices a quad.
 * `perf` goes into the statistics line the interface shows. */
uint32_t mh_marks(MhHudVertex *verts, uint32_t cap, const float *uv_scale, const float *uv_offset, const MhPerf *perf);
void mh_audio(int16_t *out, uint32_t frames, float rate);
uint32_t mh_status(char *out, uint32_t cap, const MhPerf *perf, const char *extra, uint32_t extra_len);
/* Before the game exists: the loading step, or why the start failed. The first mh_step replaces it with the game's flow. */
void mh_stage(uint32_t stage, const char *message, uint32_t len);
/* The preferences read from storage at the start, and what the interface asked to have stored since the last call
 * (NUL-terminated; 0 when there is nothing). */
void mh_prefs_stored(const char *text, uint32_t len);
uint32_t mh_prefs_take(char *out, uint32_t cap);
/* The sound setting is on and the game is not paused. */
uint32_t mh_audible(void);
/* A guest holds the interface's channel. */
uint32_t mh_interface_open(void);

#endif
