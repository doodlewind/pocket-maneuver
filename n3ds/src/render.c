/* The PICA200 renderer.
 *
 * One pass with a 24-bit depth buffer: the sky, the picked meshes page by
 * page, the skinned models, the cloak, wires and discs, then the interface.
 * The fragment stage is fixed-function: atlas texel x vertex colour x 2, then
 * the haze from a fog table. Vertex shaders are in the .v.pica files.
 *
 * Buffers the GPU reads are in linear memory. What changes every frame is
 * written into one of two copies, so the copy the GPU still reads from the
 * frame before is left alone. */
#include "render.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "color_shbin.h"
#include "hud_shbin.h"
#include "skin_shbin.h"
#include "world_shbin.h"

#define HUD_QUADS 320
#define SKY_RADIUS 1000.0f
/* maneuver_pack: metres per position unit of a static vertex. */
#define PICA_STEP (1.0f / 24.0f)
#define PICA_BACKDROP_STEP 0.25f
#define KIND_BACKDROP 3

typedef struct {
  DVLB_s *dvlb;
  shaderProgram_s program;
  C3D_AttrInfo attr;
  int projection, extra;
} Program;

typedef struct {
  const uint8_t *vtx;
  const uint16_t *idx;
  uint32_t idx_count;
} Model;

static Program world_prog, color_prog, skin_prog, hud_prog;
static int skin_bones, skin_light;
static C3D_Tex pages[4], font_tex;
static unsigned page_count;
static C3D_FogLut fog_lut;
static const MhScene *scene;
static const MhMesh *meshes;
static const uint8_t *world_vtx;
static const uint16_t *world_idx;
static Model scout, titans[3][2];
/* Static geometry. */
static MhColorVertex *sky_vb, *sun_vb;
static uint16_t *sky_ib, *cloak_ib, *quad_ib, *fan_ib;
/* Two copies of what changes every frame. */
static MhColorVertex *dynamic_vb[2];
static MhHudVertex *hud_vb[2];
static unsigned flip;
RenderStats render_stats;

static bool program_init(Program *p, const u8 *shbin, u32 size, const char *extra) {
  p->dvlb = DVLB_ParseFile((u32 *)shbin, size);
  if (!p->dvlb)
    return false;
  shaderProgramInit(&p->program);
  shaderProgramSetVsh(&p->program, &p->dvlb->DVLE[0]);
  p->projection = shaderInstanceGetUniformLocation(p->program.vertexShader, "projection");
  p->extra = extra ? shaderInstanceGetUniformLocation(p->program.vertexShader, extra) : -1;
  AttrInfo_Init(&p->attr);
  return p->projection >= 0;
}

static void use(Program *p, const C3D_Mtx *projection) {
  C3D_BindProgram(&p->program);
  C3D_SetAttrInfo(&p->attr);
  C3D_FVUnifMtx4x4(GPU_VERTEX_SHADER, p->projection, projection);
}

static void buffer(const void *data, unsigned stride, int count, u64 permutation) {
  C3D_BufInfo *b = C3D_GetBufInfo();
  BufInfo_Init(b);
  BufInfo_Add(b, data, stride, count, permutation);
}

static u32 abgr(const float c[3], float a) {
  u32 v = 0;
  for (int i = 0; i < 3; i++) {
    float x = c[i] < 0 ? 0 : c[i] > 1 ? 1 : c[i];
    v |= (u32)(x * 255.0f + 0.5f) << (8 * i);
  }
  return v | ((u32)(a * 255.0f + 0.5f) << 24);
}

static void tev_color_only(void) {
  C3D_TexEnv *e = C3D_GetTexEnv(0);
  C3D_TexEnvInit(e);
  C3D_TexEnvSrc(e, C3D_Both, GPU_PRIMARY_COLOR, 0, 0);
  C3D_TexEnvFunc(e, C3D_Both, GPU_REPLACE);
}

/* texel x vertex colour, doubled: baked colours are stored at half scale. */
static void tev_world(void) {
  C3D_TexEnv *e = C3D_GetTexEnv(0);
  C3D_TexEnvInit(e);
  C3D_TexEnvSrc(e, C3D_Both, GPU_TEXTURE0, GPU_PRIMARY_COLOR, 0);
  C3D_TexEnvFunc(e, C3D_Both, GPU_MODULATE);
  C3D_TexEnvScale(e, C3D_RGB, GPU_TEVSCALE_2);
}

static void tev_hud(void) {
  C3D_TexEnv *e = C3D_GetTexEnv(0);
  C3D_TexEnvInit(e);
  C3D_TexEnvSrc(e, C3D_Both, GPU_TEXTURE0, GPU_PRIMARY_COLOR, 0);
  C3D_TexEnvFunc(e, C3D_Both, GPU_MODULATE);
}

static unsigned level_bytes(unsigned w, unsigned h) { return w * h * 2; }

bool render_init(const RenderData *d, char *error, size_t n) {
  scene = d->scene;
  meshes = d->meshes;
  world_vtx = d->vtx;
  world_idx = d->idx;
  if (!program_init(&world_prog, world_shbin, world_shbin_size, "scale") || !program_init(&color_prog, color_shbin, color_shbin_size, NULL) ||
      !program_init(&skin_prog, skin_shbin, skin_shbin_size, "bones") || !program_init(&hud_prog, hud_shbin, hud_shbin_size, NULL)) {
    snprintf(error, n, "a shader program did not load");
    return false;
  }
  skin_bones = skin_prog.extra;
  skin_light = shaderInstanceGetUniformLocation(skin_prog.program.vertexShader, "light");
  /* World: position i16 x 4, texture coordinates i16 x 2, colour u8 x 4. In the buffer: uv, colour, position. */
  AttrInfo_AddLoader(&world_prog.attr, 0, GPU_SHORT, 4);
  AttrInfo_AddLoader(&world_prog.attr, 1, GPU_SHORT, 2);
  AttrInfo_AddLoader(&world_prog.attr, 2, GPU_UNSIGNED_BYTE, 4);
  /* Colour: position float x 3, colour u8 x 4. In the buffer: colour, position. */
  AttrInfo_AddLoader(&color_prog.attr, 0, GPU_FLOAT, 3);
  AttrInfo_AddLoader(&color_prog.attr, 1, GPU_UNSIGNED_BYTE, 4);
  /* Skin: position, normal i8 x 4, colour, bones and weights u8 x 4, in that order. */
  AttrInfo_AddLoader(&skin_prog.attr, 0, GPU_FLOAT, 3);
  AttrInfo_AddLoader(&skin_prog.attr, 1, GPU_BYTE, 4);
  AttrInfo_AddLoader(&skin_prog.attr, 2, GPU_UNSIGNED_BYTE, 4);
  AttrInfo_AddLoader(&skin_prog.attr, 3, GPU_UNSIGNED_BYTE, 4);
  /* Interface: position float x 3, texture coordinates float x 2, colour. In the buffer: uv, colour, position. */
  AttrInfo_AddLoader(&hud_prog.attr, 0, GPU_FLOAT, 3);
  AttrInfo_AddLoader(&hud_prog.attr, 1, GPU_FLOAT, 2);
  AttrInfo_AddLoader(&hud_prog.attr, 2, GPU_UNSIGNED_BYTE, 4);

  /* The atlas: every page's levels, already in the PICA's tiled layout. */
  page_count = scene->pages;
  if (page_count > 4 || d->tex_format != 5) {
    snprintf(error, n, "the atlas is not a 3DS texture");
    return false;
  }
  const uint8_t *src = d->tex;
  for (unsigned p = 0; p < page_count; p++) {
    C3D_TexInitParams params = {.width = d->tex_width, .height = d->tex_height, .maxLevel = d->tex_mips - 1, .format = GPU_RGB565, .type = GPU_TEX_2D, .onVram = true};
    if (!C3D_TexInitWithParams(&pages[p], NULL, params)) {
      snprintf(error, n, "no video memory for atlas page %u", p);
      return false;
    }
    for (unsigned l = 0; l < d->tex_mips; l++) {
      src = (const uint8_t *)(((uintptr_t)src + 15) & ~(uintptr_t)15);
      C3D_TexLoadImage(&pages[p], src, GPU_TEXFACE_2D, l);
      src += level_bytes(d->tex_width >> l, d->tex_height >> l);
    }
    C3D_TexSetFilter(&pages[p], GPU_LINEAR, GPU_LINEAR);
    C3D_TexSetFilterMipmap(&pages[p], GPU_NEAREST);
    C3D_TexSetWrap(&pages[p], GPU_REPEAT, GPU_CLAMP_TO_EDGE);
  }
  C3D_TexInitParams fp = {.width = d->font_width, .height = d->font_height, .maxLevel = 0, .format = GPU_RGBA4, .type = GPU_TEX_2D, .onVram = false};
  if (!C3D_TexInitWithParams(&font_tex, NULL, fp)) {
    snprintf(error, n, "no memory for the font texture");
    return false;
  }
  C3D_TexUpload(&font_tex, d->font_texels);
  C3D_TexSetFilter(&font_tex, GPU_NEAREST, GPU_NEAREST);
  C3D_TexSetWrap(&font_tex, GPU_CLAMP_TO_EDGE, GPU_CLAMP_TO_EDGE);

  /* Skinned models: SkinVertex records and indices, as they are in the pack. */
  const uint8_t *m = d->models;
  uint32_t count;
  memcpy(&count, m, 4);
  size_t at = 4;
  for (uint32_t i = 0; i < count; i++) {
    uint32_t head[4];
    memcpy(head, m + at, 16);
    at += 16;
    Model model = {m + at, (const uint16_t *)(m + at + head[1] * 24), head[2]};
    at = (at + head[1] * 24 + head[2] * 2 + 3) & ~(size_t)3;
    if (head[0] == 0)
      scout = model;
    else if (head[0] >= 10 && head[0] <= 12)
      titans[head[0] - 10][0] = model;
    else if (head[0] >= 20 && head[0] <= 22)
      titans[head[0] - 20][1] = model;
  }
  if (!scout.idx_count) {
    snprintf(error, n, "the pack lacks the player's model");
    return false;
  }

  sky_vb = linearAlloc(MH_SKY_VERTS * sizeof *sky_vb);
  sun_vb = linearAlloc(MH_SUN_VERTS * sizeof *sun_vb);
  sky_ib = linearAlloc(MH_SKY_INDICES * 2);
  cloak_ib = linearAlloc(MH_CLOAK_INDICES * 2);
  quad_ib = linearAlloc(HUD_QUADS * 6 * 2);
  fan_ib = linearAlloc(MH_FAN_INDICES * 2);
  for (int i = 0; i < 2; i++) {
    dynamic_vb[i] = linearAlloc((MH_CLOAK_VERTS + MH_ROPE_VERTS + MH_DISC_VERTS) * sizeof(MhColorVertex));
    hud_vb[i] = linearAlloc(HUD_QUADS * 4 * sizeof(MhHudVertex));
  }
  if (!sky_vb || !sun_vb || !sky_ib || !cloak_ib || !quad_ib || !fan_ib || !dynamic_vb[1] || !hud_vb[1]) {
    snprintf(error, n, "no linear memory for the frame buffers");
    return false;
  }
  mh_static(SKY_RADIUS, sky_vb, sky_ib, sun_vb, cloak_ib, quad_ib, HUD_QUADS, fan_ib);
  /* Haze: 1 - exp(-(depth x density)^2), as the reference computes it. */
  FogLut_Exp(&fog_lut, scene->fog_density, 2.0f, scene->clip_near, scene->clip_far);
  return true;
}

static void draw(GPU_Primitive_t prim, const uint16_t *idx, unsigned count) {
  C3D_DrawElements(prim, count, C3D_UNSIGNED_SHORT, idx);
  render_stats.draws++;
  render_stats.tris += count / 3;
}

static void fog(bool on, const float color[3]) {
  if (on) {
    C3D_FogGasMode(GPU_FOG, GPU_PLAIN_DENSITY, false);
    C3D_FogColor(abgr(color, 0) & 0xffffff);
    C3D_FogLutBind(&fog_lut);
  } else {
    C3D_FogGasMode(GPU_NO_FOG, GPU_PLAIN_DENSITY, false);
  }
}

/* The picks of one list on one atlas page. They arrive in the pack's order,
 * where meshes that share a vertex base follow each other with their indices
 * end to end: a run of them is one draw. */
static void picks(unsigned list, unsigned count, unsigned page) {
  const MhPick *p = mh_picks(list);
  bool backdrop = false;
  uint32_t base = UINT32_MAX, first = 0, n = 0;
  for (unsigned i = 0; i <= count; i++) {
    const MhMesh *m = i < count ? &meshes[p[i].mesh] : NULL;
    if (m && m->page != page)
      continue;
    if (m && n && m->vtx_first == base && m->idx_first == first + n && (m->kind == KIND_BACKDROP) == backdrop) {
      n += m->idx_count;
      continue;
    }
    if (n) {
      buffer(world_vtx + (size_t)base * 16, 16, 3, 0x021);
      draw(GPU_TRIANGLES, world_idx + first, n);
    }
    if (!m)
      break;
    if ((m->kind == KIND_BACKDROP) != backdrop) {
      backdrop = m->kind == KIND_BACKDROP;
      C3D_FVUnifSet(GPU_VERTEX_SHADER, world_prog.extra, backdrop ? PICA_BACKDROP_STEP : PICA_STEP, 1.0f / 255.0f, scene->u_range / 32768.0f, 1.0f / 32768.0f);
    }
    base = m->vtx_first;
    first = m->idx_first;
    n = m->idx_count;
  }
  if (backdrop)
    C3D_FVUnifSet(GPU_VERTEX_SHADER, world_prog.extra, PICA_STEP, 1.0f / 255.0f, scene->u_range / 32768.0f, 1.0f / 32768.0f);
}

static void skinned(const Model *model, const float *rows, const float *light) {
  C3D_FVec *b = C3D_FVUnifWritePtr(GPU_VERTEX_SHADER, skin_bones, MH_BONES * 3);
  for (int i = 0; i < MH_BONES * 3; i++)
    b[i] = FVec4_New(rows[i * 4], rows[i * 4 + 1], rows[i * 4 + 2], rows[i * 4 + 3]);
  C3D_FVec *l = C3D_FVUnifWritePtr(GPU_VERTEX_SHADER, skin_light, 4);
  for (int i = 0; i < 4; i++)
    l[i] = FVec4_New(light[i * 4], light[i * 4 + 1], light[i * 4 + 2], light[i * 4 + 3]);
  buffer(model->vtx, 24, 4, 0x3210);
  draw(GPU_TRIANGLES, model->idx, model->idx_count);
}

void render_frame(C3D_RenderTarget *target, const MhView *view, uint32_t ticks, const MhPerf *perf) {
  render_stats.draws = render_stats.tris = 0;
  flip ^= 1;
  /* The clear colour is RRGGBBAA. */
  u32 f = abgr(view->fog, 1);
  C3D_RenderTargetClear(target, C3D_CLEAR_ALL, ((f & 0xff) << 24) | ((f & 0xff00) << 8) | ((f & 0xff0000) >> 8) | 0xff, 0);
  C3D_FrameDrawOn(target);

  C3D_Mtx proj, look, vp;
  Mtx_PerspTilt(&proj, C3D_AngleFromDegrees(view->fov), C3D_AspectRatioTop, scene->clip_near, scene->clip_far, false);
  Mtx_LookAt(&look, FVec3_New(view->eye[0], view->eye[1], view->eye[2]), FVec3_New(view->eye[0] + view->look[0], view->eye[1] + view->look[1], view->eye[2] + view->look[2]), FVec3_New(0, 1, 0), false);
  Mtx_RotateZ(&look, -view->roll, false);
  Mtx_Multiply(&vp, &proj, &look);

  C3D_DepthMap(true, -1.0f, 0.0f);
  C3D_AlphaTest(false, GPU_ALWAYS, 0);
  C3D_CullFace(GPU_CULL_NONE);

  /* ---------------------------------------------------------------- sky */
  C3D_Mtx sky = vp;
  Mtx_Translate(&sky, view->eye[0], view->eye[1], view->eye[2], true);
  use(&color_prog, &sky);
  tev_color_only();
  fog(false, view->fog);
  C3D_DepthTest(false, GPU_ALWAYS, GPU_WRITE_COLOR);
  C3D_AlphaBlend(GPU_BLEND_ADD, GPU_BLEND_ADD, GPU_ONE, GPU_ZERO, GPU_ONE, GPU_ZERO);
  buffer(sky_vb, 16, 2, 0x01);
  draw(GPU_TRIANGLES, sky_ib, MH_SKY_INDICES);
  C3D_AlphaBlend(GPU_BLEND_ADD, GPU_BLEND_ADD, GPU_SRC_ALPHA, GPU_ONE_MINUS_SRC_ALPHA, GPU_ONE, GPU_ZERO);
  buffer(sun_vb, 16, 2, 0x01);
  draw(GPU_TRIANGLES, fan_ib, 2 * MH_FAN * 3);
  C3D_AlphaBlend(GPU_BLEND_ADD, GPU_BLEND_ADD, GPU_ONE, GPU_ZERO, GPU_ONE, GPU_ZERO);

  /* ---------------------------------------------------------------- world */
  C3D_DepthTest(true, GPU_GREATER, GPU_WRITE_ALL);
  fog(true, view->fog);
  if (view->world) {
    use(&world_prog, &vp);
    C3D_FVUnifSet(GPU_VERTEX_SHADER, world_prog.extra, PICA_STEP, 1.0f / 255.0f, scene->u_range / 32768.0f, 1.0f / 32768.0f);
    tev_world();
    C3D_CullFace(view->option & 1 ? GPU_CULL_NONE : view->option & 2 ? GPU_CULL_FRONT_CCW : GPU_CULL_BACK_CCW);
    for (unsigned r = 0; r < view->repeat; r++) {
      for (unsigned page = 0; page < page_count; page++) {
        C3D_TexBind(0, &pages[page]);
        /* Near to far, so depth rejects what is behind. */
        picks(1, view->near_count, page);
        picks(0, view->far_count, page);
      }
    }
  }

  /* ---------------------------------------------------------------- models */
  if (view->actors) {
    static float rows[MH_BONES * 12], light[16];
    use(&skin_prog, &vp);
    tev_color_only();
    C3D_CullFace(view->option & 1 ? GPU_CULL_NONE : view->option & 2 ? GPU_CULL_FRONT_CCW : GPU_CULL_BACK_CCW);
    if (view->show_character) {
      mh_scout_pose(rows, light);
      skinned(&scout, rows, light);
    }
    const MhGiant *g = mh_giants();
    for (unsigned i = 0; i < view->giant_count; i++) {
      const Model *model = &titans[g[i].variant % 3][g[i].level ? 1 : 0];
      if (!model->idx_count)
        continue;
      mh_giant_pose(g[i].index, g[i].sink, g[i].vis, rows, light);
      skinned(model, rows, light);
    }

    /* The cloak, the wires and the soft discs. */
    MhColorVertex *cloak = dynamic_vb[flip], *rope = cloak + MH_CLOAK_VERTS, *disc = rope + MH_ROPE_VERTS;
    MhFrame fr;
    mh_actors(ticks, cloak, rope, disc, &fr);
    GSPGPU_FlushDataCache(cloak, (MH_CLOAK_VERTS + MH_ROPE_VERTS + MH_DISC_VERTS) * sizeof(MhColorVertex));
    use(&color_prog, &vp);
    C3D_CullFace(GPU_CULL_NONE);
    if (view->show_character) {
      buffer(cloak, 16, 2, 0x01);
      draw(GPU_TRIANGLES, cloak_ib, MH_CLOAK_INDICES);
    }
    if (fr.rope_quads) {
      buffer(rope, 16, 2, 0x01);
      draw(GPU_TRIANGLES, quad_ib, fr.rope_quads * 6);
    }
    if (fr.discs) {
      C3D_AlphaBlend(GPU_BLEND_ADD, GPU_BLEND_ADD, GPU_SRC_ALPHA, GPU_ONE_MINUS_SRC_ALPHA, GPU_ONE, GPU_ZERO);
      C3D_DepthTest(true, GPU_GREATER, GPU_WRITE_COLOR);
      buffer(disc, 16, 2, 0x01);
      draw(GPU_TRIANGLES, fan_ib, fr.discs * MH_FAN * 3);
    }
  }

  /* ---------------------------------------------------------------- interface */
  C3D_Mtx ortho;
  Mtx_OrthoTilt(&ortho, 0.0f, 400.0f, 240.0f, 0.0f, -1.0f, 1.0f, true);
  MhHudVertex *hv = hud_vb[flip];
  /* The pack stores a texture's rows bottom-up, which is where the PICA starts v: texel row y is at v = y / height. */
  const float uv_scale[2] = {1.0f / font_tex.width, 1.0f / font_tex.height}, uv_offset[2] = {0.0f, 0.0f};
  MhPerf p = *perf;
  p.draws = render_stats.draws;
  p.tris = render_stats.tris;
  unsigned quads = mh_hud(hv, HUD_QUADS * 4, uv_scale, uv_offset, &p);
  if (quads) {
    GSPGPU_FlushDataCache(hv, quads * 4 * sizeof *hv);
    use(&hud_prog, &ortho);
    tev_hud();
    fog(false, view->fog);
    C3D_CullFace(GPU_CULL_NONE);
    C3D_DepthTest(false, GPU_ALWAYS, GPU_WRITE_COLOR);
    C3D_AlphaBlend(GPU_BLEND_ADD, GPU_BLEND_ADD, GPU_SRC_ALPHA, GPU_ONE_MINUS_SRC_ALPHA, GPU_ONE, GPU_ZERO);
    C3D_TexBind(0, &font_tex);
    buffer(hv, 24, 3, 0x021);
    draw(GPU_TRIANGLES, quad_ib, quads * 6);
  }
  C3D_AlphaBlend(GPU_BLEND_ADD, GPU_BLEND_ADD, GPU_ONE, GPU_ZERO, GPU_ONE, GPU_ZERO);
}
