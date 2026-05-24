// Supervisor library root — exposes pure modules for integration tests and tools.
pub mod manifest;
pub mod logging;
pub mod ipc;
pub mod lifecycle;
pub mod windows;
pub mod statusbar;

#[cfg(target_os = "linux")]
pub mod font;
#[cfg(target_os = "linux")]
pub mod display;
