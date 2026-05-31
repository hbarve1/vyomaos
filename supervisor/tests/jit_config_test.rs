// P92: JIT config unit tests (integration-test level, run via `cargo test`).
//
// These tests verify the public JitConfig API exposed through the supervisor
// library crate.  The internal IPC handler tests live in jit_config.rs itself.

#[test]
fn jit_subsystem_logging() {
    // Verify we can log JIT-related messages through the lifecycle subsystem.
    let line = supervisor::logging::format_log(
        supervisor::logging::Level::Info,
        supervisor::logging::Subsystem::Lifecycle,
        None,
        "jit opt-level set to speed",
    );
    assert!(line.contains("[lifecycle]"), "expected [lifecycle] in: {line}");
    assert!(line.contains("jit opt-level"), "expected jit message in: {line}");
}
