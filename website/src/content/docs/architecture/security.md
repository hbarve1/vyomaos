---
title: Security Architecture
description: Eight-layer defense-in-depth security model of VyomaOS -- from WASM sandbox to capability audit logging.
order: 2
---

# Security Architecture

VyomaOS implements defense-in-depth security through eight layers. Each layer operates independently, so compromising one does not disable the others. Unlike traditional operating systems that bolt on security after the fact, VyomaOS makes isolation the default architectural property.

This document describes each layer, how they interact, and how VyomaOS compares to alternative isolation approaches.

## The eight layers

```
Layer 8: Capability audit log          -- record every access
Layer 7: Per-app firewall rules        -- network-level isolation
Layer 6: 2FA user authentication       -- TOTP-based identity
Layer 5: SHA-256 binary verification   -- supply chain integrity
Layer 4: Mount namespace isolation     -- filesystem separation
Layer 3: seccomp BPF denylist          -- kernel syscall filter
Layer 2: Capability manifests          -- declare-then-use model
Layer 1: WASM sandbox                  -- memory isolation, no raw syscalls
```

## Layer 1: WASM sandbox

The foundation of VyomaOS security is WebAssembly itself. Every application runs inside a WASM sandbox with strong isolation guarantees enforced by the runtime (Wasmtime, WAMR, or wasm3).

### What the sandbox prevents

- **No raw memory access**: WASM uses linear memory with bounds checking. An app cannot read or write memory outside its own address space.
- **No raw syscalls**: WASM has no `syscall` instruction. Apps can only call functions explicitly imported from the host.
- **No pointer arithmetic exploits**: WASM pointers are 32-bit offsets into linear memory, not raw machine addresses. Buffer overflows cannot escape the sandbox.
- **No shared memory between apps**: Each app gets its own linear memory instance. There is no way to reference another app's memory.
- **No arbitrary code execution**: WASM bytecode is validated before execution. Malformed or modified binaries are rejected.

### How it works in VyomaOS

```
App (WASM binary)
  |-- Can call: imported WASI functions (fd_read, fd_write, etc.)
  |-- Cannot call: anything not explicitly imported
  |-- Memory: isolated linear memory, bounds-checked
  |
Wasmtime runtime
  |-- Validates bytecode before execution
  |-- Links only the WASI imports declared in the manifest
  |-- Traps on out-of-bounds memory access
  |
Host OS (Linux kernel)
```

The key insight: even if a WASM app contains a memory corruption bug, the corruption is confined to its own linear memory. The host OS, supervisor, and other apps are unaffected.

## Layer 2: Capability manifests

VyomaOS uses a **declare-then-use** model. Every app has a `vyoma.toml` manifest that lists exactly which system capabilities it needs:

```toml
[app]
name = "my-app"
version = "0.1.0"
wasm = "my-app.wasm"

[capabilities]
stdio = true
filesystem = false
network = false
display = true
shell = false
mouse = false
```

### How the supervisor enforces capabilities

The supervisor does not filter app syscalls. Instead, it only wires up the WASI imports that are declared:

1. Supervisor reads the app's `vyoma.toml`
2. For each capability set to `true`, the corresponding WASI imports are linked
3. For each capability set to `false` (or absent), the imports do not exist

This is fundamentally different from syscall filtering (seccomp, AppArmor, SELinux), where the interface exists and a filter blocks specific calls. In VyomaOS, **undeclared capabilities do not exist at all**.

| Declared capability | What the app gets | What happens without it |
|--------------------|-------------------|------------------------|
| `stdio = true` | stdin/stdout file descriptors | No I/O file descriptors linked |
| `filesystem = true` | `/data` mount via 9P | No filesystem mount exists |
| `network = true` | WASI sockets interface | No socket functions linked |
| `display = true` | VYOMA_DRAW protocol parsed from stdout | Draw commands treated as plain text |
| `shell = true` | `@supervisor:` commands accepted | Shell commands ignored |
| `mouse = true` | Mouse events delivered to stdin | No mouse events routed |

### Peripheral capabilities

On embedded and robotics platforms, hardware peripheral access is also capability-controlled:

```toml
[capabilities.gpio]
pins = [4, 17]
direction = "output"

[capabilities.i2c]
bus = 1
```

The supervisor's `PeripheralRegistry` tracks exclusive access. If app A holds GPIO pin 4, app B cannot claim it -- the supervisor rejects the request at spawn time.

## Layer 3: seccomp BPF denylist

Even though WASM apps cannot make raw syscalls, the Wasmtime runtime itself is a native Linux process that can. VyomaOS applies a seccomp BPF (Berkeley Packet Filter) denylist to every Wasmtime child process.

### What seccomp blocks

The denylist prevents the runtime process from making dangerous syscalls that no WASM execution should ever need:

| Blocked syscall | Reason |
|----------------|--------|
| `ptrace` | Prevents debugging/tracing other processes |
| `mount` / `umount` | Prevents filesystem topology changes |
| `reboot` | Prevents system shutdown from app context |
| `swapon` / `swapoff` | Prevents memory management interference |
| `init_module` / `delete_module` | Prevents kernel module loading |
| `kexec_load` | Prevents kernel replacement |
| `pivot_root` | Prevents root filesystem changes |

### How it works

```
Supervisor (PID 1)
  |-- Spawns wasmtime process for each app
  |-- Applies seccomp BPF filter before exec
  |-- Filter denies dangerous syscalls with EPERM
  |
Wasmtime process (child)
  |-- Cannot ptrace, mount, reboot, load modules
  |-- Can still do normal I/O (read, write, mmap)
  |-- Executes WASM bytecode within these constraints
```

This layer is a defense against runtime exploits. Even if an attacker discovers a vulnerability in Wasmtime itself and achieves native code execution, the seccomp filter prevents the most destructive actions.

## Layer 4: Mount namespace isolation

Each Wasmtime child process runs in its own Linux mount namespace. This provides filesystem-level isolation beyond what WASI capabilities enforce.

### What namespace isolation provides

- **Separate filesystem view**: Each app sees only its own mounts, not the host's full filesystem
- **Read-only rootfs**: The base filesystem is mounted read-only for app processes
- **Restricted `/data` access**: Only apps with `filesystem = true` have `/data` mounted in their namespace
- **No `/proc` or `/sys` snooping**: Sensitive kernel interfaces are not mounted in app namespaces

### Isolation hierarchy

```
Host filesystem
  |
Supervisor namespace (PID 1)
  |-- Full filesystem access
  |-- /dev/fb0, /dev/dri (display)
  |-- /data (9P mount)
  |
App namespace (per process)
  |-- Read-only rootfs view
  |-- /data only if filesystem = true
  |-- No /proc, /sys, /dev access
```

## Layer 5: SHA-256 binary verification

VyomaOS verifies the integrity of all system binaries using SHA-256 checksums. This protects against supply chain attacks and binary tampering.

### What is verified

| Component | When verified | Action on mismatch |
|-----------|--------------|-------------------|
| Wasmtime binary | At build time (Dockerfile) | Build fails |
| BusyBox binary | At build time (Dockerfile) | Build fails |
| WASM app binaries | At supervisor startup | App refused to load |
| Signed app bundles | At install time (package manager) | Install rejected |

### How app verification works

When the supervisor loads an app, it computes the SHA-256 hash of the `.wasm` file and compares it against the hash recorded in the app's signed bundle manifest:

```
Supervisor loads app
  |-- Reads my-app.wasm from /opt/vyoma/apps/my-app/
  |-- Computes SHA-256 of the binary
  |-- Compares against signed manifest hash
  |-- Match: app is spawned
  |-- Mismatch: app is rejected, error logged
```

This means even if an attacker gains write access to the app directory, they cannot substitute a modified binary without the supervisor detecting it.

## Layer 6: 2FA user authentication

VyomaOS supports TOTP-based two-factor authentication for user sessions. This prevents unauthorized access even if a password is compromised.

### How 2FA works in VyomaOS

1. User creates an account with a password
2. User enrolls a TOTP secret (compatible with Google Authenticator, Authy, etc.)
3. At login, user provides password + 6-digit TOTP code
4. Supervisor verifies both factors before granting session access
5. Failed attempts are rate-limited and logged

### What 2FA protects

| Action | Requires 2FA |
|--------|-------------|
| Local console login | Yes (if enabled) |
| SSH session | Yes (if enabled) |
| Package installation | Configurable |
| System settings changes | Configurable |
| App launch | No (per-session, not per-app) |

2FA is optional but recommended. On multi-user systems, it prevents one user from impersonating another.

## Layer 7: Per-app firewall rules

Apps with `network = true` do not get unrestricted network access. The supervisor enforces per-app firewall rules that limit which ports, protocols, and destinations each app can use.

### Firewall configuration

Network rules are declared in the app manifest:

```toml
[capabilities]
network = true

[capabilities.network]
ports = [8080]
protocols = ["tcp"]
```

### What the firewall controls

| Rule | Effect |
|------|--------|
| `ports` | App can only bind/connect on listed ports |
| `protocols` | App can only use listed protocols (tcp, udp) |
| Egress filtering | Outbound connections restricted by manifest |
| Rate limiting | Configurable per-app bandwidth limits |

An app that declares `network = true` with `ports = [8080]` can serve HTTP on port 8080 but cannot scan other ports, connect to arbitrary external hosts (unless explicitly allowed), or use UDP.

## Layer 8: Capability audit log

Every capability access is logged by the supervisor's audit subsystem. This creates a complete record of what each app accessed and when.

### What is logged

| Event | Log entry includes |
|-------|-------------------|
| App spawn | App name, capabilities granted, PID, timestamp |
| File access | App name, path, operation (read/write), timestamp |
| Network connection | App name, destination, port, protocol, timestamp |
| IPC message | Sender, recipient, message hash, timestamp |
| Capability denial | App name, requested capability, reason, timestamp |
| App exit | App name, exit code, runtime duration, timestamp |

### Log format

Audit logs use structured JSON-line format for machine parsing:

```json
{"ts":"2026-05-31T10:00:01Z","event":"spawn","app":"http-server","caps":["stdio","network"],"pid":42}
{"ts":"2026-05-31T10:00:02Z","event":"net_bind","app":"http-server","port":8080,"proto":"tcp"}
{"ts":"2026-05-31T10:00:05Z","event":"ipc_send","from":"ping","to":"pong","hash":"a1b2c3"}
{"ts":"2026-05-31T10:00:10Z","event":"cap_deny","app":"rogue-app","cap":"network","reason":"not declared"}
```

### Why audit logging matters

- **Forensics**: After a security incident, determine exactly what happened
- **Compliance**: Demonstrate that apps only accessed declared capabilities
- **Debugging**: Trace IPC message flows and capability denials
- **Monitoring**: Alert on unusual patterns (unexpected capability requests, high IPC volume)

## How the layers work together

Consider what happens when a malicious app tries to exfiltrate data:

```
1. App tries to access /etc/passwd
   -> Layer 4 (namespace): /etc/passwd not visible in app namespace
   -> Layer 2 (capability): filesystem not declared, no /data mount
   -> Layer 1 (WASM): no raw filesystem syscalls available

2. App tries to open a network socket
   -> Layer 2 (capability): network not declared, no socket imports
   -> Layer 7 (firewall): even if network were declared, port restricted
   -> Layer 1 (WASM): no socket syscalls available

3. App tries to exploit Wasmtime to get native execution
   -> Layer 3 (seccomp): ptrace, mount, reboot blocked
   -> Layer 4 (namespace): limited filesystem view
   -> Layer 5 (verification): modified binary would be detected on reload

4. Attacker tries to replace the app binary on disk
   -> Layer 5 (SHA-256): hash mismatch detected at load time
   -> Layer 8 (audit): replacement attempt logged

5. Every step above is recorded
   -> Layer 8 (audit): complete timeline available for forensics
```

## Comparison with other isolation approaches

| Property | VyomaOS | Docker | Traditional VM | Standard Linux |
|----------|---------|--------|---------------|----------------|
| Memory isolation | WASM linear memory | Process-level | Hardware (hypervisor) | Process-level |
| Syscall filtering | Capability model + seccomp | seccomp profiles | N/A (full OS) | Optional (AppArmor/SELinux) |
| Filesystem isolation | Namespace + capability | Union filesystem | Separate disk image | chroot (weak) |
| Binary verification | SHA-256 per-app | Image digest | N/A | Package manager GPG |
| Network isolation | Per-app firewall | Network namespaces | Virtual NICs | iptables |
| Default security | Deny-all (nothing exists) | Allow-most (filter some) | Full OS (attack surface) | Allow-all (add filters) |
| Binary size overhead | 1-10 KB per app | 50-500 MB per image | 1-10 GB per VM | N/A |
| Startup time | Milliseconds | Seconds | Minutes | N/A |
| Cross-platform | 6 platforms from 1 binary | x86/ARM (rebuild per arch) | Per-arch VM image | Per-arch binary |
| Audit logging | Built-in, per-capability | External (auditd) | Per-VM, external | External (auditd) |

### Key advantages over containers

Docker containers share the host kernel and have a large attack surface (the full Linux syscall table is available unless filtered). VyomaOS apps have no syscall table -- they can only call explicitly imported WASI functions. A container escape gives access to the host OS; a WASM escape gives access to a seccomp-filtered, namespace-isolated process with no sensitive mounts.

### Key advantages over VMs

Virtual machines provide strong isolation through hardware-enforced boundaries, but at enormous cost: each VM runs a full OS kernel, consumes gigabytes of RAM, and takes minutes to boot. VyomaOS apps boot in milliseconds, use kilobytes of space, and achieve meaningful isolation through the WASM sandbox without the overhead.

### Key advantages over traditional Linux

Standard Linux applications run with the full privileges of their user account. Security must be added through external tools (AppArmor profiles, SELinux policies, seccomp filters, network namespaces). Each tool has its own configuration format, failure modes, and maintenance burden. VyomaOS makes isolation the default -- you opt in to capabilities rather than opting out of permissions.

## Security configuration

### Enabling 2FA

```bash
# Inside VyomaOS shell
@supervisor: security enable-2fa
# Follow the prompts to scan the QR code with your authenticator app
```

### Viewing audit logs

```bash
# Inside VyomaOS shell
@supervisor: audit tail 50     # Last 50 audit events
@supervisor: audit app my-app  # Events for a specific app
@supervisor: audit denials     # Only capability denials
```

### Verifying binary integrity

```bash
# Verify all loaded app binaries
@supervisor: verify all

# Verify a specific app
@supervisor: verify http-server
```

## Threat model

VyomaOS is designed to be secure against:

- **Malicious apps**: Cannot escape WASM sandbox, access undeclared capabilities, or interfere with other apps
- **Supply chain attacks**: SHA-256 verification detects binary tampering
- **Privilege escalation**: No root access from app context, seccomp blocks dangerous syscalls
- **Network attacks**: Per-app firewall rules limit exposure
- **Physical access**: 2FA prevents unauthorized login even with console access

VyomaOS does **not** defend against:

- **Kernel exploits**: The Linux kernel is trusted. A kernel vulnerability could compromise everything.
- **Hardware attacks**: Side-channel attacks (Spectre, Meltdown) are out of scope for the WASM sandbox.
- **Supervisor bugs**: The supervisor is trusted code. A bug in the supervisor could bypass all layers.

These are explicit trust boundaries. The kernel and supervisor are the trusted computing base (TCB). Minimizing their size (2.3 MB kernel, ~2.9 MB supervisor) reduces the attack surface compared to a general-purpose Linux distribution.
