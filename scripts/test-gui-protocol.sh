#!/usr/bin/env bash
# test-gui-protocol.sh — runs WASM display apps with mock stdin,
# verifies VYOMA_DRAW protocol output using gui-test checker.
set -euo pipefail

WASMTIME="${WASMTIME:-wasmtime}"
GUI_TEST_BIN="tools/gui-test/target/x86_64-unknown-linux-musl/release/gui-test"
DESKTOP_WASM="apps/desktop/target/wasm32-wasip2/release/desktop.wasm"
DOCK_WASM="apps/dock/target/wasm32-wasip2/release/dock.wasm"

# Build gui-test checker if needed.
if [ ! -f "$GUI_TEST_BIN" ]; then
    echo "[gui-test] building checker..."
    RUSTFLAGS="-D warnings" cargo build \
        --manifest-path tools/gui-test/Cargo.toml \
        --target x86_64-unknown-linux-musl \
        --release
fi

run_test() {
    local name="$1"
    local wasm="$2"
    local stdin_data="$3"
    local extra_args="${4:-}"

    echo "[gui-test] $name..."
    local output
    output=$(echo "$stdin_data" | $WASMTIME "$wasm" 2>/dev/null || true)

    # shellcheck disable=SC2086
    echo "$output" | "$GUI_TEST_BIN" \
        --expect-flush 1 \
        --expect-fills 10 \
        $extra_args \
        && echo "[gui-test] $name: PASS" \
        || { echo "[gui-test] $name: FAIL"; exit 1; }
}

# Test desktop app.
if [ -f "$DESKTOP_WASM" ]; then
    run_test "desktop" "$DESKTOP_WASM" \
        "VYOMA_SYSTEM:screen:1440,900" \
        "--expect-fills 50 --expect-text VyomaOS"
else
    echo "[gui-test] SKIP desktop (not built)"
fi

# Test dock app.
if [ -f "$DOCK_WASM" ]; then
    run_test "dock" "$DOCK_WASM" \
        "VYOMA_SYSTEM:screen:1440,900" \
        "--expect-fills 20"
else
    echo "[gui-test] SKIP dock (not built)"
fi

echo "[gui-test] ALL PASS"
