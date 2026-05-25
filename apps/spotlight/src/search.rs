// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Search/filter logic for Spotlight launcher.

/// All known apps: (display_name, launch_command)
pub const ALL_APPS: &[(&str, &str)] = &[
    ("Shell",             "shell"),
    ("Notes",             "notes"),
    ("System Monitor",    "system-monitor"),
    ("Settings",          "settings"),
    ("Fractal",           "fractal"),
    ("Music Theory",      "music-theory"),
    ("Geo Quiz",          "geo-quiz"),
    ("Clock",             "clock"),
    ("Calculator",        "sci-calculator"),
    ("Chess",             "chess"),
    ("Snake",             "snake"),
    ("Tetris",            "tetris"),
    ("Pong",              "pong"),
    ("JSON Viewer",       "json-viewer"),
    ("Hex Editor",        "hex-editor"),
    ("Paint",             "paint"),
    ("Calendar",          "calendar"),
    ("Kanban",            "kanban"),
    ("Weather",           "weather"),
    ("Color Picker",      "color-picker"),
    ("Markdown Viewer",   "markdown-viewer"),
    ("Music Player",      "music-player"),
    ("Photo Editor",      "photo-editor"),
    ("Browser",           "browser"),
    ("Code Editor",       "code-editor"),
    ("Music Visualizer",  "music-viz"),
    ("Spotlight",         "spotlight"),
    ("File Manager",      "file-manager"),
    ("Text Editor",       "text-editor"),
    ("Task Manager",      "task-manager"),
];

/// Return apps whose name or command contains `query` (case-insensitive).
pub fn filter(query: &str) -> Vec<(&'static str, &'static str)> {
    let q = query.to_ascii_lowercase();
    ALL_APPS
        .iter()
        .filter(|(name, cmd)| {
            q.is_empty()
                || name.to_ascii_lowercase().contains(&q)
                || cmd.to_ascii_lowercase().contains(&q)
        })
        .map(|&(name, cmd)| (name, cmd))
        .collect()
}
