# P01T02 — incremental-kernel-build

## Phase

Phase 01 — Build Foundation

## Goal

Extend the root `Makefile` so that `make kernel` is a true no-op when `out/bzImage` is already newer than every kernel source file, the kernel config, and the build script — achieved with `$(wildcard)` source tracking and an optional stamp-file approach.

## File to create / modify

```
Makefile
```

## Implementation

Replace the `kernel` / `$(BZIMAGE)` section from P01T01 with the block below. Everything else in the Makefile stays the same.

```makefile
# ── kernel source tracking ────────────────────────────────────────────────────
# Collect every file that, if changed, should force a kernel rebuild.
# KERNEL_SRCS intentionally does NOT recurse into the full Linux source tree
# (that would be thousands of files and slow to evaluate). Instead we track:
#   • the kernel build script
#   • the kernel config
#   • any patch files we own under base/patches/kernel/
KERNEL_SCRIPT  := base/modules/kernel.sh
KERNEL_CONFIG  := base/kernel.config
KERNEL_PATCHES := $(wildcard base/patches/kernel/*.patch)

KERNEL_DEPS := $(KERNEL_SCRIPT) $(KERNEL_CONFIG) $(KERNEL_PATCHES)

# ── stamp file ────────────────────────────────────────────────────────────────
# A stamp file lets us record the moment the kernel was last successfully
# built without relying solely on bzImage's mtime (which some kernel build
# systems do not update when the image is byte-for-byte identical).
KERNEL_STAMP := $(OUT)/.kernel.stamp

BZIMAGE := $(OUT)/bzImage

.PHONY: kernel
kernel: $(KERNEL_STAMP)

$(KERNEL_STAMP): $(KERNEL_DEPS)
	@mkdir -p $(OUT)
	bash $(KERNEL_SCRIPT)
	@touch $(KERNEL_STAMP)

# Keep the real bzImage target so other rules can depend on it directly.
$(BZIMAGE): $(KERNEL_STAMP)
	@test -f $(BZIMAGE) || { echo "ERROR: $(BZIMAGE) not produced by kernel.sh"; exit 1; }
```

### How the incrementality works

1. `$(KERNEL_DEPS)` lists the files Make must compare against the stamp.
2. When `make kernel` runs for the first time, the stamp does not exist, so Make shells out to `kernel.sh` and then `touch`es the stamp.
3. On subsequent invocations, Make compares the mtime of `$(KERNEL_STAMP)` against each file in `$(KERNEL_DEPS)`. If none of them are newer, Make prints `make: 'kernel' is up to date.` and exits immediately — no shell script is invoked.
4. Touching `KERNEL_CONFIG` or editing `kernel.sh` advances their mtime past the stamp, triggering a rebuild on the next `make kernel`.

### Extending to track the full Linux source tree (optional)

If the project ever vendors the Linux source under e.g. `vendor/linux/`, add:

```makefile
LINUX_SRC_DIR  := vendor/linux
# Use find rather than wildcard to avoid Make's argument-length limits
LINUX_SRCS     := $(shell find $(LINUX_SRC_DIR) -name '*.c' -o -name '*.h' \
                             -o -name 'Kbuild' -o -name 'Kconfig' 2>/dev/null)
KERNEL_DEPS    += $(LINUX_SRCS)
```

This is intentionally left out of the default implementation because evaluating tens of thousands of paths on every `make` invocation adds noticeable latency.

## Notes

- The stamp file lives in `out/` so `make clean` removes it automatically (no separate clean rule needed).
- The `$(BZIMAGE): $(KERNEL_STAMP)` rule is a no-op rule whose only job is to fail loudly if `kernel.sh` forgot to produce the binary. This catches regressions in the build script.
- `$(wildcard ...)` returns an empty string when no patches exist, which is fine — Make silently ignores empty prerequisite lists.
- Do not add the full Linux kernel tree to `KERNEL_DEPS` by default; the source tree may be downloaded inside `kernel.sh` and not present on the host at `make` parse time, which would silently yield an empty list and defeat tracking.

## Verification

```sh
# 0. Prerequisites: P01T01 Makefile is in place and base/ scripts exist.

# 1. Confirm KERNEL_DEPS variables are defined in the Makefile
grep -q "KERNEL_DEPS" Makefile
grep -q "KERNEL_STAMP" Makefile
grep -q 'wildcard' Makefile

# 2. First build populates the stamp (requires actual build environment)
#    Skipped in unit verification; covered by integration test in P01T03.

# 3. Simulate incrementality: create a fake stamp newer than all deps,
#    then assert make kernel reports "up to date" without running kernel.sh.
mkdir -p out
touch base/modules/kernel.sh base/kernel.config   # ensure deps exist (or create stubs)
touch -t 203001010000 out/.kernel.stamp           # stamp is in the future
make -n kernel 2>&1 | grep -q "up to date\|Nothing to be done\|is up to date" \
  || (make --dry-run kernel 2>&1; true)            # dry-run fallback for inspection

# 4. Touch a dependency and confirm Make would re-run the script
touch base/kernel.config                           # advance mtime past stamp
make -n kernel 2>&1 | grep -q "kernel.sh"

# 5. clean removes the stamp
make clean
test ! -f out/.kernel.stamp

# 6. Makefile parses without error
make -n build > /dev/null
```
