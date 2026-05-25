// vyoma monitor — live crossterm dashboard showing all app heartbeats.
//
// Layout:
//   Header: "VyomaOS Monitor  [host:port]  [timestamp]"
//   Table:  NAME | STATUS (colored) | UPTIME | MEM_KB | LAST_ERROR
//
// Refresh every 2 seconds from heartbeat stream.
// Press `q` or Ctrl+C to exit cleanly.

use std::{
    collections::HashMap,
    io::{self, Write},
    sync::{atomic::{AtomicBool, Ordering}, Arc},
    time::Instant,
};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
};

use crate::{
    connection::VyomaConnection,
    protocol::{MgmtRequest, MgmtResponse},
};

// ── AppHandle ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppHandle {
    status:     String,
    uptime_s:   u64,
    mem_kb:     u64,
    last_error: String,
    seen_at:    Instant,
}

// ── redraw ────────────────────────────────────────────────────────────────────

fn redraw(apps: &HashMap<String, AppHandle>, host: &str, port: u16) -> anyhow::Result<()> {
    let mut stdout = io::stdout();
    execute!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0),
    )?;

    // Header
    let title = format!(
        "VyomaOS Monitor  {}:{}  {} apps",
        host, port, apps.len()
    );
    writeln!(stdout, "{title}")?;
    writeln!(stdout, "{}", "─".repeat(72))?;
    writeln!(
        stdout,
        "{:<20} {:<12} {:<12} {:<8} {}",
        "NAME", "STATUS", "UPTIME", "MEM_KB", "LAST_ERROR"
    )?;
    writeln!(stdout, "{}", "─".repeat(72))?;

    let mut names: Vec<&String> = apps.keys().collect();
    names.sort();

    for name in names {
        let h = &apps[name];
        let uptime = format_uptime(h.uptime_s);
        let color = status_color(&h.status);

        execute!(stdout, SetForegroundColor(color))?;
        write!(stdout, "{:<20} {:<12} ", name, h.status)?;
        execute!(stdout, ResetColor)?;
        writeln!(
            stdout,
            "{:<12} {:<8} {}",
            uptime, h.mem_kb, h.last_error
        )?;
    }

    writeln!(stdout)?;
    writeln!(stdout, "Press q or Ctrl+C to exit")?;
    stdout.flush()?;
    Ok(())
}

fn status_color(status: &str) -> Color {
    match status {
        "healthy" | "running" => Color::Green,
        "degraded"            => Color::Yellow,
        "unhealthy" | "stopped" => Color::Red,
        _                     => Color::White,
    }
}

fn format_uptime(s: u64) -> String {
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {}s", s / 60, s % 60)
    } else {
        format!("{}h {}m {}s", s / 3600, (s % 3600) / 60, s % 60)
    }
}

// ── run ────────────────────────────────────────────────────────────────────────

/// Launch the live monitor dashboard.
pub fn run(conn: &mut VyomaConnection, host: &str, port: u16) -> anyhow::Result<()> {
    let quit = Arc::new(AtomicBool::new(false));

    terminal::enable_raw_mode()?;
    execute!(io::stdout(), cursor::Hide)?;

    // Keypress watcher thread.
    let quit_flag = Arc::clone(&quit);
    std::thread::spawn(move || {
        loop {
            if let Ok(Event::Key(k)) = event::read() {
                let should_quit = k.code == KeyCode::Char('q')
                    || (k.code == KeyCode::Char('c')
                        && k.modifiers.contains(KeyModifiers::CONTROL));
                if should_quit {
                    quit_flag.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }
    });

    conn.send(&MgmtRequest::HeartbeatStream)?;

    let mut apps: HashMap<String, AppHandle> = HashMap::new();
    let mut last_draw = Instant::now();

    let result: anyhow::Result<()> = (|| {
        loop {
            if quit.load(Ordering::Relaxed) {
                return Ok(());
            }

            match conn.recv() {
                Ok(MgmtResponse::Heartbeat { module, uptime_s, mem_kb, status, last_error }) => {
                    apps.insert(module, AppHandle {
                        status,
                        uptime_s,
                        mem_kb,
                        last_error,
                        seen_at: Instant::now(),
                    });
                }
                Ok(MgmtResponse::Error { code, message }) => {
                    return Err(anyhow::anyhow!("monitor error [{code}]: {message}"));
                }
                Ok(_) => {}
                Err(e) => return Err(anyhow::anyhow!("connection error: {e}")),
            }

            if last_draw.elapsed().as_millis() >= 2000 {
                redraw(&apps, host, port)?;
                last_draw = Instant::now();
            }
        }
    })();

    // Restore terminal on exit.
    let _ = terminal::disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        cursor::Show,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0),
    );

    result
}
