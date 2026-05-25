# Makefile — ARM64 server platform (headless, network-enabled)
#
# Produces a bootable VyomaOS image for 64-bit ARM servers running headless.
# Capabilities: lifecycle, capability, ipc, net, packages, observability.
# No display or GUI modules are included.
#
# Usage:
#   make all          — full build (kernel + supervisor + apps + rootfs)
#   make kernel       — compile ARM64 Linux kernel only
#   make supervisor   — compile supervisor for aarch64-unknown-linux-musl
#   make rootfs       — assemble initramfs with server-headless profile
#   make run          — boot in QEMU (aarch64, serial console, port 8080 forwarded)
#
# Requirements: Docker image vyomaos-builder:latest

PLATFORM   := server-arm64
ARCH       := arm64
CROSS      := aarch64-linux-gnu-
KERNEL_VER := 5.10.0

REPO_ROOT  := $(shell cd ../.. && pwd)
OUT        := $(REPO_ROOT)/out/$(PLATFORM)
IMAGE      := vyomaos-builder
IMAGE_TAG  := latest

RUSTFLAGS  := -D warnings
DOCKER_RUN := docker run --rm \
	--platform linux/amd64 \
	-v "$(REPO_ROOT)":/work \
	-w /work \
	-u $$(id -u):$$(id -g) \
	$(IMAGE):$(IMAGE_TAG)

PROFILE_TOML := supervisor/src/profile/profiles/server-headless.toml

.PHONY: all kernel supervisor apps rootfs run clean help

# ── Default target ─────────────────────────────────────────────────────────────
all: kernel supervisor apps rootfs

# ── Kernel ─────────────────────────────────────────────────────────────────────
kernel: | $(OUT)
	@echo "[$(PLATFORM)] building ARM64 kernel $(KERNEL_VER)"
	$(DOCKER_RUN) sh -c \
	  "make -C /work/base/linux-$(KERNEL_VER) \
	     ARCH=arm64 \
	     CROSS_COMPILE=$(CROSS) \
	     KCONFIG_CONFIG=/work/platforms/$(PLATFORM)/kernel.config \
	     bzImage -j$$(nproc) \
	   && cp arch/arm64/boot/Image /work/$(OUT)/Image"

# ── Supervisor ─────────────────────────────────────────────────────────────────
supervisor: | $(OUT)
	@echo "[$(PLATFORM)] building supervisor for aarch64-unknown-linux-musl"
	$(DOCKER_RUN) env RUSTFLAGS="$(RUSTFLAGS)" \
	  cargo build \
	    --manifest-path supervisor/Cargo.toml \
	    --target aarch64-unknown-linux-musl \
	    --release
	cp supervisor/target/aarch64-unknown-linux-musl/release/supervisor \
	   $(OUT)/supervisor

# ── WASM apps ──────────────────────────────────────────────────────────────────
apps: | $(OUT)
	@echo "[$(PLATFORM)] building WASM apps"
	$(DOCKER_RUN) sh -c \
	  "for d in apps/*/; do \
	     name=\$$(basename \$$d); \
	     cargo build \
	       --manifest-path \$$d/Cargo.toml \
	       --target wasm32-wasip2 \
	       --release \
	     && cp \$$d/target/wasm32-wasip2/release/\$$name.wasm $(OUT)/ 2>/dev/null || true; \
	   done"

# ── Rootfs ─────────────────────────────────────────────────────────────────────
rootfs: | $(OUT)
	@echo "[$(PLATFORM)] assembling initramfs with server-headless profile"
	$(DOCKER_RUN) sh /work/platforms/$(PLATFORM)/rootfs.sh \
	  --out $(OUT) \
	  --profile $(PROFILE_TOML)

# ── QEMU boot (headless, aarch64, port 8080 forwarded) ─────────────────────────
run: all
	@echo "[$(PLATFORM)] booting in QEMU (aarch64 headless)"
	qemu-system-aarch64 \
	  -M virt \
	  -cpu cortex-a57 \
	  -m 1024M \
	  -nographic \
	  -serial mon:stdio \
	  -kernel $(OUT)/Image \
	  -initrd $(OUT)/initramfs.cpio.gz \
	  -append "console=ttyAMA0 quiet" \
	  -netdev user,id=net0,hostfwd=tcp::8080-:8080 \
	  -device virtio-net-pci,netdev=net0

# ── Helpers ────────────────────────────────────────────────────────────────────
$(OUT):
	mkdir -p $(OUT)

clean:
	rm -rf $(OUT)

help:
	@echo "Targets: all kernel supervisor apps rootfs run clean"
	@echo "Platform: $(PLATFORM) (ARM64 server, headless, network-enabled)"
