// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::collections::{HashMap, VecDeque};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

pub use supervisor::lock_or_recover;

use supervisor::manifest::BootEntry;

pub const LOG_BUF_SIZE: usize = 20;

#[derive(Clone, Debug)]
pub enum AppStatus {
    Running,
    Stopped(i32),
}

pub struct AppState {
    pub entry:            BootEntry,
    pub status:           AppStatus,
    pub start_time:       Instant,
    pub restart_count:    u32,
    pub log_buf:          VecDeque<String>,
    pub child_pid:        Option<u32>,
    pub watchdog_secs:    u32,
    pub last_output:      Arc<Mutex<Instant>>,
    pub watchdog_backoff: Arc<Mutex<u64>>,
    pub has_mouse:   bool,
    pub has_display: bool,
    pub has_audio:   bool,
    pub is_background: bool,
    pub win_region:  Option<(u32, u32, u32, u32)>,
    pub win_z:       u32,
    pub min_size:    (u32, u32),
    pub draw_ticks:      u64,
    pub last_cpu_reset:  Instant,
    pub minimized:           bool,
    pub pre_minimize_region: Option<(u32, u32, u32, u32)>,
    pub is_fullscreen:       bool,
    pub pre_fullscreen_region: Option<(u32, u32, u32, u32)>,
    pub pending_anim: Option<crate::display::animator::Animation>,
    pub surface: Option<Arc<Mutex<crate::display::Surface>>>,
    pub frame_ready: bool,
    pub menu_items: Vec<supervisor::manifest::MenuItem>,
    pub log_subscribers: Vec<mpsc::Sender<String>>,
}

pub type AppRegistry = Arc<Mutex<HashMap<String, Arc<Mutex<AppState>>>>>;
pub type Inbox = Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>;
pub type FocusedApp = Arc<Mutex<Option<String>>>;

pub struct SpawnedApp {
    pub entry:        BootEntry,
    pub name:         String,
    pub child:        Child,
    pub msg_rx:       mpsc::Receiver<String>,
    pub child_stdin:  ChildStdin,
    pub child_stdout: ChildStdout,
    pub has_display:  bool,
    pub is_shell:     bool,
}
