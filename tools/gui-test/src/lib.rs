// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
//! VYOMA_DRAW protocol parser and virtual renderer for GUI testing.
//!
//! Parse VYOMA_DRAW protocol lines and accumulate frames for assertion.

/// A single decoded draw command.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawCmd {
    FillRect { x: u32, y: u32, w: u32, h: u32, rgba: u32 },
    DrawText { x: u32, y: u32, rgba: u32, size: char, text: String },
    DrawTextWrap { x: u32, y: u32, max_w: u32, rgba: u32, size: char, text: String },
    Flush,
}

/// A completed frame — all commands between two Flush calls (inclusive of Flush).
#[derive(Debug, Default, Clone)]
pub struct Frame {
    pub commands: Vec<DrawCmd>,
}

impl Frame {
    /// Number of FillRect commands in this frame.
    pub fn fill_rect_count(&self) -> usize {
        self.commands
            .iter()
            .filter(|c| matches!(c, DrawCmd::FillRect { .. }))
            .count()
    }

    /// Number of DrawText + DrawTextWrap commands in this frame.
    pub fn text_count(&self) -> usize {
        self.commands
            .iter()
            .filter(|c| matches!(c, DrawCmd::DrawText { .. } | DrawCmd::DrawTextWrap { .. }))
            .count()
    }

    /// All text strings from DrawText and DrawTextWrap commands.
    pub fn texts(&self) -> Vec<&str> {
        self.commands
            .iter()
            .filter_map(|c| match c {
                DrawCmd::DrawText { text, .. } => Some(text.as_str()),
                DrawCmd::DrawTextWrap { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Returns true if any text command contains `needle`.
    pub fn has_text(&self, needle: &str) -> bool {
        self.texts().iter().any(|t| t.contains(needle))
    }

    /// Returns true if the region [x, x+w) x [y, y+h) is fully covered by
    /// at least one FillRect command that completely contains it.
    pub fn covers(&self, x: u32, y: u32, w: u32, h: u32) -> bool {
        self.commands.iter().any(|c| {
            if let DrawCmd::FillRect { x: rx, y: ry, w: rw, h: rh, .. } = c {
                *rx <= x
                    && *ry <= y
                    && rx.saturating_add(*rw) >= x.saturating_add(w)
                    && ry.saturating_add(*rh) >= y.saturating_add(h)
            } else {
                false
            }
        })
    }

    /// Count distinct rgba values from FillRect commands.
    pub fn unique_bg_colors(&self) -> usize {
        let mut seen: Vec<u32> = Vec::new();
        for cmd in &self.commands {
            if let DrawCmd::FillRect { rgba, .. } = cmd {
                if !seen.contains(rgba) {
                    seen.push(*rgba);
                }
            }
        }
        seen.len()
    }
}

/// Accumulates VYOMA_DRAW protocol lines into Frames.
pub struct Renderer {
    pub frames: Vec<Frame>,
    pending: Vec<DrawCmd>,
}

impl Renderer {
    pub fn new() -> Self {
        Self { frames: Vec::new(), pending: Vec::new() }
    }

    /// Feed one line of app stdout output.
    pub fn feed_line(&mut self, line: &str) {
        if let Some(cmd) = parse_line(line) {
            if cmd == DrawCmd::Flush {
                let mut frame = Frame::default();
                frame.commands = std::mem::take(&mut self.pending);
                frame.commands.push(DrawCmd::Flush);
                self.frames.push(frame);
            } else {
                self.pending.push(cmd);
            }
        }
    }

    /// Feed the entire captured stdout of an app.
    pub fn feed_output(&mut self, output: &str) {
        for line in output.lines() {
            self.feed_line(line);
        }
    }

    /// Number of completed frames (flush commands seen).
    pub fn flush_count(&self) -> usize {
        self.frames.len()
    }

    /// Last completed frame, if any.
    pub fn last_frame(&self) -> Option<&Frame> {
        self.frames.last()
    }

    /// Total FillRect commands across all frames.
    pub fn total_fill_rects(&self) -> usize {
        self.frames.iter().map(|f| f.fill_rect_count()).sum()
    }

    /// Total text commands across all frames.
    pub fn total_texts(&self) -> usize {
        self.frames.iter().map(|f| f.text_count()).sum()
    }

    /// All text strings across all frames.
    pub fn all_texts(&self) -> Vec<&str> {
        self.frames.iter().flat_map(|f| f.texts()).collect()
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a single VYOMA_DRAW protocol line.
/// Returns None if the line is not a VYOMA_DRAW command or cannot be parsed.
pub fn parse_line(line: &str) -> Option<DrawCmd> {
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
        // Format: x,y,rgba,size,text (5 parts)
        let p: Vec<&str> = args.splitn(5, ',').collect();
        if p.len() == 5 {
            let size_char = parse_size_char(p[3])?;
            return Some(DrawCmd::DrawText {
                x:    p[0].parse().ok()?,
                y:    p[1].parse().ok()?,
                rgba: parse_color(p[2])?,
                size: size_char,
                text: p[4].to_string(),
            });
        }
    }

    if let Some(args) = rest.strip_prefix("draw_text_wrap:") {
        // Format: x,y,max_w,rgba,size,text (6 parts)
        let p: Vec<&str> = args.splitn(6, ',').collect();
        if p.len() == 6 {
            let size_char = parse_size_char(p[4])?;
            return Some(DrawCmd::DrawTextWrap {
                x:     p[0].parse().ok()?,
                y:     p[1].parse().ok()?,
                max_w: p[2].parse().ok()?,
                rgba:  parse_color(p[3])?,
                size:  size_char,
                text:  p[5].to_string(),
            });
        }
    }

    None
}

/// Parse a color: decimal or 0x-prefixed hex.
fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// Parse size char: 's', 'm', or 'l'.
fn parse_size_char(s: &str) -> Option<char> {
    match s.trim() {
        "s" => Some('s'),
        "m" => Some('m'),
        "l" => Some('l'),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fill_rect_valid() {
        let cmd = parse_line("VYOMA_DRAW:fill_rect:10,20,100,50,4278190335");
        assert_eq!(
            cmd,
            Some(DrawCmd::FillRect { x: 10, y: 20, w: 100, h: 50, rgba: 4278190335 })
        );
    }

    #[test]
    fn parse_fill_rect_hex_color() {
        let cmd = parse_line("VYOMA_DRAW:fill_rect:0,0,1440,900,0x1e1e2eff");
        assert!(matches!(cmd, Some(DrawCmd::FillRect { rgba: 0x1e1e2eff, .. })));
    }

    #[test]
    fn parse_draw_text_valid() {
        let cmd = parse_line("VYOMA_DRAW:draw_text:8,24,4294967295,m,Hello World");
        assert_eq!(
            cmd,
            Some(DrawCmd::DrawText {
                x: 8,
                y: 24,
                rgba: 4294967295,
                size: 'm',
                text: "Hello World".to_string(),
            })
        );
    }

    #[test]
    fn parse_draw_text_wrap_valid() {
        let cmd = parse_line("VYOMA_DRAW:draw_text_wrap:0,100,300,4294967295,s,Some text here");
        assert_eq!(
            cmd,
            Some(DrawCmd::DrawTextWrap {
                x: 0,
                y: 100,
                max_w: 300,
                rgba: 4294967295,
                size: 's',
                text: "Some text here".to_string(),
            })
        );
    }

    #[test]
    fn parse_flush() {
        assert_eq!(parse_line("VYOMA_DRAW:flush"), Some(DrawCmd::Flush));
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(parse_line("VYOMA_DRAW:unknown_cmd:1,2,3"), None);
        assert_eq!(parse_line("NOT_VYOMA:flush"), None);
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line("random text"), None);
    }

    #[test]
    fn renderer_accumulates_frames() {
        let mut r = Renderer::new();
        r.feed_line("VYOMA_DRAW:fill_rect:0,0,100,100,4278190080");
        r.feed_line("VYOMA_DRAW:draw_text:10,10,4294967295,m,Title");
        r.feed_line("VYOMA_DRAW:flush");
        assert_eq!(r.flush_count(), 1);
        let frame = r.last_frame().unwrap();
        assert_eq!(frame.fill_rect_count(), 1);
        assert_eq!(frame.text_count(), 1);
    }

    #[test]
    fn renderer_feed_output_multiple_flushes() {
        let output = "VYOMA_DRAW:fill_rect:0,0,100,100,1000\nVYOMA_DRAW:flush\n\
                      VYOMA_DRAW:fill_rect:0,0,200,200,2000\nVYOMA_DRAW:fill_rect:50,50,20,20,3000\n\
                      VYOMA_DRAW:flush\n";
        let mut r = Renderer::new();
        r.feed_output(output);
        assert_eq!(r.flush_count(), 2);
        assert_eq!(r.total_fill_rects(), 3);
        let last = r.last_frame().unwrap();
        assert_eq!(last.fill_rect_count(), 2);
    }

    #[test]
    fn frame_has_text() {
        let mut r = Renderer::new();
        r.feed_line("VYOMA_DRAW:draw_text:0,0,4294967295,m,VyomaOS Desktop");
        r.feed_line("VYOMA_DRAW:flush");
        let frame = r.last_frame().unwrap();
        assert!(frame.has_text("VyomaOS"));
        assert!(frame.has_text("Desktop"));
        assert!(!frame.has_text("NotPresent"));
    }

    #[test]
    fn frame_unique_colors() {
        let mut r = Renderer::new();
        r.feed_line("VYOMA_DRAW:fill_rect:0,0,100,100,100");
        r.feed_line("VYOMA_DRAW:fill_rect:0,0,100,100,200");
        r.feed_line("VYOMA_DRAW:fill_rect:0,0,100,100,100"); // duplicate
        r.feed_line("VYOMA_DRAW:flush");
        let frame = r.last_frame().unwrap();
        assert_eq!(frame.unique_bg_colors(), 2);
    }

    #[test]
    fn frame_covers_region() {
        let mut r = Renderer::new();
        r.feed_line("VYOMA_DRAW:fill_rect:0,0,1440,900,100");
        r.feed_line("VYOMA_DRAW:flush");
        let frame = r.last_frame().unwrap();
        assert!(frame.covers(0, 0, 1440, 900));
        assert!(frame.covers(100, 100, 200, 200));
        assert!(!frame.covers(0, 0, 1441, 900));
    }

    #[test]
    fn renderer_all_texts_across_frames() {
        let mut r = Renderer::new();
        r.feed_line("VYOMA_DRAW:draw_text:0,0,4294967295,m,Frame1");
        r.feed_line("VYOMA_DRAW:flush");
        r.feed_line("VYOMA_DRAW:draw_text:0,0,4294967295,m,Frame2");
        r.feed_line("VYOMA_DRAW:flush");
        let all = r.all_texts();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&"Frame1"));
        assert!(all.contains(&"Frame2"));
    }
}
