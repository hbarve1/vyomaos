---
title: Contributing
description: How to contribute to VyomaOS.
---

VyomaOS is an open research project actively looking for contributors. The foundation is solid and the surface area for new work is large.

## Getting Started

```bash
# Clone and build
git clone https://github.com/hbarve1/vyomaos.git
cd vyomaos
make build

# Boot and verify
make run          # headless
make run-gui DISPLAY_BACKEND=cocoa   # macOS with GUI
```

## Good First Contributions

### Write a New WASM App

Each app is an independent Rust crate in `apps/`. You don't need to understand the full codebase.

1. `mkdir apps/your-app`
2. Create `Cargo.toml`, `src/main.rs`, and `vyoma.toml`
3. Build: `cargo build --target wasm32-wasip2 --release`
4. See the [Creating an App](/guides/creating-an-app/) guide for details

### Improve an Existing App

Browse `apps/` and pick one that interests you. Better UX, new features, bug fixes — all welcome.

### Supervisor Improvements

- New `@supervisor:` IPC commands
- New capability types
- Display system enhancements
- Performance optimizations

### Documentation

- Architecture writeups
- Tutorials for app development
- Diagrams and visualizations

### Testing

- New unit tests in `supervisor/tests/`
- Boot smoke test improvements
- CI pipeline enhancements

## Code Guidelines

### 500-Line Rule

No source file may exceed 500 lines. When a file approaches this limit, split it into focused submodules. This applies to all `.rs` files across the repo.

### Dependencies

Keep WASM app dependencies minimal — every dependency increases binary size. The goal is 1–10 KB per app.

### Manifests

Only declare capabilities your app actually uses. The security model relies on minimal capability declarations.

### Testing

```bash
make test             # Full CI: build + unit test + smoke
make unit-test        # Cargo test in Docker
make check-manifests  # Validate all vyoma.toml files
make smoke            # Headless QEMU boot test
```

## Project Structure

```
vyomaos/
├── apps/              # WASM applications (200+)
│   └── my-app/
│       ├── Cargo.toml
│       ├── vyoma.toml  # Capability manifest
│       └── src/
│           └── main.rs
├── supervisor/        # Rust PID 1 supervisor
│   ├── src/
│   │   ├── main.rs
│   │   ├── chrome.rs
│   │   ├── display/
│   │   └── ...
│   └── tests/
├── base/              # Kernel config + rootfs scripts
├── docker/            # Build environment Dockerfile
├── docs/              # Documentation + presentation
└── Makefile           # Build orchestration
```

## Communication

- **Issues**: Open a GitHub issue to discuss features, bugs, or design questions
- **Pull Requests**: Fork, branch, submit — standard GitHub workflow
- **Discussions**: Use GitHub Discussions for broader design conversations
