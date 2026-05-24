// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;
const CHAR_W: u32 = 8;
const LINE_H: u32 = 18;

const LIST_W: u32 = 220;
const OUTPUT_H: u32 = 160;
const CODE_X: u32 = LIST_W + 1;
const CODE_Y: u32 = HEADER_H;
const CODE_W: u32 = W - LIST_W - 1;
const OUTPUT_Y: u32 = H - STATUS_H - OUTPUT_H;
const CODE_LINES: usize = ((OUTPUT_Y - CODE_Y) / LINE_H) as usize;
const CODE_COLS: usize = ((CODE_W - 8) / CHAR_W) as usize;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_KW: u32     = C_SEL;
const C_STR: u32    = C_GREEN;
const C_NUM: u32    = C_ORANGE;
const C_CMT: u32    = C_HINT;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "if", "else", "for", "while", "loop", "return",
    "struct", "impl", "use", "pub", "mod", "match", "Some", "None",
    "Ok", "Err", "true", "false", "in", "as", "type", "enum", "const",
    "Vec", "String", "i32", "i64", "u32", "u64", "usize", "bool", "str",
    "println", "print", "eprintln", "format",
];

fn colorize(line: &str) -> Vec<(u32, &str)> {
    let mut segments: Vec<(u32, &str)> = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut seg_start = 0usize;
    let mut cur_col = C_TEXT;

    macro_rules! push_seg {
        ($end:expr, $col:expr) => {
            if seg_start < $end {
                segments.push(($col, &line[seg_start..$end]));
            }
        };
    }

    while i < len {
        // Line comment
        if i + 1 < len && bytes[i] == b'/' && bytes[i+1] == b'/' {
            push_seg!(i, cur_col);
            segments.push((C_CMT, &line[i..]));
            return segments;
        }
        // String literal
        if bytes[i] == b'"' {
            push_seg!(i, cur_col);
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'"' {
                if bytes[i] == b'\\' { i += 1; }
                i += 1;
            }
            if i < len { i += 1; }
            segments.push((C_STR, &line[start..i]));
            seg_start = i;
            cur_col = C_TEXT;
            continue;
        }
        // Number
        if bytes[i].is_ascii_digit() && (i == 0 || !bytes[i-1].is_ascii_alphanumeric()) {
            push_seg!(i, cur_col);
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.') {
                i += 1;
            }
            segments.push((C_NUM, &line[start..i]));
            seg_start = i;
            cur_col = C_TEXT;
            continue;
        }
        // Identifier / keyword
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &line[start..i];
            let is_kw = KEYWORDS.contains(&word);
            if is_kw {
                push_seg!(start, cur_col);
                segments.push((C_KW, word));
                seg_start = i;
                cur_col = C_TEXT;
            }
            continue;
        }
        i += 1;
    }
    push_seg!(len, cur_col);
    segments
}

struct Snippet {
    name:   &'static str,
    code:   &'static str,
    output: &'static str,
}

const SNIPPETS: &[Snippet] = &[
    Snippet {
        name: "Hello World",
        code: "fn main() {\n    println!(\"Hello, VyomaOS!\");\n}",
        output: "Hello, VyomaOS!\n\nProcess exited with code 0 (in 0.12s)",
    },
    Snippet {
        name: "Fibonacci",
        code: "fn fib(n: u64) -> u64 {\n    if n <= 1 { return n; }\n    fib(n-1) + fib(n-2)\n}\n\nfn main() {\n    for i in 0..10 {\n        println!(\"{}: {}\", i, fib(i));\n    }\n}",
        output: "0: 0\n1: 1\n2: 1\n3: 2\n4: 3\n5: 5\n6: 8\n7: 13\n8: 21\n9: 34\n\nProcess exited with code 0 (in 0.18s)",
    },
    Snippet {
        name: "FizzBuzz",
        code: "fn main() {\n    for i in 1..=20 {\n        match (i % 3, i % 5) {\n            (0, 0) => println!(\"FizzBuzz\"),\n            (0, _) => println!(\"Fizz\"),\n            (_, 0) => println!(\"Buzz\"),\n            _      => println!(\"{}\", i),\n        }\n    }\n}",
        output: "1\n2\nFizz\n4\nBuzz\nFizz\n7\n8\nFizz\nBuzz\n11\nFizz\n13\n14\nFizzBuzz\n16\n17\nFizz\n19\nBuzz\n\nProcess exited with code 0 (in 0.14s)",
    },
    Snippet {
        name: "Sort Vec",
        code: "fn main() {\n    let mut v = vec![5, 2, 8, 1, 9, 3];\n    v.sort();\n    println!(\"{:?}\", v);\n\n    v.sort_by(|a, b| b.cmp(a));\n    println!(\"{:?}\", v);\n}",
        output: "[1, 2, 3, 5, 8, 9]\n[9, 8, 5, 3, 2, 1]\n\nProcess exited with code 0 (in 0.11s)",
    },
    Snippet {
        name: "Closures",
        code: "fn apply<F: Fn(i32) -> i32>(f: F, x: i32) -> i32 {\n    f(x)\n}\n\nfn main() {\n    let double = |x| x * 2;\n    let add_ten = |x| x + 10;\n\n    println!(\"{}\", apply(double, 5));\n    println!(\"{}\", apply(add_ten, 5));\n\n    let nums: Vec<i32> = (1..=5)\n        .map(|x| x * x)\n        .filter(|x| x % 2 == 0)\n        .collect();\n    println!(\"{:?}\", nums);\n}",
        output: "10\n15\n[4, 16]\n\nProcess exited with code 0 (in 0.15s)",
    },
    Snippet {
        name: "Structs",
        code: "struct Point {\n    x: f64,\n    y: f64,\n}\n\nimpl Point {\n    fn new(x: f64, y: f64) -> Self {\n        Point { x, y }\n    }\n    fn distance(&self, other: &Point) -> f64 {\n        let dx = self.x - other.x;\n        let dy = self.y - other.y;\n        (dx*dx + dy*dy).sqrt()\n    }\n}\n\nfn main() {\n    let a = Point::new(0.0, 0.0);\n    let b = Point::new(3.0, 4.0);\n    println!(\"{}\", a.distance(&b));\n}",
        output: "5\n\nProcess exited with code 0 (in 0.16s)",
    },
    Snippet {
        name: "Enums",
        code: "enum Shape {\n    Circle(f64),\n    Rectangle(f64, f64),\n    Triangle(f64, f64, f64),\n}\n\nimpl Shape {\n    fn area(&self) -> f64 {\n        match self {\n            Shape::Circle(r) => 3.14159 * r * r,\n            Shape::Rectangle(w, h) => w * h,\n            Shape::Triangle(a, b, c) => {\n                let s = (a+b+c)/2.0;\n                (s*(s-a)*(s-b)*(s-c)).sqrt()\n            }\n        }\n    }\n}\n\nfn main() {\n    let shapes = vec![\n        Shape::Circle(5.0),\n        Shape::Rectangle(4.0, 6.0),\n        Shape::Triangle(3.0, 4.0, 5.0),\n    ];\n    for s in &shapes {\n        println!(\"{:.2}\", s.area());\n    }\n}",
        output: "78.54\n24.00\n6.00\n\nProcess exited with code 0 (in 0.19s)",
    },
    Snippet {
        name: "Iterators",
        code: "fn main() {\n    let sum: i32 = (1..=100).sum();\n    println!(\"Sum 1-100: {}\", sum);\n\n    let evens: Vec<i32> = (1..=10)\n        .filter(|x| x % 2 == 0)\n        .collect();\n    println!(\"Evens: {:?}\", evens);\n\n    let words = vec![\"hello\", \"world\", \"vyoma\"];\n    let upper: Vec<String> = words.iter()\n        .map(|w| w.to_uppercase())\n        .collect();\n    println!(\"{:?}\", upper);\n}",
        output: "Sum 1-100: 5050\nEvens: [2, 4, 6, 8, 10]\n[\"HELLO\", \"WORLD\", \"VYOMA\"]\n\nProcess exited with code 0 (in 0.13s)",
    },
    Snippet {
        name: "Error Handling",
        code: "use std::num::ParseIntError;\n\nfn parse_and_double(s: &str) -> Result<i32, ParseIntError> {\n    let n = s.trim().parse::<i32>()?;\n    Ok(n * 2)\n}\n\nfn main() {\n    let inputs = [\"42\", \"bad\", \"100\"];\n    for s in &inputs {\n        match parse_and_double(s) {\n            Ok(n)  => println!(\"{} -> {}\", s, n),\n            Err(e) => println!(\"{} -> Error: {}\", s, e),\n        }\n    }\n}",
        output: "42 -> 84\nbad -> Error: invalid digit found in string\n100 -> 200\n\nProcess exited with code 0 (in 0.14s)",
    },
    Snippet {
        name: "HashMap",
        code: "use std::collections::HashMap;\n\nfn main() {\n    let mut scores: HashMap<&str, i32> = HashMap::new();\n    scores.insert(\"Alice\", 95);\n    scores.insert(\"Bob\", 87);\n    scores.insert(\"Carol\", 91);\n\n    for (name, score) in &scores {\n        println!(\"{}: {}\", name, score);\n    }\n\n    let avg: i32 = scores.values().sum::<i32>() / scores.len() as i32;\n    println!(\"Average: {}\", avg);\n}",
        output: "Alice: 95\nBob: 87\nCarol: 91\nAverage: 91\n\nProcess exited with code 0 (in 0.17s)",
    },
];

fn draw_code(snip: &Snippet, scroll: usize, running: bool, output_scroll: usize) {
    let lines: Vec<&str> = snip.code.split('\n').collect();
    let total = lines.len();
    let vis = CODE_LINES;
    let start = scroll.min(total.saturating_sub(vis));
    let end = (start + vis).min(total);

    for (i, idx) in (start..end).enumerate() {
        let y = CODE_Y + i as u32 * LINE_H + 2;
        // Gutter
        text(CODE_X + 2, y, C_HINT, &format!("{:3}", idx + 1));

        let line = lines[idx];
        let segs = colorize(line);
        let mut sx = CODE_X + 32;
        for (col, seg) in segs {
            let max_c = (CODE_W - (sx - CODE_X) - 8) as usize / CHAR_W as usize;
            if max_c == 0 { break; }
            let display = if seg.len() > max_c { &seg[..max_c] } else { seg };
            text(sx, y, col, display);
            sx += display.len() as u32 * CHAR_W;
        }
    }

    // Output panel
    let out_bg = if running { 0x0D2B0DFF } else { C_CARD };
    fill(CODE_X, OUTPUT_Y, CODE_W, OUTPUT_H, out_bg);
    fill(CODE_X, OUTPUT_Y, CODE_W, 1, C_BORDER);
    let run_label = if running { "▶ Running..." } else { "Output (press Enter to run)" };
    text(CODE_X + 8, OUTPUT_Y + 4, if running { C_GREEN } else { C_HINT }, run_label);

    let out_lines: Vec<&str> = snip.output.split('\n').collect();
    let out_vis = ((OUTPUT_H - 22) / LINE_H) as usize;
    let os = output_scroll.min(out_lines.len().saturating_sub(out_vis));
    let oe = (os + out_vis).min(out_lines.len());
    for (i, idx) in (os..oe).enumerate() {
        let y = OUTPUT_Y + 22 + i as u32 * LINE_H;
        let col = if out_lines[idx].contains("Process exited") { C_HINT }
                  else { C_TEXT };
        let max_c = (CODE_W - 16) as usize / CHAR_W as usize;
        let d = if out_lines[idx].len() > max_c { &out_lines[idx][..max_c] } else { out_lines[idx] };
        text(CODE_X + 8, y, col, d);
    }
}

fn draw(snips: &[Snippet], selected: usize, scroll: usize, running: bool, output_scroll: usize) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Code Runner");
    text(180, 16, C_HINT, "↑↓:select  Enter:run  PgUp/Dn:scroll code  Ctrl+C:exit");

    // Snippet list
    fill(0, HEADER_H, LIST_W, H - HEADER_H - STATUS_H, C_CARD);
    fill(LIST_W, HEADER_H, 1, H - HEADER_H - STATUS_H, C_BORDER);
    text(8, HEADER_H + 6, C_HINT, "Snippets:");
    for (i, s) in snips.iter().enumerate() {
        let sy = HEADER_H + 26 + i as u32 * 24;
        if i == selected {
            fill(0, sy - 2, LIST_W, 22, 0x1C2D4EFF);
            text(8, sy, C_SEL, s.name);
        } else {
            text(8, sy, C_HINT, s.name);
        }
    }

    // Code area
    fill(CODE_X, CODE_Y, CODE_W, H - HEADER_H - STATUS_H, C_BG);
    text(CODE_X + 32, CODE_Y + 2, C_HINT, &format!("// {}", snips[selected].name));
    let code_top = CODE_Y + LINE_H;
    // Adjust: draw code starting from code_top
    let lines: Vec<&str> = snips[selected].code.split('\n').collect();
    let total = lines.len();
    let vis = CODE_LINES.saturating_sub(1); // -1 for the name line
    let start = scroll.min(total.saturating_sub(vis));
    let end = (start + vis).min(total);
    for (i, idx) in (start..end).enumerate() {
        let y = code_top + i as u32 * LINE_H + 2;
        text(CODE_X + 2, y, C_HINT, &format!("{:3}", idx + 1));
        let segs = colorize(lines[idx]);
        let mut sx = CODE_X + 32;
        for (col, seg) in segs {
            let max_c = (CODE_W - (sx - CODE_X) - 8) as usize / CHAR_W as usize;
            if max_c == 0 { break; }
            let display = if seg.len() > max_c { &seg[..max_c] } else { seg };
            text(sx, y, col, display);
            sx += display.len() as u32 * CHAR_W;
        }
    }

    // Output panel
    let out_bg = if running { 0x0D2B0DFF } else { C_CARD };
    fill(CODE_X, OUTPUT_Y, CODE_W, OUTPUT_H, out_bg);
    fill(CODE_X, OUTPUT_Y, CODE_W, 1, C_BORDER);
    let run_label = if running { "  Running..." } else { "  Output (Enter to run)" };
    let run_col = if running { C_GREEN } else { C_HINT };
    text(CODE_X + 8, OUTPUT_Y + 4, run_col, run_label);

    let out_lines: Vec<&str> = snips[selected].output.split('\n').collect();
    let out_vis = ((OUTPUT_H - 22) / LINE_H) as usize;
    let os = output_scroll.min(out_lines.len().saturating_sub(out_vis));
    let oe = (os + out_vis).min(out_lines.len());
    for (i, idx) in (os..oe).enumerate() {
        let y = OUTPUT_Y + 22 + i as u32 * LINE_H;
        let out_col = if out_lines[idx].contains("Process exited") { C_HINT }
                      else if running { C_GREEN } else { C_TEXT };
        let max_c = (CODE_W - 16) as usize / CHAR_W as usize;
        let d = if out_lines[idx].len() > max_c { &out_lines[idx][..max_c] } else { out_lines[idx] };
        text(CODE_X + 8, y, out_col, d);
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let line_count = snips[selected].code.split('\n').count();
    text(8, sb_y + 6, C_HINT, &format!("{} | {} lines | Enter=run",
        snips[selected].name, line_count));
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut selected = 0usize;
    let mut scroll = 0usize;
    let mut output_scroll = 0usize;
    let mut running = false;

    println!("@supervisor: raise code-runner");
    let _ = io::stdout().flush();
    draw(SNIPPETS, selected, scroll, running, output_scroll);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        let n = SNIPPETS.len();
        let line_count = SNIPPETS[selected].code.split('\n').count();

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => {
                if selected > 0 { selected -= 1; scroll = 0; output_scroll = 0; running = false; }
            }
            "\x1b[B" => {
                if selected + 1 < n { selected += 1; scroll = 0; output_scroll = 0; running = false; }
            }
            "\x1b[5~" => { scroll = scroll.saturating_sub(CODE_LINES / 2); }
            "\x1b[6~" => {
                scroll = (scroll + CODE_LINES / 2).min(line_count.saturating_sub(CODE_LINES));
            }
            "" => { running = !running; output_scroll = 0; }
            _ => {}
        }
        draw(SNIPPETS, selected, scroll, running, output_scroll);
    }
}
