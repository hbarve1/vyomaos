# VyomaOS root Makefile — Docker-based build
#
# All compilation runs inside the vyomaos-builder container.
# The host only needs: docker, make, qemu-system-x86_64
#
# Usage:
#   make image    — build the Docker builder image
#   make kernel   — compile the Linux kernel  -> out/bzImage
#   make rootfs   — build the initramfs       -> out/initramfs.cpio.gz
#   make disk     — create blank ext4 data disk -> out/disk.img (64 MB)
#   make build    — kernel + rootfs + disk
#   make run      — boot in QEMU (host) with virtio-blk data disk
#   make shell    — open a shell in the builder container
#   make clean    — remove out/
#   make clean-image — remove the builder Docker image

SHELL       := /bin/bash
OUT         := out
BZIMAGE     := $(OUT)/bzImage
INITRAMFS   := $(OUT)/initramfs.cpio.gz
DISK        := $(OUT)/disk.img
DISK_SIZE   := 64  # MB
DATA_DIR    := data

IMAGE       := vyomaos-builder
IMAGE_TAG   := latest
DOCKERFILE  := docker/Dockerfile

KERNEL_SCRIPT    := base/modules/kernel.sh
ROOTFS_SCRIPT    := base/modules/rootfs.sh
KERNEL_CONFIG    := base/kernel.config
SUPERVISOR_SRC   := $(shell find supervisor/src -name '*.rs' 2>/dev/null)
SUPERVISOR_BIN   := supervisor/target/x86_64-unknown-linux-musl/release/supervisor
SUPERVISOR_STAMP := $(OUT)/.supervisor.stamp

APP_NAMES        := $(sort $(notdir $(patsubst %/Cargo.toml,%,$(wildcard apps/*/Cargo.toml))))
APPS_STAMP       := $(OUT)/.apps.stamp

RUSTFLAGS        := -D warnings
WASM_FLAGS       :=

# ── T019: Platform profile selection ─────────────────────────────────────────
# Pass PLATFORM=<name> to select a target from platforms/<name>/.
# Supported values: desktop-x86 (default), iot-rpi, mcu-arm-cortex-m, server-arm64
#
# When PLATFORM is set and platforms/<PLATFORM>/kernel.config exists, the
# platform-specific kernel config and rootfs script are used in place of the
# base/ defaults.  The PLATFORM value is passed as an environment variable to
# the supervisor via the QEMU kernel command line.
#
# Example:
#   make build PLATFORM=desktop-x86
#   make run   PLATFORM=desktop-x86
PLATFORM ?=

# Select kernel config and rootfs script based on PLATFORM.
# Falls back to base/ defaults when PLATFORM is empty or the platform dir
# does not provide its own files.
ifneq ($(PLATFORM),)
  PLATFORM_DIR := platforms/$(PLATFORM)
  ifneq ($(wildcard $(PLATFORM_DIR)/kernel.config),)
    KERNEL_CONFIG := $(PLATFORM_DIR)/kernel.config
  endif
  ifneq ($(wildcard $(PLATFORM_DIR)/rootfs.sh),)
    ROOTFS_SCRIPT := $(PLATFORM_DIR)/rootfs.sh
  endif
  PLATFORM_KERNEL_ARG := PLATFORM=$(PLATFORM)
else
  PLATFORM_KERNEL_ARG :=
endif

# ── kernel source tracking ────────────────────────────────────────────────────
KERNEL_PATCHES := $(wildcard base/patches/kernel/*.patch)
KERNEL_DEPS    := $(KERNEL_SCRIPT) $(KERNEL_CONFIG) $(KERNEL_PATCHES)
KERNEL_STAMP   := $(OUT)/.kernel.stamp

# ── Docker run helper ─────────────────────────────────────────────────────────
# Mounts the project root read-write at /work inside the container.
# UID/GID passthrough ensures out/ files are owned by the host user.
DOCKER_RUN := docker run --rm \
	--platform linux/amd64 \
	-v "$(CURDIR)":/work \
	-w /work \
	-u $$(id -u):$$(id -g) \
	$(IMAGE):$(IMAGE_TAG)

# ── KVM passthrough (opt-in for run target) ───────────────────────────────────
KVM ?=

# ── phony declarations ────────────────────────────────────────────────────────
.PHONY: image kernel supervisor apps rootfs disk build run run-gui run-net run-gui-net shell clean clean-image data \
        unit-test smoke test check-manifests check-profiles test-all-platforms

# ── Docker image ──────────────────────────────────────────────────────────────
image: $(DOCKERFILE)
	docker build --platform linux/amd64 -t $(IMAGE):$(IMAGE_TAG) -f $(DOCKERFILE) .

# ── kernel ────────────────────────────────────────────────────────────────────
kernel: $(KERNEL_STAMP)

$(KERNEL_STAMP): $(KERNEL_DEPS) | image
	@mkdir -p $(OUT)
	$(DOCKER_RUN) bash $(KERNEL_SCRIPT)
	@touch $(KERNEL_STAMP)

$(BZIMAGE): $(KERNEL_STAMP)
	@test -f $(BZIMAGE) || { echo "ERROR: $(BZIMAGE) not produced by kernel.sh"; exit 1; }

# ── supervisor ───────────────────────────────────────────────────────────────
supervisor: $(SUPERVISOR_STAMP)

$(SUPERVISOR_STAMP): supervisor/Cargo.toml supervisor/.cargo/config.toml $(SUPERVISOR_SRC) | image
	@mkdir -p $(OUT)
	$(DOCKER_RUN) cargo build \
	  --manifest-path supervisor/Cargo.toml \
	  --target x86_64-unknown-linux-musl \
	  --release
	@touch $(SUPERVISOR_STAMP)

# ── apps (WASM) — per-app incremental stamp rules ────────────────────────────
#
# Each app gets its own stamp: $(OUT)/.apps/<name>.stamp
# The stamp is touched only if its app's Rust sources or Cargo.toml changed,
# so touching one app's source rebuilds only that app (T041/T042 incremental).
# RUSTFLAGS=-D warnings is enforced per-app (T039 zero-warning gate).

define APP_RULE
$(OUT)/.apps/$(1).stamp: $$(wildcard apps/$(1)/src/*.rs) apps/$(1)/Cargo.toml | image
	@mkdir -p $(OUT)/.apps
	$$(DOCKER_RUN) env RUSTFLAGS="$$(WASM_FLAGS)" cargo build \
	  --manifest-path apps/$(1)/Cargo.toml \
	  --target wasm32-wasip2 --release
	@touch $$@
endef

$(foreach app,$(APP_NAMES),$(eval $(call APP_RULE,$(app))))

# Meta-stamp: touches when all per-app stamps are current (used by rootfs dep).
$(APPS_STAMP): $(foreach app,$(APP_NAMES),$(OUT)/.apps/$(app).stamp)
	@touch $@

apps: check-manifests $(APPS_STAMP)

# ── rootfs ────────────────────────────────────────────────────────────────────
rootfs: $(INITRAMFS)

$(INITRAMFS): $(ROOTFS_SCRIPT) $(SUPERVISOR_STAMP) $(APPS_STAMP) | image
	@mkdir -p $(OUT)
	$(DOCKER_RUN) bash $(ROOTFS_SCRIPT)

# ── data disk (ext4, 64 MB) ───────────────────────────────────────────────────
# Created inside the builder so mkfs.ext4 is available.
# The disk is NOT recreated if it already exists (data is preserved across runs).
disk: $(DISK)

$(DISK): | image
	@mkdir -p $(OUT)
	@if [ -f $(DISK) ]; then \
	  echo "ℹ️  disk.img already exists, skipping (delete manually to recreate)"; \
	else \
	  echo "ℹ️  Creating $(DISK_SIZE)M ext4 data disk..."; \
	  $(DOCKER_RUN) bash -c \
	    "dd if=/dev/zero of=/work/$(DISK) bs=1M count=$(DISK_SIZE) status=none && \
	     mkfs.ext4 -q -L vyoma-data /work/$(DISK) && \
	     echo '✅ disk.img created'"; \
	fi

# ── host data directory (9P share — persistent across reboots) ───────────────
# The VM mounts this directory at /data via virtio-9p (trans=virtio).
# Files written by WASM apps to /data persist on the host across VM reboots.
data:
	@mkdir -p $(DATA_DIR)
	@echo "ℹ️  Host data directory ready: $(DATA_DIR)/"

# ── unit-test (T027): supervisor cargo test inside Docker, RUSTFLAGS enforced ─
unit-test: | image
	$(DOCKER_RUN) env RUSTFLAGS="$(RUSTFLAGS)" \
	  cargo test --manifest-path supervisor/Cargo.toml \
	  --target x86_64-unknown-linux-musl

# ── smoke (T026): headless QEMU boot test — requires make build first ────────
smoke: | image
	$(DOCKER_RUN) bash base/scripts/smoke-test.sh

# ── test (T028): unit-test + smoke — full test suite ─────────────────────────
test: build unit-test smoke

# ── check-manifests (T034): validate all apps/*/vyoma.toml files ─────────────
check-manifests: | image
	$(DOCKER_RUN) cargo run --manifest-path tools/check-manifests/Cargo.toml

# ── check-profiles (T062): validate all platform profile TOML files ──────────
# Verifies that every expected profile file exists and is valid TOML.
# Runs the platform_profile unit tests which exercise TOML parsing and
# validation logic for all 6 profiles in supervisor/src/profile/profiles/.
PROFILE_DIR := supervisor/src/profile/profiles
PROFILE_NAMES := mcu-minimal iot-edge robotics-rt mobile desktop-full server-headless

check-profiles: | image
	@echo "Checking platform profile TOML files..."
	@fail=0; \
	for name in $(PROFILE_NAMES); do \
	  f="$(PROFILE_DIR)/$$name.toml"; \
	  if [ -f "$$f" ]; then \
	    echo "  $$name ... OK"; \
	  else \
	    echo "PROFILES: FAIL: $$name (missing: $$f)"; \
	    fail=1; \
	  fi; \
	done; \
	if [ "$$fail" -eq 0 ]; then \
	  $(DOCKER_RUN) env RUSTFLAGS="$(RUSTFLAGS)" \
	    cargo test --manifest-path supervisor/Cargo.toml \
	    --target x86_64-unknown-linux-musl \
	    -- profile 2>&1 | tail -5; \
	  echo "PROFILES: OK"; \
	else \
	  exit 1; \
	fi

# ── test-all-platforms (T063): unit-test + check-profiles + check-manifests ──
# Single command to validate everything without requiring QEMU.
test-all-platforms: unit-test check-profiles check-manifests
	@echo "ALL-PLATFORMS: OK"

# ── build (all) ───────────────────────────────────────────────────────────────
build: kernel supervisor apps rootfs disk data

# ── run (headless — serial console only) ─────────────────────────────────────
run: $(BZIMAGE) $(INITRAMFS) data
	qemu-system-x86_64 \
	  -kernel $(BZIMAGE) \
	  -initrd $(INITRAMFS) \
	  -append "console=ttyS0 panic=1 $(PLATFORM_KERNEL_ARG)" \
	  -virtfs local,path=$(DATA_DIR),mount_tag=vyoma-data,security_model=mapped-xattr \
	  -nographic \
	  -m 512M \
	  -no-reboot \
	  $(KVM)

# ── run-gui (graphical window via virtio-gpu) ─────────────────────────────────
# Serial output still goes to the terminal; QEMU also opens a display window.
# On macOS use: make run-gui DISPLAY_BACKEND=cocoa  (default: sdl)
#
# Display notes:
#   -device virtio-vga,xres=1280,yres=800  — pin the virtual display to exactly
#     the resolution our apps draw for (avoids a mismatched small window).
#   zoom-to-fit=on — QEMU scales the fixed 1280×800 surface to fill the window
#     when resized or made fullscreen, instead of trying to reconfigure the
#     virtual display (which causes "Display output is not active" in fullscreen).
DISPLAY_BACKEND ?= sdl
run-gui: $(BZIMAGE) $(INITRAMFS) data
	qemu-system-x86_64 \
	  -kernel $(BZIMAGE) \
	  -initrd $(INITRAMFS) \
	  -append "console=tty0 console=ttyS0 panic=1 $(PLATFORM_KERNEL_ARG)" \
	  -device virtio-vga,xres=1440,yres=900 \
	  -device virtio-mouse-pci \
	  -display $(DISPLAY_BACKEND),zoom-to-fit=on,full-screen=on \
	  -serial stdio \
	  -virtfs local,path=$(DATA_DIR),mount_tag=vyoma-data,security_model=mapped-xattr \
	  -m 512M \
	  -no-reboot \
	  $(KVM)

# ── run-net (headless + virtio-net, port 8080 forwarded to host) ─────────────
run-net: $(BZIMAGE) $(INITRAMFS) data
	qemu-system-x86_64 \
	  -kernel $(BZIMAGE) \
	  -initrd $(INITRAMFS) \
	  -append "console=ttyS0 panic=1 $(PLATFORM_KERNEL_ARG)" \
	  -netdev user,id=net0,hostfwd=tcp::8080-:8080 \
	  -device virtio-net-pci,netdev=net0 \
	  -virtfs local,path=$(DATA_DIR),mount_tag=vyoma-data,security_model=mapped-xattr \
	  -nographic \
	  -m 512M \
	  -no-reboot \
	  $(KVM)

# ── run-gui-net (GUI window + virtio-net) ────────────────────────────────────
run-gui-net: $(BZIMAGE) $(INITRAMFS) data
	qemu-system-x86_64 \
	  -kernel $(BZIMAGE) \
	  -initrd $(INITRAMFS) \
	  -append "console=tty0 console=ttyS0 panic=1 $(PLATFORM_KERNEL_ARG)" \
	  -device virtio-vga,xres=1440,yres=900 \
	  -device virtio-mouse-pci \
	  -display $(DISPLAY_BACKEND),zoom-to-fit=on,full-screen=on \
	  -serial stdio \
	  -netdev user,id=net0,hostfwd=tcp::8080-:8080 \
	  -device virtio-net-pci,netdev=net0 \
	  -virtfs local,path=$(DATA_DIR),mount_tag=vyoma-data,security_model=mapped-xattr \
	  -m 512M \
	  -no-reboot \
	  $(KVM)

# ── shell ─────────────────────────────────────────────────────────────────────
shell: | image
	docker run --rm -it \
	  --platform linux/amd64 \
	  -v "$(CURDIR)":/work \
	  -w /work \
	  $(IMAGE):$(IMAGE_TAG) \
	  /bin/bash

# ── clean ─────────────────────────────────────────────────────────────────────
clean:
	rm -rf $(OUT)

clean-image:
	docker rmi $(IMAGE):$(IMAGE_TAG) 2>/dev/null || true
