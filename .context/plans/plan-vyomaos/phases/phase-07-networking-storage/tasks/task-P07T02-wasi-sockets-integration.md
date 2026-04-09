# P07T02 — wasi-sockets-integration

## Phase

Phase 07 — Networking & Storage

## Goal

Create a new `apps/server/` WASM application that opens a TCP listener on port 8080, accepts one connection, sends the string `"Hello from VyomaOS\n"`, and exits cleanly; declare `network = true` in its `vyoma.toml`; and update the supervisor to pass `--tcplisten 0.0.0.0:8080` to Wasmtime for any app whose manifest declares `network = true`.

## File to create / modify

```
apps/server/Cargo.toml
apps/server/src/main.rs
apps/server/.cargo/config.toml
apps/server/vyoma.toml
base/modules/scripts/boot.toml      (add server entry)
supervisor/src/main.rs              (network capability → --tcplisten flag)
```

## Implementation

### `apps/server/.cargo/config.toml`

```toml
[build]
target = "wasm32-wasip2"
```

---

### `apps/server/Cargo.toml`

```toml
[package]
name    = "server"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "server"
path = "src/main.rs"

[dependencies]
```

No external dependencies — WASI Preview 2 sockets are available through `std::net` when targeting `wasm32-wasip2` with Wasmtime's WASI implementation.

---

### `apps/server/src/main.rs`

```rust
use std::io::Write;
use std::net::{TcpListener, TcpStream};

fn main() {
    // Wasmtime passes the pre-opened listening socket via
    // --tcplisten 0.0.0.0:8080; WASI exposes it as fd 3.
    // std::net::TcpListener::bind("0.0.0.0:8080") works transparently
    // because wasmtime maps the pre-opened socket to the bind address.
    let listener = TcpListener::bind("0.0.0.0:8080")
        .expect("failed to bind TCP listener on port 8080");

    eprintln!("[server] Listening on 0.0.0.0:8080");

    // Accept exactly one connection
    let (mut stream, peer_addr) = listener.accept()
        .expect("failed to accept connection");

    eprintln!("[server] Connection from {}", peer_addr);

    stream.write_all(b"Hello from VyomaOS\n")
        .expect("failed to write response");

    eprintln!("[server] Response sent. Exiting.");
    // TcpStream is dropped here, closing the connection cleanly
}
```

**Why `accept()` once and exit:** The server is a demonstration, not a long-running daemon. It proves the network stack is end-to-end functional. Use `restart = "always"` in `boot.toml` if you want it to keep accepting connections.

---

### `apps/server/vyoma.toml`

```toml
[app]
name    = "server"
version = "0.1.0"
wasm    = "server.wasm"

[capabilities]
stdio      = true
filesystem = false
network    = true
```

`network = true` signals to the supervisor that Wasmtime must be invoked with `--tcplisten 0.0.0.0:8080`.

---

### `base/modules/scripts/boot.toml` — add server entry

```toml
[[apps]]
manifest = "/apps/server/vyoma.toml"
restart  = "always"
```

`restart = "always"` means after each connection is handled and the process exits, the supervisor restarts it, effectively creating a persistent accept loop without requiring the WASM code to loop.

---

### `supervisor/src/main.rs` — network capability handling

The `run_app` function (from P05T03) already has the skeleton; fill in the network branch:

```rust
// Inside run_app(), when building the wasmtime Command:
if manifest.capabilities.network {
    // --tcplisten <addr> tells wasmtime to pre-open a TCP socket
    // and pass it to the WASM component via WASI socket FDs.
    cmd.args(["--tcplisten", "0.0.0.0:8080"]);
}
```

Full updated `run_app` for reference:

```rust
fn run_app(entry: &BootEntry) -> (String, i32) {
    let manifest_raw = match fs::read_to_string(&entry.manifest) {
        Ok(s)  => s,
        Err(e) => {
            eprintln!("[supervisor] WARN: cannot read manifest {}: {}", entry.manifest, e);
            return (entry.manifest.clone(), -1);
        }
    };
    let manifest: AppManifest = match toml::from_str(&manifest_raw) {
        Ok(m)  => m,
        Err(e) => {
            eprintln!("[supervisor] WARN: malformed manifest {}: {}", entry.manifest, e);
            return (entry.manifest.clone(), -1);
        }
    };

    let manifest_dir = Path::new(&entry.manifest).parent().unwrap_or(Path::new("/apps"));
    let wasm_path    = manifest_dir.join(&manifest.app.wasm);

    let mut cmd = Command::new("wasmtime");
    cmd.arg("run");

    if manifest.capabilities.stdio      { cmd.arg("--inherit-stdio"); }
    if manifest.capabilities.filesystem { cmd.args(["--dir", "/data"]); }
    if manifest.capabilities.network    { cmd.args(["--tcplisten", "0.0.0.0:8080"]); }

    cmd.arg(&wasm_path);

    let code = match cmd.status() {
        Ok(s)  => s.code().unwrap_or(-1),
        Err(e) => {
            eprintln!("[supervisor] ERROR: exec failed for {}: {}", manifest.app.name, e);
            -1
        }
    };

    (manifest.app.name.clone(), code)
}
```

---

### Wasmtime `--tcplisten` flag explanation

Wasmtime's `--tcplisten` flag pre-opens a TCP socket *before* executing the WASM module. WASI Preview 2's `wasi:sockets` interface can then use this socket via a "socket capability". When the WASM code calls `TcpListener::bind("0.0.0.0:8080")`, Wasmtime maps that call to the pre-opened socket rather than performing a real `bind(2)` system call — giving the WASM app network access without requiring unrestricted socket permissions.

## Notes

- `TcpListener::bind` works in `wasm32-wasip2` only when Wasmtime is invoked with `--tcplisten`. Without that flag, the bind call returns an error because WASI disallows arbitrary socket creation.
- Port 8080 is hardcoded in both `vyoma.toml` and the supervisor's capability handler for Phase 07. A future enhancement would read the port from `vyoma.toml` (e.g. `network_port = 8080`).
- The QEMU port forwarding (`hostfwd=tcp::8080-:8080`) added in P07T01 must be in place for connections to reach the server app from the host machine.
- The `restart = "always"` policy (from P06T02) combined with a single-accept server is a valid design for a micro-server: each OS process handles one connection and exits cleanly, avoiding state accumulation across connections. This pattern is similar to inetd/xinetd.
- Wasmtime >= 18 is required for `wasi:sockets` support. Earlier versions either lack the API or have it behind a feature flag.
- Cross-reference: P07T01 adds the virtio-net kernel config and QEMU flags. This task assumes those changes are already in place.

## Verification

```sh
# 1. Confirm all server app files are present
test -f apps/server/Cargo.toml            && echo "server Cargo.toml: present"
test -f apps/server/src/main.rs           && echo "server main.rs: present"
test -f apps/server/vyoma.toml            && echo "server vyoma.toml: present"
test -f apps/server/.cargo/config.toml    && echo "server .cargo/config.toml: present"

# 2. Confirm capability is declared correctly
grep -q "network\s*=\s*true" apps/server/vyoma.toml && echo "network=true: declared"

# 3. Build the server WASM app
rustup target add wasm32-wasip2
(cd apps/server && cargo build --release 2>&1 | tail -5)
test -f apps/server/target/wasm32-wasip2/release/server.wasm && echo "server.wasm: built"

# 4. Verify the supervisor passes --tcplisten for network-capable apps
grep -q -- '--tcplisten' supervisor/src/main.rs && echo "supervisor --tcplisten: present"

# 5. Run the server locally (not inside QEMU) with wasmtime and test with curl
wasmtime run \
  --tcplisten 0.0.0.0:8080 \
  --inherit-stdio \
  apps/server/target/wasm32-wasip2/release/server.wasm &
SERVER_PID=$!
sleep 1
RESPONSE=$(curl -s --max-time 3 http://localhost:8080)
kill $SERVER_PID 2>/dev/null || true
echo "Response: '$RESPONSE'"
test "$RESPONSE" = "Hello from VyomaOS" && echo "TCP server: PASS" || echo "TCP server: FAIL"

# 6. Confirm boot.toml references the server manifest
grep -q "server/vyoma.toml" base/modules/scripts/boot.toml && echo "boot.toml: server entry present"

# 7. Build supervisor and confirm it compiles with the updated run_app
(cd supervisor && cargo build 2>&1 | tail -5)
test -x supervisor/target/debug/supervisor && echo "supervisor: compiles OK"
```
