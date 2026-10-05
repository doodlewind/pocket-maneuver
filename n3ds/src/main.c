/* Pocket Maneuver on Nintendo 3DS.
 *
 * The same simulation as the reference (core/, Rust) and the same world,
 * lowered by the world compiler for this machine (profiles/n3ds60.json):
 * 400 x 240 on the upper screen, the PICA200's fixed fragment stage, one
 * Circle Pad.
 *
 * The interface is the PocketJS guest of ui/ (guest.c): over the scene on the
 * upper screen and the whole of the touch screen. It turns 30 times a second;
 * the upper screen's part is drawn again from the same vertices on the frames
 * between, and the lower screen is drawn on the frames a turn ran.
 *
 * Controls: the Circle Pad moves; the +Control Pad (or a C-Stick) turns the
 * camera, which follows the direction of travel when left alone; L and R fire
 * the wires, B is gas, Y cuts, X fires an aimed pair, A lets go. What START,
 * SELECT and the touch screen do is the interface's. L + R + START leaves.
 *
 * Development loop over PocketJS's paired LAN wire (port 8131, compiled in
 * from vendor/pocketjs/hosts/3ds): {"t":"maneuver.control","text":"..."}
 * steers the run and is answered with a maneuver.status record; "screenshot"
 * captures both screens; a .3dsx install replaces this program. */
#include <3ds.h>
#include <citro3d.h>
#include <malloc.h>
#include <math.h>
#include <pocket3d_title.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

#include "core.h"
#include "devserver.h"
#include "gfx.h"
#include "guest.h"
#include "hbldr.h"
#include "native.h"
#include "render.h"
#include "soc.h"
#include "svcwire.h"

/* QuickJS recurses on this thread: its own limit is 384 KiB (qjs.c). */
unsigned int __stacksize__ = 1024 * 1024;
extern int __system_argc;
extern char **__system_argv;

#define TAG(a, b, c, d) ((u32)(a) | ((u32)(b) << 8) | ((u32)(c) << 16) | ((u32)(d) << 24))
#define AUDIO_RATE 22050
#define AUDIO_FRAMES 736
#define AUDIO_BUFFERS 4

static const u32 transfer = GX_TRANSFER_FLIP_VERT(0) | GX_TRANSFER_OUT_TILED(0) | GX_TRANSFER_RAW_COPY(0) | GX_TRANSFER_IN_FORMAT(GX_TRANSFER_FMT_RGBA8) | GX_TRANSFER_OUT_FORMAT(GX_TRANSFER_FMT_RGB8) |
                            GX_TRANSFER_SCALING(GX_TRANSFER_SCALE_NO);

#define PREFS_PATH "sdmc:/pocket-maneuver/interface.json"

static const char *stage = "boot";
static char app_error[256];
static FILE *boot_log;
static C3D_RenderTarget *top, *bottom;
/* The lower screen is a text console: the interface did not start. */
static bool console;

/* A line for the boot log and the dev wire. */
static void say(const char *message) {
  if (boot_log) {
    fprintf(boot_log, "%s\n", message);
    fflush(boot_log);
  }
  devserver_report_log("info", message);
}

/* Without the interface an error has no screen of its own: the lower one becomes a console for it. */
static void console_error(const char *message) {
  if (!console) {
    gfxSetDoubleBuffering(GFX_BOTTOM, false);
    consoleInit(GFX_BOTTOM, NULL);
    console = true;
  }
  printf("%s\n", message);
  GSPGPU_FlushDataCache(gfxGetFramebuffer(GFX_BOTTOM, GFX_LEFT, NULL, NULL), 320 * 240 * 2);
}

/* One frame of the interface alone, while there is no scene to draw. */
static void interface_frame(void) {
  if (!guest_running() || !C3D_FrameBegin(C3D_FRAME_SYNCDRAW))
    return;
  guest_prepare();
  C3D_RenderTargetClear(top, C3D_CLEAR_ALL, 0x000000ff, 0);
  C3D_FrameDrawOn(top);
  guest_draw_top();
  guest_draw_bottom(bottom);
  C3D_FrameEnd(0);
}

/* A loading step: the interface shows it before the step, which blocks, begins. */
static void loading(const char *message) {
  say(message);
  mh_stage(MH_STAGE_LOADING, message, strlen(message));
  /* One turn takes the state in, the next has laid it out. */
  hidScanInput();
  guest_turn_now();
  guest_turn_now();
  interface_frame();
}

/* The start failed: `app_error` says why, on the interface or, without one, on the console. */
static void failed(void) {
  say(app_error);
  mh_stage(MH_STAGE_ERROR, app_error, strlen(app_error));
  if (!guest_running())
    console_error(app_error);
}

/* ---------------------------------------------------------------- the pack */

typedef struct {
  u32 tag, offset, size, zero;
} Section;

static FILE *pack;
static Section sections[32];
static u32 section_count, pack_bytes;

static const Section *section(u32 tag) {
  for (u32 i = 0; i < section_count; i++)
    if (sections[i].tag == tag)
      return &sections[i];
  return NULL;
}

/* Reads a whole section into memory from `alloc` (malloc or linearAlloc). */
static void *read_section(u32 tag, void *(*alloc)(size_t), u32 *size) {
  const Section *s = section(tag);
  if (!s)
    return NULL;
  void *p = alloc(s->size ? s->size : 1);
  if (!p)
    return NULL;
  if (fseek(pack, s->offset, SEEK_SET) != 0 || fread(p, 1, s->size, pack) != s->size)
    return NULL;
  if (size)
    *size = s->size;
  return p;
}

static bool open_pack(void) {
  pack = fopen("romfs:/world.pack", "rb");
  if (!pack) {
    snprintf(app_error, sizeof app_error, "romfs:/world.pack is missing");
    return false;
  }
  u32 head[4];
  if (fread(head, 4, 4, pack) != 4 || head[0] != TAG('M', 'V', 'P', 'K') || head[1] != 2 || head[2] > 32 || fread(sections, sizeof(Section), head[2], pack) != head[2]) {
    snprintf(app_error, sizeof app_error, "world.pack is not a version 2 pack");
    return false;
  }
  section_count = head[2];
  fseek(pack, 0, SEEK_END);
  pack_bytes = ftell(pack);
  return true;
}

static void *linear(size_t n) { return linearAlloc(n); }

static bool load(void) {
  MhSizes sizes;
  mh_sizes(&sizes);
  if (sizes.bones != MH_BONES || sizes.cloak_verts != MH_CLOAK_VERTS || sizes.cloak_indices != MH_CLOAK_INDICES || sizes.rope_verts != MH_ROPE_VERTS || sizes.disc_verts != MH_DISC_VERTS ||
      sizes.fan != MH_FAN || sizes.fan_indices != MH_FAN_INDICES || sizes.sky_verts != MH_SKY_VERTS || sizes.sky_indices != MH_SKY_INDICES || sizes.sun_verts != MH_SUN_VERTS ||
      sizes.scene_bytes != sizeof(MhScene) || sizes.mesh_bytes != sizeof(MhMesh) || sizes.vertex_bytes != 16) {
    snprintf(app_error, sizeof app_error, "core.h does not match the core library");
    return false;
  }
  if (!open_pack())
    return false;
  static RenderData data;
  u32 scene_size = 0, mesh_size = 0, simw_size = 0, font_size = 0, tex_size = 0;
  loading("Reading the world");
  MhScene *scene = read_section(TAG('H', 'S', 'C', 'N'), malloc, &scene_size);
  MhMesh *meshes = read_section(TAG('H', 'M', 'S', 'H'), malloc, &mesh_size);
  u8 *vtx = read_section(TAG('V', 'T', 'X', '0'), linear, NULL);
  u8 *idx = read_section(TAG('I', 'D', 'X', '0'), linear, NULL);
  u8 *models = read_section(TAG('M', 'O', 'D', 'L'), linear, NULL);
  u8 *tex = read_section(TAG('T', 'E', 'X', '0'), linear, &tex_size);
  u8 *font = read_section(TAG('F', 'O', 'N', 'T'), linear, &font_size);
  if (!scene || scene_size != sizeof(MhScene) || !meshes || !vtx || !idx || !models || !tex || !font) {
    snprintf(app_error, sizeof app_error, "world.pack lacks a section, or memory ran out (%lu KiB linear free)", (unsigned long)(linearSpaceFree() / 1024));
    return false;
  }
  loading("Building the collision grid");
  u8 *simw = read_section(TAG('S', 'I', 'M', 'W'), malloc, &simw_size);
  if (!simw) {
    snprintf(app_error, sizeof app_error, "no memory for the simulation world");
    return false;
  }
  const char *e = mh_init(scene, meshes, mesh_size / sizeof(MhMesh), simw, simw_size, font, font_size);
  free(simw);
  if (e) {
    snprintf(app_error, sizeof app_error, "%s", e);
    return false;
  }
  u32 font_w = 0, font_h = 0, font_at = mh_font(font, font_size, &font_w, &font_h);
  u32 th[4];
  memcpy(th, tex, 16);
  data = (RenderData){.scene = scene,
                      .meshes = meshes,
                      .vtx = vtx,
                      .idx = (const uint16_t *)idx,
                      .models = models,
                      .tex = tex + 16,
                      .tex_width = th[0],
                      .tex_height = th[1],
                      .tex_mips = th[2],
                      .tex_format = th[3],
                      .font_texels = font + font_at,
                      .font_width = font_w,
                      .font_height = font_h};
  loading("Uploading textures");
  if (!font_at || !render_init(&data, app_error, sizeof app_error))
    return false;
  linearFree(tex);
  linearFree(font);
  fclose(pack);
  pack = NULL;
  return true;
}

/* ---------------------------------------------------------------- sound */

static ndspWaveBuf wave[AUDIO_BUFFERS];
static bool sound;
static Result sound_result;
static void csnd_start_ring(void);

static void sound_init(void) {
  /* ndsp needs the console's DSP firmware dumped to sdmc:/3ds/dspfirm.cdc; without it there is no sound. */
  sound_result = ndspInit();
  if (R_FAILED(sound_result)) {
    csnd_start_ring();
    return;
  }
  ndspSetOutputMode(NDSP_OUTPUT_STEREO);
  ndspChnReset(0);
  ndspChnSetInterp(0, NDSP_INTERP_LINEAR);
  ndspChnSetRate(0, AUDIO_RATE);
  ndspChnSetFormat(0, NDSP_FORMAT_STEREO_PCM16);
  float mix[12] = {1.0f, 1.0f};
  ndspChnSetMix(0, mix);
  for (int i = 0; i < AUDIO_BUFFERS; i++) {
    wave[i].data_vaddr = linearAlloc(AUDIO_FRAMES * 4);
    wave[i].nsamples = AUDIO_FRAMES;
    wave[i].status = NDSP_WBUF_DONE;
    if (!wave[i].data_vaddr)
      return;
  }
  sound = true;
}

/* Without the DSP firmware dump, the older CSND service still plays a looping
 * buffer: the synthesizer writes ahead of the play position, which follows the
 * system clock at the channel's actual rate. One channel, the two sides mixed. */
#define CSND_RING 8192
#define CSND_LEAD 2048
#define CSND_CHANNEL 8
static s16 *csnd_ring;
static bool csnd_on;
static u64 csnd_start;
static u32 csnd_written;
static double csnd_rate;

static void csnd_start_ring(void) {
  if (R_FAILED(csndInit()))
    return;
  csnd_ring = linearAlloc(CSND_RING * 2);
  if (!csnd_ring)
    return;
  memset(csnd_ring, 0, CSND_RING * 2);
  GSPGPU_FlushDataCache(csnd_ring, CSND_RING * 2);
  /* The channel's timer divides the 67.03 MHz base clock by a whole number. */
  csnd_rate = 67027964.0 / (double)(u32)(67027964.0 / AUDIO_RATE);
  if (R_FAILED(csndPlaySound(CSND_CHANNEL, SOUND_FORMAT_16BIT | SOUND_REPEAT, AUDIO_RATE, 1.0f, 0.0f, csnd_ring, csnd_ring, CSND_RING * 2)))
    return;
  csnd_start = svcGetSystemTick();
  csnd_written = CSND_LEAD;
  csnd_on = true;
}

static void csnd_fill(void) {
  static s16 stereo[1024 * 2];
  u32 played = (u32)((double)(svcGetSystemTick() - csnd_start) * csnd_rate / SYSCLOCK_ARM11);
  /* Fell behind (a pause, a long load): start again ahead of the play position. */
  if ((s32)(csnd_written - played) < 256 || (s32)(csnd_written - played) > CSND_RING - 1024)
    csnd_written = played + CSND_LEAD / 2;
  s32 want = (s32)(played + CSND_LEAD - csnd_written);
  if (want <= 0)
    return;
  if (want > 1024)
    want = 1024;
  mh_audio(stereo, want, AUDIO_RATE);
  for (s32 i = 0; i < want; i++)
    csnd_ring[(csnd_written + i) % CSND_RING] = (s16)((stereo[i * 2] + stereo[i * 2 + 1]) / 2);
  csnd_written += want;
  GSPGPU_FlushDataCache(csnd_ring, CSND_RING * 2);
}

/* Refills at most two finished buffers a frame: 736 frames at 22.05 kHz is two display refreshes.
 * While the game is paused or the sound setting is off nothing is rendered: the queued buffers run
 * out, and the looping CSND buffer is cleared once. */
static void sound_fill(void) {
  static bool silent;
  if (!mh_audible()) {
    if (csnd_on && !silent) {
      memset(csnd_ring, 0, CSND_RING * 2);
      GSPGPU_FlushDataCache(csnd_ring, CSND_RING * 2);
    }
    silent = true;
    return;
  }
  silent = false;
  if (csnd_on) {
    csnd_fill();
    return;
  }
  if (!sound)
    return;
  int filled = 0;
  for (int i = 0; i < AUDIO_BUFFERS && filled < 2; i++) {
    if (wave[i].status != NDSP_WBUF_DONE)
      continue;
    mh_audio(wave[i].data_pcm16, AUDIO_FRAMES, AUDIO_RATE);
    DSP_FlushDataCache(wave[i].data_pcm16, AUDIO_FRAMES * 4);
    ndspChnWaveBufAdd(0, &wave[i]);
    filled++;
  }
}

/* ---------------------------------------------------------------- the wire */

static MhPerf perf;
static float cpu_ms, gpu_ms, wait_ms;
static u32 frame_no;
static bool new3ds;

/* `text` as the inside of a JSON string. */
static void escape(char *out, size_t capacity, const char *text) {
  size_t n = 0;
  for (; *text && n + 2 < capacity; text++) {
    unsigned char c = *text;
    if (c == '"' || c == '\\')
      out[n++] = '\\';
    out[n++] = c < ' ' ? ' ' : c;
  }
  out[n] = 0;
}

static void report_status(void) {
  static char body[3072], extra[1024], line[4200], guest[2 * 192];
  escape(guest, sizeof guest, guest_error());
  struct mallinfo heap = mallinfo();
  int n = snprintf(extra, sizeof extra,
                   "\"build\":\"" MANEUVER_BUILD_ID "\",\"phase\":\"%s\",\"error\":\"%s\",\"packBytes\":%lu,\"linearFree\":%lu,\"vramFree\":%lu,\"heap\":{\"used\":%lu,\"size\":%lu},\"cpuMsTotal\":%.3f,\"gpuWaitMs\":%.3f,\"sound\":%s,\"soundResult\":\"%08lx\",\"new3ds\":%s,\"uniforms\":[%d,%d,%d,%d],"
                   "\"guest\":{\"running\":%s,\"open\":%s,\"error\":\"%s\",\"turnMs\":%.3f,\"commands\":%lu,\"vertices\":%lu,\"dropped\":%lu}",
                   stage, app_error, (unsigned long)pack_bytes, (unsigned long)linearSpaceFree(), (unsigned long)vramSpaceFree(), (unsigned long)heap.uordblks, (unsigned long)envGetHeapSize(), cpu_ms, wait_ms,
                   sound ? "\"ndsp\"" : csnd_on ? "\"csnd\"" : "false", (unsigned long)sound_result, new3ds ? "true" : "false", render_debug[0], render_debug[1], render_debug[2], render_debug[3],
                   guest_running() ? "true" : "false", mh_interface_open() ? "true" : "false", guest, guest_turn_ms(), (unsigned long)gfx_frame_commands(), (unsigned long)gfx_frame_vertices(), (unsigned long)gfx_dropped_vertices());
  if (!strcmp(stage, "running")) {
    mh_status(body, sizeof body, &perf, extra, n);
    /* {"target":... becomes {"t":"maneuver.status","target":... */
    n = snprintf(line, sizeof line, "{\"t\":\"maneuver.status\",%s", body + 1);
  } else {
    n = snprintf(line, sizeof line, "{\"t\":\"maneuver.status\",\"target\":\"3ds\",\"stage\":\"%s\",%s}", stage, extra);
  }
  devserver_send_ctrl(line, n);
}

/* A word of a control line that is this host's to carry out:
 *   press=MASK       PocketJS BTN bits, pressed on the interface as a thumb would
 *   touch=X,Y        a stylus held on the lower screen; touch=off lifts it
 *   ui=start|pause|resume|restart|title   what the interface would ask of the game */
static void host_word(const char *word) {
  static const struct {
    const char *verb, *line;
  } asks[] = {{"start", "{\"type\":\"start\"}"},
              {"pause", "{\"type\":\"pause\",\"on\":true}"},
              {"resume", "{\"type\":\"pause\",\"on\":false}"},
              {"restart", "{\"type\":\"restart\"}"},
              {"title", "{\"type\":\"title\"}"}};
  int x, y;
  if (!strncmp(word, "press=", 6))
    guest_press(strtoul(word + 6, NULL, 0));
  else if (!strncmp(word, "touch=", 6))
    guest_touch(sscanf(word + 6, "%d,%d", &x, &y) == 2, x, y);
  else if (!strncmp(word, "ui=", 3))
    for (unsigned i = 0; i < sizeof asks / sizeof *asks; i++)
      if (!strcmp(word + 3, asks[i].verb))
        svcwire_send_line(asks[i].line, strlen(asks[i].line));
}

/* The "text" member of a control line: words for this host and for Game::control, no escapes. */
static void controls(const char *line) {
  if (!strstr(line, "\"maneuver.control\""))
    return;
  const char *t = strstr(line, "\"text\":\"");
  const char *end = t ? strchr(t + 8, '"') : NULL;
  if (end) {
    t += 8;
    static char words[1024];
    snprintf(words, sizeof words, "%.*s", (int)(end - t), t);
    char *save = NULL;
    for (char *word = strtok_r(words, " ", &save); word; word = strtok_r(NULL, " ", &save))
      host_word(word);
    if (!strcmp(stage, "running"))
      mh_control(t, end - t);
  }
  report_status();
}

/* Both screens, read back from their targets after the GPU finished them. */
static bool capture(void) {
  uint8_t *upper = NULL, *lower = NULL;
  if (!devserver_screenshot_begin(frame_no, 400, 240, 320, 240, &upper, &lower))
    return false;
  C3D_SyncDisplayTransfer(top->frameBuf.colorBuf, GX_BUFFER_DIM(240, 400), (u32 *)upper, GX_BUFFER_DIM(240, 400), transfer);
  GSPGPU_InvalidateDataCache(upper, 400 * 240 * 3);
  if (bottom) {
    C3D_SyncDisplayTransfer(bottom->frameBuf.colorBuf, GX_BUFFER_DIM(240, 320), (u32 *)lower, GX_BUFFER_DIM(240, 320), transfer);
    GSPGPU_InvalidateDataCache(lower, 320 * 240 * 3);
  } else {
    memset(lower, 0, 320 * 240 * 3);
  }
  return true;
}

/* What the interface asked to have stored goes to the card. */
static void keep_prefs(void) {
  static char text[4096];
  uint32_t n = mh_prefs_take(text, sizeof text);
  FILE *f = n ? fopen(PREFS_PATH, "w") : NULL;
  if (f) {
    fwrite(text, 1, n, f);
    fclose(f);
  }
}

static void read_pad(MhPad *pad) {
  u32 held = hidKeysHeld();
  circlePosition c, cs;
  hidCircleRead(&c);
  hidCstickRead(&cs);
  pad->buttons = 0;
  if (held & (KEY_L | KEY_ZL))
    pad->buttons |= MH_HOOK_L;
  if (held & (KEY_R | KEY_ZR))
    pad->buttons |= MH_HOOK_R;
  if (held & KEY_B)
    pad->buttons |= MH_GAS;
  if (held & KEY_Y)
    pad->buttons |= MH_SLASH;
  if (held & KEY_X)
    pad->buttons |= MH_ZIP;
  if (held & KEY_A)
    pad->buttons |= MH_DROP;
  if (held & KEY_START)
    pad->buttons |= MH_START;
  if (held & KEY_SELECT)
    pad->buttons |= MH_SELECT;
  /* The Circle Pad reaches about 150 units. */
  pad->lx = fmaxf(-1.0f, fminf(1.0f, c.dx / 150.0f));
  pad->ly = fmaxf(-1.0f, fminf(1.0f, c.dy / 150.0f));
  pad->rx = (held & KEY_DRIGHT ? 0.8f : 0.0f) - (held & KEY_DLEFT ? 0.8f : 0.0f);
  pad->ry = (held & KEY_DUP ? 0.8f : 0.0f) - (held & KEY_DDOWN ? 0.8f : 0.0f);
  /* A C-Stick (New 3DS) reaches about 100 units. */
  if (abs(cs.dx) > 12 || abs(cs.dy) > 12) {
    pad->rx = fmaxf(-1.0f, fminf(1.0f, cs.dx / 100.0f));
    pad->ry = fmaxf(-1.0f, fminf(1.0f, cs.dy / 100.0f));
  }
}

int main(void) {
  gfxInitDefault();
  gfxSet3D(false);
  /* The Pocket3D title card, on both screens, before the GPU is set up. */
  pocket3d_title_play();
  mkdir("sdmc:/pocket-maneuver", 0777);
  boot_log = fopen("sdmc:/pocket-maneuver/boot.log", "w");
  say("Pocket Maneuver " MANEUVER_BUILD_ID);
  char error[256] = {0};
  PocketRuntimeState state = {0};
  if (__system_argc > 0 && __system_argv)
    native_set_running_path(__system_argv[0]);
  mkdir("sdmc:/pocketjs", 0777);
  mkdir("sdmc:/pocketjs/runtime", 0777);
  devserver_allow_packages(false);
  DevserverInitResult dev = devserver_init(&state, error, sizeof error);
  say(dev == DEVSERVER_READY ? "Remote debugger ready" : error);
  /* A New 3DS runs at 804 MHz when asked; an Old 3DS ignores it. */
  osSetSpeedupEnable(true);
  APT_CheckNew3DS(&new3ds);

  bool gpu_ok = C3D_Init(C3D_DEFAULT_CMDBUF_SIZE * 2), gpu_stalled = false;
  top = gpu_ok ? C3D_RenderTargetCreate(240, 400, GPU_RB_RGBA8, GPU_RB_DEPTH24_STENCIL8) : NULL;
  if (top)
    C3D_RenderTargetSetOutput(top, GFX_TOP, GFX_LEFT, transfer);
  Result romfs = romfsInit();
  stage = "loading";
  devserver_set_runtime(&state, NULL, stage, 0);
  devserver_poll();

  /* The interface comes up before the pack is read, so it can show the reading. */
  if (top && R_SUCCEEDED(romfs)) {
    static char prefs[4096];
    FILE *kept = fopen(PREFS_PATH, "r");
    if (kept) {
      mh_prefs_stored(prefs, fread(prefs, 1, sizeof prefs - 1, kept));
      fclose(kept);
    }
    /* The interface draws without depth: the lower target has a colour buffer only. */
    bottom = C3D_RenderTargetCreate(240, 320, GPU_RB_RGBA8, -1);
    if (bottom && guest_boot(error, sizeof error)) {
      C3D_RenderTargetSetOutput(bottom, GFX_BOTTOM, GFX_LEFT, transfer);
    } else {
      if (bottom)
        C3D_RenderTargetDelete(bottom);
      else
        snprintf(error, sizeof error, "no video memory for the lower screen");
      bottom = NULL;
      say(error);
      console_error("The interface did not start:");
      console_error(error);
    }
  }

  bool loaded = false;
  if (!gpu_ok || !top)
    snprintf(app_error, sizeof app_error, "PICA initialization failed");
  else if (R_FAILED(romfs))
    snprintf(app_error, sizeof app_error, "romfsInit failed: %08lx", (unsigned long)romfs);
  else
    loaded = load();
  if (loaded) {
    sound_init();
    stage = "running";
  } else {
    stage = "load-error";
    failed();
  }
  devserver_set_runtime(&state, NULL, stage, 0);

  u64 last = svcGetSystemTick(), next_retry = 0;
  u32 ticks = 1, late = 0, last_count = C3D_FrameCounter(0);
  float worst = 0, avg = 16.7f;
  bool shot = false, capture_pending = false;
  while (aptMainLoop()) {
    hidScanInput();
    u32 held = hidKeysHeld();
    if ((held & (KEY_L | KEY_R | KEY_START)) == (KEY_L | KEY_R | KEY_START))
      break;
    if (!capture_pending)
      devserver_poll();
    u64 now = svcGetSystemTick();
    if (dev != DEVSERVER_READY && now >= next_retry) {
      dev = devserver_init(&state, error, sizeof error);
      next_retry = now + SYSCLOCK_ARM11 * 3ULL;
    }
    char launch[POCKET_RUNTIME_NATIVE_NAME_BYTES + 1], path[POCKET_NATIVE_PATH_BYTES];
    if (devserver_take_launch(launch) && native_path_for(launch, path)) {
      if (hbldr_launch_on_exit(path, error, sizeof error)) {
        devserver_flush(1000);
        devserver_report_native("launching", launch, "exiting to start it");
        devserver_poll();
        devserver_flush(1000);
        for (unsigned i = 0; i < 200; i++) {
          devserver_poll();
          svcSleepThread(1000000);
        }
        break;
      }
      devserver_report_native("launch-error", launch, error);
    }
    static char control[8192];
    size_t n;
    while ((n = devserver_recv_ctrl(control, sizeof control - 1)) > 0) {
      control[n] = 0;
      char *save = NULL;
      for (char *line = strtok_r(control, "\n", &save); line; line = strtok_r(NULL, "\n", &save)) {
        if (strstr(line, "\"screenshot\""))
          devserver_request_screenshot();
        controls(line);
      }
    }
    if (native_receiving() || gpu_stalled || !top) {
      svcSleepThread(1000000);
      continue;
    }
    if (!loaded) {
      /* The interface says why; without it the console already has. */
      if (guest_running()) {
        guest_turn(1.0f / 60);
        interface_frame();
        /* A capture reads the targets once the GPU has finished the frame just queued. */
        if (devserver_take_screenshot_request() && C3D_FrameBegin(C3D_FRAME_SYNCDRAW)) {
          bool captured = capture();
          C3D_FrameEnd(0);
          if (captured)
            devserver_screenshot_ready();
        }
      } else {
        svcSleepThread(1000000);
      }
      continue;
    }
    float frame_ms = (float)(now - last) * 1000.0f / SYSCLOCK_ARM11;
    last = now;
    u32 pace = C3D_FrameCounter(0);

    /* ------------------------------------------------------------ input, simulation and the interface's turn */
    MhPad pad;
    read_pad(&pad);
    /* Does what the interface asked on its last turn, then shows it the game as it now is. */
    mh_step(&pad, ticks);
    sound_fill();
    keep_prefs();
    MhView view;
    mh_view(&view);
    u64 turn_start = svcGetSystemTick();
    float sim_ms = (float)(turn_start - now) * 1000.0f / SYSCLOCK_ARM11;
    bool was_running = guest_running();
    guest_turn(ticks / 60.0f);
    if (was_running && !guest_running())
      devserver_report_log("error", guest_error());
    float ui_ms = (float)(svcGetSystemTick() - turn_start) * 1000.0f / SYSCLOCK_ARM11;

    /* ------------------------------------------------------------ the previous frame leaves the GPU */
    u64 wait_start = svcGetSystemTick();
    while (!C3D_FrameBegin(C3D_FRAME_NONBLOCK)) {
      if (!capture_pending)
        devserver_poll();
      svcSleepThread(100000);
      if (svcGetSystemTick() - wait_start > SYSCLOCK_ARM11 * 4ULL) {
        stage = "gpu-timeout";
        say("GPU timeout: the debugger remains available");
        devserver_set_runtime(&state, NULL, stage, 0);
        gpu_stalled = true;
        break;
      }
    }
    if (gpu_stalled)
      continue;
    gpu_ms = C3D_GetDrawingTime();
    if (capture_pending) {
      devserver_screenshot_ready();
      capture_pending = false;
    }
    if (shot) {
      capture_pending = capture();
      shot = false;
    }
    /* One frame per display refresh at most. */
    while (C3D_FrameCounter(0) == pace) {
      if (!capture_pending)
        devserver_poll();
      svcSleepThread(100000);
    }
    u64 build_start = svcGetSystemTick();
    wait_ms = (float)(build_start - wait_start) * 1000.0f / SYSCLOCK_ARM11;
    /* One tick per display refresh since the last frame: a late frame catches up. */
    u32 count = C3D_FrameCounter(0);
    ticks = count - last_count;
    last_count = count;
    if (ticks < 1)
      ticks = 1;
    if (ticks > 3)
      ticks = 3;
    if (ticks > 1)
      late++;
    avg += (frame_ms - avg) * 0.05f;
    worst = frame_no % 120 == 0 ? frame_ms : fmaxf(worst, frame_ms);
    perf = (MhPerf){.frame = avg, .worst = worst, .late = late, .frames = frame_no, .sim = sim_ms, .build = perf.build, .draw = 0, .gpu = gpu_ms, .draws = render_stats.draws, .tris = render_stats.tris};

    /* The interface's vertices when it took a turn; the scene; the interface over it; the lower
     * screen on the frames its content changed (it keeps what it showed on the others). */
    bool turned = guest_prepare();
    render_frame(top, &view, ticks, &perf);
    guest_draw_top();
    if (turned)
      guest_draw_bottom(bottom);
    C3D_FrameEnd(0);
    perf.build = (float)(svcGetSystemTick() - build_start) * 1000.0f / SYSCLOCK_ARM11;
    perf.draws = render_stats.draws;
    perf.tris = render_stats.tris;
    cpu_ms = sim_ms + ui_ms + perf.build;
    frame_no++;
    devserver_set_runtime(&state, NULL, stage, frame_no);
    devserver_set_frame_stats(frame_no, render_stats.draws, render_stats.tris * 3, 0);
    devserver_set_frame_timing((uint32_t)(ui_ms * 1000), (uint32_t)(sim_ms * 1000), (uint32_t)(perf.build * 1000), (uint32_t)(gpu_ms * 1000), (uint32_t)(frame_ms * 1000));
    shot = shot || devserver_take_screenshot_request();
  }

  if (gpu_ok && !gpu_stalled) {
    /* Retire the frame in flight before the targets go. */
    for (unsigned i = 0; i < 4000 && !C3D_FrameBegin(C3D_FRAME_NONBLOCK); i++) {
      devserver_poll();
      svcSleepThread(1000000);
    }
    C3D_FrameEnd(0);
    guest_shutdown();
    if (top)
      C3D_RenderTargetDelete(top);
    if (bottom)
      C3D_RenderTargetDelete(bottom);
    C3D_Fini();
  }
  if (sound)
    ndspExit();
  if (csnd_on) {
    CSND_SetPlayState(CSND_CHANNEL, 0);
    csndExecCmds(true);
    csndExit();
  }
  if (!gpu_stalled || !capture_pending)
    devserver_shutdown();
  soc_shutdown();
  if (R_SUCCEEDED(romfs))
    romfsExit();
  if (native_exit_pending() && !native_finish_exit(error, sizeof error))
    say(error);
  if (boot_log)
    fclose(boot_log);
  gfxExit();
  return 0;
}
