// Management server — T009
//
// Binds a TcpListener on 0.0.0.0:9090 and spawns one thread per client.
// Each client thread is handled by mgmt_handlers::handle_client().

use std::{
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex},
    thread,
};

use crate::AppRegistry;
use crate::mgmt_handlers;

// ── MgmtServer ────────────────────────────────────────────────────────────────

pub struct MgmtServer {
    registry: AppRegistry,
}

impl MgmtServer {
    pub fn new(registry: AppRegistry) -> Self {
        MgmtServer { registry }
    }

    /// Bind `addr` and loop accepting connections.
    ///
    /// Each accepted connection gets its own thread running `handle_client`.
    /// Panics if `bind` fails — caller should handle via `thread::spawn`.
    pub fn start(self, addr: SocketAddr) {
        let listener = match TcpListener::bind(addr) {
            Ok(l) => {
                eprintln!("[mgmt] listening on {addr}");
                l
            }
            Err(e) => {
                eprintln!("[mgmt] FATAL: cannot bind {addr}: {e}");
                return;
            }
        };

        loop {
            match listener.accept() {
                Ok((stream, peer)) => {
                    eprintln!("[mgmt] connection from {peer}");
                    let registry = Arc::clone(&self.registry);
                    thread::Builder::new()
                        .name(format!("mgmt-client-{peer}"))
                        .spawn(move || mgmt_handlers::handle_client(stream, registry))
                        .ok();
                }
                Err(e) => {
                    eprintln!("[mgmt] accept error: {e}");
                }
            }
        }
    }
}
