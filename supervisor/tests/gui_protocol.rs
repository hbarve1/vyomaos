// Integration tests for VYOMA_DRAW protocol parsing.
//
// These tests verify the text-level protocol without requiring a Linux
// framebuffer, WASM runtime, or QEMU. They operate on raw strings only
// and are therefore portable to any host (macOS, Linux CI, etc.).

// ── Protocol parser (inlined — draw_cmd.rs is Linux-only) ────────────────────

#[derive(Debug, Clone, PartialEq)]
enum DrawCmd {
    FillRect { x: u32, y: u32, w: u32, h: u32, rgba: u32 },
    DrawText { x: u32, y: u32, rgba: u32, size: char, text: String },
    DrawTextWrap { x: u32, y: u32, max_w: u32, rgba: u32, size: char, text: String },
    Flush,
}

fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

fn parse_size_char(s: &str) -> Option<char> {
    match s.trim() {
        "s" => Some('s'),
        "m" => Some('m'),
        "l" => Some('l'),
        _ => None,
    }
}

fn parse_draw_cmd(line: &str) -> Option<DrawCmd> {
    let rest = line.strip_prefix("VYOMA_DRAW:")?;

    if rest == "flush" {
        return Some(DrawCmd::Flush);
    }

    if let Some(args) = rest.strip_prefix("fill_rect:") {
        let p: Vec<&str> = args.splitn(5, ',').collect();
        if p.len() == 5 {
            return Some(DrawCmd::FillRect {
                x:    p[0].parse().ok()?,
                y:    p[1].parse().ok()?,
                w:    p[2].parse().ok()?,
                h:    p[3].parse().ok()?,
                rgba: parse_color(p[4])?,
            });
        }
    }

    if let Some(args) = rest.strip_prefix("draw_text:") {
        let p: Vec<&str> = args.splitn(5, ',').collect();
        if p.len() == 5 {
            let size = parse_size_char(p[3])?;
            return Some(DrawCmd::DrawText {
                x:    p[0].parse().ok()?,
                y:    p[1].parse().ok()?,
                rgba: parse_color(p[2])?,
                size,
                text: p[4].to_string(),
            });
        }
    }

    if let Some(args) = rest.strip_prefix("draw_text_wrap:") {
        let p: Vec<&str> = args.splitn(6, ',').collect();
        if p.len() == 6 {
            let size = parse_size_char(p[4])?;
            return Some(DrawCmd::DrawTextWrap {
                x:     p[0].parse().ok()?,
                y:     p[1].parse().ok()?,
                max_w: p[2].parse().ok()?,
                rgba:  parse_color(p[3])?,
                size,
                text:  p[5].to_string(),
            });
        }
    }

    None
}

/// Parse all VYOMA_DRAW commands from a multi-line string.
fn parse_output(output: &str) -> Vec<DrawCmd> {
    output.lines().filter_map(parse_draw_cmd).collect()
}

// ── Test 1: parse_draw_commands_from_app_output ───────────────────────────────

#[test]
fn parse_draw_commands_from_app_output() {
    let output = "\
VYOMA_DRAW:fill_rect:0,0,1440,900,4278190335\n\
VYOMA_DRAW:draw_text:8,24,4294967295,m,VyomaOS\n\
VYOMA_DRAW:draw_text_wrap:10,50,200,4294967295,s,Some description\n\
VYOMA_DRAW:flush\n\
some-other-line-ignored\n\
@supervisor: raise\n";

    let cmds = parse_output(output);

    assert_eq!(cmds.len(), 4, "expected 4 draw commands (3 draw + 1 flush)");

    assert!(
        matches!(&cmds[0], DrawCmd::FillRect { x: 0, y: 0, w: 1440, h: 900, .. }),
        "first command must be fill_rect covering full screen"
    );
    assert!(
        matches!(&cmds[1], DrawCmd::DrawText { text, .. } if text == "VyomaOS"),
        "second command must be draw_text with 'VyomaOS'"
    );
    assert!(
        matches!(&cmds[2], DrawCmd::DrawTextWrap { text, .. } if text == "Some description"),
        "third command must be draw_text_wrap"
    );
    assert_eq!(cmds[3], DrawCmd::Flush, "fourth command must be flush");
}

// ── Test 2: flush_separates_frames ───────────────────────────────────────────

#[test]
fn flush_separates_frames() {
    let output = "\
VYOMA_DRAW:fill_rect:0,0,100,100,100\n\
VYOMA_DRAW:flush\n\
VYOMA_DRAW:fill_rect:0,0,200,200,200\n\
VYOMA_DRAW:fill_rect:0,0,300,300,300\n\
VYOMA_DRAW:flush\n";

    let cmds = parse_output(output);

    // Count flushes — they are frame boundaries.
    let flush_count = cmds.iter().filter(|c| **c == DrawCmd::Flush).count();
    assert_eq!(flush_count, 2, "must parse exactly 2 flush commands");

    // Count fill_rects.
    let fill_count = cmds
        .iter()
        .filter(|c| matches!(c, DrawCmd::FillRect { .. }))
        .count();
    assert_eq!(fill_count, 3, "must parse exactly 3 fill_rect commands across both frames");

    // Verify ordering: fill → flush → fill → fill → flush.
    let kinds: Vec<&str> = cmds
        .iter()
        .map(|c| match c {
            DrawCmd::FillRect { .. } => "fill",
            DrawCmd::Flush => "flush",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["fill", "flush", "fill", "fill", "flush"]);
}

// ── Test 3: desktop_minimum_output_coverage ───────────────────────────────────

#[test]
fn desktop_minimum_output_coverage() {
    // Construct a minimal valid desktop frame: background + 24 icon labels + flush.
    let mut lines = Vec::new();

    // Background fill — covers 1440×900.
    lines.push("VYOMA_DRAW:fill_rect:0,0,1440,900,507716863".to_string());

    // 24 icon tiles (6 columns × 4 rows), each with 2 fill_rects (icon bg + label bg).
    let icon_names = [
        "Terminal", "Files", "Browser", "Settings",
        "Clock",    "Notes", "Music",   "Photos",
        "Mail",     "Maps",  "Weather", "Calendar",
        "Calc",     "Editor","Games",   "Store",
        "Video",    "Code",  "Docs",    "Tasks",
        "Monitor",  "Wifi",  "Disk",    "System",
    ];
    for (i, name) in icon_names.iter().enumerate() {
        let col = (i % 6) as u32;
        let row = (i / 6) as u32;
        let x = 60 + col * 220;
        let y = 80 + row * 180;
        // Icon background.
        lines.push(format!("VYOMA_DRAW:fill_rect:{x},{y},160,120,855638527"));
        // Icon label.
        lines.push(format!("VYOMA_DRAW:draw_text:{},{},4294967295,m,{}", x + 10, y + 130, name));
    }

    // Flush.
    lines.push("VYOMA_DRAW:flush".to_string());

    let output = lines.join("\n");
    let cmds = parse_output(&output);

    let flush_count = cmds.iter().filter(|c| **c == DrawCmd::Flush).count();
    assert!(flush_count >= 1, "must have at least 1 flush; got {flush_count}");

    let fill_count = cmds
        .iter()
        .filter(|c| matches!(c, DrawCmd::FillRect { .. }))
        .count();
    assert!(
        fill_count >= 20,
        "must have at least 20 fill_rect commands; got {fill_count}"
    );

    let text_count = cmds
        .iter()
        .filter(|c| matches!(c, DrawCmd::DrawText { .. } | DrawCmd::DrawTextWrap { .. }))
        .count();
    assert!(
        text_count >= 24,
        "must have at least 24 text commands (one per icon); got {text_count}"
    );

    // Verify all icon names are present as text.
    for name in &icon_names {
        let found = cmds.iter().any(|c| {
            matches!(c, DrawCmd::DrawText { text, .. } | DrawCmd::DrawTextWrap { text, .. }
                if text.contains(name))
        });
        assert!(found, "icon label '{name}' not found in draw commands");
    }
}
