// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
//! gui-test — VYOMA_DRAW protocol checker CLI.
//!
//! Reads VYOMA_DRAW lines from stdin, runs the Renderer, then validates
//! assertions passed as CLI flags.
//!
//! USAGE: gui-test [OPTIONS]
//!   --expect-flush N     Assert at least N flush commands
//!   --expect-fills N     Assert at least N total fill_rect commands
//!   --expect-text STR    Assert at least one text command contains STR
//!   --sw W               Virtual screen width (informational, default 1440)
//!   --sh H               Virtual screen height (informational, default 900)

use std::io::{self, BufRead};

use gui_test::Renderer;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut expect_flush: Option<usize> = None;
    let mut expect_fills: Option<usize> = None;
    let mut expect_texts: Vec<String>   = Vec::new();
    let mut screen_w: u32               = 1440;
    let mut screen_h: u32               = 900;

    // Simple positional flag parser.
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--expect-flush" => {
                i += 1;
                expect_flush = Some(args.get(i).and_then(|v| v.parse().ok()).unwrap_or(0));
            }
            "--expect-fills" => {
                i += 1;
                expect_fills = Some(args.get(i).and_then(|v| v.parse().ok()).unwrap_or(0));
            }
            "--expect-text" => {
                i += 1;
                if let Some(s) = args.get(i) {
                    expect_texts.push(s.clone());
                }
            }
            "--sw" => {
                i += 1;
                screen_w = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(1440);
            }
            "--sh" => {
                i += 1;
                screen_h = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(900);
            }
            other => {
                eprintln!("gui-test: unknown flag: {other}");
            }
        }
        i += 1;
    }

    // Feed stdin into the renderer.
    let stdin = io::stdin();
    let mut renderer = Renderer::new();
    for line in stdin.lock().lines() {
        match line {
            Ok(l) => renderer.feed_line(&l),
            Err(_) => break,
        }
    }

    let frames      = renderer.flush_count();
    let fill_rects  = renderer.total_fill_rects();
    let texts       = renderer.total_texts();
    let last_frame  = renderer.last_frame();
    let unique_cols = last_frame.map(|f| f.unique_bg_colors()).unwrap_or(0);

    // Validate assertions.
    let mut failures: Vec<String> = Vec::new();

    if let Some(n) = expect_flush {
        if frames < n {
            failures.push(format!("expected flush >= {n} but got {frames}"));
        }
    }

    if let Some(n) = expect_fills {
        if fill_rects < n {
            failures.push(format!("expected fill_rects >= {n} but got {fill_rects}"));
        }
    }

    for needle in &expect_texts {
        let found = renderer.all_texts().iter().any(|t| t.contains(needle.as_str()));
        if !found {
            failures.push(format!("expected text containing {:?} but not found", needle));
        }
    }

    // Print summary (screen dimensions are informational).
    let _ = screen_w;
    let _ = screen_h;
    println!(
        "GUI-TEST: frames={frames} fill_rects={fill_rects} texts={texts} unique_colors={unique_cols}"
    );

    if failures.is_empty() {
        println!("GUI-TEST: PASS");
        std::process::exit(0);
    } else {
        for msg in &failures {
            println!("GUI-TEST: FAIL: {msg}");
        }
        std::process::exit(1);
    }
}
