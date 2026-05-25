// Hot-swap coordinator — T036
//
// Provides `HotSwapCoordinator`: an in-process module registry that supports
// live replacement of a named module without stopping siblings.
//
// In production the supervisor would use this to:
//   1. Stop the target wasmtime child process.
//   2. Start the replacement process under the new name.
//   3. Transfer inbox channel so IPC resumes transparently.
//
// For the purposes of this implementation and unit tests we model the registry
// as a `HashMap<String, ModuleState>`.  The caller (supervisor's restart command)
// is responsible for wiring the actual child process management.

use std::collections::HashMap;

// ── ModuleState ───────────────────────────────────────────────────────────────

/// Lifecycle state of a managed module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleState {
    Running,
    Stopped,
}

// ── HotSwapError ──────────────────────────────────────────────────────────────

/// Errors returned by `HotSwapCoordinator::swap`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotSwapError {
    /// No module with the given name is registered.
    NotFound(String),
}

impl std::fmt::Display for HotSwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HotSwapError::NotFound(name) => {
                write!(f, "hot-swap: module '{name}' not found")
            }
        }
    }
}

// ── HotSwapCoordinator ────────────────────────────────────────────────────────

/// Tracks running module names and orchestrates live module replacement.
///
/// # Guarantees
/// - `swap(old, new)` only removes `old` and inserts `new`; all other entries
///   remain in `Running` state.
/// - If `old` does not exist, `Err(HotSwapError::NotFound)` is returned and
///   the registry is unchanged.
pub struct HotSwapCoordinator {
    modules: HashMap<String, ModuleState>,
}

impl HotSwapCoordinator {
    /// Create an empty coordinator.
    pub fn new() -> Self {
        Self { modules: HashMap::new() }
    }

    /// Register a new module as `Running`.
    pub fn register(&mut self, name: String) {
        self.modules.insert(name, ModuleState::Running);
    }

    /// Replace `old_name` with `new_name` atomically.
    ///
    /// Returns `Ok(())` on success.  Siblings are unaffected.
    pub fn swap(
        &mut self,
        old_name: &str,
        new_name: &str,
    ) -> Result<(), HotSwapError> {
        if !self.modules.contains_key(old_name) {
            return Err(HotSwapError::NotFound(old_name.to_string()));
        }
        // Remove old module.
        self.modules.remove(old_name);
        // Insert replacement.
        self.modules.insert(new_name.to_string(), ModuleState::Running);
        Ok(())
    }

    /// Query the state of a module, or `None` if not registered.
    pub fn state(&self, name: &str) -> Option<ModuleState> {
        self.modules.get(name).cloned()
    }

    /// Total number of registered modules.
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }

    /// Remove all entries for `name` (called on normal module exit).
    pub fn unregister(&mut self, name: &str) {
        self.modules.remove(name);
    }
}

impl Default for HotSwapCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_query() {
        let mut c = HotSwapCoordinator::new();
        c.register("app-a".to_string());
        assert_eq!(c.state("app-a"), Some(ModuleState::Running));
    }

    #[test]
    fn swap_replaces_correctly() {
        let mut c = HotSwapCoordinator::new();
        c.register("old".to_string());
        c.register("bystander".to_string());
        c.swap("old", "new").unwrap();
        assert_eq!(c.state("old"),        None);
        assert_eq!(c.state("new"),        Some(ModuleState::Running));
        assert_eq!(c.state("bystander"), Some(ModuleState::Running));
    }

    #[test]
    fn swap_nonexistent_returns_err() {
        let mut c = HotSwapCoordinator::new();
        let r = c.swap("ghost", "new");
        assert!(matches!(r, Err(HotSwapError::NotFound(_))));
    }
}
