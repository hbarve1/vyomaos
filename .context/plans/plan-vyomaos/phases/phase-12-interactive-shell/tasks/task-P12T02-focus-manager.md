# P12T02 — Focus Manager

## Phase
Phase 12 — Interactive Shell

## Goal
Track which app currently receives keyboard input. Add a `shell: bool` capability field. The app with `shell = true` receives default focus at boot. `@supervisor: focus <appname>` changes focus at runtime.

## Files to modify

```
supervisor/src/main.rs    — focused_app state, shell capability field
apps/shell/vyoma.toml     — shell = true
```

## Capability addition

```rust
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Capabilities {
    // ... existing fields ...
    /// Receives keyboard input from /dev/tty0 by default at boot.
    #[serde(default)]
    shell: bool,
}
```

## Focus change via @supervisor: IPC

Handled in P12T03. The `@supervisor: focus <name>` command updates `focused_app`:

```rust
"focus" => {
    if let Some(name) = args {
        *focused.lock().unwrap() = Some(name.to_string());
        eprintln!("vyoma-supervisor: focus → {name}");
    }
}
```

## Notes

- Only one app has focus at a time (single-focused model)
- Future: Tab key in the input thread cycles focus between shell-capable apps
