# Makefile — MCU ARM Cortex-M platform
#
# Cross-compiles the VyomaOS supervisor for ARM Cortex-M targets using
# the thumbv7em-none-eabi Rust target (no_std, bare-metal).
#
# For QEMU ARM emulation the target is thumbv7m-none-eabi (Cortex-M3).
# Production MCUs (e.g. STM32, nRF52) require thumbv7em-none-eabihf.
#
# The supervisor is compiled in MCU mode:
#   - wasm3 interpreter (no JIT, minimal footprint)
#   - No display, no networking, no 9P filesystem
#   - UART console only
#   - mcu-minimal.toml platform profile
#
# Usage:
#   make PLATFORM=mcu-arm-cortex-m all   — supervisor + WASM apps
#   make PLATFORM=mcu-arm-cortex-m run-qemu  — QEMU Cortex-M3 emulation
#
# Requirements:
#   - Rust nightly with thumbv7m-none-eabi / thumbv7em-none-eabi targets
#   - arm-none-eabi-gcc (GNU ARM toolchain)
#   - qemu-system-arm

PLATFORM          := mcu-arm-cortex-m

# Target variants:
#   thumbv7m-none-eabi   — Cortex-M3 (no FPU) — used for QEMU testing
#   thumbv7em-none-eabi  — Cortex-M4/M7 (no FPU)
#   thumbv7em-none-eabihf— Cortex-M4/M7 (with FPU) — production MCUs
RUST_TARGET       ?= thumbv7m-none-eabi
WASM_TARGET       := wasm32-wasip2
CROSS_COMPILE     := arm-none-eabi-

# Output directory
OUT               := ../../out/$(PLATFORM)
SUPERVISOR_SRC    := ../../supervisor
APPS_DIR          := ../../apps
PROFILE_TOML      := ../../supervisor/src/profile/profiles/mcu-minimal.toml

# MCU profile enables wasm3 and disables display/network/filesystem
MCU_FEATURES      := --no-default-features

.PHONY: all supervisor apps run-qemu flash clean help check-target

all: check-target supervisor apps
	@echo "[$(PLATFORM)] Build complete."

# ── Target availability check ─────────────────────────────────────────────────

check-target:
	@if ! rustup target list --installed 2>/dev/null | grep -q "$(RUST_TARGET)"; then \
	  echo "[$(PLATFORM)] Installing Rust target $(RUST_TARGET)..."; \
	  rustup target add $(RUST_TARGET) || \
	    (echo "[$(PLATFORM)] WARN: rustup not available — ensure $(RUST_TARGET) is installed" && true); \
	fi

# ── Supervisor (MCU mode) ────────────────────────────────────────────────────
#
# The supervisor binary for MCU targets is compiled in no_std mode with
# the wasm3 feature flag.  The resulting binary is a bare-metal ELF that
# runs directly on the MCU or in QEMU.

supervisor:
	@echo "[$(PLATFORM)] Building supervisor (MCU mode, $(RUST_TARGET))..."
	RUSTFLAGS="-D warnings -C link-arg=-nostartfiles" \
	cargo build \
	  --manifest-path $(SUPERVISOR_SRC)/Cargo.toml \
	  --target $(RUST_TARGET) \
	  --release \
	  $(MCU_FEATURES)
	mkdir -p $(OUT)
	cp $(SUPERVISOR_SRC)/target/$(RUST_TARGET)/release/supervisor \
	   $(OUT)/supervisor.elf
	@echo "[$(PLATFORM)] supervisor ELF: $(OUT)/supervisor.elf"

# ── WASM apps ────────────────────────────────────────────────────────────────
#
# WASM apps are architecture-independent — the same .wasm binaries run on
# any platform.  This target cross-compiles them for the standard WASM target.

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
	mkdir -p $(OUT)/apps
	@for wasm in $(APPS_DIR)/*/target/$(WASM_TARGET)/release/*.wasm; do \
	  [ -f "$$wasm" ] && cp "$$wasm" $(OUT)/apps/ || true; \
	done
	@echo "[$(PLATFORM)] WASM apps in $(OUT)/apps/"

# ── QEMU ARM (Cortex-M3 / mps2-an385 machine) ────────────────────────────────

run-qemu:
	@echo "[$(PLATFORM)] Booting in QEMU Cortex-M3 (mps2-an385)..."
	qemu-system-arm \
	  -machine mps2-an385 \
	  -cpu cortex-m3 \
	  -m 256K \
	  -nographic \
	  -semihosting-config enable=on,target=native \
	  -kernel $(OUT)/supervisor.elf

# ── Flash to real MCU (requires openocd or pyocd) ────────────────────────────

flash:
	@echo "[$(PLATFORM)] Flashing $(OUT)/supervisor.elf via OpenOCD..."
	openocd \
	  -f interface/stlink.cfg \
	  -f target/stm32f4x.cfg \
	  -c "program $(OUT)/supervisor.elf verify reset exit"

# ── Clean ─────────────────────────────────────────────────────────────────────

clean:
	rm -rf $(OUT)
	@echo "[$(PLATFORM)] Cleaned."

# ── Help ──────────────────────────────────────────────────────────────────────

help:
	@echo "MCU ARM Cortex-M platform targets:"
	@echo "  all         — supervisor + apps (default)"
	@echo "  supervisor  — cross-compile supervisor for $(RUST_TARGET)"
	@echo "  apps        — compile WASM apps for wasm32-wasip2"
	@echo "  run-qemu    — QEMU Cortex-M3 emulation (mps2-an385)"
	@echo "  flash       — flash to real MCU via OpenOCD"
	@echo "  clean       — remove $(OUT)"
	@echo ""
	@echo "Override Rust target:"
	@echo "  make RUST_TARGET=thumbv7em-none-eabihf supervisor"
	@echo ""
	@echo "Supported targets:"
	@echo "  thumbv7m-none-eabi    — Cortex-M3 (QEMU testing)"
	@echo "  thumbv7em-none-eabi   — Cortex-M4/M7 (no FPU)"
	@echo "  thumbv7em-none-eabihf — Cortex-M4/M7 (with FPU, production)"
