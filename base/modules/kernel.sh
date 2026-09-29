#!/usr/bin/env bash
# Build/reuse a kernel only when all selected inputs and published outputs match.
source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

build_kernel() (
    set -euo pipefail
    export LC_ALL=C
    local platform="${PLATFORM:-desktop-x86}"
    if [[ "$platform" != desktop-x86 ]]; then
        log_error "Unsupported PLATFORM '$platform'; supported: desktop-x86"
        return 1
    fi
    local kernel_config="${KERNEL_CONFIG:-$PROJECT_ROOT/base/kernel.config}"
    [[ "$kernel_config" = /* ]] || kernel_config="$PROJECT_ROOT/$kernel_config"
    if [[ ! -f "$kernel_config" ]]; then
        log_error "Kernel config not found: $kernel_config"
        return 1
    fi
    kernel_config="$(realpath "$kernel_config")"
    local patch_dir="$PROJECT_ROOT/base/patches/kernel"
    local patches=()
    shopt -s nullglob
    patches=("$patch_dir"/*.patch)
    mkdir -p "$OUTDIR/kernel"
    # Direct script calls and concurrent make invocations share this lock.
    exec 9>"$OUTDIR/kernel/build.lock"
    flock 9
    local stamp="$OUTDIR/.kernel.stamp"
    local inputs source_inputs source_key input_key toolchain
    inputs="$(mktemp "$OUTDIR/kernel/inputs.XXXXXX")"
    trap 'rm -f "$inputs"' EXIT
    toolchain="$(gcc --version; ld --version; make --version)"
    source_inputs="$(printf '%s\n' "$KERNEL_VERSION" "$KERNEL_SHA256";
        if ((${#patches[@]})); then sha256sum "${patches[@]}"; fi)"
    source_key="$(printf '%s\n' "$source_inputs" "$toolchain" | sha256sum | cut -d ' ' -f1)"
    {
        printf 'platform=%s\nconfig=%s\n' "$platform" "$kernel_config"
        printf '%s\n' "$source_inputs"
        sha256sum "$kernel_config" "$PROJECT_ROOT/base/versions.sh" \
            "$PROJECT_ROOT/base/config.sh" "$PROJECT_ROOT/base/modules/kernel.sh"
        printf '%s\n' "$toolchain"
    } > "$inputs"
    input_key="$(sha256sum "$inputs" | cut -d ' ' -f1)"
    if [[ -s "$stamp" && -s "$KERNEL_FILE" && -s "$OUTDIR/kernel.config" ]] &&
        [[ "$(head -n 1 "$stamp")" == "$input_key" ]] &&
        tail -n +2 "$stamp" | sha256sum --check --status; then
        log_info "Kernel inputs and artifact digests match; reusing $KERNEL_FILE"
        return 0
    fi

    # Invalidate acceptance before any fallible operation. An older image may
    # remain for diagnosis, but it cannot be accepted after a failed rebuild.
    rm -f "$stamp"
    download_file "$KERNEL_SOURCE_URL" "$KERNEL_TAR_FILE" "kernel source" "$KERNEL_SHA256"
    local source_dir="$OUTDIR/kernel/source-$source_key"
    if [[ ! -f "$source_dir/.prepared" ]]; then
        # Only this builder-owned, fingerprinted tree is disposable.
        rm -rf "$source_dir"
        mkdir -p "$source_dir"
        tar -xJf "$KERNEL_TAR_FILE" -C "$source_dir" --strip-components=1
        local patch_file
        for patch_file in "${patches[@]}"; do
            patch -d "$source_dir" -p1 --batch < "$patch_file"
        done
        touch "$source_dir/.prepared"
    fi
    log_info "Building kernel with $kernel_config (inputs $input_key)"
    # Regenerate from the selected fragment, never from a previous .config.
    make -C "$source_dir" ARCH=x86_64 CC=gcc HOSTCC=gcc LD=ld KCONFIG_CONFIG=.config KCONFIG_ALLCONFIG="$kernel_config" allnoconfig
    make -C "$source_dir" ARCH=x86_64 CC=gcc HOSTCC=gcc LD=ld KCONFIG_CONFIG=.config -j"${KERNEL_JOBS:-$(nproc)}" bzImage
    test -s "$source_dir/arch/x86/boot/bzImage"
    cp "$source_dir/arch/x86/boot/bzImage" "$KERNEL_FILE.tmp"
    mv "$KERNEL_FILE.tmp" "$KERNEL_FILE"
    cp "$source_dir/.config" "$OUTDIR/kernel.config"
    cp "$inputs" "$OUTDIR/kernel.inputs"
    {
        printf '%s\n' "$input_key"
        sha256sum "$KERNEL_FILE" "$OUTDIR/kernel.config"
    } > "$stamp.tmp"
    mv "$stamp.tmp" "$stamp"
    log_success "Kernel built: $KERNEL_FILE ($(du -sh "$KERNEL_FILE" | cut -f1))"
)

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    build_kernel
fi
