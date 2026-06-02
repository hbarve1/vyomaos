// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Internationalization (i18n) support for supervisor UI strings.
//!
//! Provides a static string table with built-in locales (en, es, hi) and a
//! `t(key)` lookup function that falls back to English for missing keys.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

// ── Locale model ────────────────────────────────────────────────────────────

/// A locale contains a language code and a table of translated strings.
#[allow(dead_code)]
pub struct Locale {
    pub code: &'static str,
    strings: HashMap<&'static str, &'static str>,
}

impl Locale {
    /// Build a `Locale` from a static slice of (key, value) pairs.
    fn from_pairs(code: &'static str, pairs: &[(&'static str, &'static str)]) -> Self {
        let mut strings = HashMap::with_capacity(pairs.len());
        for &(k, v) in pairs {
            strings.insert(k, v);
        }
        Self { code, strings }
    }

    /// Look up a key in this locale's string table.
    pub fn get(&self, key: &str) -> Option<&'static str> {
        self.strings.get(key).copied()
    }
}

// ── Built-in locale data ────────────────────────────────────────────────────

const EN_STRINGS: &[(&str, &str)] = &[
    ("brand",             "VyomaOS"),
    ("lock_screen",       "Lock Screen"),
    ("settings",          "Settings"),
    ("notifications",     "Notifications"),
    ("shutdown",          "Shut Down"),
    ("restart",           "Restart"),
    ("volume",            "Volume"),
    ("muted",             "Muted"),
    ("workspace",         "Workspace"),
    ("search",            "Search"),
    ("file",              "File"),
    ("edit",              "Edit"),
    ("view",              "View"),
    ("window",            "Window"),
    ("help",              "Help"),
    ("close",             "Close"),
    ("minimize",          "Minimize"),
    ("maximize",          "Maximize"),
    ("quit",              "Quit"),
    ("about",             "About"),
    ("preferences",       "Preferences"),
    ("copy",              "Copy"),
    ("paste",             "Paste"),
    ("cut",               "Cut"),
    ("select_all",        "Select All"),
    ("undo",              "Undo"),
];

const ES_STRINGS: &[(&str, &str)] = &[
    ("brand",             "VyomaOS"),
    ("lock_screen",       "Bloquear Pantalla"),
    ("settings",          "Ajustes"),
    ("notifications",     "Notificaciones"),
    ("shutdown",          "Apagar"),
    ("restart",           "Reiniciar"),
    ("volume",            "Volumen"),
    ("muted",             "Silenciado"),
    ("workspace",         "Espacio de Trabajo"),
    ("search",            "Buscar"),
    ("file",              "Archivo"),
    ("edit",              "Editar"),
    ("view",              "Vista"),
    ("window",            "Ventana"),
    ("help",              "Ayuda"),
    ("close",             "Cerrar"),
    ("minimize",          "Minimizar"),
    ("maximize",          "Maximizar"),
    ("quit",              "Salir"),
    ("about",             "Acerca de"),
    ("preferences",       "Preferencias"),
    ("copy",              "Copiar"),
    ("paste",             "Pegar"),
    ("cut",               "Cortar"),
    ("select_all",        "Seleccionar Todo"),
    ("undo",              "Deshacer"),
];

const HI_STRINGS: &[(&str, &str)] = &[
    ("brand",             "VyomaOS"),
    ("lock_screen",       "\u{0932}\u{0949}\u{0915} \u{0938}\u{094D}\u{0915}\u{094D}\u{0930}\u{0940}\u{0928}"),
    ("settings",          "\u{0938}\u{0947}\u{091F}\u{093F}\u{0902}\u{0917}\u{094D}\u{0938}"),
    ("notifications",     "\u{0938}\u{0942}\u{091A}\u{0928}\u{093E}\u{090F}\u{0901}"),
    ("shutdown",          "\u{092C}\u{0902}\u{0926} \u{0915}\u{0930}\u{0947}\u{0902}"),
    ("restart",           "\u{092A}\u{0941}\u{0928}\u{0903} \u{0906}\u{0930}\u{0902}\u{092D} \u{0915}\u{0930}\u{0947}\u{0902}"),
    ("volume",            "\u{0927}\u{094D}\u{0935}\u{0928}\u{093F}"),
    ("muted",             "\u{092E}\u{094D}\u{092F}\u{0942}\u{091F}"),
    ("workspace",         "\u{0915}\u{093E}\u{0930}\u{094D}\u{092F}\u{0915}\u{094D}\u{0937}\u{0947}\u{0924}\u{094D}\u{0930}"),
    ("search",            "\u{0916}\u{094B}\u{091C}\u{0947}\u{0902}"),
    ("file",              "\u{092B}\u{093C}\u{093E}\u{0907}\u{0932}"),
    ("edit",              "\u{0938}\u{0902}\u{092A}\u{093E}\u{0926}\u{0928}"),
    ("view",              "\u{0926}\u{0943}\u{0936}\u{094D}\u{092F}"),
    ("window",            "\u{0935}\u{093F}\u{0902}\u{0921}\u{094B}"),
    ("help",              "\u{0938}\u{0939}\u{093E}\u{092F}\u{0924}\u{093E}"),
    ("close",             "\u{092C}\u{0902}\u{0926} \u{0915}\u{0930}\u{0947}\u{0902}"),
    ("minimize",          "\u{091B}\u{094B}\u{091F}\u{093E} \u{0915}\u{0930}\u{0947}\u{0902}"),
    ("maximize",          "\u{092C}\u{0921}\u{093C}\u{093E} \u{0915}\u{0930}\u{0947}\u{0902}"),
    ("quit",              "\u{091B}\u{094B}\u{0921}\u{093C}\u{0947}\u{0902}"),
    ("about",             "\u{092C}\u{093E}\u{0930}\u{0947} \u{092E}\u{0947}\u{0902}"),
    ("preferences",       "\u{092A}\u{094D}\u{0930}\u{093E}\u{0925}\u{092E}\u{093F}\u{0915}\u{0924}\u{093E}\u{090F}\u{0901}"),
    ("copy",              "\u{0915}\u{0949}\u{092A}\u{0940}"),
    ("paste",             "\u{092A}\u{0947}\u{0938}\u{094D}\u{091F}"),
    ("cut",               "\u{0915}\u{091F}"),
    ("select_all",        "\u{0938}\u{092C} \u{091A}\u{0941}\u{0928}\u{0947}\u{0902}"),
    ("undo",              "\u{092A}\u{0942}\u{0930}\u{094D}\u{0935}\u{0935}\u{0924}"),
];

// ── Locale registry ─────────────────────────────────────────────────────────

/// Active locale code. Defaults to "en".
static ACTIVE_LOCALE: OnceLock<Mutex<&'static str>> = OnceLock::new();

/// All built-in locales, keyed by language code.
static LOCALES: OnceLock<HashMap<&'static str, Locale>> = OnceLock::new();

fn active_locale_lock() -> &'static Mutex<&'static str> {
    ACTIVE_LOCALE.get_or_init(|| Mutex::new("en"))
}

fn locale_map() -> &'static HashMap<&'static str, Locale> {
    LOCALES.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("en", Locale::from_pairs("en", EN_STRINGS));
        m.insert("es", Locale::from_pairs("es", ES_STRINGS));
        m.insert("hi", Locale::from_pairs("hi", HI_STRINGS));
        m
    })
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Look up a translated string by key. Falls back to English if the current
/// locale does not contain the key.
pub fn t(key: &str) -> &'static str {
    let code = *active_locale_lock().lock().unwrap();
    let map = locale_map();

    // Try current locale first.
    if let Some(locale) = map.get(code) {
        if let Some(val) = locale.get(key) {
            return val;
        }
    }

    // Fallback to English.
    if code != "en" {
        if let Some(en) = map.get("en") {
            if let Some(val) = en.get(key) {
                return val;
            }
        }
    }

    // Key not found in any locale — return a static fallback string.
    "???"
}

/// Return the currently active locale code (e.g. "en", "es", "hi").
pub fn current_locale() -> &'static str {
    *active_locale_lock().lock().unwrap()
}

/// Switch the active locale. Returns `true` if the locale code is known,
/// `false` if it is not recognized (active locale remains unchanged).
pub fn set_locale(code: &str) -> bool {
    let map = locale_map();
    // We need a &'static str for the code. Match against known codes.
    let static_code: Option<&'static str> = map.keys().find(|&&k| k == code).copied();
    match static_code {
        Some(c) => {
            *active_locale_lock().lock().unwrap() = c;
            true
        }
        None => false,
    }
}

/// Return a list of all supported locale codes.
pub fn supported_locales() -> Vec<&'static str> {
    let mut codes: Vec<&'static str> = locale_map().keys().copied().collect();
    codes.sort();
    codes
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // All i18n tests share a global locale, so they must run sequentially
    // within a single test function to avoid races from parallel test execution.
    #[test]
    fn i18n_all() {
        // ── English lookup ──────────────────────────────────────────────
        set_locale("en");
        assert_eq!(t("brand"), "VyomaOS");
        assert_eq!(t("shutdown"), "Shut Down");
        assert_eq!(t("settings"), "Settings");

        // ── Spanish lookup ──────────────────────────────────────────────
        set_locale("es");
        assert_eq!(t("shutdown"), "Apagar");
        assert_eq!(t("settings"), "Ajustes");
        assert_eq!(t("lock_screen"), "Bloquear Pantalla");

        // ── Hindi lookup ────────────────────────────────────────────────
        set_locale("hi");
        assert_eq!(t("brand"), "VyomaOS");
        // Hindi "settings" should not be the English word.
        assert_ne!(t("settings"), "Settings");

        // ── Fallback to "???" for missing key ───────────────────────────
        set_locale("es");
        assert_eq!(t("brand"), "VyomaOS");
        assert_eq!(t("nonexistent_key_xyz"), "???");

        // ── set_locale with unknown code returns false ──────────────────
        set_locale("en");
        assert!(!set_locale("xx"));
        assert_eq!(current_locale(), "en");

        // ── set_locale with known code returns true ─────────────────────
        assert!(set_locale("es"));
        assert_eq!(current_locale(), "es");

        // ── supported_locales lists all built-in codes ──────────────────
        let codes = supported_locales();
        assert!(codes.contains(&"en"));
        assert!(codes.contains(&"es"));
        assert!(codes.contains(&"hi"));

        // Reset to English for any subsequent tests.
        set_locale("en");
    }
}
