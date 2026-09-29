# VyomaOS hermetic build environment
# Base: ubuntu:22.04 (LTS, well-supported apt mirror)
FROM ubuntu:22.04@sha256:281c5745f657873d78e5531fc5ba8575f46ab7769b94550ac99543f122679986

# ── avoid interactive prompts from apt ────────────────────────────────────────
ENV DEBIAN_FRONTEND=noninteractive
ENV TZ=UTC

# Minimal Ubuntu has no TLS trust store. Bootstrap the CA bundle from this
# checksum-pinned Ubuntu package before contacting the HTTPS snapshot service.
ADD --checksum=sha256:6e8cdcc8c86103acd4fc14649eac62ff2037108389074a7b167567af33c32245 https://snapshot.ubuntu.com/ubuntu/20260928T000000Z/pool/main/c/ca-certificates/ca-certificates_20260601~22.04.1_all.deb /tmp/vyoma-ca.deb
RUN dpkg-deb -x /tmp/vyoma-ca.deb /tmp/vyoma-ca && \
    mkdir -p /etc/ssl/certs && \
    cat /tmp/vyoma-ca/usr/share/ca-certificates/mozilla/*.crt > /etc/ssl/certs/ca-certificates.crt && \
    rm -rf /tmp/vyoma-ca /tmp/vyoma-ca.deb

# Freeze package resolution as well as the base filesystem. Archive signatures
# remain required; historical snapshot metadata may be past Valid-Until.
RUN printf '%s\n' \
    'deb [check-valid-until=no] https://snapshot.ubuntu.com/ubuntu/20260928T000000Z jammy main universe' \
    'deb [check-valid-until=no] https://snapshot.ubuntu.com/ubuntu/20260928T000000Z jammy-updates main universe' \
    'deb [check-valid-until=no] https://snapshot.ubuntu.com/ubuntu/20260928T000000Z jammy-security main universe' \
    > /etc/apt/sources.list

# ── system packages ───────────────────────────────────────────────────────────
# Kernel build deps: bc bison flex libelf-dev libssl-dev libncurses-dev
# Build tools: build-essential git wget curl ca-certificates xz-utils file
# musl cross-compilation: musl-tools (provides musl-gcc wrapper and headers)
# QEMU for smoke tests: qemu-system-x86
# Build script utilities: cpio gzip rsync
RUN apt-get -o Acquire::Retries=3 update -qq && \
    apt-get -o Acquire::Retries=3 install -y --no-install-recommends \
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
        python3 \
        python3-venv \
        socat \
    && rm -rf /var/lib/apt/lists/*

# ── Rust via rustup ───────────────────────────────────────────────────────────
# Install system-wide so all users in the container have access.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

COPY rust-toolchain.toml /opt/vyoma/rust-toolchain.toml
WORKDIR /opt/vyoma
ARG RUSTUP_VERSION=1.27.1
RUN curl --proto '=https' --tlsv1.2 -sSf \
      "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" \
      -o /tmp/rustup-init && \
    echo '6aeece6993e902708983b209d04c0d1dbb14ebb405ddb87def578d41f920f56d  /tmp/rustup-init' | sha256sum -c - && \
    chmod +x /tmp/rustup-init && \
    /tmp/rustup-init -y \
        --no-modify-path \
        --profile minimal \
        --default-toolchain none && \
    rm /tmp/rustup-init && \
    rustup show && \
    rustup default "$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)" && \
    rustc --version && cargo --version

# ── Rust targets ──────────────────────────────────────────────────────────────
# x86_64-unknown-linux-musl  — supervisor binary (statically linked)
# wasm32-wasip2              — WASI Preview 2 apps
RUN rustup target add x86_64-unknown-linux-musl wasm32-wasip2

# Use Rust's default linker and bundled musl CRT (see T0-01 evidence).

# ── allow non-root users to use cargo ────────────────────────────────────────
# The container is run with -u $(id -u):$(id -g) for correct file ownership,
# so the cargo registry and rustup dirs must be world-writable.
RUN chmod -R a+rwX /usr/local/cargo /usr/local/rustup

RUN dpkg-query -W > /opt/vyoma/builder-packages.txt

# ── working directory ─────────────────────────────────────────────────────────
WORKDIR /work

CMD ["/bin/bash"]
