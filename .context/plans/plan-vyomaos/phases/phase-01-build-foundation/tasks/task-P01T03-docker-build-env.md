# P01T03 — docker-build-env

## Phase

Phase 01 — Build Foundation

## Goal

Produce a hermetic `docker/Dockerfile` based on `ubuntu:22.04` that installs every tool needed to build VyomaOS: Linux kernel build dependencies, a musl cross-toolchain, Rustup with the `x86_64-unknown-linux-musl` and `wasm32-wasip2` targets, and Wasmtime build prerequisites.

## File to create / modify

```
docker/Dockerfile
```

## Implementation

```dockerfile
# syntax=docker/dockerfile:1
# VyomaOS hermetic build environment
# Base: ubuntu:22.04 (LTS, well-supported apt mirror)
FROM ubuntu:22.04

# ── avoid interactive prompts from apt ────────────────────────────────────────
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC

# ── system packages ───────────────────────────────────────────────────────────
# Kernel build deps (from `make help` in a Linux tree):
#   bc bison flex libelf-dev libssl-dev libncurses-dev
# Generic build tools:
#   build-essential git wget curl ca-certificates xz-utils
# musl cross-compilation:
#   musl-tools  (provides musl-gcc wrapper and headers)
# QEMU for smoke tests:
#   qemu-system-x86
RUN apt-get update -qq && \
    apt-get install -y --no-install-recommends \
        # build essentials
        build-essential \
        git \
        wget \
        curl \
        ca-certificates \
        xz-utils \
        file \
        # kernel build
        bc \
        bison \
        flex \
        libelf-dev \
        libssl-dev \
        libncurses-dev \
        # musl static linking
        musl-tools \
        # QEMU smoke-test runner
        qemu-system-x86 \
        # misc utilities used by build scripts
        cpio \
        gzip \
        rsync \
    && rm -rf /var/lib/apt/lists/*

# ── Rust via rustup ───────────────────────────────────────────────────────────
# Install as root so the toolchain is available system-wide inside the
# container. CARGO_HOME / RUSTUP_HOME are set to predictable paths.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

# Pin rustup installer version for reproducibility; bump as needed.
ARG RUSTUP_VERSION=1.27.1
RUN curl --proto '=https' --tlsv1.2 -sSf \
      "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" \
      -o /tmp/rustup-init && \
    chmod +x /tmp/rustup-init && \
    /tmp/rustup-init -y \
        --no-modify-path \
        --profile minimal \
        --default-toolchain stable && \
    rm /tmp/rustup-init && \
    # Verify installation
    rustc --version && cargo --version

# ── Rust targets ──────────────────────────────────────────────────────────────
# x86_64-unknown-linux-musl  — supervisor binary (statically linked)
# wasm32-wasip2              — WASI Preview 2 apps (hello-world etc.)
RUN rustup target add x86_64-unknown-linux-musl wasm32-wasip2

# ── linker configuration for musl target ──────────────────────────────────────
# cargo needs to know to use musl-gcc as the linker when cross-compiling for
# x86_64-unknown-linux-musl on an x86_64 host.
RUN mkdir -p /usr/local/cargo/config.d && \
    printf '[target.x86_64-unknown-linux-musl]\nlinker = "musl-gcc"\n' \
      > /usr/local/cargo/config.d/musl.toml

# ── working directory ─────────────────────────────────────────────────────────
WORKDIR /work

# Default command: open a shell (CI overrides this with `make build`)
CMD ["/bin/bash"]
```

## Notes

- `ubuntu:22.04` is used rather than Alpine so that kernel build dependencies resolve without patching. Alpine's musl-based libc causes problems with some kernel build scripts.
- `--profile minimal` avoids installing rust-docs and other large optional components; `rust-analyzer` and `clippy` can be added per developer need in a separate `docker/Dockerfile.dev`.
- `RUSTUP_HOME` and `CARGO_HOME` are placed under `/usr/local` so that any non-root user inside the container still has the binaries on `$PATH` via the `ENV PATH=` line.
- The `musl.toml` cargo config applies only to the musl target and does not affect native or WASM builds.
- `wasm32-wasip2` is the Rust-side target name for WASI Preview 2 (component model). It requires Rust 1.78+; `stable` as of early 2025 satisfies this.
- `qemu-system-x86` is installed so `make run` works inside the container during integration testing without an additional image layer.
- Do not install Wasmtime from apt or cargo-install here; the rootfs build script downloads the pre-built static binary directly (see P04T01). The container only needs Wasmtime if you want to run WASM locally during development — add that as an optional `ARG` build argument if needed.

## Verification

```sh
# Build the image (run from project root)
docker build -t vyomaos-builder:test docker/

# 1. Rust toolchain is present and includes both targets
docker run --rm vyomaos-builder:test rustc --version
docker run --rm vyomaos-builder:test cargo --version
docker run --rm vyomaos-builder:test \
  rustup target list --installed | grep -q "x86_64-unknown-linux-musl"
docker run --rm vyomaos-builder:test \
  rustup target list --installed | grep -q "wasm32-wasip2"

# 2. Kernel build dependencies are present
docker run --rm vyomaos-builder:test which bc
docker run --rm vyomaos-builder:test which flex
docker run --rm vyomaos-builder:test dpkg -l libelf-dev | grep -q '^ii'

# 3. musl toolchain is present
docker run --rm vyomaos-builder:test which musl-gcc
docker run --rm vyomaos-builder:test musl-gcc --version

# 4. musl linker config is wired up
docker run --rm vyomaos-builder:test \
  cat /usr/local/cargo/config.d/musl.toml | grep -q "musl-gcc"

# 5. Compile a trivial musl binary inside the container
docker run --rm -v "$PWD":/work -w /work vyomaos-builder:test bash -c '
  echo "fn main(){println!(\"ok\");}" > /tmp/t.rs
  rustc --target x86_64-unknown-linux-musl -o /tmp/t /tmp/t.rs
  file /tmp/t | grep -q "statically linked"
'

# 6. Compile a trivial WASM module inside the container
docker run --rm -v "$PWD":/work -w /work vyomaos-builder:test bash -c '
  echo "fn main(){}" > /tmp/w.rs
  rustc --target wasm32-wasip2 -o /tmp/w.wasm /tmp/w.rs
  file /tmp/w.wasm | grep -q "WebAssembly"
'

# 7. Full project build (requires P01T01 Makefile and all scripts in place)
# docker run --rm -v "$PWD":/work -w /work vyomaos-builder:test make build
```
