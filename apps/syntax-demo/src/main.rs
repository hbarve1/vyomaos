// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

mod app;

use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HDR: u32 = 40;
const SB_H: u32 = 28;
const PANE_W: u32 = W / 2;     // 600 each
const CONTENT_Y: u32 = HDR;
const CONTENT_H: u32 = H - HDR - SB_H;
const LINE_H: u32 = 18;
const GUTTER_W: u32 = 40;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_YELLOW: u32 = 0xD29922FF;
const C_PURPLE: u32 = 0xBC8CFFFF;
const C_RED: u32    = 0xFF7B72FF;
const C_CARD: u32   = 0x161B22FF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

const RUST_EXAMPLE: &[&str] = &[
    "use std::collections::HashMap;",
    "",
    "struct Cache {",
    "    data: HashMap<String, Vec<u32>>,",
    "    max_size: usize,",
    "}",
    "",
    "impl Cache {",
    "    fn new(max_size: usize) -> Self {",
    "        Cache { data: HashMap::new(), max_size }",
    "    }",
    "",
    "    fn insert(&mut self, key: String, val: u32) {",
    "        // Evict if at capacity",
    "        if self.data.len() >= self.max_size {",
    "            self.data.clear();",
    "        }",
    "        self.data.entry(key).or_default().push(val);",
    "    }",
    "",
    "    fn get(&self, key: &str) -> Option<&Vec<u32>> {",
    "        self.data.get(key)",
    "    }",
    "}",
    "",
    "fn main() {",
    "    let mut cache = Cache::new(100);",
    "    cache.insert(\"hits\".to_string(), 42);",
    "    if let Some(v) = cache.get(\"hits\") {",
    "        println!(\"found: {:?}\", v);",
    "    }",
    "}",
];

const PYTHON_EXAMPLE: &[&str] = &[
    "from dataclasses import dataclass, field",
    "from typing import Optional, List",
    "",
    "@dataclass",
    "class Node:",
    "    value: int",
    "    left: Optional['Node'] = None",
    "    right: Optional['Node'] = None",
    "",
    "class BST:",
    "    def __init__(self):",
    "        self.root = None",
    "",
    "    def insert(self, val: int) -> None:",
    "        # Recursive insert",
    "        def _insert(node, v):",
    "            if node is None:",
    "                return Node(v)",
    "            if v < node.value:",
    "                node.left = _insert(node.left, v)",
    "            else:",
    "                node.right = _insert(node.right, v)",
    "            return node",
    "        self.root = _insert(self.root, val)",
    "",
    "    def inorder(self) -> List[int]:",
    "        result = []",
    "        def _walk(n):",
    "            if n: _walk(n.left); result.append(n.value); _walk(n.right)",
    "        _walk(self.root)",
    "        return result",
    "",
    "if __name__ == '__main__':",
    "    t = BST()",
    "    for x in [5, 3, 7, 1, 4, 6, 8]:",
    "        t.insert(x)",
    "    print(t.inorder())  # [1, 3, 4, 5, 6, 7, 8]",
];

const JSON_EXAMPLE: &[&str] = &[
    "{",
    "  \"app\": \"VyomaOS\",",
    "  \"version\": \"1.0.0\",",
    "  \"architecture\": \"wasm32-wasip2\",",
    "  \"capabilities\": {",
    "    \"display\": true,",
    "    \"stdio\": true,",
    "    \"filesystem\": false,",
    "    \"network\": false",
    "  },",
    "  \"window\": {",
    "    \"x\": 60,",
    "    \"y\": 30,",
    "    \"width\": 1200,",
    "    \"height\": 760",
    "  },",
    "  \"dependencies\": [",
    "    \"wasmtime-43.0.0\",",
    "    \"linux-5.10-allnoconfig\",",
    "    \"musl-1.2.4\"",
    "  ],",
    "  \"build\": {",
    "    \"rust\": \"1.87.0\",",
    "    \"target\": \"wasm32-wasip2\",",
    "    \"profile\": \"release\",",
    "    \"opt_level\": \"z\",",
    "    \"strip\": true",
    "  }",
    "}",
];

fn main() {
    let stdin = io::stdin();
    let mut a = app::App::new();

    println!("@supervisor: raise syntax-demo");
    let _ = io::stdout().flush();
    app::draw(&a);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        let all_count = a.lines.len() + 1;
        let vis_lines = ((CONTENT_H - 21) / LINE_H) as usize;

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }

            "\x7f" => {
                if !a.cur.is_empty() {
                    a.cur.pop();
                } else if !a.lines.is_empty() {
                    a.cur = a.lines.pop().unwrap_or_default();
                }
            }

            "" => {
                // Enter — finalize line
                a.lines.push(std::mem::take(&mut a.cur));
                // Auto-scroll to bottom
                if all_count + 1 > vis_lines {
                    a.scroll = all_count + 1 - vis_lines;
                }
            }

            // Scroll
            "\x1b[A" => { if a.scroll > 0 { a.scroll -= 1; } }
            "\x1b[B" => {
                let max = (a.lines.len() + 1).saturating_sub(vis_lines);
                if a.scroll < max { a.scroll += 1; }
            }

            // Language switch
            "1" => {
                a.lang = app::Lang::Rust;
                a.lines = RUST_EXAMPLE.iter().map(|s| s.to_string()).collect();
                a.cur.clear(); a.scroll = 0;
                a.status = "Switched to Rust example.".to_string();
            }
            "2" => {
                a.lang = app::Lang::Python;
                a.lines = PYTHON_EXAMPLE.iter().map(|s| s.to_string()).collect();
                a.cur.clear(); a.scroll = 0;
                a.status = "Switched to Python example.".to_string();
            }
            "3" => {
                a.lang = app::Lang::Json;
                a.lines = JSON_EXAMPLE.iter().map(|s| s.to_string()).collect();
                a.cur.clear(); a.scroll = 0;
                a.status = "Switched to JSON example.".to_string();
            }

            // Ctrl+W = clear
            "\x17" => {
                a.lines.clear();
                a.cur.clear();
                a.scroll = 0;
                a.status = "Cleared.".to_string();
            }

            s if s.len() == 1 && s.chars().next().map_or(false, |c| !c.is_control()) => {
                a.cur.push_str(s);
            }

            _ => {}
        }

        app::draw(&a);
    }
}
