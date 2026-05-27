// Observability subsystem — T005
//
// Structured logs + health heartbeat baseline (available on all platforms).
// Optional OpenTelemetry adapter lives in otel.rs (desktop/server only).

pub mod heartbeat;

pub use heartbeat::{Heartbeat, ModuleStatus};
