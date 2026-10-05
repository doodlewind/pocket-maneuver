// Pocket Maneuver on the iPod touch 4 (iOS 6): the shell around the renderer
// (render.c), the simulation and the device-independent runtime (../core,
// the 3DS core's source), and the interface, a PocketJS guest (ui/, QuickJS)
// drawn over the scene. The device has a touch panel and no pad: the stick,
// the keys and the view all come from the interface's touch presentation as
// `drive` and `look` commands, which the core applies to the simulation.
//
// The device builds from the macOS SDK's C headers, so UIKit is reached
// through the Objective-C runtime. The main thread owns UIKit and receives
// touches; one render thread owns the GL context, the game and the guest, and
// they share only `shared` under its lock. Sound leaves through an audio
// queue that drains a ring the render thread fills.
//
// The EAGL layer is the portrait screen at 320x480, opaque and untransformed,
// with nothing over it: Core Animation shows the frame as it is instead of
// compositing it with the same GPU. The interface is drawn into a texture
// when what it shows changes, and that texture is laid over the frame.
// Everything is drawn a quarter turn round, for a device held with its home
// button on the right.
//
// Development loop (tools/ipod.ts): `tmp/control.txt` holds a nonce and words
// for this shell and for Game::control; `tmp/status.json` reports the run.
#include "contact_latch.h"
#include "pocket_runtime.h"
#include "render.h"
#include "svcwire.h"
#define GL_SILENCE_DEPRECATION 1
#include <AudioToolbox/AudioQueue.h>
#include <OpenGL/gl3.h>
#include <dlfcn.h>
#include <mach/mach.h>
#include <mach/mach_time.h>
#include <math.h>
#include <objc/message.h>
#include <objc/runtime.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#ifndef MANEUVER_BUILD
#define MANEUVER_BUILD "development"
#endif
// Guest turns a second: one per frame, so the stick and the keys reach the
// game the frame after a thumb moves. A turn advances the UI core by the
// display refreshes since the last one.
#ifndef TURN_HZ
#define TURN_HZ 60
#endif
typedef struct {
  float x, y;
} Spot; // CGPoint: CGFloat is a float on this device
typedef struct {
  float x, y, width, height;
} Frame; // CGRect
extern int UIApplicationMain(int, char **, id, id);
extern void glDiscardFramebufferEXT(GLenum, GLsizei, const GLenum *);
extern uint64_t ui_draw_hash(void);
extern int32_t ui_gl_render_over(int32_t, int32_t, int32_t, int32_t, int32_t, int32_t);
enum { WINDOW = 120 };

static struct {
  PocketContactLatch touches;
  bool active, parked;
} shared = {.active = true};
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t changed = PTHREAD_COND_INITIALIZER;
static char bundle[1024], tmp[1024], documents[1024];
static id context, view;
static GLuint framebuffer, colorbuffer;

static SEL sel(const char *name) { return sel_registerName(name); }
static id cls(const char *name) { return (id)objc_getClass(name); }
static id send(id object, const char *name) { return ((id(*)(id, SEL))objc_msgSend)(object, sel(name)); }
static id send_id(id object, const char *name, id value) { return ((id(*)(id, SEL, id))objc_msgSend)(object, sel(name), value); }
static void send_int(id object, const char *name, int value) { ((void (*)(id, SEL, int))objc_msgSend)(object, sel(name), value); }
static id string(const char *text) {
  return ((id(*)(id, SEL, const char *))objc_msgSend)(cls("NSString"), sel("stringWithUTF8String:"), text);
}
static double now(void) {
  static mach_timebase_info_data_t rate;
  if (!rate.denom)
    mach_timebase_info(&rate);
  return (double)mach_absolute_time() * rate.numer / rate.denom * 1e-9;
}
// Replaces the file in one step: the host never reads half a status.
static void write_file(const char *directory, const char *name, const void *data, size_t size) {
  char path[1100], staging[1110];
  snprintf(path, sizeof path, "%s/%s", directory, name);
  snprintf(staging, sizeof staging, "%s.new", path);
  FILE *file = fopen(staging, "wb");
  if (!file)
    return;
  fwrite(data, 1, size, file);
  fclose(file);
  rename(staging, path);
}
// A whole file, with a NUL after it; `before` goes in front.
static char *read_file(const char *directory, const char *name, const char *before, size_t *size) {
  char path[1100];
  snprintf(path, sizeof path, "%s/%s", directory, name);
  FILE *file = fopen(path, "rb");
  if (!file)
    return NULL;
  fseek(file, 0, SEEK_END);
  size_t length = ftell(file), lead = strlen(before);
  rewind(file);
  char *data = malloc(lead + length + 1);
  memcpy(data, before, lead);
  *size = lead + fread(data + lead, 1, length, file);
  data[*size] = 0;
  fclose(file);
  return data;
}

// ---- sound

// The synthesizer renders at 22.05 kHz on the render thread, a few blocks
// ahead; the queue's own thread takes blocks from the ring and plays silence
// when it runs dry.
enum { RATE = 22050, BLOCK = 512, RING = 8192 };
static int16_t ring[RING * 2];
static volatile unsigned ring_read, ring_written;
static AudioQueueRef queue;

// The pinned sysroot's AudioToolbox cannot be linked against (its export
// table did not survive the extraction from the shared cache), so the four
// calls are looked up in the device's own copy when sound starts.
static OSStatus (*queue_new)(const AudioStreamBasicDescription *, AudioQueueOutputCallback, void *, CFRunLoopRef, CFStringRef, UInt32, AudioQueueRef *);
static OSStatus (*queue_allocate)(AudioQueueRef, UInt32, AudioQueueBufferRef *);
static OSStatus (*queue_enqueue)(AudioQueueRef, AudioQueueBufferRef, UInt32, const AudioStreamPacketDescription *);
static OSStatus (*queue_start)(AudioQueueRef, const AudioTimeStamp *);

static void sound_block(void *unused, AudioQueueRef from, AudioQueueBufferRef block) {
  (void)unused;
  int16_t *out = block->mAudioData;
  unsigned read = ring_read, have = ring_written - read;
  if (have > BLOCK)
    have = BLOCK;
  for (unsigned i = 0; i < have; i++) {
    out[i * 2] = ring[(read + i) % RING * 2];
    out[i * 2 + 1] = ring[(read + i) % RING * 2 + 1];
  }
  memset(out + have * 2, 0, (BLOCK - have) * 4);
  ring_read = read + have;
  block->mAudioDataByteSize = BLOCK * 4;
  queue_enqueue(from, block, 0, NULL);
}
static bool sound_start(void) {
  void *toolbox = dlopen("/System/Library/Frameworks/AudioToolbox.framework/AudioToolbox", RTLD_NOW);
  if (!toolbox)
    return false;
  queue_new = dlsym(toolbox, "AudioQueueNewOutput");
  queue_allocate = dlsym(toolbox, "AudioQueueAllocateBuffer");
  queue_enqueue = dlsym(toolbox, "AudioQueueEnqueueBuffer");
  queue_start = dlsym(toolbox, "AudioQueueStart");
  if (!queue_new || !queue_allocate || !queue_enqueue || !queue_start)
    return false;
  const AudioStreamBasicDescription format = {.mSampleRate = RATE, .mFormatID = kAudioFormatLinearPCM,
                                              .mFormatFlags = kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked,
                                              .mBytesPerPacket = 4, .mFramesPerPacket = 1, .mBytesPerFrame = 4, .mChannelsPerFrame = 2, .mBitsPerChannel = 16};
  if (queue_new(&format, sound_block, NULL, NULL, NULL, 0, &queue))
    return false;
  for (unsigned i = 0; i < 3; i++) {
    AudioQueueBufferRef block;
    if (queue_allocate(queue, BLOCK * 4, &block))
      return false;
    sound_block(NULL, queue, block);
  }
  return !queue_start(queue, NULL);
}
// Once a frame: keeps about three blocks rendered ahead of the queue.
static void sound_fill(void) {
  static int16_t pcm[1024 * 2];
  int want = 3 * BLOCK - (int)(ring_written - ring_read);
  if (want <= 0)
    return;
  if (want > 1024)
    want = 1024;
  if (mh_audible())
    mh_audio(pcm, want, RATE);
  else
    memset(pcm, 0, want * 4);
  unsigned at = ring_written;
  for (int i = 0; i < want; i++) {
    ring[(at + i) % RING * 2] = pcm[i * 2];
    ring[(at + i) % RING * 2 + 1] = pcm[i * 2 + 1];
  }
  ring_written = at + want;
}

// ---- render thread

enum { LOADING, RUNNING, FAILED };
// Milliseconds per presented frame: the frame, its work, the simulation, the
// guest's turn, the interface's redraw, the scene's commands, handing the
// frame over (which waits for the display to take the last one), the present.
enum { INTERVAL, WORK, SIM, GUEST, REDRAW, SCENE, KICK, PRESENT, TIMINGS };
static float timing[TIMINGS][WINDOW];
static int stage = LOADING;
static bool sound, guest;
static unsigned frames, late, redraws;
static char command[40], captured[40], failure[256];
// The interface is drawn into one of two textures in turn: the frame the GPU
// still works on reads the other, so a redraw never waits for it.
static GLuint interface_target[2], interface_texture[2], overlay;
static unsigned interface_shown;
static MhPerf perf;
// Contacts a command holds on the panel, for driving the interface from the
// host; `tap` lifts them again after that many turns.
static struct {
  int count;
  float at[4][2];
} fingers, fingered;
static int tap;
// A measurement the device takes by itself: `mark=SECONDS` starts one three
// seconds later, and the host reads it when it is done. Asking the device
// anything over SSH costs its one core a few frames, so nothing is asked
// while the window is open.
static struct {
  double from, until, seconds;
  unsigned frames, late, max_tris, max_draws;
  float worst;
  double sums[TIMINGS];
  bool done;
} measured;

// The status record on its way to the file.
static char report[6144];
static int report_size;
static pthread_mutex_t report_lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t report_ready = PTHREAD_COND_INITIALIZER;

static int compare(const void *a, const void *b) { return (*(const float *)a > *(const float *)b) - (*(const float *)a < *(const float *)b); }
static int summary(char *out, size_t capacity, const char *name, const float *a) {
  float sorted[WINDOW], sum = 0;
  unsigned n = frames < WINDOW ? frames : WINDOW;
  for (unsigned i = 0; i < n; i++)
    sum += sorted[i] = a[i];
  qsort(sorted, n, sizeof *sorted, compare);
  return snprintf(out, capacity, "\"%s\":{\"mean\":%.3f,\"p95\":%.3f,\"max\":%.3f},", name, n ? sum / n : 0, n ? sorted[n * 95 / 100] : 0,
                  n ? sorted[n - 1] : 0);
}
static void status(void) {
  static const char *const names[TIMINGS] = {"intervalMs", "workMs", "simMs", "guestMs", "interfaceDrawMs", "sceneMs", "kickMs", "presentMs"};
  static const char *const stages[] = {"loading", "running", "failed"};
  static char text[6144], extra[2560], error[2 * sizeof failure];
  unsigned e = 0;
  for (const char *c = failure; *c && e < sizeof error - 2; c++) {
    if (*c == '"' || *c == '\\')
      error[e++] = '\\';
    error[e++] = *c < ' ' ? ' ' : *c;
  }
  error[e] = 0;
  task_basic_info_data_t task;
  mach_msg_type_number_t count = TASK_BASIC_INFO_COUNT;
  if (task_info(mach_task_self(), TASK_BASIC_INFO, (task_info_t)&task, &count) != KERN_SUCCESS)
    task.resident_size = 0;
  int at = snprintf(extra, sizeof extra, "\"build\":\"" MANEUVER_BUILD "\",\"fps\":%.3f,", perf.frame > 0 ? 1000 / perf.frame : 0);
  // The last 120 presented frames.
  for (unsigned i = 0; i < TIMINGS; i++)
    at += summary(extra + at, sizeof extra - at, names[i], timing[i]);
  at += snprintf(extra + at, sizeof extra - at,
                 "\"interfaceRedraws\":%u,\"guest\":{\"running\":%s,\"error\":\"%s\",\"turnHz\":%d},\"sound\":%s,"
                 "\"residentBytes\":%u,\"glError\":%u,\"lastCommand\":\"%s\",\"capture\":\"%s\"",
                 redraws, guest ? "true" : "false", guest ? "" : pocket_runtime_error(), TURN_HZ, sound ? "true" : "false",
                 (unsigned)task.resident_size, glGetError(), command, captured);
  at += snprintf(extra + at, sizeof extra - at, ",\"window\":{\"done\":%s,\"seconds\":%.3f,\"frames\":%u,\"late\":%u,\"worstMs\":%.3f,\"maxTris\":%u,\"maxDraws\":%u,\"meanMs\":{",
                 measured.done ? "true" : "false", measured.seconds, measured.frames, measured.late, measured.worst, measured.max_tris, measured.max_draws);
  for (unsigned i = 0; i < TIMINGS; i++)
    at += snprintf(extra + at, sizeof extra - at, "%s\"%.*s\":%.3f", i ? "," : "", (int)strlen(names[i]) - 2, names[i], measured.frames ? measured.sums[i] / measured.frames : 0);
  at += snprintf(extra + at, sizeof extra - at, "}}");
  if (stage == RUNNING)
    at = mh_status(text, sizeof text, &perf, extra, at);
  else
    at = snprintf(text, sizeof text, "{\"target\":\"ipod\",\"stage\":\"%s\",\"error\":\"%s\",%s}", stages[stage], error, extra);
  // A file write can take longer than the frame has left: another thread does it.
  pthread_mutex_lock(&report_lock);
  memcpy(report, text, report_size = at);
  pthread_cond_signal(&report_ready);
  pthread_mutex_unlock(&report_lock);
}
static void *reporter(void *unused) {
  (void)unused;
  static char text[sizeof report];
  for (;;) {
    pthread_mutex_lock(&report_lock);
    while (!report_size)
      pthread_cond_wait(&report_ready, &report_lock);
    int size = report_size;
    memcpy(text, report, size);
    report_size = 0;
    pthread_mutex_unlock(&report_lock);
    write_file(tmp, "status.json", text, size);
  }
  return NULL;
}

// The interface's own drawable, and the pass that lays it over the frame.
static void cover(void) {
  glGenTextures(2, interface_texture);
  glGenFramebuffers(2, interface_target);
  for (unsigned i = 0; i < 2; i++) {
    glBindTexture(GL_TEXTURE_2D, interface_texture[i]);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, WIDTH, HEIGHT, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    glBindFramebuffer(GL_FRAMEBUFFER, interface_target[i]);
    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, interface_texture[i], 0);
  }
  static const char *const sources[2] = {
    // The interface is landscape: its x runs down the portrait drawable.
    "attribute vec2 aPos; varying vec2 vAt;\n"
    "void main() { gl_Position = vec4(aPos, 0.0, 1.0); vAt = vec2(0.5 - aPos.y * 0.5, 0.5 + aPos.x * 0.5); }\n",
    "precision mediump float; uniform sampler2D uInterface; varying vec2 vAt;\n"
    "void main() { gl_FragColor = texture2D(uInterface, vAt); }\n"};
  overlay = glCreateProgram();
  for (unsigned i = 0; i < 2; i++) {
    GLuint shader = glCreateShader(i ? GL_FRAGMENT_SHADER : GL_VERTEX_SHADER);
    glShaderSource(shader, 1, &sources[i], NULL);
    glCompileShader(shader);
    glAttachShader(overlay, shader);
    glDeleteShader(shader);
  }
  glBindAttribLocation(overlay, 0, "aPos");
  glLinkProgram(overlay);
}

// A word of a control message that is this shell's:
//   touch=X,Y[;X,Y…]  fingers held on the panel, in the interface's pixels; touch=off lifts them
//   tap=X,Y           one finger down for a few turns
//   ui=start|pause|resume|restart|title   what the interface would ask of the game
//   screen=1          writes the next presented frame to tmp/screen.rgba
//   mark=SECONDS      measures that long, starting in three seconds (`window` in the status)
static bool screen;
static void host_word(const char *word) {
  static const struct {
    const char *verb, *line;
  } asks[] = {{"start", "{\"type\":\"start\"}"},
              {"pause", "{\"type\":\"pause\",\"on\":true}"},
              {"resume", "{\"type\":\"pause\",\"on\":false}"},
              {"restart", "{\"type\":\"restart\"}"},
              {"title", "{\"type\":\"title\"}"}};
  if (!strncmp(word, "touch=", 6) || !strncmp(word, "tap=", 4)) {
    tap = word[1] == 'a' ? 6 : 0;
    fingers.count = 0;
    for (const char *at = strchr(word, '=') + 1; fingers.count < 4 && sscanf(at, "%f,%f", &fingers.at[fingers.count][0], &fingers.at[fingers.count][1]) == 2;) {
      fingers.count++;
      at = strchr(at, ';');
      if (!at++)
        break;
    }
  } else if (!strncmp(word, "ui=", 3)) {
    for (unsigned i = 0; i < sizeof asks / sizeof *asks; i++)
      if (!strcmp(word + 3, asks[i].verb))
        svcwire_send_line(asks[i].line, strlen(asks[i].line));
  } else if (!strcmp(word, "screen=1"))
    screen = true;
  else if (!strncmp(word, "mark=", 5)) {
    memset(&measured, 0, sizeof measured);
    measured.from = now() + 3;
    measured.until = measured.from + atof(word + 5);
  }
}

static void *render(void *unused) {
  (void)unused;
  send_id(cls("EAGLContext"), "setCurrentContext:", context);
  pthread_t writer;
  pthread_create(&writer, NULL, reporter, NULL);
  cover();
  // The interface is up before the pack is read, so the load shows through it.
  mh_stage(MH_STAGE_LOADING, "Reading the world", 17);
  size_t script_size, pak_size, prefs_size;
  char rate[40];
  snprintf(rate, sizeof rate, "globalThis.__simHz=%d;", TURN_HZ);
  char *script = read_file(bundle, "maneuver.js", rate, &script_size), *pak = read_file(bundle, "maneuver.pak", "", &pak_size);
  char *prefs = read_file(documents, "interface.json", "", &prefs_size);
  if (prefs)
    mh_prefs_stored(prefs, prefs_size);
  free(prefs);
  guest = script && pak && pocket_runtime_boot(script, script_size, (const uint8_t *)pak, pak_size, WIDTH, HEIGHT) && pocket_runtime_gl_initialize();
  if (!guest)
    snprintf(failure, sizeof failure, "interface: %s", script && pak ? pocket_runtime_error() : "maneuver.js or maneuver.pak is missing");

  char path[1200], words[1024];
  double previous = now(), started = previous, reported = 0;
  unsigned owed = 0; // display refreshes since the guest's last turn
  float average = 16.7f;
  uint64_t drawn_hash = 0;
  snprintf(path, sizeof path, "%s/control.txt", tmp);
  unlink(path);
  for (;;) {
    id pool = send(send(cls("NSAutoreleasePool"), "alloc"), "init");
    pthread_mutex_lock(&lock);
    while (!shared.active) {
      // Background GL kills the process: finish, then wait to be resumed.
      glFinish();
      shared.parked = true;
      pthread_cond_broadcast(&changed);
      pthread_cond_wait(&changed, &lock);
      previous = started = now();
    }
    shared.parked = false;
    pthread_mutex_unlock(&lock);

    // Commands from the host (tools/ipod.ts ctl): a nonce on the first line,
    // then words for this shell and for Game::control. The file is replaced
    // in one step and acknowledged by its nonce in the status.
    FILE *file = fopen(path, "rb");
    if (file) {
      words[fread(words, 1, sizeof words - 1, file)] = 0;
      fclose(file);
      unlink(path);
      char *text = strchr(words, '\n');
      if (text) {
        *text++ = 0;
        snprintf(command, sizeof command, "%s", words);
        text[strcspn(text, "\r\n")] = 0;
        if (stage == RUNNING)
          mh_control(text, strlen(text));
        char *save = NULL;
        for (char *word = strtok_r(text, " ", &save); word; word = strtok_r(NULL, " ", &save))
          host_word(word);
      }
      reported = 0;
    }

    double start = now();
    float dt = fminf((float)(start - started), 0.1f);
    started = start;
    // One simulation tick per display refresh since the last frame: a late frame catches up.
    unsigned ticks = (unsigned)(dt * 60 + 0.5f);
    ticks = ticks < 1 ? 1 : ticks > 3 ? 3 : ticks;
    unsigned slot = frames % WINDOW;

    // The pack loads once the interface has had two frames to say so.
    if (stage == LOADING && frames >= 2) {
      snprintf(path, sizeof path, "%s/world.pack", bundle);
      if (render_load(path, failure, sizeof failure)) {
        stage = RUNNING;
        sound = sound_start();
      } else {
        stage = FAILED;
        mh_stage(MH_STAGE_ERROR, failure, strlen(failure));
      }
      snprintf(path, sizeof path, "%s/control.txt", tmp);
      start = started = now();
    }

    // The game: what the interface asked for since the last frame, then the ticks.
    timing[SIM][slot] = 0;
    if (stage == RUNNING) {
      const MhPad nobody = {0};
      mh_step(&nobody, ticks);
      if (sound)
        sound_fill();
      static char keep[4096];
      uint32_t n = mh_prefs_take(keep, sizeof keep);
      if (n)
        write_file(documents, "interface.json", keep, n);
      timing[SIM][slot] = (float)(now() - start) * 1000;
    }

    // The interface's turn: what the fingers are doing goes in, the game's
    // state is read and commands are left for the next mh_step. It is redrawn
    // into its texture when what it shows has changed.
    double turning = now();
    timing[GUEST][slot] = timing[REDRAW][slot] = 0;
    owed += ticks;
    if (guest && owed >= 60 / TURN_HZ) {
      unsigned elapsed = owed > 3 ? 3 : owed;
      owed = 0;
      PocketRuntimeContactsInput input;
      pthread_mutex_lock(&lock);
      if (tap && !--tap)
        fingers.count = 0;
      for (int i = 0; i < 4; i++) {
        if (i < fingers.count)
          pocket_contact_event(&shared.touches, i < fingered.count ? POCKET_TOUCH_MOVE : POCKET_TOUCH_DOWN, -1 - i, fingers.at[i][0], fingers.at[i][1], WIDTH, HEIGHT);
        else if (i < fingered.count)
          pocket_contact_event(&shared.touches, POCKET_TOUCH_UP, -1 - i, fingered.at[i][0], fingered.at[i][1], WIDTH, HEIGHT);
      }
      fingered = fingers;
      pocket_contacts_sample(&shared.touches, &input, WIDTH, HEIGHT, WIDTH, HEIGHT, pocket_runtime_hit_test_bounds);
      pthread_mutex_unlock(&lock);
      input.buttons = 0;
      if (!pocket_runtime_frame_contacts(&input, elapsed)) {
        // The guest threw: the game goes on without it, and the status says why.
        guest = false;
        snprintf(failure, sizeof failure, "interface: %s", pocket_runtime_error());
        svcwire_shutdown();
      }
      double turned = now();
      // The texture follows what the interface shows on every other frame at
      // most: laying it out and drawing it costs about 6 ms here, and a
      // frame that pays that twice in a row misses its refresh.
      uint64_t hash = guest && (frames & 1) ? ui_draw_hash() : drawn_hash;
      if (hash != drawn_hash) {
        drawn_hash = hash;
        redraws++;
        interface_shown ^= 1;
        glBindFramebuffer(GL_FRAMEBUFFER, interface_target[interface_shown]);
        glViewport(0, 0, WIDTH, HEIGHT);
        glDisable(GL_SCISSOR_TEST);
        glClearColor(0, 0, 0, 0);
        glClear(GL_COLOR_BUFFER_BIT);
        ui_gl_render_over(0, 0, WIDTH, HEIGHT, WIDTH, HEIGHT);
      }
      timing[GUEST][slot] = (float)(turned - turning) * 1000;
      timing[REDRAW][slot] = (float)(now() - turned) * 1000;
    }

    double drawing = now();
    glBindFramebuffer(GL_FRAMEBUFFER, framebuffer);
    if (stage == RUNNING) {
      MhView seen;
      mh_view(&seen);
      render_frame(&seen, ticks, &perf);
    } else {
      glViewport(0, 0, HEIGHT, WIDTH);
      glDisable(GL_SCISSOR_TEST);
      glClearColor(0.04f, 0.05f, 0.07f, 1);
      glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
    if (guest) {
      static const float corners[8] = {-1, -1, 1, -1, -1, 1, 1, 1};
      glDisable(GL_DEPTH_TEST);
      glDisable(GL_CULL_FACE);
      glDepthMask(GL_FALSE);
      glEnable(GL_BLEND);
      glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA); // the interface's texture holds premultiplied colour
      glBindBuffer(GL_ARRAY_BUFFER, 0);
      glEnableVertexAttribArray(0);
      glDisableVertexAttribArray(1);
      glDisableVertexAttribArray(2);
      glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, corners);
      glActiveTexture(GL_TEXTURE0);
      glBindTexture(GL_TEXTURE_2D, interface_texture[interface_shown]);
      glUseProgram(overlay);
      glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
      glDepthMask(GL_TRUE);
      glDisable(GL_BLEND);
    }
    double drawn = now();
    timing[SCENE][slot] = (float)(drawn - drawing) * 1000;
    if (screen) {
      // The frame as presented: rows of the portrait drawable from its bottom; the host turns them.
      static uint8_t pixels[WIDTH * HEIGHT * 4];
      glReadPixels(0, 0, HEIGHT, WIDTH, GL_RGBA, GL_UNSIGNED_BYTE, pixels);
      write_file(tmp, "screen.rgba", pixels, sizeof pixels);
      snprintf(captured, sizeof captured, "%s", command);
      screen = false;
      reported = 0;
      started = start = now(); // a readback is not a frame's work
    }
    const GLenum depth = GL_DEPTH_ATTACHMENT;
    glDiscardFramebufferEXT(GL_FRAMEBUFFER, 1, &depth);
    glBindRenderbuffer(GL_RENDERBUFFER, colorbuffer);
    double submitted = now();
    timing[KICK][slot] = (float)(submitted - drawn) * 1000;
    ((BOOL(*)(id, SEL, unsigned))objc_msgSend)(context, sel("presentRenderbuffer:"), GL_RENDERBUFFER);
    double presented = now();
    float interval = (float)(presented - previous) * 1000;
    timing[WORK][slot] = (float)(submitted - start) * 1000;
    timing[PRESENT][slot] = (float)(presented - submitted) * 1000;
    timing[INTERVAL][slot] = interval;
    previous = presented;
    frames++;
    if (stage == RUNNING) {
      // A frame that took more than a refresh and a half is late.
      late += interval > 25;
      average += (interval - average) * 0.05f;
      float worst = 0;
      for (unsigned i = 0; i < (frames < WINDOW ? frames : WINDOW); i++)
        worst = fmaxf(worst, timing[INTERVAL][i]);
      perf = (MhPerf){.frame = average, .worst = worst, .late = late, .frames = frames, .sim = timing[SIM][slot], .build = timing[SCENE][slot],
                      .draw = timing[GUEST][slot] + timing[REDRAW][slot], .gpu = 0, .draws = render_stats.draws, .tris = render_stats.tris};
    }
    if (measured.until && !measured.done && presented >= measured.from) {
      measured.frames++;
      measured.late += interval > 25;
      measured.worst = fmaxf(measured.worst, interval);
      if (render_stats.tris > measured.max_tris)
        measured.max_tris = render_stats.tris;
      if (render_stats.draws > measured.max_draws)
        measured.max_draws = render_stats.draws;
      for (unsigned i = 0; i < TIMINGS; i++)
        measured.sums[i] += timing[i][slot];
      if (presented >= measured.until) {
        measured.done = true;
        measured.seconds = presented - measured.from;
        reported = 0;
      }
    }
    if (presented - reported > 0.5) {
      reported = presented;
      status();
    }
    send(pool, "drain");
  }
  return NULL;
}

// ---- main thread

// Every finger goes to the interface, in its landscape pixels.
static void touched(id self, SEL _cmd, id touches, id event) {
  (void)_cmd, (void)event;
  id all = send(touches, "allObjects");
  unsigned count = ((unsigned (*)(id, SEL))objc_msgSend)(all, sel("count"));
  pthread_mutex_lock(&lock);
  for (unsigned i = 0; i < count; i++) {
    id touch = ((id(*)(id, SEL, unsigned))objc_msgSend)(all, sel("objectAtIndex:"), i);
    Spot at = ((Spot(*)(id, SEL, id))objc_msgSend_stret)(touch, sel("locationInView:"), self);
    int phase = ((int (*)(id, SEL))objc_msgSend)(touch, sel("phase")); // began, moved, stationary, ended, cancelled
    pocket_contact_event(&shared.touches, phase == 0 ? POCKET_TOUCH_DOWN : phase == 3 ? POCKET_TOUCH_UP : phase == 4 ? POCKET_TOUCH_CANCEL : POCKET_TOUCH_MOVE,
                         (int)((uintptr_t)touch >> 4 & 0x3fffffff), at.y, HEIGHT - at.x, WIDTH, HEIGHT);
  }
  pthread_mutex_unlock(&lock);
}
static Class layer_class(id self, SEL _cmd) {
  (void)self, (void)_cmd;
  return objc_getClass("CAEAGLLayer");
}
static void active(id self, SEL _cmd, id application) {
  (void)self, (void)application;
  pthread_mutex_lock(&lock);
  shared.active = _cmd == sel("applicationDidBecomeActive:");
  pocket_contacts_cancel(&shared.touches);
  pthread_cond_broadcast(&changed);
  while (!shared.active && !shared.parked)
    pthread_cond_wait(&changed, &lock);
  pthread_mutex_unlock(&lock);
}
static BOOL launched(id self, SEL _cmd, id application, id options) {
  (void)self, (void)_cmd, (void)options;
  snprintf(bundle, sizeof bundle, "%s", ((const char *(*)(id, SEL))objc_msgSend)(send(send(cls("NSBundle"), "mainBundle"), "bundlePath"), sel("UTF8String")));
  snprintf(tmp, sizeof tmp, "%s/tmp", getenv("HOME"));
  snprintf(documents, sizeof documents, "%s/Documents", getenv("HOME"));

  // PocketJS's link stubs carry no UIKit version, and UIKit gives an app that
  // old one pixel per point: the layer is 320 by 480 pixels.
  id window = ((id(*)(id, SEL, Frame))objc_msgSend)(send(cls("UIWindow"), "alloc"), sel("initWithFrame:"), (Frame){0, 0, HEIGHT, WIDTH});
  view = ((id(*)(id, SEL, Frame))objc_msgSend)(send(cls("ManeuverView"), "alloc"), sel("initWithFrame:"), (Frame){0, 0, HEIGHT, WIDTH});
  send_id(window, "addSubview:", view);
  send_int(view, "setMultipleTouchEnabled:", 1);
  send_int(send(view, "layer"), "setOpaque:", 1);
  context = ((id(*)(id, SEL, int))objc_msgSend)(send(cls("EAGLContext"), "alloc"), sel("initWithAPI:"), 2);
  send_id(cls("EAGLContext"), "setCurrentContext:", context);
  GLuint depth;
  glGenFramebuffers(1, &framebuffer);
  glBindFramebuffer(GL_FRAMEBUFFER, framebuffer);
  glGenRenderbuffers(1, &colorbuffer);
  glBindRenderbuffer(GL_RENDERBUFFER, colorbuffer);
  ((BOOL(*)(id, SEL, unsigned, id))objc_msgSend)(context, sel("renderbufferStorage:fromDrawable:"), GL_RENDERBUFFER, send(view, "layer"));
  glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, colorbuffer);
  glGenRenderbuffers(1, &depth);
  glBindRenderbuffer(GL_RENDERBUFFER, depth);
  glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT24, HEIGHT, WIDTH); // OES_depth24
  glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth);
  if (!context || glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
    snprintf(failure, sizeof failure, "OpenGL ES 2 is not available");
  send_id(cls("EAGLContext"), "setCurrentContext:", NULL);
  send(window, "makeKeyAndVisible");
  send_int(application, "setIdleTimerDisabled:", 1);
  // The guest's parser recurses: give its thread the main thread's megabyte.
  pthread_t thread;
  pthread_attr_t attributes;
  pthread_attr_init(&attributes);
  pthread_attr_setstacksize(&attributes, 1 << 20);
  pthread_create(&thread, &attributes, render, NULL);
  return 1;
}
int main(int argc, char **argv) {
  send(send(cls("NSAutoreleasePool"), "alloc"), "init");
  Class surface = objc_allocateClassPair(objc_getClass("UIView"), "ManeuverView", 0);
  class_addMethod(object_getClass((id)surface), sel("layerClass"), (IMP)layer_class, "#@:");
  static const char *const touches[] = {"touchesBegan:withEvent:", "touchesMoved:withEvent:", "touchesEnded:withEvent:",
                                        "touchesCancelled:withEvent:"};
  for (unsigned i = 0; i < 4; i++)
    class_addMethod(surface, sel(touches[i]), (IMP)touched, "v@:@@");
  objc_registerClassPair(surface);
  Class delegate = objc_allocateClassPair(objc_getClass("NSObject"), "ManeuverDelegate", 0);
  class_addMethod(delegate, sel("application:didFinishLaunchingWithOptions:"), (IMP)launched, "c@:@@");
  class_addMethod(delegate, sel("applicationWillResignActive:"), (IMP)active, "v@:@");
  class_addMethod(delegate, sel("applicationDidBecomeActive:"), (IMP)active, "v@:@");
  objc_registerClassPair(delegate);
  return UIApplicationMain(argc, argv, NULL, string("ManeuverDelegate"));
}
