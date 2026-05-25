---
title: IPC Broker
description: Inter-process communication between WASM apps.
---

VyomaOS apps don't communicate directly. All messages flow through the supervisor's IPC broker, which intercepts stdout writes and routes them to the appropriate target.

## Message Format

```
@<target>: <message>
```

Apps write this to stdout. The supervisor strips the `@sender:` prefix before delivering to the target app's stdin.

## Routing Modes

### Unicast

Send a message to a specific app by name:

```rust
println!("@calculator: compute 2+2");
```

The target app receives `compute 2+2` on stdin.

### Broadcast

Send a message to all running apps:

```rust
println!("@broadcast: system shutting down");
```

Every running app receives `system shutting down` on stdin.

### Reply

Reply to the last app that sent you a message:

```rust
println!("@reply: acknowledged");
```

The supervisor tracks the last sender per-app and routes the reply back.

### Supervisor Commands

Send commands to the supervisor itself:

```rust
println!("@supervisor: list");
println!("@supervisor: ping");
println!("@supervisor: uptime");
println!("@supervisor: version");
println!("@supervisor: apps");
println!("@supervisor: focus shell");
println!("@supervisor: loglevel debug");
println!("@supervisor: kill old-app");
```

## Supervisor Command Reference

| Command | Response | Description |
|---------|----------|-------------|
| `list` | App name list | List all running apps |
| `apps` | App name list | Alias for `list` |
| `ping` | `pong <timestamp>` | Health check |
| `uptime` | Duration string | System uptime since boot |
| `version` | Version string | VyomaOS version |
| `focus <name>` | (none) | Switch keyboard focus to named app |
| `loglevel <level>` | (none) | Set per-app log level filter |
| `kill <name>` | (none) | Terminate the named app |

## Receiving Messages

Apps receive IPC messages on stdin, one line at a time:

```rust
use std::io::BufRead;

fn main() {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        if let Ok(msg) = line {
            let msg = msg.trim();
            if msg.starts_with("VYOMA_INPUT:key:") {
                // Keyboard input
                let key = &msg[16..];
                handle_key(key);
            } else {
                // IPC message from another app
                handle_ipc(msg);
            }
        }
    }
}
```

## Design Decisions

**Why supervisor-mediated IPC?**

- **Security**: The supervisor can log, audit, and rate-limit messages
- **Simplicity**: No shared memory, no sockets between apps, no complex synchronization
- **Debuggability**: All messages are visible in supervisor logs
- **Capability enforcement**: Only apps with `stdio = true` can participate in IPC

**Why stdout/stdin?**

- WASI Preview 2 guarantees stdin/stdout availability
- No custom host functions or WASI extensions needed
- Any language that can print a string can do IPC
- Easy to test apps outside VyomaOS (just pipe text)

## Limitations

- Messages are single-line only (no embedded newlines)
- The target app must be running for delivery
- No message persistence or queuing — if the target is dead, the message is dropped
- No request/response correlation — apps must implement their own sequencing
