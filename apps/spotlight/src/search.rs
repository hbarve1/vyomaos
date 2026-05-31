// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Search sources: Apps, Files, Commands.

/// A single search result with category, display name, and action detail.
#[derive(Clone)]
pub struct SearchResult {
    pub category: Category,
    pub name: String,
    pub detail: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Category {
    App,
    File,
    Command,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::App => "Apps",
            Category::File => "Files",
            Category::Command => "Commands",
        }
    }
}

/// All known apps: (display_name, launch_command)
const ALL_APPS: &[(&str, &str)] = &[
    ("Shell", "shell"),
    ("Notes", "notes"),
    ("System Monitor", "system-monitor"),
    ("Settings", "settings"),
    ("System Preferences", "system-preferences"),
    ("Fractal", "fractal"),
    ("Music Theory", "music-theory"),
    ("Geo Quiz", "geo-quiz"),
    ("Clock", "clock"),
    ("Calculator", "sci-calculator"),
    ("Chess", "chess"),
    ("Snake", "snake"),
    ("Tetris", "tetris"),
    ("Pong", "pong"),
    ("JSON Viewer", "json-viewer"),
    ("Hex Editor", "hex-editor"),
    ("Paint", "paint"),
    ("Calendar", "calendar"),
    ("Kanban", "kanban"),
    ("Weather", "weather"),
    ("Color Picker", "color-picker"),
    ("Markdown Viewer", "markdown-viewer"),
    ("Music Player", "music-player"),
    ("Photo Editor", "photo-editor"),
    ("Browser", "browser"),
    ("Code Editor", "code-editor"),
    ("Music Visualizer", "music-viz"),
    ("File Manager", "file-manager"),
    ("Text Editor", "text-editor"),
    ("Task Manager", "task-manager"),
    ("Terminal", "terminal"),
    ("Activity Monitor", "activity-monitor"),
    ("Image Viewer", "image-viewer"),
    ("Finder", "finder"),
    ("Pomodoro", "pomodoro"),
    ("Stopwatch", "stopwatch"),
    ("World Clock", "world-clock"),
    ("Alarm", "alarm"),
    ("Sudoku", "sudoku"),
    ("Minesweeper", "minesweeper"),
    ("Spreadsheet", "spreadsheet"),
    ("RSS Reader", "rss-reader"),
    ("Video Player", "video-player"),
    ("Audio Player", "audio-player"),
    ("Password Manager", "password-manager"),
    ("Unit Converter", "unit-converter"),
    ("Quick Look", "quick-look"),
    ("Snippets", "snippets"),
    ("Mind Map", "mind-map"),
    ("Presentation", "presentation"),
    ("Log Viewer", "log-viewer"),
];

/// Supervisor commands: (display_name, command_string)
const ALL_COMMANDS: &[(&str, &str)] = &[
    ("List processes", "ps"),
    ("Kill app", "kill"),
    ("Restart app", "restart"),
    ("Focus app", "focus"),
    ("List apps", "list"),
    ("Reload config", "reload"),
    ("Show logs", "log"),
    ("Follow logs", "logf"),
    ("Raise window", "raise"),
    ("Lower window", "lower"),
    ("Run app", "run"),
];

const MAX_PER_CATEGORY: usize = 10;

/// Search all sources filtered by `query` (case-insensitive substring).
pub fn search(query: &str, files: &[String]) -> Vec<SearchResult> {
    let q = query.to_ascii_lowercase();
    let mut results = Vec::new();

    // --- Apps ---
    let mut app_count = 0;
    for &(name, cmd) in ALL_APPS {
        if app_count >= MAX_PER_CATEGORY {
            break;
        }
        if q.is_empty()
            || name.to_ascii_lowercase().contains(&q)
            || cmd.to_ascii_lowercase().contains(&q)
        {
            results.push(SearchResult {
                category: Category::App,
                name: name.to_string(),
                detail: cmd.to_string(),
            });
            app_count += 1;
        }
    }

    // --- Files ---
    if !q.is_empty() {
        let mut file_count = 0;
        for path in files {
            if file_count >= MAX_PER_CATEGORY {
                break;
            }
            let fname = path.rsplit('/').next().unwrap_or(path);
            if fname.to_ascii_lowercase().contains(&q)
                || path.to_ascii_lowercase().contains(&q)
            {
                results.push(SearchResult {
                    category: Category::File,
                    name: fname.to_string(),
                    detail: path.clone(),
                });
                file_count += 1;
            }
        }
    }

    // --- Commands ---
    let mut cmd_count = 0;
    for &(name, cmd) in ALL_COMMANDS {
        if cmd_count >= MAX_PER_CATEGORY {
            break;
        }
        if q.is_empty()
            || name.to_ascii_lowercase().contains(&q)
            || cmd.to_ascii_lowercase().contains(&q)
        {
            results.push(SearchResult {
                category: Category::Command,
                name: name.to_string(),
                detail: cmd.to_string(),
            });
            cmd_count += 1;
        }
    }

    results
}
