# Makefile — IoT Raspberry Pi platform (ARM64)
#
# Builds the VyomaOS supervisor and WASM apps for ARM64 (aarch64-linux-musl),
# then creates an initramfs suitable for booting on a Raspberry Pi 4 or
# QEMU virt-machine in ARM64 mode.
#
# Usage (from project root):
#   make PLATFORM=iot-rpi build     — full build: kernel + supervisor + rootfs
#   make PLATFORM=iot-rpi run-gui   — boot in QEMU ARM64 with virtio-gpu
#
# Required host tools:
#   - aarch64-linux-musl-gcc (cross-compiler)
#   - qemu-system-aarch64    (ARM64 emulation)
#   - docker                 (hermetic builds)

PLATFORM          := iot-rpi
ARCH              := arm64
CROSS_COMPILE     := aarch64-linux-musl-
RUST_TARGET       := aarch64-unknown-linux-musl
WASM_TARGET       := wasm32-wasip2

# Paths (relative to project root)
OUT               := ../../out/$(PLATFORM)
SUPERVISOR_SRC    := ../../supervisor
APPS_DIR          := ../../apps
PROFILE_TOML      := ../../supervisor/src/profile/profiles/iot-edge.toml

# Kernel
KERNEL_VERSION    := 5.10
KERNEL_SRC        := ../../base/linux-$(KERNEL_VERSION)
KERNEL_CONFIG     := kernel.config

# Runtime: wasm3 interpreter (lightweight for IoT)
RUNTIME           := wasm3

.PHONY: all supervisor apps kernel rootfs run-qemu clean help

all: supervisor apps rootfs

# ── Supervisor ────────────────────────────────────────────────────────────────

supervisor:
	@echo "[$(PLATFORM)] Building supervisor for $(RUST_TARGET)..."
	RUSTFLAGS="-D warnings" \
	cargo build \
	  --manifest-path $(SUPERVISOR_SRC)/Cargo.toml \
	  --target $(RUST_TARGET) \
	  --release
	@echo "[$(PLATFORM)] Supervisor built: $(SUPERVISOR_SRC)/target/$(RUST_TARGET)/release/supervisor"

# ── WASM apps ────────────────────────────────────────────────────────────────

apps:
	@echo "[$(PLATFORM)] Building WASM apps for $(WASM_TARGET)..."
	@for app in $(APPS_DIR)/*/Cargo.toml; do \
	  dir=$$(dirname $$app); \
	  name=$$(basename $$dir); \
	  echo "  [app] $$name"; \
	  cargo build \
	    --manifest-path $$app \
	    --target $(WASM_TARGET) \
	    --release 2>&1 || echo "  [warn] $$name failed"; \
	done
	@echo "[$(PLATFORM)] WASM apps built."

# ── Kernel ────────────────────────────────────────────────────────────────────

kernel:
	@echo "[$(PLATFORM)] Compiling Linux $(KERNEL_VERSION) for $(ARCH)..."
	$(MAKE) -C $(KERNEL_SRC) \
	  ARCH=$(ARCH) \
	  CROSS_COMPILE=$(CROSS_COMPILE) \
	  defconfig
	cp $(KERNEL_CONFIG) $(KERNEL_SRC)/.config
	$(MAKE) -C $(KERNEL_SRC) \
	  ARCH=$(ARCH) \
	  CROSS_COMPILE=$(CROSS_COMPILE) \
	  -j$$(nproc) Image
	mkdir -p $(OUT)
	cp $(KERNEL_SRC)/arch/arm64/boot/Image $(OUT)/Image
	@echo "[$(PLATFORM)] Kernel image: $(OUT)/Image"

# ── Root filesystem ───────────────────────────────────────────────────────────

rootfs:
	@echo "[$(PLATFORM)] Building initramfs via rootfs.sh..."
	bash rootfs.sh
	@echo "[$(PLATFORM)] Initramfs ready."

# ── QEMU boot ────────────────────────────────────────────────────────────────

run-qemu:
	@echo "[$(PLATFORM)] Booting in QEMU ARM64..."
	qemu-system-aarch64 \
	  -machine virt \
	  -cpu cortex-a72 \
	  -m 512M \
	  -smp 2 \
	  -nographic \
	  -kernel $(OUT)/Image \
	  -initrd $(OUT)/initramfs.cpio.gz \
	  -append "console=ttyAMA0 PLATFORM=$(PLATFORM)" \
	  -virtfs local,path=../../data,mount_tag=data,security_model=passthrough,id=data \
	  -netdev user,id=net0,hostfwd=tcp::8080-:8080 \
	  -device virtio-net-pci,netdev=net0

# ── Clean ─────────────────────────────────────────────────────────────────────

clean:
	rm -rf $(OUT)
	@echo "[$(PLATFORM)] Cleaned."

# ── Help ──────────────────────────────────────────────────────────────────────

help:
	@echo "IoT RPi (ARM64) platform targets:"
	@echo "  all        — supervisor + apps + rootfs"
	@echo "  supervisor — cross-compile supervisor for aarch64-linux-musl"
	@echo "  apps       — compile WASM apps for wasm32-wasip2"
	@echo "  kernel     — cross-compile Linux kernel for ARM64"
	@echo "  rootfs     — build initramfs via rootfs.sh"
	@echo "  run-qemu   — boot in QEMU ARM64 virt machine"
	@echo "  clean      — remove $(OUT)"
