# Minimal environment to build Vyoma OS and example WASM apps
FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive \
    TZ=UTC

# Base tools and build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl tar gzip xz-utils \
    make gcc g++ flex bison bc \
    libelf-dev libssl-dev libncurses-dev \
    cpio qemu-system-x86 \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Rust toolchain for building Rust→WASM apps
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --default-toolchain stable \
    && /root/.cargo/bin/rustup target add wasm32-unknown-unknown
# Install Wasmtime CLI (WASM runtime)
RUN curl -fsSL https://wasmtime.dev/install.sh | bash
ENV PATH="/root/.wasmtime/bin:/root/.cargo/bin:${PATH}" \
    INCLUDE_REAL_WASMTIME=true

# Workdir and source (copies only the docker/ dir when building from here)
WORKDIR /workspace
COPY . /workspace

# Provide a place for WASM and copy a prebuilt hello-world.wasm placed next to this Dockerfile
RUN install -d /opt/wasm
COPY hello-world.wasm /opt/wasm/hello-world.wasm

# Default command: run the hello-world WASM via Wasmtime
# Put hello-world.wasm in this folder and build with:
#   cd docker && docker build -t vyomaos .
CMD ["wasmtime", "--invoke", "add", "/opt/wasm/hello-world.wasm", "15", "27"]
