// A/B slot manager — T004

/// Identifies which of the two firmware slots is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotLabel {
    A,
    B,
}

impl SlotLabel {
    /// Return the other slot.
    pub fn other(self) -> Self {
        match self {
            SlotLabel::A => SlotLabel::B,
            SlotLabel::B => SlotLabel::A,
        }
    }
}

impl std::fmt::Display for SlotLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SlotLabel::A => write!(f, "A"),
            SlotLabel::B => write!(f, "B"),
        }
    }
}

/// Per-module A/B slot state.
#[derive(Debug, Clone)]
pub struct AbSlot {
    /// Name of the WASM module this slot belongs to.
    pub module_name: String,
    /// Currently active (booted) slot.
    pub active: SlotLabel,
    /// Path to the WASM binary in slot A.
    pub slot_a_path: String,
    /// Path to the WASM binary in slot B (empty when not yet populated).
    pub slot_b_path: String,
    /// Whether the active slot has passed its initial health check.
    pub health_confirmed: bool,
}

impl AbSlot {
    /// Create a new `AbSlot` for a module starting in slot A.
    pub fn new(module_name: impl Into<String>, initial_path: impl Into<String>) -> Self {
        Self {
            module_name: module_name.into(),
            active: SlotLabel::A,
            slot_a_path: initial_path.into(),
            slot_b_path: String::new(),
            health_confirmed: false,
        }
    }

    /// Return the filesystem path for the active slot.
    pub fn active_path(&self) -> &str {
        match self.active {
            SlotLabel::A => &self.slot_a_path,
            SlotLabel::B => &self.slot_b_path,
        }
    }

    /// Swap the active slot (used after a successful update).
    pub fn swap(&mut self) {
        self.active = self.active.other();
        self.health_confirmed = false;
    }

    /// Roll back to the previously active slot.
    pub fn rollback(&mut self) {
        self.active = self.active.other();
        self.health_confirmed = true; // previous slot was already confirmed
    }
}
