# Pocket Maneuver for Nintendo 3DS: the C host and renderer, the Rust core
# (core/, built before this runs) and PocketJS's dev transport, into a .3dsx
# with the world pack in its ROMFS. tools/n3ds.ts runs this inside the pinned
# devkitARM container.
DEVKITPRO ?= /opt/devkitpro
DEVKITARM ?= $(DEVKITPRO)/devkitARM
CC := $(DEVKITARM)/bin/arm-none-eabi-gcc
ROOT := /maneuver
BUILD := $(ROOT)/.pocket-build/3ds/build
SOURCE := $(ROOT)/n3ds/src
POCKET := $(ROOT)/vendor/pocketjs/hosts/3ds
TITLE := $(ROOT)/vendor/pocketjs/engine/pocket3d/crates/pocket3d-title
CORE := $(ROOT)/n3ds/core/target/armv6k-nintendo-3ds/release/libmaneuver_n3ds_core.a
ICON := $(SOURCE)/../icon.png
OUT := $(ROOT)/dist/3ds/pocket-maneuver.3dsx
ROMFS := $(ROOT)/.pocket-build/3ds/romfs
ARCH := -march=armv6k -mtune=mpcore -mfloat-abi=hard -mtp=soft
CFLAGS := -O3 -g -Wall -Wextra -std=gnu11 -ffunction-sections -fdata-sections -mword-relocations $(ARCH) \
  -D__3DS__ -DPOCKETJS_RUNTIME_SLOT='"pocket-maneuver"' \
  -include $(BUILD)/config.h -I$(SOURCE) -I$(BUILD) -I$(POCKET)/src -I$(POCKET)/include -I$(TITLE)/include -I$(DEVKITPRO)/libctru/include -I$(DEVKITPRO)/portlibs/3ds/include
LDFLAGS := -specs=3dsx.specs $(ARCH) -Wl,--gc-sections -Wl,-Map,$(BUILD)/maneuver.map
SHADERS := world color skin hud
OWN := main.o render.o
WIRE := devserver.o dev_protocol.o native.o hbldr.o soc.o
OBJECTS := $(addprefix $(BUILD)/,$(OWN) $(addsuffix _shbin.o,$(SHADERS)) $(WIRE))
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
$(addprefix $(BUILD)/,$(WIRE)): $(BUILD)/%.o: $(POCKET)/src/%.c | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@
$(BUILD)/maneuver.elf: $(OBJECTS) $(CORE)
	$(CC) $(LDFLAGS) $(OBJECTS) $(CORE) -L$(DEVKITPRO)/libctru/lib -L$(DEVKITPRO)/portlibs/3ds/lib -lcitro3d -lmbedcrypto -lctru -lm -o $@
$(BUILD)/maneuver.smdh: $(ICON) | $(BUILD)
	$(DEVKITPRO)/tools/bin/smdhtool --create 'Pocket Maneuver' 'Wire gear over a walled town' 'Pocket Nexus' $< $@
$(OUT): $(BUILD)/maneuver.elf $(BUILD)/maneuver.smdh $(ROMFS)/world.pack
	mkdir -p $(dir $@)
	$(DEVKITPRO)/tools/bin/3dsxtool $< $@ --smdh=$(BUILD)/maneuver.smdh --romfs=$(ROMFS)

.SECONDARY:
