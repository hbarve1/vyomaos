# FINAL Spec: Browser & WebView Engine (Round 79)

**macOS Analogue**: Safari / WebKit / WKWebView
**Status**: FINAL — all blocking issues resolved
**Subsystem**: R79 Browser & WebView Engine
**Date**: 2026-05-30

---

## 1. Architecture

Two components ship together:

1. **`apps/browser/`** — standalone WASM browser app with UI chrome (URL bar, nav buttons, content area)
2. **VYOMA_WEBVIEW: protocol** — supervisor-side protocol for embedding a browser view in any other WASM app

### File Layout

```
apps/browser/
  Cargo.toml
  vyoma.toml
  src/
    main.rs          ≤500 lines  — event loop, UI chrome, keyboard/mouse dispatch
    net.rs           ≤200 lines  — HTTP fetch via VYOMA_NET:, response buffering
    history.rs       ≤150 lines  — back/forward URL stack
    html/
      mod.rs         ≤500 lines  — HTML tokenizer + parser → DOM tree
      dom.rs         ≤400 lines  — Node enum, DOM tree, attribute map
      layout.rs      ≤500 lines  — block/inline layout engine (BlockBox, InlineBox)
      render.rs      ≤400 lines  — layout tree → VYOMA_DRAW commands
      css.rs         ≤400 lines  — inline style parser, ComputedStyle, CSS inheritance
```

### Supervisor additions

`supervisor/src/webview.rs` (≤400 lines) — `WebViewState` registry, VYOMA_WEBVIEW: command dispatch, shared surface management. Hooked into the existing display flush pass alongside existing per-window surfaces.

---

## 2. HTML Tokenizer

File: `apps/browser/src/html/mod.rs`

### Tokenizer States

```rust
#[derive(Debug, PartialEq)]
enum TokState {
    Data,
    TagOpen,
    TagName,
    BeforeAttrName,
    AttrName,
    BeforeAttrValue,
    AttrValueDoubleQuote,
    AttrValueSingleQuote,
    AttrValueUnquoted,
    EndTag,
    Comment,
    Script,   // ignore everything until </script>
    Style,    // collect content until </style>
}
```

### Token Enum

```rust
#[derive(Debug, Clone)]
pub enum Token {
    Doctype,
    StartTag { name: String, attrs: HashMap<String, String>, self_closing: bool },
    EndTag   { name: String },
    Text(String),
    Comment,
    StyleBlock(String),   // collected <style> content (reserved for future CSS)
}
```

### Supported Tags

Block: `html`, `head`, `title`, `body`, `div`, `p`, `h1`–`h6`, `ul`, `ol`, `li`, `pre`, `blockquote`, `table`, `tr`, `td`, `th`

Inline: `a`, `span`, `strong`, `em`, `b`, `i`, `code`, `br`, `hr`, `img`

Ignored with contents: `script`
Collected: `style`

### Entity Decoding (B1 — blocking issue fix)

Decode HTML entities immediately when producing `Token::Text`:

```rust
fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            out.push(ch);
            continue;
        }
        let mut entity = String::new();
        for ec in chars.by_ref() {
            if ec == ';' { break; }
            entity.push(ec);
        }
        match entity.as_str() {
            "amp"  => out.push('&'),
            "lt"   => out.push('<'),
            "gt"   => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            "nbsp" => out.push('\u{00A0}'),
            _ => { out.push('&'); out.push_str(&entity); out.push(';'); }
        }
    }
    out
}
```

---

## 3. DOM Tree

File: `apps/browser/src/html/dom.rs`

```rust
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Node {
    Element {
        tag:      String,
        attrs:    HashMap<String, String>,
        children: Vec<Node>,
    },
    Text(String),
    Comment,
}

impl Node {
    pub fn tag(&self) -> Option<&str> {
        match self {
            Node::Element { tag, .. } => Some(tag.as_str()),
            _ => None,
        }
    }

    pub fn attr(&self, key: &str) -> Option<&str> {
        match self {
            Node::Element { attrs, .. } => attrs.get(key).map(|s| s.as_str()),
            _ => None,
        }
    }

    pub fn children(&self) -> &[Node] {
        match self {
            Node::Element { children, .. } => children,
            _ => &[],
        }
    }

    pub fn text_content(&self) -> String {
        match self {
            Node::Text(t) => t.clone(),
            Node::Element { children, .. } => {
                children.iter().map(|c| c.text_content()).collect::<Vec<_>>().join("")
            }
            Node::Comment => String::new(),
        }
    }
}
```

### Parser Entry Point

`pub fn parse_html(input: &str) -> Node` in `mod.rs` — tokenizes the input, then builds a DOM tree via a stack-based parser. Returns the root `<html>` element (or a synthetic `div` wrapper if the markup is a fragment).

Stack-based construction: push an open element onto the node stack on `StartTag`; on `EndTag` pop it and append as child to the new top. Void elements (`br`, `hr`, `img`) are self-closing and never pushed.

---

## 4. Layout Engine

File: `apps/browser/src/html/layout.rs`

### Box Types

```rust
#[derive(Debug, Clone)]
pub struct BlockBox {
    pub node:     Option<*const Node>,  // raw ptr — layout is ephemeral
    pub style:    ComputedStyle,
    pub children: Vec<LayoutBox>,
    pub x: i32, pub y: i32,
    pub w: i32, pub h: i32,
}

#[derive(Debug, Clone)]
pub struct InlineBox {
    pub text:  String,
    pub href:  Option<String>,          // set when inside <a>
    pub style: ComputedStyle,
    pub x: i32, pub y: i32,
    pub w: i32, pub h: i32,
}

#[derive(Debug, Clone)]
pub enum LayoutBox {
    Block(BlockBox),
    Inline(InlineBox),
}
```

### Layout Algorithm

```rust
pub fn build_layout_tree(node: &Node, avail_w: i32, depth: u32) -> Vec<LayoutBox> {
    // B2 fix: limit recursion depth to prevent stack overflow on deep HTML
    if depth > 50 {
        eprintln!("[browser] layout: max depth 50 reached, truncating subtree");
        return vec![];
    }
    // ... (map Node to LayoutBox, recurse into children)
}

pub fn layout_block(b: &mut BlockBox, avail_w: i32) {
    let inner_w = avail_w
        - (b.style.margin[1] + b.style.margin[3]) as i32
        - (b.style.padding[1] + b.style.padding[3]) as i32;
    let mut cursor_y = b.y + b.style.margin[0] as i32 + b.style.padding[0] as i32;
    for child in &mut b.children {
        match child {
            LayoutBox::Block(cb) => {
                cb.x = b.x + b.style.padding[3] as i32;
                cb.y = cursor_y;
                cb.w = inner_w;
                layout_block(cb, inner_w);
                cursor_y += cb.h + cb.style.margin[2] as i32;
            }
            LayoutBox::Inline(ib) => {
                ib.x = b.x + b.style.padding[3] as i32;
                ib.y = cursor_y;
                cursor_y += wrap_inline(ib, inner_w);
            }
        }
    }
    b.h = cursor_y - b.y + b.style.padding[2] as i32;
}
```

### Text Wrapping

Character width approximation: 8px per char at medium (16px) size; scale by `font_size / 16`.

```rust
fn wrap_inline(ib: &mut InlineBox, avail_w: i32) -> i32 {
    let char_w = (8.0 * (ib.style.font_size as f32 / 16.0)) as i32;
    let line_h = ib.style.font_size as i32 + 4;
    let chars_per_line = (avail_w / char_w).max(1) as usize;
    let text_len = ib.text.len();
    let lines = (text_len + chars_per_line - 1) / chars_per_line;
    ib.w = avail_w;
    ib.h = (lines as i32) * line_h;
    ib.h
}
```

Heading sizes: h1=32, h2=28, h3=24, h4=20, h5=18, h6=16.

---

## 5. CSS Subset

File: `apps/browser/src/html/css.rs`

### ComputedStyle

```rust
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub color:       u32,        // RGBA; default 0xCDD6F4FF
    pub background:  u32,        // RGBA; default 0x00000000 (transparent)
    pub font_size:   u32,        // px; default 16
    pub font_weight: u32,        // 400 normal, 700 bold
    pub margin:      [u32; 4],   // top right bottom left; default [0,0,0,0]
    pub padding:     [u32; 4],   // top right bottom left; default [0,0,0,0]
    pub display:     Display,    // default Block
}

#[derive(Debug, Clone, PartialEq)]
pub enum Display { Block, Inline, None }

impl Default for ComputedStyle {
    fn default() -> Self {
        ComputedStyle {
            color:       0xCDD6F4FF,
            background:  0x00000000,
            font_size:   16,
            font_weight: 400,
            margin:      [0; 4],
            padding:     [0; 4],
            display:     Display::Block,
        }
    }
}
```

### CSS Inheritance (B5 — blocking issue fix)

Inherited properties: `color`, `font_size`, `font_weight`. Non-inherited: `background`, `margin`, `padding`.

```rust
impl ComputedStyle {
    /// Merge inherited properties from parent into self (self is the child).
    /// Only overwrites fields that were NOT explicitly set on the child.
    pub fn inherit(&mut self, parent: &ComputedStyle, explicitly_set: &[&str]) {
        if !explicitly_set.contains(&"color") {
            self.color = parent.color;
        }
        if !explicitly_set.contains(&"font-size") {
            self.font_size = parent.font_size;
        }
        if !explicitly_set.contains(&"font-weight") {
            self.font_weight = parent.font_weight;
        }
    }
}
```

Callers in `build_layout_tree` pass the parent `ComputedStyle` down and track which properties the node's `style=` attribute explicitly declared.

### Inline Style Parser

```rust
pub fn parse_inline_style(style_attr: &str) -> (ComputedStyle, Vec<String>) {
    let mut s = ComputedStyle::default();
    let mut set: Vec<String> = Vec::new();
    for decl in style_attr.split(';') {
        let decl = decl.trim();
        if decl.is_empty() { continue; }
        let mut parts = decl.splitn(2, ':');
        let prop = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let val  = parts.next().unwrap_or("").trim();
        match prop.as_str() {
            "color"            => { s.color      = parse_color(val); set.push(prop); }
            "background-color" => { s.background = parse_color(val); set.push(prop); }
            "font-size"        => { s.font_size  = parse_px(val, 16); set.push(prop); }
            "font-weight"      => {
                s.font_weight = if val == "bold" || val == "700" { 700 } else { 400 };
                set.push(prop);
            }
            "margin"           => { s.margin  = parse_shorthand(val); set.push(prop); }
            "padding"          => { s.padding = parse_shorthand(val); set.push(prop); }
            "display" => {
                s.display = match val { "none" => Display::None, "inline" => Display::Inline, _ => Display::Block };
                set.push(prop);
            }
            _ => {}
        }
    }
    (s, set)
}
```

### Color Parsing

```rust
pub fn parse_color(val: &str) -> u32 {
    let named: &[(&str, u32)] = &[
        ("black",   0x000000FF), ("white",   0xFFFFFFFF),
        ("red",     0xFF0000FF), ("green",   0x00FF00FF),
        ("blue",    0x0000FFFF), ("gray",    0x888888FF),
        ("grey",    0x888888FF), ("yellow",  0xFFFF00FF),
        ("orange",  0xFF8800FF), ("purple",  0x8800FFFF),
        ("cyan",    0x00FFFFFF), ("magenta", 0xFF00FFFF),
    ];
    for &(name, rgba) in named {
        if val.eq_ignore_ascii_case(name) { return rgba; }
    }
    let v = val.trim_start_matches('#');
    match v.len() {
        3 => {
            let r = u8::from_str_radix(&v[0..1].repeat(2), 16).unwrap_or(0);
            let g = u8::from_str_radix(&v[1..2].repeat(2), 16).unwrap_or(0);
            let b = u8::from_str_radix(&v[2..3].repeat(2), 16).unwrap_or(0);
            ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
        }
        6 => {
            let r = u8::from_str_radix(&v[0..2], 16).unwrap_or(0);
            let g = u8::from_str_radix(&v[2..4], 16).unwrap_or(0);
            let b = u8::from_str_radix(&v[4..6], 16).unwrap_or(0);
            ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
        }
        _ => 0xCDD6F4FF,
    }
}

fn parse_px(val: &str, default: u32) -> u32 {
    val.trim_end_matches("px").parse::<u32>().unwrap_or(default)
}

fn parse_shorthand(val: &str) -> [u32; 4] {
    let parts: Vec<u32> = val.split_whitespace()
        .map(|v| parse_px(v, 0)).collect();
    match parts.len() {
        1 => [parts[0]; 4],
        2 => [parts[0], parts[1], parts[0], parts[1]],
        3 => [parts[0], parts[1], parts[2], parts[1]],
        4 => [parts[0], parts[1], parts[2], parts[3]],
        _ => [0; 4],
    }
}
```

---

## 6. Rendering

File: `apps/browser/src/html/render.rs`

Maps layout boxes to VYOMA_DRAW commands emitted to stdout.

```rust
pub fn render_layout(boxes: &[LayoutBox], offset_y: i32) {
    for lb in boxes {
        render_box(lb, offset_y);
    }
    println!("VYOMA_DRAW:flush");
}

fn render_box(lb: &LayoutBox, offset_y: i32) {
    match lb {
        LayoutBox::Block(b) => render_block(b, offset_y),
        LayoutBox::Inline(i) => render_inline(i, offset_y),
    }
}

fn render_block(b: &BlockBox, off: i32) {
    // background (skip if transparent)
    if b.style.background & 0xFF != 0 {
        println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}",
            b.x, b.y + off, b.w, b.h, b.style.background);
    }
    for child in &b.children {
        render_box(child, off);
    }
}

fn render_inline(ib: &InlineBox, off: i32) {
    if ib.text.trim().is_empty() { return; }

    let font_sz = match ib.style.font_size {
        0..=12  => 's',
        13..=20 => 'm',
        _       => 'l',
    };
    let color = if ib.href.is_some() { 0x89B4FAFF } else { ib.style.color };

    println!("VYOMA_DRAW:draw_text:{},{},{},{},{}",
        ib.x, ib.y + off, color, font_sz, ib.text);

    // underline for links
    if ib.href.is_some() {
        let baseline = ib.y + off + ib.style.font_size as i32;
        println!("VYOMA_DRAW:fill_rect:{},{},{},1,{}", ib.x, baseline, ib.w, 0x89B4FAFF);
    }
}
```

### Special Element Rendering

Called from `render_block` before recursing into children, keyed on `tag`:

```rust
fn render_special_block(tag: &str, b: &BlockBox, off: i32) {
    match tag {
        "hr" => {
            println!("VYOMA_DRAW:fill_rect:{},{},{},1,0x45475AFF",
                b.x, b.y + off + 4, b.w, 0x45475AFF);
        }
        "img" => {
            // placeholder until image loading is implemented
            println!("VYOMA_DRAW:fill_rect:{},{},{},{},0x313244FF",
                b.x, b.y + off, b.w.max(64), b.h.max(32));
            println!("VYOMA_DRAW:draw_text:{},{},0x888888FF,s,img",
                b.x + 4, b.y + off + 8);
        }
        _ => {}
    }
}
```

`<pre>`/`<code>` blocks: render their text children using font size `'s'` (small, monospace-simulated) and preserve whitespace.

---

## 7. VYOMA_WEBVIEW Protocol

### App → Supervisor Commands

```
VYOMA_WEBVIEW:create|<view_id>|<x>|<y>|<w>|<h>
VYOMA_WEBVIEW:load|<view_id>|<url>
VYOMA_WEBVIEW:load_html|<view_id>|<base64_html>
VYOMA_WEBVIEW:destroy|<view_id>
```

- `create` — registers a new WebView region. Supervisor allocates a `WebViewState`.
- `load` — fetches URL via VYOMA_NET:, renders HTML, stores pixel result.
- `load_html` — accepts inline HTML (base64-encoded to avoid pipe-delimiter ambiguity).
- `destroy` — frees all resources for `view_id`.

### Supervisor → App Events

```
VYOMA_WEBVIEW:loaded|<view_id>|<title>
VYOMA_WEBVIEW:error|<view_id>|<reason>
VYOMA_WEBVIEW:navigate|<view_id>|<url>
```

- `loaded` — page was fetched and rendered; pixel buffer is ready.
- `error` — network error, parse failure, or binary Content-Type (B3).
- `navigate` — user clicked `<a href>` inside the view; app decides whether to allow/redirect.

### Supervisor State

File: `supervisor/src/webview.rs`

```rust
pub struct WebViewState {
    pub view_id:  String,
    pub owner:    String,          // app name that created this view
    pub url:      String,
    pub title:    String,
    pub x: i32, pub y: i32,
    pub w: u32,  pub h: u32,
    pub surface:  Option<ShmSurface>,   // rendered pixel buffer (reuses Surface type)
    pub pending:  bool,                  // fetch in flight
}

pub struct WebViewRegistry {
    views: HashMap<String, WebViewState>,
}

impl WebViewRegistry {
    pub fn handle_command(&mut self, app: &str, line: &str) -> Option<String> {
        // parse VYOMA_WEBVIEW: prefix, dispatch to create/load/load_html/destroy
        // returns Some(response) to route back to app stdin, or None
        todo!()
    }
}
```

The render path inside the supervisor for WebView reuses the existing `blit_surface` compositor primitive from `display.rs` — the `WebViewState::surface` is blitted at `(x, y)` during the same flush pass as normal windows.

---

## 8. Browser UI Chrome

File: `apps/browser/src/main.rs`

### Layout Constants

```rust
const CHROME_H:       i32 = 40;   // URL bar row height
const BTN_W:          i32 = 40;
const VIEWPORT_X:     i32 = 0;

const COLOR_CHROME_BG:  u32 = 0x313244FF;
const COLOR_BTN_HOVER:  u32 = 0x45475AFF;
const COLOR_URL_BG:     u32 = 0x1E1E2EFF;
const COLOR_URL_TEXT:   u32 = 0xCDD6F4FF;
const COLOR_CONTENT_BG: u32 = 0x1E1E2EFF;
```

### Chrome Drawing

```rust
fn draw_chrome(url: &str, can_back: bool, can_fwd: bool, win_w: i32) {
    // chrome bar background
    println!("VYOMA_DRAW:fill_rect:0,0,{},{},{}", win_w, CHROME_H, COLOR_CHROME_BG);

    // back button
    let back_col = if can_back { COLOR_URL_TEXT } else { 0x585B70FF };
    println!("VYOMA_DRAW:draw_text:8,8,{},m,<", back_col);

    // forward button
    let fwd_col = if can_fwd { COLOR_URL_TEXT } else { 0x585B70FF };
    println!("VYOMA_DRAW:draw_text:48,8,{},m,>", fwd_col);

    // reload button
    println!("VYOMA_DRAW:draw_text:88,8,{},m,R", COLOR_URL_TEXT);

    // URL bar background
    println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", BTN_W * 3, 4, win_w - BTN_W * 3 - 8, 32, COLOR_URL_BG);

    // URL text (truncated if too long)
    let display_url: String = url.chars().take(80).collect();
    println!("VYOMA_DRAW:draw_text:{},{},{},s,{}", BTN_W * 3 + 6, 12, COLOR_URL_TEXT, display_url);
}
```

### Keyboard Shortcuts

| Key combination | Action |
|---|---|
| `Ctrl+L` | Focus URL bar; clear and accept typed URL |
| `Enter` (URL bar focused) | Navigate to typed URL |
| `Ctrl+R` | Reload current page |
| `Backspace` (URL bar not focused) | Go back in history |
| `Alt+Left` | Go back |
| `Alt+Right` | Go forward |

Input events arrive on stdin as `VYOMA_INPUT:key:<keycode>` and `VYOMA_INPUT:char:<char>` lines; parsed in `main.rs` event loop.

### Event Loop Skeleton

```rust
fn main() {
    let mut state = BrowserState {
        url:         String::from("about:blank"),
        history:     History::new(),
        url_focused: false,
        url_draft:   String::new(),
        win_w:       960,
        win_h:       700,
        content_y:   CHROME_H,
    };

    draw_chrome(&state.url, false, false, state.win_w);
    println!("VYOMA_DRAW:flush");

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        handle_event(&mut state, &line);
    }
}
```

---

## 9. App Manifest

File: `apps/browser/vyoma.toml`

```toml
[app]
name    = "browser"
version = "0.1.0"
wasm    = "browser.wasm"

[lifecycle]
restart = "never"

[capabilities]
stdio      = true
display    = true
network    = true
filesystem = true
mouse      = true
```

### Cargo.toml

```toml
[package]
name    = "browser"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "browser"
path = "src/main.rs"

[dependencies]
base64 = "0.21"
```

No heavy HTML parser or CSS engine crates — the subsystem implements its own minimal tokenizer and style engine to keep the binary small and `wasm32-wasip2` compatible.

---

## 10. Blocking Issues (B1–B5)

### B1 — HTML Entity Decoding

**Problem**: Without entity decoding, `&amp;`, `&lt;`, `&gt;`, `&nbsp;`, `&quot;` appear as literal strings in rendered text (e.g. "Tom &amp; Jerry" renders as "Tom &amp; Jerry").

**Fix**: `decode_entities(input: &str) -> String` called on every `Token::Text` value at tokenizer output time (see Section 2). Handles the five mandatory entities plus `&apos;`. Unrecognized entities are passed through verbatim (tolerant parsing).

### B2 — Stack Overflow on Deep DOM Trees

**Problem**: Recursive `build_layout_tree` and `layout_block` functions overflow the WASM stack on HTML with 100+ nesting levels (common in real-world pages with deeply nested divs).

**Fix**: `build_layout_tree` accepts a `depth: u32` parameter. When `depth > 50`, it logs a warning to stderr and returns an empty `Vec<LayoutBox>` immediately, truncating the subtree. The rendered page may be incomplete but will not crash.

```rust
if depth > 50 {
    eprintln!("[browser] layout depth limit reached at tag, truncating");
    return vec![];
}
```

### B3 — Binary Response Content-Type

**Problem**: `VYOMA_NET:response` for image URLs (`.jpg`, `.png`, `.gif`) or other binary resources returns non-UTF-8 bytes. Attempting `String::from_utf8` on the body panics or corrupts the parse.

**Fix**: In `net.rs`, after receiving the response body, check the Content-Type header before UTF-8 conversion:

```rust
pub fn is_html_content_type(ct: &str) -> bool {
    ct.starts_with("text/html") || ct.starts_with("application/xhtml")
        || ct.is_empty()   // assume HTML if no Content-Type (common in dev servers)
}

// In fetch response handler:
if !is_html_content_type(&content_type) {
    // Emit error event rather than trying to parse binary as HTML
    println!("VYOMA_WEBVIEW:error|{}|binary content-type: {}", view_id, content_type);
    return;
}
match String::from_utf8(body_bytes) {
    Ok(html)  => parse_and_render(html),
    Err(_)    => println!("VYOMA_WEBVIEW:error|{}|non-utf8 response body", view_id),
}
```

### B4 — Relative URL Resolution

**Problem**: Most real HTML pages use relative hrefs: `href="about.html"`, `href="/docs/index.html"`, `href="//cdn.example.com/lib.js"`. Without resolution, these are fetched as-is and fail.

**Fix**: `resolve_url` in `net.rs`:

```rust
pub fn resolve_url(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_owned();
    }
    if href.starts_with("//") {
        // protocol-relative
        let scheme = if base.starts_with("https") { "https:" } else { "http:" };
        return format!("{}{}", scheme, href);
    }
    // parse base into scheme + host + path
    let (scheme_host, base_path) = split_base(base);
    if href.starts_with('/') {
        return format!("{}{}", scheme_host, href);
    }
    // relative to current directory
    let dir = base_path.rfind('/').map(|i| &base_path[..=i]).unwrap_or("/");
    let joined = format!("{}{}", dir, href);
    // collapse ".." segments (simple pass)
    format!("{}{}", scheme_host, normalize_path(&joined))
}

fn split_base(base: &str) -> (&str, &str) {
    // returns ("https://host", "/path/page.html")
    if let Some(idx) = base.find("://") {
        let after = &base[idx + 3..];
        if let Some(slash) = after.find('/') {
            let host_end = idx + 3 + slash;
            return (&base[..host_end], &base[host_end..]);
        }
        return (base, "/");
    }
    ("", base)
}
```

All `<a href>` and `<img src>` values are passed through `resolve_url(current_url, href)` before being dispatched.

### B5 — CSS Color Inheritance

**Problem**: Without explicit inheritance, inner elements default to `color: 0xCDD6F4FF` regardless of what their parent declares. A page setting `<body style="color: #ff0000">` would not propagate red to its children.

**Fix**: `ComputedStyle::inherit(parent, explicitly_set)` (see Section 5). In `build_layout_tree`, each node's style is computed by:

1. Starting from `ComputedStyle::default()`
2. Applying tag-level defaults (e.g. `h1` gets `font_size: 32`, `font_weight: 700`)
3. Parsing the element's `style=` attribute via `parse_inline_style`, recording which properties were explicitly set
4. Calling `style.inherit(&parent_style, &explicitly_set_props)` to pull in inherited values for anything not explicitly set

This ensures color, font-size, and font-weight flow naturally through the document tree.

---

## Summary

| Section | Deliverable |
|---|---|
| `apps/browser/src/main.rs` | Event loop, UI chrome (URL bar, nav buttons), keyboard shortcuts |
| `apps/browser/src/html/mod.rs` | Tokenizer (8 states) + stack-based DOM parser |
| `apps/browser/src/html/dom.rs` | `Node` enum + `parse_html` entry point |
| `apps/browser/src/html/layout.rs` | `BlockBox`/`InlineBox`, `build_layout_tree`, `layout_block`, text wrapping |
| `apps/browser/src/html/render.rs` | `render_layout` → VYOMA_DRAW commands |
| `apps/browser/src/html/css.rs` | `ComputedStyle`, `parse_inline_style`, `parse_color`, inheritance |
| `apps/browser/src/net.rs` | HTTP fetch, Content-Type guard (B3), `resolve_url` (B4) |
| `apps/browser/src/history.rs` | Back/forward URL stack |
| `supervisor/src/webview.rs` | `WebViewRegistry`, `WebViewState`, VYOMA_WEBVIEW: dispatch |
| `apps/browser/vyoma.toml` | Capabilities manifest |
| B1–B5 | Entity decode, depth limit, binary CT guard, URL resolution, CSS inheritance |
