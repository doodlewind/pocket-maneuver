// The OpenGL ES 2 renderer: the 3DS's passes (n3ds/src/render.c) on the
// SGX535, from a pack lowered the same way with texels in row order
// (profiles/ipod60.json).
//
// One pass with a 24-bit depth buffer: the sky, the picked meshes page by
// page, the skinned models, the cloak, wires and discs, then the marks on the
// world. The interface is laid over it afterwards (main.c). The fragment
// stage is atlas texel x vertex colour x 2, then the haze, whose share is
// computed per vertex as the reference's 1 - exp(-(depth x density)^2).
//
// The drawable is the portrait screen: every matrix ends with a quarter turn,
// for a device held with its home button on the right.
#include "render.h"
#define GL_SILENCE_DEPRECATION 1
#include <OpenGL/gl3.h>
#include <fcntl.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

// Quads of marks a frame (three reticles, fourteen streaks, the target and its distance).
#define MARK_QUADS 96
#define SKY_RADIUS 1000.0f
// maneuver_pack: metres per position unit of a static vertex.
#define STEP (1.0f / 24.0f)
#define BACKDROP_STEP 0.25f
#define KIND_BACKDROP 3
#define TAG(a, b, c, d) ((uint32_t)(a) | ((uint32_t)(b) << 8) | ((uint32_t)(c) << 16) | ((uint32_t)(d) << 24))
// maneuver_pack::tex_format: 16-bit texels in row order.
enum { GLES_RGB565 = 8, GLES_RGBA4 = 9 };
// APPLE_texture_max_level: the atlas's levels stop where a level would mix two strips.
#define TEXTURE_MAX_LEVEL 0x813D

enum { WORLD, COLOR, SKIN, RIGID, MARK, PROGRAMS };
typedef struct {
  GLuint id;
  GLint mvp, density, fog, scale, bones, light;
} Program;
typedef struct {
  GLuint vertices, indices;
  unsigned count;
} Model;
typedef struct {
  uint32_t tag, offset, size, zero;
} Section;

// One source for every program: a define picks the vertex layout.
static const char *const vertex_source =
  "uniform mat4 uMvp;\n"
  "uniform float uDensity;\n"
  "attribute vec3 aPos;\n"
  "attribute vec4 aColor;\n"
  "varying mediump vec4 vColor;\n"
  "varying mediump float vHaze;\n"
  "#ifdef WORLD\n"
  // Positions on the world's grid; x: metres per unit, y and z: texture coordinate scales.
  "attribute vec2 aUv;\n"
  "uniform vec3 uScale;\n"
  "varying highp vec2 vUv;\n"
  "void main() {\n"
  "  gl_Position = uMvp * vec4(aPos * uScale.x, 1.0);\n"
  "  vUv = aUv * uScale.yz;\n"
  "  vColor = aColor;\n"
  "#endif\n"
  "#ifdef COLOR\n"
  "void main() {\n"
  "  gl_Position = uMvp * vec4(aPos, 1.0);\n"
  "  vColor = aColor;\n"
  "#endif\n"
  "#ifdef MARK\n"
  "attribute vec2 aUv;\n"
  "varying mediump vec2 vUv;\n"
  "void main() {\n"
  "  gl_Position = uMvp * vec4(aPos, 1.0);\n"
  "  vUv = aUv;\n"
  "  vColor = aColor;\n"
  "#endif\n"
  "#ifdef SKIN\n"
  // uBones holds three rows per bone, and aBones.xy each of a vertex's two
  // bones' first row. A vertex of this stage costs what its instructions do,
  // about four times a world vertex, and the models are a third of a frame's
  // vertices: the two bones' rows are blended before one transform, RIGID (a
  // giant beyond the near distance) takes the heavier bone alone, and the
  // light is three sRGB terms to add (uLight: sun direction, constant, from
  // above, from the sun), as the PSP's fixed-function lights have it.
  "attribute vec4 aNormal;\n"
  "attribute vec4 aBones;\n"
  "uniform vec4 uBones[57];\n"
  "uniform vec4 uLight[4];\n"
  "void main() {\n"
  "  vec4 p = vec4(aPos, 1.0);\n"
  "  vec3 n = aNormal.xyz * 0.007874016;\n"
  "#ifdef RIGID\n"
  "  int a = int(mix(aBones.y, aBones.x, step(127.5, aBones.z)));\n"
  "  vec4 r0 = uBones[a], r1 = uBones[a + 1], r2 = uBones[a + 2];\n"
  "#else\n"
  "  int a = int(aBones.x), b = int(aBones.y);\n"
  "  vec2 w = aBones.zw * 0.0039215686;\n"
  "  vec4 r0 = uBones[a] * w.x + uBones[b] * w.y;\n"
  "  vec4 r1 = uBones[a + 1] * w.x + uBones[b + 1] * w.y;\n"
  "  vec4 r2 = uBones[a + 2] * w.x + uBones[b + 2] * w.y;\n"
  "#endif\n"
  "  vec3 at = vec3(dot(r0, p), dot(r1, p), dot(r2, p));\n"
  "  vec3 m = vec3(dot(r0.xyz, n), dot(r1.xyz, n), dot(r2.xyz, n));\n"
  "  vColor = vec4(aColor.rgb * (uLight[1].xyz + uLight[2].xyz * max(m.y, 0.0) + uLight[3].xyz * max(dot(uLight[0].xyz, m), 0.0)), 1.0);\n"
  "  gl_Position = uMvp * vec4(at, 1.0);\n"
  "#endif\n"
  "  float depth = gl_Position.w * uDensity;\n"
  "  vHaze = exp(-depth * depth);\n"
  "}\n";
static const char *const fragment_source =
  "precision mediump float;\n"
  "uniform lowp vec3 uFog;\n"
  "varying mediump vec4 vColor;\n"
  "varying mediump float vHaze;\n"
  "#ifdef WORLD\n"
  // Baked colours are stored at half scale.
  "uniform sampler2D uTex;\n"
  "varying highp vec2 vUv;\n"
  "void main() { gl_FragColor = vec4(mix(uFog, texture2D(uTex, vUv).rgb * vColor.rgb * 2.0, vHaze), 1.0); }\n"
  "#endif\n"
  "#ifdef MARK\n"
  "uniform sampler2D uTex;\n"
  "varying mediump vec2 vUv;\n"
  "void main() { gl_FragColor = texture2D(uTex, vUv) * vColor; }\n"
  "#endif\n"
  "#if defined(COLOR) || defined(SKIN)\n"
  "void main() { gl_FragColor = vec4(mix(uFog, vColor.rgb, vHaze), vColor.a); }\n"
  "#endif\n";

static Program programs[PROGRAMS];
static GLuint pages[4], font_texture, world_vertices, world_indices;
static unsigned page_count, font_width, font_height;
static MhScene scene;
static MhMesh *meshes;
static Model scout, titans[3][2];
// Static geometry, and what changes every frame.
static MhColorVertex sky_vertices[MH_SKY_VERTS], sun_vertices[MH_SUN_VERTS];
static uint16_t sky_indices[MH_SKY_INDICES], cloak_indices[MH_CLOAK_INDICES], quad_indices[MARK_QUADS * 6], fan_indices[MH_FAN_INDICES];
static MhColorVertex moving[MH_CLOAK_VERTS + MH_ROPE_VERTS + MH_DISC_VERTS];
static MhHudVertex marks[MARK_QUADS * 4];
RenderStats render_stats;

static bool link_program(Program *p, const char *define, char *error, size_t capacity) {
  p->id = glCreateProgram();
  for (unsigned i = 0; i < 2; i++) {
    const char *source[2] = {define, i ? fragment_source : vertex_source};
    GLuint shader = glCreateShader(i ? GL_FRAGMENT_SHADER : GL_VERTEX_SHADER);
    glShaderSource(shader, 2, source, NULL);
    glCompileShader(shader);
    GLint ok = 0;
    glGetShaderiv(shader, GL_COMPILE_STATUS, &ok);
    if (!ok) {
      int at = snprintf(error, capacity, "%s shader%s", i ? "fragment" : "vertex", define);
      glGetShaderInfoLog(shader, (GLsizei)(capacity - at), NULL, error + at);
      return false;
    }
    glAttachShader(p->id, shader);
    glDeleteShader(shader);
  }
  glBindAttribLocation(p->id, 0, "aPos");
  glBindAttribLocation(p->id, 1, "aUv");
  glBindAttribLocation(p->id, 1, "aNormal");
  glBindAttribLocation(p->id, 2, "aColor");
  glBindAttribLocation(p->id, 3, "aBones");
  glLinkProgram(p->id);
  GLint ok = 0;
  glGetProgramiv(p->id, GL_LINK_STATUS, &ok);
  if (!ok) {
    int at = snprintf(error, capacity, "program%s", define);
    glGetProgramInfoLog(p->id, (GLsizei)(capacity - at), NULL, error + at);
    return false;
  }
  p->mvp = glGetUniformLocation(p->id, "uMvp");
  p->density = glGetUniformLocation(p->id, "uDensity");
  p->fog = glGetUniformLocation(p->id, "uFog");
  p->scale = glGetUniformLocation(p->id, "uScale");
  p->bones = glGetUniformLocation(p->id, "uBones");
  p->light = glGetUniformLocation(p->id, "uLight");
  glUseProgram(p->id);
  glUniform1i(glGetUniformLocation(p->id, "uTex"), 0);
  return true;
}

static GLuint buffer(GLenum target, const void *data, size_t size) {
  GLuint id;
  glGenBuffers(1, &id);
  glBindBuffer(target, id);
  glBufferData(target, size, data, GL_STATIC_DRAW);
  return id;
}

bool render_load(const char *path, char *error, size_t capacity) {
  MhSizes sizes;
  mh_sizes(&sizes);
  if (sizes.bones != MH_BONES || sizes.cloak_verts != MH_CLOAK_VERTS || sizes.cloak_indices != MH_CLOAK_INDICES || sizes.rope_verts != MH_ROPE_VERTS ||
      sizes.disc_verts != MH_DISC_VERTS || sizes.fan != MH_FAN || sizes.fan_indices != MH_FAN_INDICES || sizes.sky_verts != MH_SKY_VERTS ||
      sizes.sky_indices != MH_SKY_INDICES || sizes.sun_verts != MH_SUN_VERTS || sizes.scene_bytes != sizeof(MhScene) || sizes.mesh_bytes != sizeof(MhMesh) ||
      sizes.vertex_bytes != 16) {
    snprintf(error, capacity, "core.h does not match the core library");
    return false;
  }
  // The pack is mapped for the load only: the GPU and the core keep their own copies.
  int file = open(path, O_RDONLY);
  struct stat info;
  if (file < 0 || fstat(file, &info) || info.st_size < 16) {
    snprintf(error, capacity, "world.pack is missing");
    return false;
  }
  const uint8_t *pack = mmap(NULL, info.st_size, PROT_READ, MAP_PRIVATE, file, 0);
  close(file);
  const uint32_t *head = (const uint32_t *)pack;
  if (pack == MAP_FAILED || head[0] != TAG('M', 'V', 'P', 'K') || head[1] != 2 || head[2] > 32) {
    snprintf(error, capacity, "world.pack is not a version 2 pack");
    return false;
  }
  const Section *sections = (const Section *)(pack + 16);
  const uint8_t *part[7] = {0};
  uint32_t size[7] = {0};
  static const uint32_t tags[7] = {TAG('H', 'S', 'C', 'N'), TAG('H', 'M', 'S', 'H'), TAG('V', 'T', 'X', '0'), TAG('I', 'D', 'X', '0'),
                                   TAG('M', 'O', 'D', 'L'), TAG('T', 'E', 'X', '0'), TAG('F', 'O', 'N', 'T')};
  const uint8_t *world = NULL;
  uint32_t world_size = 0;
  for (uint32_t i = 0; i < head[2]; i++) {
    if (sections[i].offset + (uint64_t)sections[i].size > (uint64_t)info.st_size)
      continue;
    for (unsigned k = 0; k < 7; k++)
      if (sections[i].tag == tags[k])
        part[k] = pack + sections[i].offset, size[k] = sections[i].size;
    if (sections[i].tag == TAG('S', 'I', 'M', 'W'))
      world = pack + sections[i].offset, world_size = sections[i].size;
  }
  bool ok = false;
  for (unsigned k = 0; k < 7; k++)
    if (!part[k])
      world = NULL;
  if (!world || size[0] != sizeof scene) {
    snprintf(error, capacity, "world.pack lacks a section");
    goto done;
  }
  memcpy(&scene, part[0], sizeof scene);
  meshes = malloc(size[1]);
  memcpy(meshes, part[1], size[1]);
  const char *refusal = mh_init(&scene, meshes, size[1] / sizeof *meshes, world, world_size, part[6], size[6]);
  if (refusal) {
    snprintf(error, capacity, "%s", refusal);
    goto done;
  }

  static const char *const defines[PROGRAMS] = {"\n#define WORLD\n", "\n#define COLOR\n", "\n#define SKIN\n", "\n#define SKIN\n#define RIGID\n", "\n#define MARK\n"};
  for (unsigned i = 0; i < PROGRAMS; i++)
    if (!link_program(&programs[i], defines[i], error, capacity))
      goto done;

  // The atlas: after the header, every page's levels, each on a 16-byte boundary of the section.
  uint32_t texture[4];
  memcpy(texture, part[5], 16);
  page_count = scene.pages;
  if (page_count > 4 || texture[3] != GLES_RGB565) {
    snprintf(error, capacity, "the atlas is not an OpenGL ES texture: cook with --profile ipod60");
    goto done;
  }
  bool capped = strstr((const char *)glGetString(GL_EXTENSIONS), "GL_APPLE_texture_max_level") != NULL;
  glPixelStorei(GL_UNPACK_ALIGNMENT, 2);
  glGenTextures(page_count, pages);
  size_t at = 16;
  for (unsigned p = 0; p < page_count; p++) {
    glBindTexture(GL_TEXTURE_2D, pages[p]);
    for (unsigned l = 0; l < texture[2]; l++) {
      at = (at + 15) & ~(size_t)15;
      unsigned w = texture[0] >> l, h = texture[1] >> l;
      // Without the cap a texture needs every level down to one texel: only the first is used then.
      if (capped || !l)
        glTexImage2D(GL_TEXTURE_2D, l, GL_RGB, w, h, 0, GL_RGB, GL_UNSIGNED_SHORT_5_6_5, part[5] + at);
      at += w * h * 2;
    }
    if (capped)
      glTexParameteri(GL_TEXTURE_2D, TEXTURE_MAX_LEVEL, texture[2] - 1);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, capped ? GL_LINEAR_MIPMAP_NEAREST : GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
  }
  uint32_t font_at = mh_font(part[6], size[6], &font_width, &font_height);
  if (!font_at) {
    snprintf(error, capacity, "the pack's font does not load");
    goto done;
  }
  glGenTextures(1, &font_texture);
  glBindTexture(GL_TEXTURE_2D, font_texture);
  glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, font_width, font_height, 0, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4, part[6] + font_at);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);

  world_vertices = buffer(GL_ARRAY_BUFFER, part[2], size[2]);
  world_indices = buffer(GL_ELEMENT_ARRAY_BUFFER, part[3], size[3]);
  // Skinned models: SkinVertex records and indices, as they are in the pack.
  const uint8_t *m = part[4];
  uint32_t count;
  memcpy(&count, m, 4);
  at = 4;
  for (uint32_t i = 0; i < count; i++) {
    uint32_t model[4];
    memcpy(model, m + at, 16);
    at += 16;
    Model loaded = {buffer(GL_ARRAY_BUFFER, m + at, model[1] * 24), buffer(GL_ELEMENT_ARRAY_BUFFER, m + at + model[1] * 24, model[2] * 2), model[2]};
    at = (at + model[1] * 24 + model[2] * 2 + 3) & ~(size_t)3;
    if (model[0] == 0)
      scout = loaded;
    else if (model[0] >= 10 && model[0] <= 12)
      titans[model[0] - 10][0] = loaded;
    else if (model[0] >= 20 && model[0] <= 22)
      titans[model[0] - 20][1] = loaded;
  }
  if (!scout.count) {
    snprintf(error, capacity, "the pack lacks the player's model");
    goto done;
  }
  mh_static(SKY_RADIUS, sky_vertices, sky_indices, sun_vertices, cloak_indices, quad_indices, MARK_QUADS, fan_indices);
  ok = glGetError() == GL_NO_ERROR;
  if (!ok)
    snprintf(error, capacity, "OpenGL refused an upload");
done:
  munmap((void *)pack, info.st_size);
  return ok;
}

// ---- matrices: row-major, applied to column vectors (as maneuver_handheld::mat)

static void multiply(float *out, const float *a, const float *b) {
  float o[16];
  for (int r = 0; r < 4; r++)
    for (int c = 0; c < 4; c++)
      o[r * 4 + c] = a[r * 4] * b[c] + a[r * 4 + 1] * b[4 + c] + a[r * 4 + 2] * b[8 + c] + a[r * 4 + 3] * b[12 + c];
  memcpy(out, o, sizeof o);
}
// OpenGL reads a matrix column by column.
static void upload(GLint location, const float *m) {
  float t[16];
  for (int r = 0; r < 4; r++)
    for (int c = 0; c < 4; c++)
      t[c * 4 + r] = m[r * 4 + c];
  glUniformMatrix4fv(location, 1, GL_FALSE, t);
}
static void use(unsigned which, const float *mvp, float density, const float *fog) {
  const Program *p = &programs[which];
  glUseProgram(p->id);
  upload(p->mvp, mvp);
  glUniform1f(p->density, density);
  glUniform3fv(p->fog, 1, fog);
}
static void draw(const void *indices, unsigned count) {
  glDrawElements(GL_TRIANGLES, count, GL_UNSIGNED_SHORT, indices);
  render_stats.draws++;
  render_stats.tris += count / 3;
}
// Colour, then position (MhColorVertex), from memory.
static void colored(const MhColorVertex *vertices) {
  glBindBuffer(GL_ARRAY_BUFFER, 0);
  glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
  glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 16, vertices->pos);
  glVertexAttribPointer(2, 4, GL_UNSIGNED_BYTE, GL_TRUE, 16, vertices->color);
}

// The picks of one list on one atlas page. They arrive in the pack's order,
// where meshes that share a vertex base follow each other with their indices
// end to end: a run of them is one draw.
static void picks(unsigned list, unsigned count, unsigned page) {
  const MhPick *p = mh_picks(list);
  const float u = scene.u_range / 32768.0f, v = 1.0f / 32768.0f;
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
      // uv, colour, position.
      const uint8_t *at = (const uint8_t *)((size_t)base * 16);
      glVertexAttribPointer(0, 3, GL_SHORT, GL_FALSE, 16, at + 8);
      glVertexAttribPointer(1, 2, GL_SHORT, GL_FALSE, 16, at);
      glVertexAttribPointer(2, 4, GL_UNSIGNED_BYTE, GL_TRUE, 16, at + 4);
      draw((const void *)((size_t)first * 2), n);
    }
    if (!m)
      break;
    if ((m->kind == KIND_BACKDROP) != backdrop) {
      backdrop = m->kind == KIND_BACKDROP;
      glUniform3f(programs[WORLD].scale, backdrop ? BACKDROP_STEP : STEP, u, v);
    }
    base = m->vtx_first;
    first = m->idx_first;
    n = m->idx_count;
  }
  if (backdrop)
    glUniform3f(programs[WORLD].scale, STEP, u, v);
}

// The scene's light on a model (sun direction and visibility, sun, sky,
// bounce; linear) as three sRGB colours to add: a constant, one from straight
// above and the sun's. Exact for a normal facing the horizon, straight up and
// the sun, close in between (maneuver_handheld::scene::Scene::lights).
static void light_terms(const float *light, float *out) {
  const float *sun = light + 4, *sky = light + 8, *bounce = light + 12, lift = fmaxf(light[1], 0);
  memcpy(out, light, 3 * sizeof *out);
  for (int c = 0; c < 3; c++) {
#define HEMISPHERE(up) ((bounce[c] + (sky[c] - bounce[c]) * (up)) * 0.92f)
#define ENCODE(x) powf(fmaxf(x, 0), 1 / 2.2f)
    float side = ENCODE(HEMISPHERE(0.5f)), top = ENCODE(HEMISPHERE(1.0f)), lit = ENCODE(sun[c] * light[3] + HEMISPHERE(0.5f + 0.5f * light[1]));
#undef HEMISPHERE
#undef ENCODE
    out[4 + c] = side;
    out[8 + c] = fmaxf(top - side, 0);
    out[12 + c] = fmaxf(lit - side - out[8 + c] * lift, 0);
  }
}

static void skinned(unsigned which, const Model *model, const float *rows, const float *light) {
  float terms[16] = {0};
  light_terms(light, terms);
  glUniform4fv(programs[which].bones, MH_BONES * 3, rows);
  glUniform4fv(programs[which].light, 4, terms);
  glBindBuffer(GL_ARRAY_BUFFER, model->vertices);
  glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, model->indices);
  // Position, normal i8 x 4, colour, bones and weights u8 x 4.
  glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 24, (const void *)0);
  glVertexAttribPointer(1, 4, GL_BYTE, GL_FALSE, 24, (const void *)12);
  glVertexAttribPointer(2, 4, GL_UNSIGNED_BYTE, GL_TRUE, 24, (const void *)16);
  glVertexAttribPointer(3, 4, GL_UNSIGNED_BYTE, GL_FALSE, 24, (const void *)20);
  draw((const void *)0, model->count);
}

static void cull(int option) {
  if (option & 1) {
    glDisable(GL_CULL_FACE);
    return;
  }
  glEnable(GL_CULL_FACE);
  glCullFace(option & 2 ? GL_FRONT : GL_BACK);
}

void render_frame(const MhView *view, uint32_t ticks, const MhPerf *perf) {
  render_stats.draws = render_stats.tris = 0;
  // The frame is the interface's landscape screen turned a quarter: the drawable's x is its y, the drawable's y its -x.
  static const float quarter[16] = {0, 1, 0, 0, -1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1};
  const float f = 1.0f / tanf(view->fov * (float)M_PI / 360.0f), near = scene.clip_near, far = scene.clip_far;
  const float projection[16] = {f / ((float)WIDTH / HEIGHT), 0, 0, 0, 0, f, 0, 0, 0, 0, (far + near) / (near - far), 2 * far * near / (near - far), 0, 0, -1, 0};
  // The eye looks along `look`, rolled about it (maneuver_handheld::mat::view).
  const float *e = view->eye, *l = view->look;
  float right[3] = {-l[2], 0, l[0]}, length = sqrtf(right[0] * right[0] + right[2] * right[2]);
  if (length < 1e-6f)
    right[0] = 1, right[2] = 0, length = 1;
  right[0] /= length, right[2] /= length;
  const float up[3] = {right[1] * l[2] - right[2] * l[1], right[2] * l[0] - right[0] * l[2], right[0] * l[1] - right[1] * l[0]};
  const float s = sinf(view->roll), c = cosf(view->roll);
  float r[3], u[3];
  for (int i = 0; i < 3; i++)
    r[i] = right[i] * c + up[i] * s, u[i] = up[i] * c - right[i] * s;
  const float eye[16] = {r[0], r[1], r[2], -(r[0] * e[0] + r[1] * e[1] + r[2] * e[2]), u[0], u[1], u[2], -(u[0] * e[0] + u[1] * e[1] + u[2] * e[2]),
                         -l[0], -l[1], -l[2], l[0] * e[0] + l[1] * e[1] + l[2] * e[2], 0, 0, 0, 1};
  float turned[16], vp[16];
  multiply(turned, quarter, projection);
  multiply(vp, turned, eye);

  glViewport(0, 0, HEIGHT, WIDTH);
  glDisable(GL_SCISSOR_TEST);
  glDepthMask(GL_TRUE);
  glClearColor(view->fog[0], view->fog[1], view->fog[2], 1);
  glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
  glActiveTexture(GL_TEXTURE0);
  glFrontFace(GL_CCW);
  for (unsigned i = 0; i < 3; i++)
    glEnableVertexAttribArray(i);
  glDisableVertexAttribArray(3);

  // ---- sky
  const float there[16] = {1, 0, 0, e[0], 0, 1, 0, e[1], 0, 0, 1, e[2], 0, 0, 0, 1};
  float sky[16];
  multiply(sky, vp, there);
  use(COLOR, sky, 0, view->fog);
  glDisable(GL_DEPTH_TEST);
  glDisable(GL_CULL_FACE);
  glDisable(GL_BLEND);
  glDisableVertexAttribArray(1);
  colored(sky_vertices);
  draw(sky_indices, MH_SKY_INDICES);
  glEnable(GL_BLEND);
  glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
  colored(sun_vertices);
  draw(fan_indices, 2 * MH_FAN * 3);
  glDisable(GL_BLEND);

  // ---- world
  glEnable(GL_DEPTH_TEST);
  glDepthFunc(GL_LESS);
  if (view->world) {
    use(WORLD, vp, scene.fog_density, view->fog);
    glUniform3f(programs[WORLD].scale, STEP, scene.u_range / 32768.0f, 1.0f / 32768.0f);
    cull(view->option);
    glEnableVertexAttribArray(1);
    glBindBuffer(GL_ARRAY_BUFFER, world_vertices);
    glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, world_indices);
    for (unsigned repeat = 0; repeat < view->repeat; repeat++)
      for (unsigned page = 0; page < page_count; page++) {
        glBindTexture(GL_TEXTURE_2D, pages[page]);
        // Near to far, so depth rejects what is behind.
        picks(1, view->near_count, page);
        picks(0, view->far_count, page);
      }
  }

  // ---- models
  if (view->actors) {
    static float rows[MH_BONES * 12], light[16];
    cull(view->option);
    glEnableVertexAttribArray(1);
    glEnableVertexAttribArray(3);
    use(SKIN, vp, scene.fog_density, view->fog);
    if (view->show_character) {
      mh_scout_pose(rows, light);
      skinned(SKIN, &scout, rows, light);
    }
    const MhGiant *g = mh_giants();
    for (unsigned i = 0; i < view->giant_count; i++) {
      const Model *model = &titans[g[i].variant % 3][g[i].level ? 1 : 0];
      if (!model->count)
        continue;
      mh_giant_pose(g[i].index, g[i].sink, g[i].vis, rows, light);
      skinned(g[i].level ? RIGID : SKIN, model, rows, light);
    }
    glDisableVertexAttribArray(1);
    glDisableVertexAttribArray(3);

    // The cloak, the wires and the soft discs.
    MhColorVertex *cloak = moving, *rope = cloak + MH_CLOAK_VERTS, *disc = rope + MH_ROPE_VERTS;
    MhFrame frame;
    mh_actors(ticks, cloak, rope, disc, &frame);
    use(COLOR, vp, scene.fog_density, view->fog);
    glDisable(GL_CULL_FACE);
    if (view->show_character) {
      colored(cloak);
      draw(cloak_indices, MH_CLOAK_INDICES);
    }
    if (frame.rope_quads) {
      colored(rope);
      draw(quad_indices, frame.rope_quads * 6);
    }
    if (frame.discs) {
      glEnable(GL_BLEND);
      glDepthMask(GL_FALSE);
      colored(disc);
      draw(fan_indices, frame.discs * MH_FAN * 3);
      glDepthMask(GL_TRUE);
    }
  }

  // ---- marks, in the interface's pixels from its top left corner
  const float pixels[16] = {2.0f / WIDTH, 0, 0, -1, 0, -2.0f / HEIGHT, 0, 1, 0, 0, 1, 0, 0, 0, 0, 1};
  float ortho[16];
  multiply(ortho, quarter, pixels);
  const float uv_scale[2] = {1.0f / font_width, 1.0f / font_height}, uv_offset[2] = {0, 0};
  MhPerf measured = *perf;
  measured.draws = render_stats.draws;
  measured.tris = render_stats.tris;
  unsigned quads = mh_marks(marks, MARK_QUADS * 4, uv_scale, uv_offset, &measured);
  if (quads) {
    use(MARK, ortho, 0, view->fog);
    glDisable(GL_DEPTH_TEST);
    glDisable(GL_CULL_FACE);
    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
    glBindTexture(GL_TEXTURE_2D, font_texture);
    glBindBuffer(GL_ARRAY_BUFFER, 0);
    glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
    glEnableVertexAttribArray(1);
    // uv, colour, position.
    glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 24, marks->pos);
    glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 24, marks->uv);
    glVertexAttribPointer(2, 4, GL_UNSIGNED_BYTE, GL_TRUE, 24, marks->color);
    draw(quad_indices, quads * 6);
  }
  glDisable(GL_BLEND);
}
