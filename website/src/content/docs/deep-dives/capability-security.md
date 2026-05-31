---
title: "Capability-Based Security: From Theory to Implementation"
description: A deep technical analysis of capability-based security in VyomaOS — from historical capability systems like KeyKOS and EROS to the practical implementation using WASM sandboxing and vyoma.toml manifests.
order: 3
---

# Capability-Based Security: From Theory to Implementation

Most operating systems use access control lists to determine what a program can do. A file has permissions. A user has a role. A process runs with the identity of the user who launched it. The system checks: does this identity have permission to access this resource?

VyomaOS inverts this model. Instead of checking identity against a permission database, VyomaOS gives each application exactly the capabilities it declares — and nothing else. An app that does not declare network access cannot open a socket. Not because a firewall blocks it. Not because a security policy denies it. Because the network interface does not exist in the app's sandbox.

This is capability-based security. It has a fifty-year academic history, and VyomaOS is one of the first practical operating systems to implement it using WebAssembly as the enforcement mechanism.

---

## A Brief History of Capability Systems

### The Idea (1966)

Dennis and Van Horn introduced the concept of a capability in their 1966 paper "Programming Semantics for Multiprogrammed Computations." A capability is an unforgeable token that grants its holder specific access to a specific resource. If you hold a capability to read a file, you can read that file. If you do not hold the capability, you cannot — regardless of your identity or role.

The key insight: capabilities couple designation (naming a resource) with authority (having access to it). In traditional systems, these are separate — you can name a file (`/etc/shadow`) without having authority to read it. This separation is the root cause of the confused deputy problem.

### The Confused Deputy Problem (1988)

Hardy's "confused deputy" paper formalized a fundamental flaw in identity-based access control. Consider a compiler that writes output files on behalf of its users. The compiler runs with its own identity, which has write access to its billing log. A malicious user tells the compiler to write output to the billing log's path. The compiler, confused about which authority it is exercising (its own vs. the user's), overwrites the billing log.

In a capability system, this cannot happen. The user passes the compiler a capability to write to a specific output file. The compiler cannot use this capability to write to the billing log, because the capability designates a different resource. There is no confusion about authority because authority is tied to the capability, not to the process's identity.

### KeyKOS (1983) and EROS (1999)

KeyKOS was the first production capability operating system, deployed on IBM System/370 mainframes. Every resource — memory pages, I/O ports, process contexts — was accessed through capabilities called "keys." A process could only access resources it held keys for, and keys could not be forged.

EROS (Extremely Reliable Operating System) refined KeyKOS for modern hardware with persistent capabilities, confinement mechanisms, and formal verification of security properties. EROS proved that capability systems could be implemented efficiently on commodity hardware.

### seL4 (2009)

seL4 is a formally verified microkernel that uses capabilities for all resource access. Every system call takes a capability as an argument. The kernel maintains a capability derivation tree that tracks how capabilities are created, copied, and revoked. seL4's formal verification proves that its capability enforcement is correct — not just tested, but mathematically proven.

seL4 demonstrated that capability-based security is compatible with high performance. Its IPC performance matches or exceeds non-capability microkernels.

### The Gap

Despite fifty years of research and multiple working implementations, capability-based security has not reached mainstream operating systems. Linux, Windows, and macOS all use identity-based access control. The reason is pragmatic: retrofitting capabilities onto an existing POSIX system requires rewriting every application to pass capabilities explicitly. The ecosystem momentum of traditional access control is enormous.

VyomaOS avoids this problem by starting fresh with WebAssembly, which is capability-based by design.

---

## The Capability Model in VyomaOS

### The Core Principle

Every VyomaOS application is a WebAssembly module. WebAssembly modules start with zero capabilities. They cannot access the filesystem, the network, the display, or even stdin/stdout unless the host runtime explicitly provides these interfaces.

The VyomaOS supervisor reads each app's `vyoma.toml` manifest at boot time. The manifest declares which capabilities the app needs. The supervisor provides exactly those capabilities — no more, no less.

```toml
# apps/http-server/vyoma.toml
[app]
name    = "http-server"
version = "0.1.0"
wasm    = "http-server.wasm"

[capabilities]
stdio      = true
network    = true
filesystem = true
```

This manifest declares that `http-server` needs stdio (for logging), network access (to serve HTTP), and filesystem access (to read files from `/data`). The supervisor grants these three capabilities. The app does not receive display access, shell access, mouse input, or any hardware peripheral access.

### What "No Access" Means

In traditional systems, "no access" means a syscall returns `EPERM` or `EACCES`. The resource exists, the interface exists, but the policy denies the operation. This is security by denial — the mechanism exists, and a policy layer prevents its use.

In VyomaOS, "no access" means the interface does not exist. When the supervisor configures Wasmtime for an app without `network = true`, it does not wire up the `wasi:sockets/tcp` interface. If the WASM module tries to call a socket function, there is no function to call. The WebAssembly validator rejects the module at load time if it imports an interface the supervisor did not provide.

```
Traditional OS:
  app calls socket() → kernel checks permissions → returns EPERM

VyomaOS:
  app tries to import wasi:sockets/tcp → Wasmtime finds no such import → module fails to load
  
  OR: app does not import wasi:sockets/tcp → socket functions do not exist in the module's address space
```

This distinction matters for security. In a traditional OS, the socket syscall mechanism exists in the kernel, and a kernel vulnerability could allow bypassing the permission check. In VyomaOS, there is no mechanism to bypass — the interface is not present. An app without network capabilities cannot open a socket any more than it can call a function that does not exist.

### The Manifest as a Capability Declaration

The `vyoma.toml` manifest serves as the capability declaration for each app. The supervisor parses it at boot time and uses it to configure the Wasmtime runtime:

| Manifest field | WASI interface provided | Host resource |
|---------------|------------------------|---------------|
| `stdio = true` | `wasi:io/streams` | Supervisor-mediated stdin/stdout pipes |
| `filesystem = true` | `wasi:filesystem/types` | `/data` directory (9P mount) |
| `network = true` | `wasi:sockets/tcp` | virtio-net device |
| `display = true` | stdout + `VYOMA_DRAW:` protocol | `/dev/fb0` framebuffer |
| `shell = true` | stdout + `@supervisor:` commands | Supervisor command interface |
| `mouse = true` | stdin + `VYOMA_INPUT:mouse:` events | Mouse event stream |

Capabilities not listed in the manifest are not provided. There is no default set of capabilities. There is no "root" mode that grants all capabilities. Every app starts with nothing and receives only what it declares.

---

## Comparison with Traditional Security Models

### vs. Discretionary Access Control (DAC)

DAC (Unix file permissions, Windows ACLs) associates permissions with resources and identities. A file has an owner, a group, and permission bits. A process runs with a user identity. The kernel checks: does this user's identity match the resource's permissions?

**The confused deputy problem is inherent to DAC.** A setuid program runs with elevated privileges. If it can be tricked into accessing a resource on behalf of an unprivileged user, the elevated privileges apply to the attacker's request. `sudo`, `su`, `passwd`, and `mount` have all had confused deputy vulnerabilities.

VyomaOS has no user identities. Apps do not run "as" a user. They hold capabilities. A capability to write to `/data/app-config.toml` does not grant write access to `/data/other-app-config.toml`. There is no identity to confuse.

```
DAC:
  Process P runs as user U
  File F is owned by user U with mode 0644
  Kernel checks: U matches F.owner → allow read/write
  
  Problem: P can read/write ANY file owned by U, not just the ones it needs

Capability:
  Process P holds capability C for /data/app-config.toml
  C grants read/write to exactly one path
  No identity check — the capability IS the authority
```

### vs. Mandatory Access Control (MAC)

MAC (SELinux, AppArmor) layers a system-wide policy on top of DAC. A central policy file defines which processes can access which resources, regardless of DAC permissions. SELinux policies are typically thousands of lines of a domain-specific policy language:

```
# SELinux policy fragment — this is what real-world MAC looks like
allow httpd_t httpd_sys_content_t:file { read open getattr };
allow httpd_t httpd_log_t:file { write create append };
allow httpd_t self:tcp_socket { create accept listen bind };
dontaudit httpd_t shadow_t:file { read };
```

MAC solves the confused deputy problem but introduces its own: **policy complexity**. SELinux policies for a typical Linux distribution are 100,000+ lines. Writing correct policies requires deep expertise. Incorrect policies either break applications (too restrictive) or leave security gaps (too permissive). Most users and many administrators disable SELinux entirely because the policy language is too complex to manage.

VyomaOS replaces this with a manifest that fits in 10 lines:

```toml
[capabilities]
stdio      = true
network    = true
filesystem = true
```

There is no policy language. There is no central policy file. Each app declares its own capabilities. The supervisor enforces them. The total "policy" for the entire system is the set of `vyoma.toml` files across all apps — each one simple enough to audit in seconds.

### vs. Containers (Docker, Podman)

Containers use Linux namespaces and cgroups to isolate processes. A container has its own filesystem view, network stack, PID namespace, and user namespace. This provides strong isolation, but it relies on the Linux kernel's namespace implementation being correct.

The container security model has a fundamental weakness: **the kernel is shared.** Every container runs on the same kernel. A kernel vulnerability that allows namespace escape compromises every container on the host. Container escape vulnerabilities have been discovered regularly:

- CVE-2019-5736 (runc): malicious container overwrites the host runc binary
- CVE-2020-15257 (containerd): abstract Unix socket allows container escape
- CVE-2022-0185 (kernel): heap overflow in filesystem context API allows namespace escape

VyomaOS does not use containers. WASM apps do not share a kernel interface — they interact with the supervisor through WASI interfaces, and the supervisor is the only process that talks to the kernel. A vulnerability in Wasmtime's WASM execution could potentially escape the sandbox, but the attack surface is dramatically smaller than the entire Linux kernel syscall interface (300+ syscalls vs. a handful of WASI functions).

```
Container:
  App → Linux kernel (300+ syscalls) → hardware
  Attack surface: entire kernel syscall table
  
VyomaOS:
  WASM app → WASI interface (5-10 functions) → Supervisor → Linux kernel → hardware
  Attack surface: WASI interface + Wasmtime runtime
```

### vs. Seccomp-BPF

Seccomp-BPF allows a process to install a BPF program that filters its own syscalls. Chrome, Firefox, and Docker use seccomp to restrict the syscalls available to sandboxed processes.

Seccomp is a deny-list approach: you start with all syscalls available and block the dangerous ones. This is error-prone — you must enumerate every dangerous syscall. Miss one, and the sandbox has a hole. New kernel versions add new syscalls that may not be in the deny list.

VyomaOS uses an allow-list approach: apps start with no capabilities and receive only what they declare. New kernel features do not create new attack surface because apps never interact with the kernel directly.

VyomaOS does also apply seccomp-BPF to the Wasmtime child processes as a defense-in-depth measure. But seccomp is the backup mechanism, not the primary security boundary. The primary boundary is the WASM sandbox itself.

---

## Concrete Examples

### What Happens When an App Tries Undeclared Access

Consider a malicious or buggy app that tries to access the network without declaring `network = true` in its manifest.

**Scenario 1: The app imports `wasi:sockets/tcp`**

The WASM binary contains an import declaration for `wasi:sockets/tcp`. When the supervisor loads the module into Wasmtime, it does not provide this interface (because the manifest does not declare `network = true`). Wasmtime's module instantiation fails with an "unresolved import" error. The app never starts.

```
[lifecycle] app "malicious-app" failed to instantiate: 
  import `wasi:sockets/tcp` has no matching export
```

**Scenario 2: The app does not import network interfaces but tries to access the network via raw memory manipulation**

The app cannot do this. WebAssembly's linear memory is isolated from host memory. The app's memory is a contiguous byte array that the runtime manages. The app cannot read or write memory outside this array. There is no way to construct a file descriptor, a socket, or a kernel data structure by manipulating memory — these concepts do not exist in the WASM execution model.

**Scenario 3: The app exploits a Wasmtime bug to escape the sandbox**

This is the theoretical risk of any sandbox. Wasmtime is written in Rust (memory-safe), extensively fuzzed, and has had formal verification applied to its compiler. It has a strong security track record. But no software is bug-free. VyomaOS adds defense-in-depth:

- Seccomp-BPF on the Wasmtime process restricts available syscalls
- Linux namespaces isolate the Wasmtime process from the host
- The supervisor runs with minimal kernel capabilities (no `CAP_SYS_ADMIN`)

### Capability Enforcement for Hardware Peripherals

On embedded and robotics platforms, VyomaOS extends capability enforcement to hardware peripherals:

```toml
# A sensor-reading app on the robotics platform
[app]
name = "temperature-sensor"
version = "0.1.0"
wasm = "temperature-sensor.wasm"

[capabilities]
stdio = true

[capabilities.i2c]
bus = 1

[capabilities.gpio]
pins = [4]
direction = "input"
```

This app can read from I2C bus 1 and GPIO pin 4 (input only). It cannot:

- Access any other I2C bus
- Access any GPIO pin other than 4
- Set GPIO pin 4 to output mode
- Access SPI, UART, or ADC peripherals
- Access the network, filesystem, or display

The supervisor's `PeripheralRegistry` tracks exclusive access to hardware peripherals. If two apps declare the same GPIO pin, the supervisor detects the conflict at boot time and refuses to start the second app. This prevents hardware contention bugs that are common in embedded systems where multiple processes fight over shared I/O pins.

```
[capability] app "temperature-sensor" granted exclusive access: I2C bus 1, GPIO pin 4 (input)
[capability] app "motor-controller" denied: GPIO pin 4 already claimed by "temperature-sensor"
```

### IPC as a Controlled Capability

Inter-process communication in VyomaOS is mediated by the supervisor. Apps cannot communicate directly — they write messages to stdout in the format `@<target>: <message>`, and the supervisor routes them.

This means the supervisor can:

- **Audit all IPC traffic**: every message passes through the supervisor and can be logged
- **Rate-limit IPC**: prevent denial-of-service via message flooding
- **Enforce communication policies**: restrict which apps can talk to each other (not yet implemented, but architecturally possible)
- **Prevent information leaks**: an app cannot read another app's memory or file descriptors because these are isolated by the WASM sandbox

```rust
// App A sends a message to App B
println!("@appB: sensor reading: 23.5");

// Supervisor intercepts, routes to App B's stdin
// App B reads from stdin
let mut line = String::new();
std::io::stdin().read_line(&mut line).unwrap();
// line contains: "sensor reading: 23.5"
```

App A cannot forge the sender identity (the supervisor knows which app wrote to stdout). App B cannot read App A's messages unless A explicitly addresses them to B. The communication channel is the capability — if App A does not know App B's name, it cannot send messages to it.

---

## The Manifest Lifecycle

### Boot-Time Validation

When the VyomaOS supervisor starts, it reads `boot.toml` to discover which apps to launch. For each app, it:

1. Reads the app's `vyoma.toml` manifest
2. Validates the manifest against the schema (unknown fields are rejected)
3. Checks for peripheral conflicts (exclusive access violations)
4. Configures Wasmtime with only the declared WASI interfaces
5. Spawns the app with the configured runtime

If a manifest is invalid — malformed TOML, unknown capability fields, conflicting peripheral claims — the supervisor logs an error and skips the app. Other apps continue to boot normally.

```
[manifest] apps/broken-app/vyoma.toml: ERROR unknown field "nework" (did you mean "network"?)
[manifest] apps/broken-app: skipped due to manifest errors
[lifecycle] apps/hello-world: started (capabilities: stdio)
[lifecycle] apps/http-server: started (capabilities: stdio, network, filesystem)
```

### No Runtime Escalation

Once an app is running, its capabilities are fixed. There is no mechanism for an app to request additional capabilities at runtime. There is no `escalate_privileges()` call. There is no `sudo` equivalent. The capabilities declared in the manifest at boot time are the capabilities the app has for its entire lifetime.

This is a deliberate design decision. Runtime capability escalation introduces the risk of social engineering attacks (tricking the user into granting permissions) and confused deputy problems (tricking a privileged service into granting capabilities). By making capabilities immutable after boot, VyomaOS eliminates these attack vectors entirely.

### Manifest Auditing

Because every app's capabilities are declared in a plain-text TOML file, auditing the security posture of the entire system requires reading a set of small, human-readable files:

```bash
# Audit all capabilities in the system
for manifest in apps/*/vyoma.toml; do
    echo "=== $(dirname $manifest | xargs basename) ==="
    grep -A 20 '\[capabilities\]' "$manifest"
    echo
done
```

Compare this to auditing SELinux policies (100,000+ lines of policy language), Docker container configurations (Dockerfiles, compose files, runtime flags, kernel capabilities), or traditional Unix permissions (recursive `ls -la` across the entire filesystem).

---

## Limitations and Honest Assessment

### Coarse-Grained Filesystem Access

VyomaOS currently grants filesystem access as a binary: `filesystem = true` gives an app access to the entire `/data` directory. There is no per-file or per-directory capability. This means an app with filesystem access can read and write any file in `/data`, including files created by other apps.

Future work will introduce fine-grained filesystem capabilities:

```toml
# Future: per-directory capabilities
[capabilities.filesystem]
read  = ["/data/my-app/config"]
write = ["/data/my-app/state"]
```

### No Runtime Capability Discovery

Apps cannot query their own capabilities at runtime. An app with `network = true` cannot ask "which ports am I allowed to bind?" — it tries to bind a port, and either succeeds or fails. This is a limitation of the current WASI interface, which does not include capability introspection.

### Trust in the Supervisor

The entire capability model depends on the supervisor being correct. If the supervisor has a bug that grants capabilities not declared in the manifest, the security model breaks. This is mitigated by:

- Writing the supervisor in Rust (memory safety by construction)
- Keeping the supervisor codebase modular (62+ modules, each under 500 lines)
- Unit testing manifest parsing and capability enforcement
- The supervisor being the only component that needs to be trusted — apps are untrusted by design

### Trust in Wasmtime

The WASM sandbox is enforced by Wasmtime (or wasm3/WAMR on embedded platforms). A vulnerability in the runtime could allow sandbox escape. This is a real risk, but the attack surface is far smaller than the Linux kernel syscall interface. Wasmtime is written in Rust, heavily fuzzed, and has had components formally verified. It has a strong security track record with zero known sandbox escape vulnerabilities in production.

---

## Why This Approach Works for VyomaOS

Capability-based security has been theoretically superior to identity-based access control for fifty years. It has not been adopted because retrofitting capabilities onto existing systems is impractical — every application must be rewritten to pass capabilities explicitly.

VyomaOS avoids this problem because it does not retrofit capabilities onto an existing system. It starts fresh with an application model (WebAssembly) that is capability-based by design. WASM modules start with nothing. The supervisor grants capabilities based on manifests. There is no legacy code to accommodate, no POSIX compatibility to maintain, no existing applications that expect ambient authority.

The result is a security model that is:

- **Simple**: capabilities declared in 10-line TOML files, not 100,000-line policy databases
- **Auditable**: every app's authority is visible in its manifest
- **Enforceable**: the WASM sandbox makes capability violations impossible, not just detectable
- **Composable**: new capabilities (hardware peripherals, IPC policies, fine-grained filesystem access) can be added without changing the enforcement mechanism

This is what capability security looks like when you can design the system from scratch instead of bolting it onto fifty years of accumulated design decisions.
