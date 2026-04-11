# Phase 01 — Build Foundation

## Goal

Replace the ad-hoc bash build scripts with a root `Makefile` that tracks dependencies properly so the kernel is not rebuilt on every change, and wrap the entire build in a Docker container for hermetic, reproducible builds on any host.

## Gate

```sh
# Kernel only rebuilds when kernel sources or config change
touch base/modules/rootfs.sh && make build
# Verify bzImage mtime is OLDER than the rootfs rebuild
stat out/bzImage out/initramfs.cpio.gz | grep Modify

# Docker build produces identical sha256 on two consecutive runs
docker build -t vyomaos-builder docker/ --no-cache
docker build -t vyomaos-builder docker/ --no-cache
# Both image IDs must match
```

## Dependencies

- Existing `base/` shell scripts (read-only reference)
- Docker installed on host
- `make` available

## Tasks

### P01T01 — Root Makefile
[task-P01T01-root-makefile.md](tasks/task-P01T01-root-makefile.md)

### P01T02 — Incremental Kernel Build
[task-P01T02-incremental-kernel-build.md](tasks/task-P01T02-incremental-kernel-build.md)

### P01T03 — Docker Build Environment
[task-P01T03-docker-build-env.md](tasks/task-P01T03-docker-build-env.md)
