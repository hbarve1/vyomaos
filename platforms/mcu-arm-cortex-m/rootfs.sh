#!/bin/sh
# rootfs.sh — MCU ARM Cortex-M platform firmware image builder
#
# For bare-metal MCU targets there is no Linux initramfs.
# This script produces a firmware binary image (ELF + optional bin/hex)
# that packages the supervisor, wasm3 interpreter, and WASM apps into a
# single flashable artifact.
#
# In QEMU mode (-machine mps2-an385), the ELF is loaded directly.
# For real MCUs, use `make flash` to program via OpenOCD.
#
# Usage:
#   bash platforms/mcu-arm-cortex-m/rootfs.sh
#
# Environment variables:
#   RUST_TARGET  — Rust target triple (default: thumbv7m-none-eabi)
#   OUT_DIR      — Output directory   (default: out/mcu-arm-cortex-m)

set -e

PLATFORM="mcu-arm-cortex-m"
RUST_TARGET="${RUST_TARGET:-thumbv7m-none-eabi}"
WASM_TARGET="wasm32-wasip2"
OUT_DIR="${OUT_DIR:-out/mcu-arm-cortex-m}"
PROFILE_SRC="supervisor/src/profile/profiles/mcu-minimal.toml"

# Change to repo root
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

echo "[rootfs:$PLATFORM] Starting firmware image build..."

mkdir -p "$OUT_DIR/apps"

# ── Supervisor ELF ────────────────────────────────────────────────────────────

SUPERVISOR_ELF="supervisor/target/$RUST_TARGET/release/supervisor"
if [ ! -f "$SUPERVISOR_ELF" ]; then
  echo "[rootfs:$PLATFORM] WARN: supervisor ELF not found at $SUPERVISOR_ELF"
  echo "[rootfs:$PLATFORM] Run: cargo build --target $RUST_TARGET --release"
else
  cp "$SUPERVISOR_ELF" "$OUT_DIR/supervisor.elf"
  echo "[rootfs:$PLATFORM] supervisor ELF: OK ($SUPERVISOR_ELF)"

  # Generate flat binary (for MCU flash tools)
  if command -v arm-none-eabi-objcopy > /dev/null 2>&1; then
    arm-none-eabi-objcopy -O binary \
      "$OUT_DIR/supervisor.elf" \
      "$OUT_DIR/supervisor.bin"
    echo "[rootfs:$PLATFORM] supervisor.bin: OK"

    # Generate Intel HEX (for openocd / J-Link)
    arm-none-eabi-objcopy -O ihex \
      "$OUT_DIR/supervisor.elf" \
      "$OUT_DIR/supervisor.hex"
    echo "[rootfs:$PLATFORM] supervisor.hex: OK"
  else
    echo "[rootfs:$PLATFORM] WARN: arm-none-eabi-objcopy not found — skipping .bin/.hex"
  fi
fi

# ── WASM apps ────────────────────────────────────────────────────────────────

WASM_COPIED=0
for app_dir in apps/*/; do
  app_name="$(basename "$app_dir")"
  wasm_bin="$app_dir/target/$WASM_TARGET/release/$app_name.wasm"
  if [ -f "$wasm_bin" ]; then
    cp "$wasm_bin" "$OUT_DIR/apps/$app_name.wasm"
    WASM_COPIED=$((WASM_COPIED + 1))
  fi
done
echo "[rootfs:$PLATFORM] WASM apps copied: $WASM_COPIED"

# ── Platform profile ──────────────────────────────────────────────────────────

if [ -f "$PROFILE_SRC" ]; then
  cp "$PROFILE_SRC" "$OUT_DIR/mcu-minimal.toml"
  echo "[rootfs:$PLATFORM] Platform profile: mcu-minimal.toml"
else
  echo "[rootfs:$PLATFORM] WARN: profile not found at $PROFILE_SRC"
fi

# ── Linker script (for real MCU flash layout) ─────────────────────────────────
# In production the linker script is specified in Cargo.toml / .cargo/config.toml.
# Here we generate a reference template.

cat > "$OUT_DIR/link.x" << 'LINK_EOF'
/* VyomaOS MCU linker script — ARM Cortex-M3 (STM32F4 layout) */
MEMORY
{
  FLASH  (rx)  : ORIGIN = 0x08000000, LENGTH = 512K
  RAM    (rwx) : ORIGIN = 0x20000000, LENGTH = 256K
}

SECTIONS
{
  .text : {
    KEEP(*(.vector_table))
    *(.text .text.*)
    *(.rodata .rodata.*)
  } > FLASH

  .data : {
    *(.data .data.*)
  } > RAM AT > FLASH

  .bss (NOLOAD) : {
    *(.bss .bss.*)
  } > RAM

  .stack (NOLOAD) : {
    . = ALIGN(8);
    . = . + 0x4000; /* 16 KB supervisor stack */
    . = ALIGN(8);
  } > RAM
}
LINK_EOF
echo "[rootfs:$PLATFORM] linker script: $OUT_DIR/link.x"

# ── Summary ───────────────────────────────────────────────────────────────────

echo "[rootfs:$PLATFORM] Firmware image build done."
echo "[rootfs:$PLATFORM] Artifacts in $OUT_DIR/:"
ls -lh "$OUT_DIR/" 2>/dev/null || true
echo "[rootfs:$PLATFORM] Test with: make PLATFORM=mcu-arm-cortex-m run-qemu"
