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

APPS_SRC         := $(shell find apps -name '*.rs' -o -name 'Cargo.toml' 2>/dev/null)
APPS_STAMP       := $(OUT)/.apps.stamp

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
.PHONY: image kernel supervisor apps rootfs disk build run run-gui run-net run-gui-net shell clean clean-image data

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

# ── apps (WASM) ───────────────────────────────────────────────────────────────
apps: $(APPS_STAMP)

$(APPS_STAMP): $(APPS_SRC) | image
	@mkdir -p $(OUT)
	$(DOCKER_RUN) cargo build --manifest-path apps/hello-world/Cargo.toml    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/calculator/Cargo.toml     --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/factorial/Cargo.toml      --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/ping/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/pong/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/storage-demo/Cargo.toml   --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/gui-demo/Cargo.toml       --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/http-server/Cargo.toml   --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/shell/Cargo.toml        --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/ticker/Cargo.toml         --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/file-manager/Cargo.toml    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/text-editor/Cargo.toml    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/system-monitor/Cargo.toml  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/window-manager/Cargo.toml  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/taskbar/Cargo.toml         --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/app-launcher/Cargo.toml   --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/settings/Cargo.toml       --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/dns-resolver/Cargo.toml   --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/browser/Cargo.toml        --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/ssh-client/Cargo.toml     --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/network-config/Cargo.toml    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/virtual-keyboard/Cargo.toml   --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/color-picker/Cargo.toml       --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/process-inspector/Cargo.toml  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/font-chooser/Cargo.toml       --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/app-store/Cargo.toml          --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/audio-player/Cargo.toml       --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/image-viewer/Cargo.toml       --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/hex-editor/Cargo.toml         --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/calendar/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/markdown-viewer/Cargo.toml    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/password-manager/Cargo.toml  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/task-manager/Cargo.toml      --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/clock/Cargo.toml            --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/weather/Cargo.toml          --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/sci-calculator/Cargo.toml  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/pomodoro/Cargo.toml        --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/log-viewer/Cargo.toml     --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/diff-viewer/Cargo.toml    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/csv-viewer/Cargo.toml     --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/json-viewer/Cargo.toml    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/menu-bar/Cargo.toml       --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/dock/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/spotlight/Cargo.toml      --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/app-switcher/Cargo.toml        --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/notification-center/Cargo.toml --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/mission-control/Cargo.toml     --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/desktop/Cargo.toml             --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/context-menu/Cargo.toml        --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/finder/Cargo.toml                  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/system-preferences/Cargo.toml      --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/activity-monitor/Cargo.toml        --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/quick-look/Cargo.toml              --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/spaces-switcher/Cargo.toml         --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/screen-lock/Cargo.toml             --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/clipboard-history/Cargo.toml       --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/widget-board/Cargo.toml            --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/tmux/Cargo.toml                   --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/font-preview/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/draw-pad/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/world-clock/Cargo.toml            --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/stopwatch/Cargo.toml              --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/unit-converter/Cargo.toml         --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/qr-viewer/Cargo.toml              --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/emoji-picker/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/snake/Cargo.toml                  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/minesweeper/Cargo.toml             --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/fifteen-puzzle/Cargo.toml          --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/breakout/Cargo.toml                --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/memory-game/Cargo.toml             --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/tetris/Cargo.toml                  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/pong/Cargo.toml                    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/space-invaders/Cargo.toml          --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/2048/Cargo.toml                    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/wordle/Cargo.toml                  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/sudoku/Cargo.toml                  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/chess/Cargo.toml                   --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/typing-tutor/Cargo.toml            --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/paint/Cargo.toml                   --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/music-viz/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/flashcard/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/budget/Cargo.toml                  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/habit-tracker/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/recipe/Cargo.toml                  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/expense-split/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/word-counter/Cargo.toml            --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/countdown/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/quiz/Cargo.toml                    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/dice/Cargo.toml                    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/color-gen/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/maze/Cargo.toml                    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/pixel-art/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/simon/Cargo.toml                   --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/hangman/Cargo.toml                 --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/typing-race/Cargo.toml             --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/asteroids/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/math-quiz/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/paint-pro/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/music-player/Cargo.toml            --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/code-editor/Cargo.toml             --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/crypto-ticker/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/photo-filter/Cargo.toml            --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/terminal/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/map-viewer/Cargo.toml             --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/file-diff/Cargo.toml              --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/spreadsheet/Cargo.toml            --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/image-gallery/Cargo.toml          --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/text-adventure/Cargo.toml         --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/music-composer/Cargo.toml         --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/chat/Cargo.toml                   --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/morse/Cargo.toml                  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/ascii-art/Cargo.toml              --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/stock-chart/Cargo.toml            --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/code-runner/Cargo.toml            --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/genealogy/Cargo.toml              --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/mind-map/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/net-monitor/Cargo.toml            --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/kanban/Cargo.toml                 --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/presentation/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/notes/Cargo.toml                  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/md-editor/Cargo.toml              --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/term2/Cargo.toml                  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/spreadsheet2/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/draw2/Cargo.toml                  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/archiver/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/syslog/Cargo.toml                 --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/irc/Cargo.toml                    --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/photo-editor/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/rss-reader/Cargo.toml             --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/video-player/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/ebook/Cargo.toml                  --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/speed-test/Cargo.toml             --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/pomodoro-pro/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/astronomy/Cargo.toml              --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/clipboard-pro/Cargo.toml          --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/system-info/Cargo.toml            --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/timeline/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/lang-flash/Cargo.toml             --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/budget2/Cargo.toml               --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/snippets/Cargo.toml              --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/star-map/Cargo.toml              --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/font-editor/Cargo.toml           --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/recipe-planner/Cargo.toml        --target wasm32-wasip2 --release
	$(DOCKER_RUN) cargo build --manifest-path apps/syntax-demo/Cargo.toml           --target wasm32-wasip2 --release
	@touch $(APPS_STAMP)

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

# ── build (all) ───────────────────────────────────────────────────────────────
build: kernel supervisor apps rootfs disk data

# ── run (headless — serial console only) ─────────────────────────────────────
run: $(BZIMAGE) $(INITRAMFS) data
	qemu-system-x86_64 \
	  -kernel $(BZIMAGE) \
	  -initrd $(INITRAMFS) \
	  -append "console=ttyS0 panic=1" \
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
	  -append "console=tty0 console=ttyS0 panic=1" \
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
	  -append "console=ttyS0 panic=1" \
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
	  -append "console=tty0 console=ttyS0 panic=1" \
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
