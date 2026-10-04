# Pocket Maneuver for Nintendo 3DS: the C host and renderer, the Rust core
# (core/, built before this runs), PocketJS's dev transport and the pieces the
# interface (ui/) runs on, into a .3dsx with the world pack and the interface's
# bundle in its ROMFS. tools/n3ds.ts runs this inside the pinned devkitARM
# container.
DEVKITPRO ?= /opt/devkitpro
DEVKITARM ?= $(DEVKITPRO)/devkitARM
CC := $(DEVKITARM)/bin/arm-none-eabi-gcc
ROOT := /maneuver
BUILD := $(ROOT)/.pocket-build/3ds/build
SOURCE := $(ROOT)/n3ds/src
POCKET := $(ROOT)/vendor/pocketjs/hosts/3ds
PICA_KERNEL := $(ROOT)/vendor/pocketjs/devices/3ds/pocket-3ds-pica/include
CORE := $(ROOT)/n3ds/core/target/armv6k-nintendo-3ds/release/libmaneuver_n3ds_core.a
# The interface is a PocketJS guest: PocketJS's UI core (a Rust static library,
# built on the host like core/), QuickJS (built in this container by
# tools/n3ds.ts), the guest driver (qjs.c), the PICA DrawList backend (gfx.c)
# and its input mapping (input.c). Each Rust library keeps its allocator and
# panic handler to itself (both are built with LTO), so the two link side by
# side. JS_NO_NAN_BOXING must match the QuickJS archive on every unit that sees
# quickjs.h.
UI_CORE := $(ROOT)/.pocket-build/3ds/ui-core/armv6k-nintendo-3ds/release/libpocketjs_3ds_core.a
QUICKJS := $(ROOT)/.pocket-build/3ds/quickjs
ICON := $(SOURCE)/../icon.png
OUT := $(ROOT)/dist/3ds/pocket-maneuver.3dsx
ROMFS := $(ROOT)/.pocket-build/3ds/romfs
ARCH := -march=armv6k -mtune=mpcore -mfloat-abi=hard -mtp=soft
CFLAGS := -O3 -g -Wall -Wextra -std=gnu11 -ffunction-sections -fdata-sections -mword-relocations $(ARCH) \
  -D__3DS__ -DJS_NO_NAN_BOXING -DPOCKETJS_RUNTIME_SLOT='"pocket-maneuver"' \
  -include $(BUILD)/config.h -I$(SOURCE) -I$(BUILD) -I$(POCKET)/src -I$(POCKET)/include -I$(PICA_KERNEL) -I$(QUICKJS) -I$(DEVKITPRO)/libctru/include -I$(DEVKITPRO)/portlibs/3ds/include
LDFLAGS := -specs=3dsx.specs $(ARCH) -Wl,--gc-sections -Wl,-Map,$(BUILD)/maneuver.map
SHADERS := world color skin hud
OWN := main.o render.o guest.o
WIRE := devserver.o dev_protocol.o native.o hbldr.o soc.o
GUEST := gfx.o qjs.o input.o
OBJECTS := $(addprefix $(BUILD)/,$(OWN) $(addsuffix _shbin.o,$(SHADERS)) $(WIRE) $(GUEST) vshader_shbin.o)
.PHONY: all
all: $(OUT)
$(BUILD):
	mkdir -p $@
$(BUILD)/%.shbin: $(SOURCE)/%.v.pica | $(BUILD)
	$(DEVKITPRO)/tools/bin/picasso -o $@ $<
$(BUILD)/%_shbin.s $(BUILD)/%_shbin.h &: $(BUILD)/%.shbin
	cd $(BUILD) && $(DEVKITPRO)/tools/bin/bin2s -a 4 -H $*_shbin.h $*.shbin > $*_shbin.s
$(BUILD)/%_shbin.o: $(BUILD)/%_shbin.s
	$(CC) $(ARCH) -c $< -o $@
$(addprefix $(BUILD)/,$(OWN)): $(BUILD)/%.o: $(SOURCE)/%.c $(wildcard $(SOURCE)/*.h) $(BUILD)/config.h $(addprefix $(BUILD)/,$(addsuffix _shbin.h,$(SHADERS))) | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@
$(addprefix $(BUILD)/,$(WIRE) input.o): $(BUILD)/%.o: $(POCKET)/src/%.c | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@
# The DrawList backend's vertex shader, from PocketJS.
$(BUILD)/vshader.shbin: $(POCKET)/src/vshader.v.pica | $(BUILD)
	$(DEVKITPRO)/tools/bin/picasso -o $@ $<
$(BUILD)/gfx.o: $(POCKET)/src/gfx.c $(BUILD)/vshader_shbin.h | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@
# The guest's DevTools would take the control lines meant for this program off
# the dev wire: the guest driver asks guest.c whether a debugger is attached.
$(BUILD)/qjs.o: $(POCKET)/src/qjs.c | $(BUILD)
	$(CC) $(CFLAGS) -Ddevserver_active=guest_devtools -c $< -o $@
$(BUILD)/maneuver.elf: $(OBJECTS) $(CORE) $(UI_CORE) $(QUICKJS)/libquickjs.a
	$(CC) $(LDFLAGS) $(OBJECTS) $(CORE) $(UI_CORE) $(QUICKJS)/libquickjs.a -L$(DEVKITPRO)/libctru/lib -L$(DEVKITPRO)/portlibs/3ds/lib -lcitro3d -lmbedcrypto -lctru -lm -o $@
$(BUILD)/maneuver.smdh: $(ICON) | $(BUILD)
	$(DEVKITPRO)/tools/bin/smdhtool --create 'Pocket Maneuver' 'Wire gear over a walled town' 'Pocket Nexus' $< $@
$(OUT): $(BUILD)/maneuver.elf $(BUILD)/maneuver.smdh $(ROMFS)/world.pack $(ROMFS)/maneuver.js $(ROMFS)/maneuver.pak
	mkdir -p $(dir $@)
	$(DEVKITPRO)/tools/bin/3dsxtool $< $@ --smdh=$(BUILD)/maneuver.smdh --romfs=$(ROMFS)

.SECONDARY:
