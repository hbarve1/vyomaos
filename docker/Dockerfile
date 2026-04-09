# syntax=docker/dockerfile:1
# VyomaOS hermetic build environment
# Base: ubuntu:22.04 (LTS, well-supported apt mirror)
FROM --platform=linux/amd64 ubuntu:22.04

# ── avoid interactive prompts from apt ────────────────────────────────────────
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC

# ── system packages ───────────────────────────────────────────────────────────
# Kernel build deps: bc bison flex libelf-dev libssl-dev libncurses-dev
# Build tools: build-essential git wget curl ca-certificates xz-utils file
# musl cross-compilation: musl-tools (provides musl-gcc wrapper and headers)
# QEMU for smoke tests: qemu-system-x86
# Build script utilities: cpio gzip rsync
RUN apt-get update -qq && \
    apt-get install -y --no-install-recommends \
        build-essential \
        git \
        wget \
        curl \
        ca-certificates \
        xz-utils \
        file \
        bc \
        bison \
        flex \
        libelf-dev \
        libssl-dev \
        libncurses-dev \
        musl-tools \
        qemu-system-x86 \
        cpio \
        gzip \
        rsync \
    && rm -rf /var/lib/apt/lists/*

# ── Rust via rustup ───────────────────────────────────────────────────────────
# Install system-wide so all users in the container have access.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

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
    rustc --version && cargo --version

# ── Rust targets ──────────────────────────────────────────────────────────────
# x86_64-unknown-linux-musl  — supervisor binary (statically linked)
# wasm32-wasip2              — WASI Preview 2 apps
RUN rustup target add x86_64-unknown-linux-musl wasm32-wasip2

# ── musl linker config ────────────────────────────────────────────────────────
# Tell cargo to use musl-gcc when cross-compiling for x86_64-unknown-linux-musl.
RUN mkdir -p /usr/local/cargo/config.d && \
    printf '[target.x86_64-unknown-linux-musl]\nlinker = "musl-gcc"\n' \
      > /usr/local/cargo/config.d/musl.toml

# ── working directory ─────────────────────────────────────────────────────────
WORKDIR /work

CMD ["/bin/bash"]
