// Supervisor library root — exposes pure modules for integration tests and tools.
pub mod manifest;
pub mod logging;
pub mod ipc;
pub mod lifecycle;
pub mod windows;
pub mod statusbar;
pub mod chrome {
    //! Z-layer constants re-exported for tests and external crates.
    //! The full chrome module (title bars, menu bar, etc.) lives in the binary crate.
    pub const Z_DESKTOP:  u32 = 0;    // desktop wallpaper — always background
    pub const Z_APP:      u32 = 10;   // default app window layer
    pub const Z_DOCK:     u32 = 100;  // dock — always above app windows
    pub const Z_OVERLAY:  u32 = 255;  // notifications, system overlays
}

// 043: Core foundation modules
pub mod runtime;
pub mod hal;
pub mod profile;
pub mod ota;
pub mod observability;
pub mod capability;

#[cfg(target_os = "linux")]
pub mod font;
#[cfg(target_os = "linux")]
pub mod display;
