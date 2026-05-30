# FINAL Spec: System Preferences & Settings (Round 78)

**macOS Analogue**: System Preferences / System Settings (macOS 13+)
**Status**: FINAL — all blocking issues resolved
**Date**: 2026-05-30
**Components**: `supervisor/src/prefs/` + `apps/settings/`

---

## 1. Architecture

### Supervisor Module: `supervisor/src/prefs/`

Four focused files within the 500-line limit:

| File | Limit | Responsibility |
|------|-------|----------------|
| `mod.rs` | ≤200 lines | `PrefsStore` global (`OnceLock<Arc<Mutex<PrefsStore>>>`), `handle_prefs_line()` entry point |
| `store.rs` | ≤300 lines | Key-value store, typed get/set, atomic JSON persistence to `/data/.vyoma/prefs/prefs.json` |
| `schema.rs` | ≤200 lines | `PrefKey` enum, `PrefValue` enum, string↔key conversion, default values per key |
| `handlers.rs` | ≤150 lines | `VYOMA_PREFS:` verb dispatch: get, set, list, reset, watch, unwatch |

**Global singleton** (`mod.rs`):
```rust
use std::sync::{Arc, Mutex, OnceLock};

static PREFS_STORE: OnceLock<Arc<Mutex<PrefsStore>>> = OnceLock::new();

pub fn prefs_store() -> Arc<Mutex<PrefsStore>> {
    PREFS_STORE
        .get_or_init(|| {
            let store = PrefsStore::load_or_default();
            Arc::new(Mutex::new(store))
        })
        .clone()
}
```

**Watcher registry** lives inside `PrefsStore`:
```rust
pub struct PrefsStore {
    values:   HashMap<String, PrefValue>,
    watchers: HashMap<String, Vec<String>>, // key -> [app_name, ...]
    data_dir: PathBuf,
}
```

### WASM Settings App: `apps/settings/src/`

| File | Limit | Responsibility |
|------|-------|----------------|
| `main.rs` | ≤500 lines | Event loop, panel switching, top-level VYOMA_DRAW rendering, mouse/keyboard dispatch |
| `panels/mod.rs` | ≤100 lines | `Panel` trait, `PanelKind` enum, `build_panels()` factory |
| `panels/display.rs` | ≤300 lines | Display & Resolution panel: brightness slider, theme toggle, resolution picker |
| `panels/sound.rs` | ≤300 lines | Audio panel: output volume slider, input volume slider, mute toggle |
| `panels/network.rs` | ≤200 lines | Network info display: IP, status, hostname (read-only) |
| `panels/general.rs` | ≤300 lines | General panel: language, timezone, time format toggle |

**Panel trait** (`panels/mod.rs`):
```rust
pub trait Panel {
    fn title(&self) -> &str;
    fn render(&self, state: &AppState) -> Vec<DrawCmd>;
    fn handle_mouse(&mut self, x: i32, y: i32, pressed: bool, state: &mut AppState);
    fn handle_key(&mut self, key: &str, state: &mut AppState);
}
```

---

## 2. VYOMA_PREFS Protocol

### App → Supervisor

```
VYOMA_PREFS:get|<key>
VYOMA_PREFS:set|<key>|<value>
VYOMA_PREFS:list
VYOMA_PREFS:reset|<key>
VYOMA_PREFS:watch|<key>
VYOMA_PREFS:unwatch|<key>
```

- `<key>` is a dot-separated string, e.g. `display.brightness`
- `<value>` is always a UTF-8 string; type coercion is done in the supervisor against the known schema
- `list` returns all keys visible to the calling app (currently all keys; future: namespace-gated)
- `watch` and `unwatch` register/deregister the calling app for push notifications on that key

### Supervisor → App (written to app's stdin)

```
VYOMA_PREFS:value|<key>|<value>
VYOMA_PREFS:list_result|<json_array>
VYOMA_PREFS:changed|<key>|<value>
VYOMA_PREFS:error|<key>|<reason>
```

- `value` is the synchronous reply to `get` and `reset`
- `list_result` carries a JSON array of `{"key": "...", "value": "...", "type": "..."}` objects
- `changed` is pushed asynchronously to all registered watchers when any app calls `set`
- `error` carries a human-readable reason string (e.g. `"unknown_key"`, `"type_mismatch"`, `"value_out_of_range"`)

### Parsing entry point (`mod.rs`):

```rust
/// Called by the supervisor's per-app stdout handler.
/// `app_name` identifies the calling app for watcher registration.
pub fn handle_prefs_line(line: &str, app_name: &str) -> Option<String> {
    let rest = line.strip_prefix("VYOMA_PREFS:")?;
    let store = prefs_store();
    handlers::dispatch(rest, app_name, store)
}
```

Return value `Option<String>` is the synchronous reply written back to the calling app's stdin. Watcher notifications are sent separately via the IPC broker.

---

## 3. Preference Schema

### `schema.rs`

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PrefKey {
    // Display
    DisplayBrightness,
    DisplayTheme,
    DisplayResolution,
    // Audio
    AudioOutputVolume,
    AudioInputVolume,
    AudioMuted,
    // Notifications
    NotifDnd,
    NotifSound,
    // General / Locale
    LocaleLanguage,
    LocaleTimeZone,
    LocaleTimeFormat,
    // Keyboard
    KeyboardRepeatRate,
    KeyboardRepeatDelay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PrefValue {
    Bool(bool),
    U8(u8),
    U32(u32),
    Str(String),
}

impl PrefKey {
    /// Canonical dot-separated string key used in the JSON store and protocol.
    pub fn as_str(&self) -> &'static str {
        match self {
            PrefKey::DisplayBrightness   => "display.brightness",
            PrefKey::DisplayTheme        => "display.theme",
            PrefKey::DisplayResolution   => "display.resolution",
            PrefKey::AudioOutputVolume   => "audio.output_volume",
            PrefKey::AudioInputVolume    => "audio.input_volume",
            PrefKey::AudioMuted          => "audio.muted",
            PrefKey::NotifDnd            => "notif.dnd",
            PrefKey::NotifSound          => "notif.sound",
            PrefKey::LocaleLanguage      => "locale.language",
            PrefKey::LocaleTimeZone      => "locale.timezone",
            PrefKey::LocaleTimeFormat    => "locale.time_format",
            PrefKey::KeyboardRepeatRate  => "keyboard.repeat_rate",
            PrefKey::KeyboardRepeatDelay => "keyboard.repeat_delay",
        }
    }

    pub fn from_str(s: &str) -> Option<PrefKey> {
        match s {
            "display.brightness"    => Some(PrefKey::DisplayBrightness),
            "display.theme"         => Some(PrefKey::DisplayTheme),
            "display.resolution"    => Some(PrefKey::DisplayResolution),
            "audio.output_volume"   => Some(PrefKey::AudioOutputVolume),
            "audio.input_volume"    => Some(PrefKey::AudioInputVolume),
            "audio.muted"           => Some(PrefKey::AudioMuted),
            "notif.dnd"             => Some(PrefKey::NotifDnd),
            "notif.sound"           => Some(PrefKey::NotifSound),
            "locale.language"       => Some(PrefKey::LocaleLanguage),
            "locale.timezone"       => Some(PrefKey::LocaleTimeZone),
            "locale.time_format"    => Some(PrefKey::LocaleTimeFormat),
            "keyboard.repeat_rate"  => Some(PrefKey::KeyboardRepeatRate),
            "keyboard.repeat_delay" => Some(PrefKey::KeyboardRepeatDelay),
            _ => None,
        }
    }

    pub fn default_value(&self) -> PrefValue {
        match self {
            PrefKey::DisplayBrightness   => PrefValue::U8(100),
            PrefKey::DisplayTheme        => PrefValue::Str("dark".into()),
            PrefKey::DisplayResolution   => PrefValue::Str("1920x1080".into()),
            PrefKey::AudioOutputVolume   => PrefValue::U8(75),
            PrefKey::AudioInputVolume    => PrefValue::U8(50),
            PrefKey::AudioMuted          => PrefValue::Bool(false),
            PrefKey::NotifDnd            => PrefValue::Bool(false),
            PrefKey::NotifSound          => PrefValue::Bool(true),
            PrefKey::LocaleLanguage      => PrefValue::Str("en-US".into()),
            PrefKey::LocaleTimeZone      => PrefValue::Str("UTC".into()),
            PrefKey::LocaleTimeFormat    => PrefValue::Str("24h".into()),
            PrefKey::KeyboardRepeatRate  => PrefValue::U32(30),
            PrefKey::KeyboardRepeatDelay => PrefValue::U32(400),
        }
    }

    /// Parse a string value from the protocol into the correct PrefValue variant.
    pub fn parse_value(&self, raw: &str) -> Result<PrefValue, &'static str> {
        match self {
            PrefKey::DisplayBrightness | PrefKey::AudioOutputVolume | PrefKey::AudioInputVolume => {
                raw.parse::<u8>().map(PrefValue::U8).map_err(|_| "type_mismatch")
            }
            PrefKey::AudioMuted | PrefKey::NotifDnd | PrefKey::NotifSound => {
                match raw {
                    "true"  => Ok(PrefValue::Bool(true)),
                    "false" => Ok(PrefValue::Bool(false)),
                    _ => Err("type_mismatch"),
                }
            }
            PrefKey::KeyboardRepeatRate | PrefKey::KeyboardRepeatDelay => {
                raw.parse::<u32>().map(PrefValue::U32).map_err(|_| "type_mismatch")
            }
            PrefKey::DisplayTheme => {
                if matches!(raw, "dark" | "light") {
                    Ok(PrefValue::Str(raw.into()))
                } else {
                    Err("value_out_of_range")
                }
            }
            PrefKey::LocaleTimeFormat => {
                if matches!(raw, "24h" | "12h") {
                    Ok(PrefValue::Str(raw.into()))
                } else {
                    Err("value_out_of_range")
                }
            }
            _ => Ok(PrefValue::Str(raw.into())),
        }
    }
}

impl PrefValue {
    pub fn to_display_string(&self) -> String {
        match self {
            PrefValue::Bool(b)  => b.to_string(),
            PrefValue::U8(n)    => n.to_string(),
            PrefValue::U32(n)   => n.to_string(),
            PrefValue::Str(s)   => s.clone(),
        }
    }

    pub fn to_json_value(&self) -> serde_json::Value {
        match self {
            PrefValue::Bool(b)  => serde_json::Value::from(*b),
            PrefValue::U8(n)    => serde_json::Value::from(*n),
            PrefValue::U32(n)   => serde_json::Value::from(*n),
            PrefValue::Str(s)   => serde_json::Value::from(s.as_str()),
        }
    }
}
```

---

## 4. Storage Format

**Path**: `/data/.vyoma/prefs/prefs.json`

**JSON layout** (flat dot-key map):
```json
{
  "display.brightness": 100,
  "display.theme": "dark",
  "display.resolution": "1920x1080",
  "audio.output_volume": 75,
  "audio.input_volume": 50,
  "audio.muted": false,
  "notif.dnd": false,
  "notif.sound": true,
  "locale.language": "en-US",
  "locale.timezone": "UTC",
  "locale.time_format": "24h",
  "keyboard.repeat_rate": 30,
  "keyboard.repeat_delay": 400
}
```

**`store.rs` — load and persist** (key excerpt):

```rust
use std::{collections::HashMap, fs, io::Write, path::PathBuf};
use serde_json::Value as JVal;

use crate::prefs::schema::{PrefKey, PrefValue};

const ALL_KEYS: &[PrefKey] = &[
    PrefKey::DisplayBrightness, PrefKey::DisplayTheme, PrefKey::DisplayResolution,
    PrefKey::AudioOutputVolume, PrefKey::AudioInputVolume, PrefKey::AudioMuted,
    PrefKey::NotifDnd, PrefKey::NotifSound,
    PrefKey::LocaleLanguage, PrefKey::LocaleTimeZone, PrefKey::LocaleTimeFormat,
    PrefKey::KeyboardRepeatRate, PrefKey::KeyboardRepeatDelay,
];

impl PrefsStore {
    pub fn load_or_default() -> Self {
        let data_dir = PathBuf::from("/data/.vyoma/prefs");
        let path = data_dir.join("prefs.json");
        let mut values: HashMap<String, PrefValue> = HashMap::new();

        // Seed defaults first
        for key in ALL_KEYS {
            values.insert(key.as_str().to_string(), key.default_value());
        }

        // Overwrite with persisted values (unknown keys silently ignored)
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(JVal::Object(map)) = serde_json::from_slice(&bytes) {
                for (k, v) in map {
                    if let Some(pkey) = PrefKey::from_str(&k) {
                        let raw = match &v {
                            JVal::Bool(b)   => b.to_string(),
                            JVal::Number(n) => n.to_string(),
                            JVal::String(s) => s.clone(),
                            _ => continue,
                        };
                        if let Ok(pval) = pkey.parse_value(&raw) {
                            values.insert(k, pval);
                        }
                    }
                }
            }
        }

        PrefsStore { values, watchers: HashMap::new(), data_dir }
    }

    /// Atomic write-then-rename (R41 pattern).
    pub fn persist(&self) -> std::io::Result<()> {
        let dir = &self.data_dir;
        fs::create_dir_all(dir)?;
        let tmp = dir.join("prefs.json.tmp");
        let final_path = dir.join("prefs.json");

        let mut map = serde_json::Map::new();
        for (k, v) in &self.values {
            map.insert(k.clone(), v.to_json_value());
        }
        let json = serde_json::to_string_pretty(&JVal::Object(map))?;

        let mut f = fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
        drop(f);
        fs::rename(tmp, final_path)?;
        Ok(())
    }
}
```

---

## 5. Change Notification System

Watchers are stored as `HashMap<String, Vec<String>>` mapping dot-key → vec of app names.

**Deadlock-safe notification flow** (see B2 in Blocking Issues):

```rust
/// Returns (reply_line, Vec<(app_name, notification_line)>) to send after lock is dropped.
pub fn set_value(
    store: &Arc<Mutex<PrefsStore>>,
    key_str: &str,
    raw_value: &str,
) -> Result<Vec<(String, String)>, String> {
    let pkey = PrefKey::from_str(key_str).ok_or_else(|| "unknown_key".to_string())?;
    let pval = pkey.parse_value(raw_value).map_err(|e| e.to_string())?;

    // --- Hold lock: mutate + persist + collect watcher names ---
    let notifications: Vec<(String, String)> = {
        let mut guard = store.lock().unwrap();
        guard.values.insert(key_str.to_string(), pval.clone());
        guard.persist().map_err(|e| e.to_string())?;

        let notif_line = format!("VYOMA_PREFS:changed|{}|{}", key_str, pval.to_display_string());
        guard
            .watchers
            .get(key_str)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|app| (app, notif_line.clone()))
            .collect()
    };
    // --- Lock released; IPC sends happen outside ---

    Ok(notifications)
}
```

The caller (`handlers.rs`) then iterates `notifications` and routes each via the existing IPC broker (`ipc::send_to_app(app_name, line)`), which has its own lock — no nested locking occurs.

**Watch / unwatch** (`store.rs`):
```rust
pub fn add_watcher(&mut self, key_str: &str, app_name: &str) {
    self.watchers
        .entry(key_str.to_string())
        .or_default()
        .push(app_name.to_string());
}

pub fn remove_watcher(&mut self, key_str: &str, app_name: &str) {
    if let Some(list) = self.watchers.get_mut(key_str) {
        list.retain(|a| a != app_name);
    }
}
```

---

## 6. Settings App UI

### Layout

```
+------+------------------------------------------+
| 200  |                                          |
|      |   Panel content area                     |
| Side |   (960 - 200 = 760 px wide)              |
| bar  |                                          |
|      |                                          |
+------+------------------------------------------+
```

- Sidebar: x=0, y=40 (below chrome titlebar), w=200, h=remaining
- Content: x=200, y=40, w=760, h=remaining
- Active panel highlighted in sidebar with accent fill rect

### Panel list (sidebar)
Panels rendered top-to-bottom, each 44px tall, 200px wide:
1. Display
2. Sound
3. Notifications
4. Network
5. Keyboard
6. General

### Sidebar rendering (`main.rs`):
```rust
fn render_sidebar(panels: &[Box<dyn Panel>], active: usize) -> Vec<String> {
    let mut cmds = vec![];
    let bg:     u32 = 0x1C1C1EFF;
    let accent: u32 = 0x0A84FFFF;
    let text:   u32 = 0xFFFFFFFF;
    let dim:    u32 = 0x8E8E93FF;

    cmds.push(format!("VYOMA_DRAW:fill_rect:0,40,200,{},{}",  660, bg));
    for (i, panel) in panels.iter().enumerate() {
        let y = 40 + i as i32 * 44;
        let fg = if i == active { text } else { dim };
        if i == active {
            cmds.push(format!("VYOMA_DRAW:fill_rect:0,{},200,44,{}", y, accent));
        }
        cmds.push(format!("VYOMA_DRAW:draw_text:16,{},{},m,{}", y + 14, fg, panel.title()));
    }
    cmds
}
```

### Widget primitives

**Slider** (horizontal, draggable):
- Track: filled rect, h=6, rounded appearance via two overlaid rects
- Handle: 18×18 filled circle (rendered as fill_rect 18×18, offset by -9 from center)
- Value range: [min, max] → mapped to pixel x in `[track_x, track_x + track_w]`

```rust
pub struct SliderState {
    pub key:     &'static str,
    pub label:   &'static str,
    pub value:   u8,   // 0-100
    pub track_x: i32,
    pub track_y: i32,
    pub track_w: i32,
    pub dragging: bool,
}

impl SliderState {
    pub fn render(&self) -> Vec<String> {
        let track_bg:  u32 = 0x3A3A3CFF;
        let track_fg:  u32 = 0x0A84FFFF;
        let handle_c:  u32 = 0xFFFFFFFF;
        let label_c:   u32 = 0xFFFFFFFF;

        let filled_w = (self.value as i32 * self.track_w) / 100;
        let handle_x = self.track_x + filled_w - 9;

        vec![
            format!("VYOMA_DRAW:draw_text:{},{},{},m,{}", self.track_x, self.track_y - 22, label_c, self.label),
            format!("VYOMA_DRAW:fill_rect:{},{},{}6,{}", self.track_x, self.track_y, self.track_w, track_bg),
            format!("VYOMA_DRAW:fill_rect:{},{},{},6,{}", self.track_x, self.track_y, filled_w, track_fg),
            format!("VYOMA_DRAW:fill_rect:{},{},18,18,{}", handle_x, self.track_y - 6, handle_c),
        ]
    }

    /// Returns Some(new_value) if the drag produced a value change.
    pub fn handle_mouse(&mut self, x: i32, y: i32, pressed: bool) -> Option<u8> {
        let in_track = x >= self.track_x - 9
            && x <= self.track_x + self.track_w + 9
            && y >= self.track_y - 8
            && y <= self.track_y + 14;

        if pressed && in_track {
            self.dragging = true;
        }
        if !pressed {
            self.dragging = false;
        }
        if self.dragging {
            let clamped = (x - self.track_x).clamp(0, self.track_w);
            let new_val = ((clamped * 100) / self.track_w) as u8;
            if new_val != self.value {
                self.value = new_val;
                return Some(new_val);
            }
        }
        None
    }
}
```

**Toggle** (40×24 pill, bool):
```rust
pub struct ToggleState {
    pub key:   &'static str,
    pub label: &'static str,
    pub value: bool,
    pub x:     i32,
    pub y:     i32,
}

impl ToggleState {
    pub fn render(&self) -> Vec<String> {
        let bg:     u32 = if self.value { 0x30D158FF } else { 0x3A3A3CFF };
        let knob:   u32 = 0xFFFFFFFF;
        let label_c:u32 = 0xFFFFFFFF;
        let knob_x = if self.value { self.x + 20 } else { self.x + 4 };

        vec![
            format!("VYOMA_DRAW:draw_text:{},{},{},m,{}", self.x + 48, self.y + 4, label_c, self.label),
            format!("VYOMA_DRAW:fill_rect:{},{},40,24,{}", self.x, self.y, bg),
            format!("VYOMA_DRAW:fill_rect:{},{},16,16,{}", knob_x, self.y + 4, knob),
        ]
    }

    pub fn hit_test(&self, x: i32, y: i32) -> bool {
        x >= self.x && x <= self.x + 40 && y >= self.y && y <= self.y + 24
    }
}
```

---

## 7. Display Panel Example

`panels/display.rs` — complete state structure and render:

```rust
use crate::panels::{Panel, AppState, DrawCmd};

pub struct DisplayPanel {
    brightness: SliderState,
    theme_toggle: ToggleState,   // true = light, false = dark
    resolution_idx: usize,
}

const RESOLUTIONS: &[&str] = &["1920x1080", "1280x720", "2560x1440"];

impl DisplayPanel {
    pub fn new(brightness: u8, theme: &str, resolution: &str) -> Self {
        DisplayPanel {
            brightness: SliderState {
                key: "display.brightness", label: "Brightness",
                value: brightness, track_x: 260, track_y: 120, track_w: 480, dragging: false,
            },
            theme_toggle: ToggleState {
                key: "display.theme", label: "Light Mode",
                value: theme == "light", x: 260, y: 200,
            },
            resolution_idx: RESOLUTIONS.iter().position(|r| *r == resolution).unwrap_or(0),
        }
    }
}

impl Panel for DisplayPanel {
    fn title(&self) -> &str { "Display" }

    fn render(&self, _state: &AppState) -> Vec<DrawCmd> {
        let bg:    u32 = 0x1C1C1EFF;
        let title: u32 = 0xFFFFFFFF;
        let sub:   u32 = 0x8E8E93FF;
        let mut cmds = vec![
            format!("VYOMA_DRAW:fill_rect:200,40,760,660,{}", bg),
            format!("VYOMA_DRAW:draw_text:220,56,{},l,Display", title),
        ];
        // Brightness
        cmds.extend(self.brightness.render());
        // Theme
        cmds.extend(self.theme_toggle.render());
        // Resolution picker header
        cmds.push(format!("VYOMA_DRAW:draw_text:260,280,{},m,Resolution", sub));
        for (i, res) in RESOLUTIONS.iter().enumerate() {
            let y = 310 + i as i32 * 36;
            let color = if i == self.resolution_idx { 0x0A84FFFF_u32 } else { 0xAEAEB2FF_u32 };
            cmds.push(format!("VYOMA_DRAW:fill_rect:260,{},20,20,{}", y, color));
            cmds.push(format!("VYOMA_DRAW:draw_text:290,{},0xFFFFFFFF,m,{}", y + 2, res));
        }
        cmds
    }

    fn handle_mouse(&mut self, x: i32, y: i32, pressed: bool, state: &mut AppState) {
        // Brightness slider drag
        if let Some(new_val) = self.brightness.handle_mouse(x, y, pressed) {
            println!("VYOMA_PREFS:set|display.brightness|{}", new_val);
        }
        // Theme toggle click
        if pressed && self.theme_toggle.hit_test(x, y) {
            self.theme_toggle.value = !self.theme_toggle.value;
            let theme = if self.theme_toggle.value { "light" } else { "dark" };
            println!("VYOMA_PREFS:set|display.theme|{}", theme);
        }
        // Resolution pick
        for (i, _) in RESOLUTIONS.iter().enumerate() {
            let ry = 310 + i as i32 * 36;
            if pressed && x >= 260 && x <= 280 && y >= ry && y <= ry + 20 {
                self.resolution_idx = i;
                println!("VYOMA_PREFS:set|display.resolution|{}", RESOLUTIONS[i]);
            }
        }
    }

    fn handle_key(&mut self, _key: &str, _state: &mut AppState) {}
}
```

On startup, `main.rs` sends `VYOMA_PREFS:get|display.brightness`, `VYOMA_PREFS:get|display.theme`, and `VYOMA_PREFS:get|display.resolution`, then parses `VYOMA_PREFS:value|...|...` responses from stdin to initialize panel state.

---

## 8. Capability

No new capability field is required. The settings app declares in `apps/settings/vyoma.toml`:

```toml
[app]
name    = "settings"
version = "0.1.0"
wasm    = "settings.wasm"

[capabilities]
stdio      = true
display    = true
shell      = true
filesystem = true
mouse      = true
```

`display = true` → VYOMA_DRAW access for rendering
`shell = true` → allows `@supervisor:` commands (e.g. for future restart-on-locale-change)
`filesystem = true` → not strictly required by prefs (supervisor-side), but retained for future panel needs

The supervisor `Capabilities` struct in `manifest.rs` requires no new fields. `VYOMA_PREFS:` is handled as a universal protocol line (like `VYOMA_DRAW:`), not gated behind a capability field. Any app that declares `stdio = true` can use the protocol. Future namespace-level access control can be layered on top without breaking existing apps.

---

## 9. Integration with Other Subsystems

### R67 Audio
After a successful `VYOMA_PREFS:set|audio.output_volume|<val>` write (inside `handlers.rs`, after lock is released):

```rust
// handlers.rs — after set_value() returns notifications
if key_str == "audio.output_volume" {
    if let Ok(vol) = raw_value.parse::<u8>() {
        crate::audio::mixer::set_master_volume(vol);
    }
}
if key_str == "audio.muted" {
    let muted = raw_value == "true";
    crate::audio::mixer::set_muted(muted);
}
```

`AudioMixer::set_master_volume` and `set_muted` hold their own mutex independently.

### R73 Notifications
```rust
// handlers.rs
if key_str == "notif.dnd" {
    let dnd = raw_value == "true";
    crate::notify::center::set_dnd(dnd);
}
```

`NotifyCenter::set_dnd` sets an atomic bool, no lock needed.

### R80 L10n
```rust
// handlers.rs
if key_str == "locale.language" {
    crate::l10n::catalog::reload(raw_value);
}
```

`l10n::catalog::reload` swaps the active locale catalog under its own `RwLock`.

All integration calls occur **after** the `PrefsStore` mutex is released, preventing any cross-subsystem deadlock.

---

## 10. Blocking Issues (B1–B5)

### B1 — `deny_unknown_fields` on `Capabilities`
**File**: `supervisor/src/manifest.rs`
**Issue**: `Capabilities` uses `#[serde(deny_unknown_fields)]`. Although the settings subsystem adds no new capability field, any future extension (e.g. `prefs_namespace`) would fail to parse silently as an unknown field error at startup.
**Fix**: Confirm the field list is stable. If `prefs_namespace` is ever added:
```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    // ... existing fields ...
    #[serde(default)]
    pub prefs_namespace: Option<String>,  // e.g. "display" restricts set to display.* keys
}
```
For this spec, no new field is added. The compile-time check from `deny_unknown_fields` is a safety net, not a blocker.

### B2 — Watch Notification Delivery Deadlock
**Issue**: Naive implementation sends watcher notifications while holding the `PrefsStore` mutex. The IPC broker (`ipc::send_to_app`) acquires its own mutex. If any IPC path ever tries to acquire `PrefsStore` (e.g. an app's stdin handler triggering a `VYOMA_PREFS:get`), the system deadlocks (ABBA lock order).
**Fix** (already shown in Section 5): Collect watcher names and the notification string inside the lock, then release the lock before calling `ipc::send_to_app`. The `set_value` function returns `Vec<(app_name, notif_line)>` to the caller in `handlers.rs`, which sends them after the mutex guard is dropped.

```rust
// handlers.rs
let notifications = store::set_value(&store_arc, key_str, raw_value)?;
// MutexGuard is dropped at end of set_value — safe to call IPC now
for (app_name, line) in notifications {
    ipc::send_to_app(&app_name, &line);
}
```

### B3 — JSON Persistence of Mixed `PrefValue` Types
**Issue**: Serializing `HashMap<String, PrefValue>` with `serde_json` where `PrefValue` is `#[serde(untagged)]` produces correct JSON for flat values, but reading back requires knowing the target type. Without schema-guided deserialization, `75` (a JSON number) could be read back as `U32(75)` instead of `U8(75)`, causing a type mismatch on the next `parse_value` call.
**Fix**: Use `PrefValue::to_json_value()` (defined in `schema.rs`) to write each value as a native `serde_json::Value`. On read-back, use `PrefKey::parse_value(&raw_string)` to re-coerce each value into its schema-correct variant. The round-trip is: Rust type → JSON number/string → string → Rust type via `parse_value`. This avoids storing type tags in JSON and keeps the file human-readable.

### B4 — Settings App Slider Drag State (Mouse Capture)
**Issue**: If the user presses on a slider handle and moves the mouse quickly, the cursor may leave the handle bounds before the next event, causing the drag to drop prematurely.
**Fix**: Use a per-panel `dragging: Option<(&'static str, )>` field that captures the active slider key on `mouse_down`, tracks position updates regardless of bounds, and releases on `mouse_up`. The `SliderState.dragging: bool` flag (shown in Section 6) achieves this — it is set on press within the hit region and cleared only on release, not on position bounds check.

State machine:
```
IDLE   --[press in hit region]--> DRAGGING(key)
DRAGGING(key) --[mouse_up]--> IDLE
DRAGGING(key) --[mouse_move]--> DRAGGING(key)  // update value, emit VYOMA_PREFS:set
```
The `AppState` in `main.rs` holds `active_drag: Option<usize>` (slider index) to prevent two sliders from simultaneously entering drag state.

### B5 — Concurrent `VYOMA_PREFS:set` Writers
**Issue**: Two apps simultaneously calling `VYOMA_PREFS:set` on different keys. The supervisor processes app stdout on separate per-app threads.
**Fix**: `Arc<Mutex<PrefsStore>>` already serializes all access. The disk write (`persist()`) occurs inside the lock (see `store.rs`), so the on-disk file is always consistent with the in-memory state at the time of the write. Watcher notifications are sent after the lock is released (B2 fix), which is correct — the second writer's `set` will simply queue behind the first writer's lock acquisition. No additional mechanism is needed. The `Mutex` is the single synchronization primitive; no `RwLock` is used because `set` + `persist` must be atomic with respect to concurrent `set` calls.

---

## Summary

| Component | Files | New Lines | New Capability |
|-----------|-------|-----------|----------------|
| `supervisor/src/prefs/` | 4 | ~850 | None |
| `apps/settings/src/` | 6 | ~1100 | None (uses existing) |
| `apps/settings/vyoma.toml` | 1 | 12 | None |

All blocking issues resolved. The implementation is ready to proceed to task decomposition.
