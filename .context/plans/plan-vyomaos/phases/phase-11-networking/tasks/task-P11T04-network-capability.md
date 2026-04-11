# P11T04 — Supervisor Network Capability Wiring

## Phase
Phase 11 — Networking

## Goal
Verify and harden the `network: true` capability path in the supervisor. The `spawn_app` function already passes `-S tcplisten=0.0.0.0:8080` when `caps.network = true`, but the port should come from the manifest rather than being hard-coded.

## Files to modify

```
supervisor/src/main.rs       — read port from manifest; update audit log
apps/*/vyoma.toml            — add optional port field
```

## Proposed manifest extension

```toml
[capabilities]
network      = true
network_port = 8080   # optional; defaults to 8080 if network=true
```

## Supervisor change

```rust
// In spawn_app, replace:
cmd.args(["-S", "tcplisten=0.0.0.0:8080"]);
// With:
let port = manifest.capabilities.network_port.unwrap_or(8080);
cmd.args(["-S", &format!("tcplisten=0.0.0.0:{port}")]);
```

## Audit log update

```
[security] http-server capabilities — stdio:yes fs:yes net:yes(port=8080) display:no seccomp:denylist
```

## Verification

```sh
# App without network=true cannot bind (wasmtime rejects the socket call)
# App with network=true and port=8080 successfully accepts connections
curl http://localhost:8080/health
```
