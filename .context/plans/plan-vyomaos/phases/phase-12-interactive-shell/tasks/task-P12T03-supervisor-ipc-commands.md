# P12T03 — @supervisor: IPC Command Handler

## Phase
Phase 12 — Interactive Shell

## Goal
Extend the IPC router to intercept messages addressed to the special `supervisor` target name. Handle a small set of privileged commands that allow WASM apps to inspect and control the running system.

## Files to modify

```
supervisor/src/main.rs    — extend route_or_print / handle @supervisor: messages
```

## Commands

| Command | Action |
|---------|--------|
| `@supervisor: list` | Sends newline-separated app names back to the requesting app's stdin |
| `@supervisor: run /apps/<name>/vyoma.toml` | Spawns a new app instance (requires implementing on-demand spawn outside boot phase) |
| `@supervisor: kill <name>` | Sends SIGTERM to a running app |
| `@supervisor: focus <name>` | Changes keyboard input focus |
| `@supervisor: status` | Sends back JSON summary to requesting app's stdin |

## Implementation sketch

```rust
fn route_or_print(line: &str, sender: &str, inbox: &Inbox,
                  has_display: bool, focused: &FocusedApp) {
    // ... existing VYOMA_DRAW handling ...

    if let Some(rest) = line.strip_prefix('@') {
        if let Some((target, msg)) = rest.split_once(": ") {
            if target == "supervisor" {
                handle_supervisor_command(msg, sender, inbox, focused);
                return;
            }
            // ... existing IPC routing ...
        }
    }
    println!("[{sender}] {line}");
}

fn handle_supervisor_command(cmd: &str, sender: &str, inbox: &Inbox, focused: &FocusedApp) {
    let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
    match parts[0] {
        "list" => {
            let names: Vec<String> = inbox.lock().unwrap().keys().cloned().collect();
            let reply = names.join("\n");
            if let Some(tx) = inbox.lock().unwrap().get(sender) {
                let _ = tx.send(reply);
            }
        }
        "focus" => {
            if let Some(name) = parts.get(1) {
                *focused.lock().unwrap() = Some(name.to_string());
            }
        }
        "status" => {
            let count = inbox.lock().unwrap().len();
            let reply = format!(r#"{{"running":{count}}}"#);
            if let Some(tx) = inbox.lock().unwrap().get(sender) {
                let _ = tx.send(reply);
            }
        }
        _ => eprintln!("vyoma-supervisor: unknown command from {sender}: {cmd}"),
    }
}
```

## Notes

- `run` and `kill` commands require on-demand app spawning outside the boot phase — significant new supervisor logic, defer to a sub-task
- The `supervisor` target name is reserved and cannot be used as an app name (enforce in boot config validation)
