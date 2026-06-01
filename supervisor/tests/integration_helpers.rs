// Integration test helpers — Week 1, Task 1d
//
// Reusable scaffolding for integration tests in weeks 2-4.
// These helpers create lightweight mocks for the supervisor's core types
// using the library crate's public API.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};

// ── Mock Inbox ──────────────────────────────────────────────────────────────

/// A mock inbox backed by mpsc channels.
///
/// `senders` maps app names to their channel senders (mimicking the supervisor's
/// `Inbox` type).  The companion `receivers` map lets test code read messages
/// that were "delivered" to each app.
pub struct MockInbox {
    pub senders: Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>,
    pub receivers: HashMap<String, mpsc::Receiver<String>>,
}

impl MockInbox {
    /// Create an empty mock inbox with no registered apps.
    pub fn new() -> Self {
        Self {
            senders: Arc::new(Mutex::new(HashMap::new())),
            receivers: HashMap::new(),
        }
    }

    /// Register an app in the inbox.  Returns the receiver end so tests can
    /// assert on messages delivered to this app.
    pub fn register(&mut self, name: &str) -> &mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel();
        self.senders.lock().unwrap().insert(name.to_string(), tx);
        self.receivers.insert(name.to_string(), rx);
        self.receivers.get(name).unwrap()
    }

    /// Send a message to a registered app (simulates IPC delivery).
    /// Returns `true` if the app was registered and the send succeeded.
    pub fn send(&self, target: &str, msg: &str) -> bool {
        let senders = self.senders.lock().unwrap();
        if let Some(tx) = senders.get(target) {
            tx.send(msg.to_string()).is_ok()
        } else {
            false
        }
    }

    /// Collect all pending messages for an app (non-blocking).
    pub fn drain(&self, app: &str) -> Vec<String> {
        if let Some(rx) = self.receivers.get(app) {
            let mut msgs = Vec::new();
            while let Ok(m) = rx.try_recv() {
                msgs.push(m);
            }
            msgs
        } else {
            Vec::new()
        }
    }
}

// ── Mock App Registry ───────────────────────────────────────────────────────

/// A lightweight mock app registry that tracks app names and their declared
/// capabilities.  This does *not* replicate the full `AppState` from the
/// binary crate — it stores only the data needed for integration testing
/// against the library modules (manifest, IPC, OTA).
pub struct MockAppRegistry {
    pub apps: HashMap<String, MockAppEntry>,
}

/// Minimal per-app metadata for integration tests.
#[derive(Debug)]
pub struct MockAppEntry {
    pub name: String,
    pub caps: supervisor::manifest::Capabilities,
    pub running: bool,
}

impl MockAppRegistry {
    pub fn new() -> Self {
        Self { apps: HashMap::new() }
    }

    /// Add a mock app with the given name and capabilities.
    pub fn add_app(&mut self, name: &str, caps: supervisor::manifest::Capabilities) {
        self.apps.insert(name.to_string(), MockAppEntry {
            name: name.to_string(),
            caps,
            running: true,
        });
    }

    /// Add a mock app with default (stdio-only) capabilities.
    pub fn add_default_app(&mut self, name: &str) {
        let mut caps = supervisor::manifest::Capabilities::default();
        caps.stdio = true;
        self.add_app(name, caps);
    }

    /// Return a sorted list of running app names.
    pub fn running_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.apps.values()
            .filter(|e| e.running)
            .map(|e| e.name.clone())
            .collect();
        names.sort();
        names
    }

    /// Mark an app as stopped.
    pub fn stop_app(&mut self, name: &str) {
        if let Some(entry) = self.apps.get_mut(name) {
            entry.running = false;
        }
    }
}

// ── Convenience constructors ────────────────────────────────────────────────

/// Create a `MockInbox` with the given app names pre-registered.
pub fn mock_inbox_with(names: &[&str]) -> MockInbox {
    let mut inbox = MockInbox::new();
    for name in names {
        inbox.register(name);
    }
    inbox
}

/// Create a `MockAppRegistry` with the given app names (default capabilities).
pub fn mock_registry_with(names: &[&str]) -> MockAppRegistry {
    let mut reg = MockAppRegistry::new();
    for name in names {
        reg.add_default_app(name);
    }
    reg
}

// ── Self-tests for the helpers ──────────────────────────────────────────────

#[test]
fn test_mock_inbox_send_and_drain() {
    let mut inbox = MockInbox::new();
    inbox.register("ping");
    inbox.register("pong");

    assert!(inbox.send("ping", "hello from pong"));
    assert!(inbox.send("ping", "second msg"));
    assert!(!inbox.send("nonexistent", "nobody home"));

    let msgs = inbox.drain("ping");
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0], "hello from pong");
    assert_eq!(msgs[1], "second msg");

    // drain again should be empty
    assert!(inbox.drain("ping").is_empty());
}

#[test]
fn test_mock_inbox_with_helper() {
    let inbox = mock_inbox_with(&["alpha", "beta"]);
    assert!(inbox.send("alpha", "test"));
    assert!(inbox.send("beta", "test"));
    assert!(!inbox.send("gamma", "nope"));
}

#[test]
fn test_mock_registry_running_names() {
    let mut reg = mock_registry_with(&["b-app", "a-app", "c-app"]);
    let names = reg.running_names();
    assert_eq!(names, vec!["a-app", "b-app", "c-app"]);

    reg.stop_app("b-app");
    let names = reg.running_names();
    assert_eq!(names, vec!["a-app", "c-app"]);
}

#[test]
fn test_mock_registry_add_with_capabilities() {
    let mut reg = MockAppRegistry::new();
    let mut caps = supervisor::manifest::Capabilities::default();
    caps.stdio = true;
    caps.network = true;
    caps.display = true;
    reg.add_app("browser", caps);

    let entry = reg.apps.get("browser").unwrap();
    assert!(entry.caps.stdio);
    assert!(entry.caps.network);
    assert!(entry.caps.display);
    assert!(!entry.caps.filesystem);
}

#[test]
fn test_mock_inbox_register_returns_receiver() {
    let mut inbox = MockInbox::new();
    let _rx = inbox.register("test-app");
    inbox.send("test-app", "direct-check");
    let rx = inbox.receivers.get("test-app").unwrap();
    assert_eq!(rx.try_recv().unwrap(), "direct-check");
}
