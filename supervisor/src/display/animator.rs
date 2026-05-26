// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Window animation state machine.

/// The type of window animation.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimKind {
    Open,
    Close,
    Minimize,
}

/// Interpolated animation state at a given point in time.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct AnimState {
    /// Scale factor for the window layer (1.0 = full size).
    pub scale: f32,
    /// Opacity of the window layer (0 = transparent, 255 = opaque).
    pub alpha: u8,
    /// Whether the animation has completed.
    pub done: bool,
}

/// A wall-clock bounded animation between two visual states.
#[allow(dead_code)]
pub struct Animation {
    pub kind:        AnimKind,
    pub start_ms:    u64,
    pub duration_ms: u32,
    from_scale:      f32,
    to_scale:        f32,
    from_alpha:      u8,
    to_alpha:        u8,
}

impl Animation {
    /// Create a new animation starting at `start_ms`.
    pub fn new(kind: AnimKind, start_ms: u64) -> Self {
        let (duration_ms, from_scale, to_scale, from_alpha, to_alpha) = match kind {
            AnimKind::Open     => (300, 0.85, 1.0,  0,   255),
            AnimKind::Close    => (250, 1.0,  0.85, 255,   0),
            AnimKind::Minimize => (300, 1.0,  0.2,  255,   0),
        };
        Self {
            kind,
            start_ms,
            duration_ms,
            from_scale,
            to_scale,
            from_alpha,
            to_alpha,
        }
    }

    /// Sample the animation state at `elapsed_ms` milliseconds since start.
    /// Snaps to final state if elapsed >= duration.
    #[allow(dead_code)]
    pub fn sample(&self, elapsed_ms: u64) -> AnimState {
        let t = if elapsed_ms >= self.duration_ms as u64 {
            1.0_f32
        } else {
            elapsed_ms as f32 / self.duration_ms as f32
        };
        let t = t.clamp(0.0, 1.0);
        let scale = self.from_scale
            + (self.to_scale - self.from_scale) * t;
        let alpha = (self.from_alpha as f32
            + (self.to_alpha as f32 - self.from_alpha as f32) * t) as u8;
        AnimState {
            scale,
            alpha,
            done: elapsed_ms >= self.duration_ms as u64,
        }
    }
}

/// Return current time in milliseconds since the UNIX epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
