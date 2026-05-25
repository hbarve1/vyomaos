# Makefile — ARM64 mobile/tablet platform (TM06)
#
# Target: ARM64 SBC or phone hardware (e.g. Raspberry Pi 4, Snapdragon tablet)
# Runtime: QEMU-aarch64 for CI; physical ARM64 hardware for deployment.
#
# The supervisor is cross-compiled to aarch64-unknown-linux-musl.
# WASM apps are unchanged — same wasm32-wasip2 binaries as x86-64 target.
#
# Usage:
#   make PLATFORM=mobile-arm64 build   # build ARM64 image via root Makefile
#   make all                           # platform self-test (prints status)

PLATFORM := mobile-arm64
ARCH     := aarch64
CROSS    := aarch64-linux-musl-

# ── Paths (relative to repo root when invoked via root Makefile) ──────────────
PLATFORM_DIR := platforms/$(PLATFORM)
PROFILE_SRC  := supervisor/src/profile/profiles/mobile.toml
PROFILE_DEST := /etc/vyoma/profiles/mobile.toml

.PHONY: all build check-tools

all: check-tools
	@echo "[$(PLATFORM)] ARM64 mobile platform build"
	@echo "  kernel config : $(PLATFORM_DIR)/kernel.config"
	@echo "  rootfs script : $(PLATFORM_DIR)/rootfs.sh"
	@echo "  profile       : $(PROFILE_SRC)"
	@echo "  supervisor    : cross-compiled for $(ARCH)"
	@echo "  apps          : wasm32-wasip2 (arch-independent)"

build:
	@echo "[$(PLATFORM)] build: use 'make build PLATFORM=mobile-arm64' from repo root"

check-tools:
	@command -v qemu-system-aarch64 >/dev/null 2>&1 \
		|| echo "  warn: qemu-system-aarch64 not found (required for QEMU CI runs)"
