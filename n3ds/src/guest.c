#include "guest.h"

#include <3ds.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "core.h"
#include "gfx.h"
#include "input.h"
#include "offload.h"
#include "pocket_core.h"
#include "qjs.h"
#include "svcwire.h"

/* The guest turns this often, and is told so before it mounts. */
#define TURN (1.0f / 30)
static const char rate[] = "globalThis.__simHz=30;";

static bool running;
static char failure[192];
static float owed = TURN, turn_ms;
/* A turn ran and its DrawLists have not been built into vertices yet. */
static bool fresh;
static size_t words[2];
/* The DrawList backend sets its vertex layout once, on citro3d's own objects;
 * the scene sets its own for every draw. */
static C3D_AttrInfo attributes;
static C3D_BufInfo buffers;
/* Buttons held at any moment since the last turn: a press shorter than a turn reaches the guest. */
static int32_t held;
/* The stylus, when it touched since the last turn. */
static bool touched;
static uint32_t touched_at;
static uint32_t presses[16];
static unsigned press_count, press_turns;
static bool stylus;
static uint32_t stylus_at;

/* PocketJS's guest driver reaches its companion services through these; this program has none. */
void offload_frame(void) {}
int offload_session(void) { return 0; }
bool offload_submit(const char *bytes, size_t length) {
  (void)bytes, (void)length;
  return false;
}
size_t offload_take(char *out) {
  (void)out;
  return 0;
}
/* The guest's DevTools would take the control lines meant for this program
 * off the dev wire: qjs.c is compiled to ask here whether a debugger is attached. */
bool guest_devtools(void) { return false; }

static char *read_file(const char *path, const char *before, size_t *size) {
  FILE *file = fopen(path, "rb");
  if (!file)
    return NULL;
  fseek(file, 0, SEEK_END);
  size_t length = ftell(file), lead = strlen(before);
  rewind(file);
  char *data = malloc(lead + length + 1);
  if (data) {
    memcpy(data, before, lead);
    *size = lead + fread(data + lead, 1, length, file);
    data[*size] = 0;
  }
  fclose(file);
  return data;
}

bool guest_boot(char *error, size_t capacity) {
  size_t script_size = 0, pak_size = 0;
  /* The pak is the guest's for good; the script only until it has run. */
  char *pak = read_file("romfs:/maneuver.pak", "", &pak_size);
  char *script = read_file("romfs:/maneuver.js", rate, &script_size);
  ui_init(1);
  ui_set_viewport(400, 240);
  if (!pak || !script)
    snprintf(failure, sizeof failure, "maneuver.js or maneuver.pak is missing");
  else if (!ui_create_auxiliary_surface(320, 240))
    snprintf(failure, sizeof failure, "no memory for the touch screen");
  else {
    ui_feed_pak((const uint8_t *)pak, pak_size);
    if (!qjs_boot(script, script_size, (const uint8_t *)pak, pak_size))
      snprintf(failure, sizeof failure, "%s", qjs_last_error());
    else if (!gfx_init(400, 240))
      snprintf(failure, sizeof failure, "no memory for the interface");
    else {
      attributes = *C3D_GetAttrInfo();
      buffers = *C3D_GetBufInfo();
      input_init();
      running = true;
    }
  }
  free(script);
  if (!running)
    svcwire_shutdown();
  snprintf(error, capacity, "%s", failure);
  return running;
}

void guest_shutdown(void) {
  if (!running)
    return;
  running = false;
  svcwire_shutdown();
  qjs_shutdown();
  gfx_shutdown();
  input_shutdown();
}

bool guest_running(void) { return running; }
const char *guest_error(void) { return failure; }
float guest_turn_ms(void) { return turn_ms; }

void guest_press(uint32_t buttons) {
  if (press_count < sizeof presses / sizeof *presses)
    presses[press_count++] = buttons;
}

void guest_touch(bool down, int x, int y) {
  stylus = down;
  stylus_at = ((uint32_t)y & 0x1ff) << 9 | ((uint32_t)x & 0x1ff);
}

static void note(void) {
  held |= input_buttons();
  uint32_t at;
  if (input_touch(&at)) {
    touched = true;
    touched_at = at;
  }
}

static void turn(void) {
  u64 start = svcGetSystemTick();
  int32_t buttons = held;
  held = 0;
  /* A press from a control message is held two turns and let go for one. */
  if (press_count) {
    if (++press_turns < 3)
      buttons |= presses[0];
    else {
      memmove(presses, presses + 1, --press_count * sizeof *presses);
      press_turns = 0;
    }
  }
  uint32_t touch = stylus ? stylus_at : touched_at;
  size_t touches = touched || stylus;
  touched = false;
  int32_t hit = 0;
  ui_touch_hits_auxiliary(touches ? &touch : NULL, touches, &hit, 1);
  if (!qjs_frame(buttons, input_analog(), &touch, &hit, touches, input_right_analog())) {
    snprintf(failure, sizeof failure, "%s", qjs_last_error());
    running = false;
    /* With the channel closed the pad keeps the game's flow itself. */
    svcwire_shutdown();
    return;
  }
  /* Two core ticks to a turn: the core counts sixtieths. */
  ui_tick();
  ui_tick();
  words[0] = ui_draw();
  words[1] = ui_draw_auxiliary();
  fresh = true;
  float ms = (float)(svcGetSystemTick() - start) * 1000.0f / SYSCLOCK_ARM11;
  turn_ms += (ms - turn_ms) * 0.1f;
}

void guest_turn(float dt) {
  if (!running)
    return;
  note();
  owed += dt;
  if (owed > 2 * TURN)
    owed = 2 * TURN;
  if (owed < TURN)
    return;
  owed -= TURN;
  /* A turn costs milliseconds however little changed: an idle guest takes one when there is a reason. */
  if (!mh_guest_due(held, touched || stylus) && !press_count) {
    held = 0;
    return;
  }
  turn();
}

void guest_turn_now(void) {
  if (!running)
    return;
  note();
  turn();
}

bool guest_prepare(void) {
  if (!running || !fresh)
    return false;
  fresh = false;
  /* The vertices are one arena for both screens: the frames between two turns draw the upper screen's again from it. */
  gfx_begin_frame();
  gfx_prepare_surface(0, ui_draw_list_ptr(), words[0], 400, 240);
  gfx_prepare_surface(1, ui_draw_auxiliary_list_ptr(), words[1], 320, 240);
  gfx_finish_frame();
  return true;
}

static void draw(uint32_t surface) {
  C3D_SetAttrInfo(&attributes);
  C3D_SetBufInfo(&buffers);
  C3D_AlphaTest(false, GPU_ALWAYS, 0);
  C3D_StencilTest(false, GPU_ALWAYS, 0, 0xff, 0xff);
  C3D_FogGasMode(GPU_NO_FOG, GPU_PLAIN_DENSITY, false);
  C3D_SetScissor(GPU_SCISSOR_DISABLE, 0, 0, 0, 0);
  gfx_draw_surface(surface);
}

void guest_draw_top(void) {
  if (!running)
    return;
  C3D_SetViewport(0, 0, 240, 400);
  draw(0);
}

void guest_draw_bottom(C3D_RenderTarget *bottom) {
  C3D_RenderTargetClear(bottom, C3D_CLEAR_COLOR, 0x000000ff, 0);
  C3D_FrameDrawOn(bottom);
  if (!running)
    return;
  C3D_SetViewport(0, 0, 240, 320);
  draw(1);
}
