"""
VyomaOS — World-Class Presentation Builder
Dark tech theme, 18 slides, conference-grade design.
"""

from pptx import Presentation
from pptx.util import Inches, Pt, Emu
from pptx.dml.color import RGBColor
from pptx.enum.text import PP_ALIGN
from pptx.util import Inches, Pt
import pptx.oxml.ns as nsmap
from lxml import etree
import copy

# ── Palette ────────────────────────────────────────────────────────────────
BG         = RGBColor(0x0D, 0x11, 0x17)   # near-black
BG2        = RGBColor(0x16, 0x1B, 0x27)   # card background
PURPLE     = RGBColor(0x7C, 0x3A, 0xED)   # primary accent
PURPLE_LT  = RGBColor(0xA7, 0x8B, 0xFA)   # light purple
GREEN      = RGBColor(0x10, 0xB9, 0x81)   # security/capability green
GREEN_LT   = RGBColor(0x6E, 0xE7, 0xB7)   # light green
ORANGE     = RGBColor(0xF5, 0x9E, 0x0B)   # warning/highlight
BLUE       = RGBColor(0x38, 0xBD, 0xF8)   # info blue
RED        = RGBColor(0xF8, 0x71, 0x71)   # red accent
TEXT       = RGBColor(0xE2, 0xE8, 0xF0)   # primary text
MUTED      = RGBColor(0x64, 0x74, 0x8B)   # muted text
DIVIDER    = RGBColor(0x1E, 0x29, 0x3B)   # subtle divider
WHITE      = RGBColor(0xFF, 0xFF, 0xFF)
CYAN       = RGBColor(0x06, 0xB6, 0xD4)

SW = 10.0   # slide width  (inches)
SH = 5.625  # slide height (inches)

prs = Presentation()
prs.slide_width  = Inches(SW)
prs.slide_height = Inches(SH)

BLANK = prs.slide_layouts[6]   # truly blank layout


# ── Helpers ────────────────────────────────────────────────────────────────

def add_rect(slide, x, y, w, h, fill=None, line=None, line_w=Pt(0)):
    shape = slide.shapes.add_shape(1, Inches(x), Inches(y), Inches(w), Inches(h))
    shape.line.fill.background()
    if fill:
        shape.fill.solid()
        shape.fill.fore_color.rgb = fill
    else:
        shape.fill.background()
    if line:
        shape.line.color.rgb = line
        shape.line.width = line_w
    else:
        shape.line.fill.background()
    return shape


def add_text(slide, text, x, y, w, h,
             size=18, bold=False, color=TEXT, align=PP_ALIGN.LEFT,
             italic=False, wrap=True):
    txb = slide.shapes.add_textbox(Inches(x), Inches(y), Inches(w), Inches(h))
    txb.word_wrap = wrap
    tf = txb.text_frame
    tf.word_wrap = wrap
    p = tf.paragraphs[0]
    p.alignment = align
    run = p.add_run()
    run.text = text
    run.font.size = Pt(size)
    run.font.bold = bold
    run.font.italic = italic
    run.font.color.rgb = color
    run.font.name = "Inter" if bold else "Inter"
    return txb


def add_multiline(slide, lines, x, y, w, h,
                  size=14, bold=False, color=TEXT, align=PP_ALIGN.LEFT,
                  line_spacing=1.2):
    """lines: list of (text, color, bold, size) or plain str"""
    txb = slide.shapes.add_textbox(Inches(x), Inches(y), Inches(w), Inches(h))
    txb.word_wrap = True
    tf = txb.text_frame
    tf.word_wrap = True
    first = True
    for item in lines:
        if isinstance(item, str):
            txt, col, bld, sz = item, color, bold, size
        else:
            txt, col, bld, sz = item
        if first:
            p = tf.paragraphs[0]
            first = False
        else:
            p = tf.add_paragraph()
        p.alignment = align
        run = p.add_run()
        run.text = txt
        run.font.size = Pt(sz)
        run.font.bold = bld
        run.font.color.rgb = col
    return txb


def bg(slide, color=BG):
    add_rect(slide, 0, 0, SW, SH, fill=color)


def top_bar(slide, accent=PURPLE):
    add_rect(slide, 0, 0, SW, 0.07, fill=accent)


def bottom_bar(slide, accent=PURPLE):
    add_rect(slide, 0, SH - 0.07, SW, 0.07, fill=accent)


def slide_number(slide, n, total=18):
    add_text(slide, f"{n:02d} / {total}", SW - 1.1, SH - 0.38, 1.0, 0.3,
             size=9, color=MUTED, align=PP_ALIGN.RIGHT)


def section_tag(slide, label, color=PURPLE):
    add_rect(slide, 0.45, 0.22, 1.6, 0.22, fill=color)
    add_text(slide, label.upper(), 0.47, 0.22, 1.56, 0.22,
             size=7, bold=True, color=WHITE, align=PP_ALIGN.CENTER)


def card(slide, x, y, w, h, fill=BG2, line=DIVIDER):
    add_rect(slide, x, y, w, h, fill=fill, line=line, line_w=Pt(1))


def bullet_dot(slide, x, y, color=PURPLE):
    add_rect(slide, x, y, 0.06, 0.06, fill=color)


def icon_box(slide, x, y, size=0.55, color=PURPLE, label=""):
    add_rect(slide, x, y, size, size, fill=color)
    if label:
        add_text(slide, label, x, y + size * 0.15, size, size * 0.7,
                 size=16, bold=True, color=WHITE, align=PP_ALIGN.CENTER)


def divider_line(slide, x, y, w, color=DIVIDER):
    add_rect(slide, x, y, w, 0.015, fill=color)


def logo_mark(slide, x=0.3, y=0.18):
    """Simple V mark in purple"""
    add_rect(slide, x, y, 0.08, 0.28, fill=PURPLE)
    add_rect(slide, x + 0.08, y + 0.14, 0.08, 0.14, fill=PURPLE)
    add_rect(slide, x + 0.16, y, 0.08, 0.28, fill=PURPLE_LT)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 01  — TITLE
# ══════════════════════════════════════════════════════════════════════════════
def slide_01():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    # Gradient feel: left panel
    add_rect(s, 0, 0, 4.2, SH, fill=RGBColor(0x11, 0x0D, 0x1F))
    # Top accent
    add_rect(s, 0, 0, 4.2, 0.06, fill=PURPLE)
    # Right top accent
    add_rect(s, 4.2, 0, SW - 4.2, 0.06, fill=GREEN)

    # Grid lines (decorative)
    for i in range(10):
        add_rect(s, 4.2, i * 0.56, SW - 4.2, 0.006,
                 fill=RGBColor(0x1E, 0x29, 0x3B))
    for j in range(10):
        add_rect(s, 4.2 + j * 0.58, 0, 0.006, SH,
                 fill=RGBColor(0x1E, 0x29, 0x3B))

    # Logo V shape
    logo_mark(s, 0.45, 1.2)
    add_text(s, "VYOMA", 0.75, 1.2, 1.8, 0.45,
             size=28, bold=True, color=WHITE)
    add_text(s, "OS", 0.75, 1.6, 0.8, 0.38,
             size=28, bold=True, color=PURPLE_LT)

    add_text(s, "The WASM-First Operating System", 0.42, 2.18, 3.4, 0.4,
             size=13, color=MUTED)

    divider_line(s, 0.42, 2.7, 3.3, color=PURPLE)

    add_text(s, "Capability-Secure  ·  18 MB  ·  < 5s Boot", 0.42, 2.82, 3.5, 0.3,
             size=10, color=GREEN_LT)

    add_text(s, "200+ WASM Apps", 0.42, 3.18, 2.5, 0.28,
             size=10, color=MUTED)

    # Right panel — big hero number
    add_text(s, "wasm32\n-wasip2", 5.4, 0.9, 3.8, 1.2,
             size=36, bold=True, color=RGBColor(0x2D, 0x1F, 0x52), align=PP_ALIGN.CENTER)

    add_text(s, "Every. App. Sandboxed.", 4.6, 2.4, 5.0, 0.5,
             size=18, bold=True, color=PURPLE_LT, align=PP_ALIGN.CENTER)

    add_text(s, "Linux kernel provides hardware abstraction only.\nRust PID 1 enforces capability isolation.\nNo C userland. No shell injection surface.", 4.6, 3.1, 5.0, 0.9,
             size=10.5, color=MUTED, align=PP_ALIGN.CENTER)

    bottom_bar(s, PURPLE)
    slide_number(s, 1)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 02  — THE PROBLEM
# ══════════════════════════════════════════════════════════════════════════════
def slide_02():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, RED)
    section_tag(s, "The Problem", RED)

    add_text(s, "Modern OSes carry 40 years of legacy attack surface.", 0.45, 0.6, 9.1, 0.55,
             size=22, bold=True, color=TEXT)

    # 3 problem cards
    problems = [
        ("C Userland", "Shared libraries, POSIX quirks, shell\ninjection. Every app inherits all of it.", RED),
        ("Coarse Permissions", "Android/Linux DAC: either you have\naccess, or you don't. No fine-grained\ncapability model.", ORANGE),
        ("Non-Deterministic Binaries", "ELF binaries vary by libc/arch. No\nreproducibility guarantee. Supply\nchain attacks thrive.", ORANGE),
    ]

    for i, (title, body, col) in enumerate(problems):
        cx = 0.45 + i * 3.18
        card(s, cx, 1.4, 2.98, 2.8)
        add_rect(s, cx, 1.4, 2.98, 0.07, fill=col)
        add_text(s, title, cx + 0.18, 1.55, 2.6, 0.4,
                 size=13, bold=True, color=col)
        add_text(s, body, cx + 0.18, 2.05, 2.6, 1.5,
                 size=10.5, color=MUTED)

    # Bottom quote
    divider_line(s, 0.45, 4.45, 9.1, color=DIVIDER)
    add_text(s,
             '"VyomaOS starts with the lesson learned: the runtime IS the OS boundary."',
             0.9, 4.6, 8.2, 0.4, size=11, italic=True, color=PURPLE_LT, align=PP_ALIGN.CENTER)

    bottom_bar(s, RED)
    slide_number(s, 2)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 03  — THE VISION
# ══════════════════════════════════════════════════════════════════════════════
def slide_03():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, PURPLE)
    section_tag(s, "Vision")

    add_text(s, "A fully capable general-purpose OS.\nBuilt entirely on WASM.", 0.45, 0.55, 7.0, 0.9,
             size=26, bold=True, color=TEXT)

    add_text(s, "From 18 MB embedded appliance  →  Full desktop OS  ·  Same security model at every scale.",
             0.45, 1.6, 9.1, 0.4, size=12, color=MUTED)

    # Vision pillars
    pillars = [
        (GREEN,   "pkg install",    "WASM-native\npackage manager"),
        (PURPLE,  "capability: []", "Fine-grained\npermission model"),
        (BLUE,    "byte-identical", "Deterministic\nbinaries"),
        (ORANGE,  "< 5s boot",      "Minimal kernel\n(2.3 MB)"),
        (CYAN,    "200+ apps",      "Rich app\necosystem"),
    ]

    for i, (col, code, label) in enumerate(pillars):
        cx = 0.45 + i * 1.84
        card(s, cx, 2.15, 1.72, 2.2)
        add_rect(s, cx + 0.2, 2.35, 1.32, 0.55, fill=RGBColor(0x0D, 0x11, 0x17))
        add_text(s, code, cx + 0.22, 2.38, 1.28, 0.48,
                 size=8.5, bold=True, color=col)
        add_text(s, label, cx + 0.1, 3.1, 1.52, 0.7,
                 size=10, color=TEXT, align=PP_ALIGN.CENTER)

    bottom_bar(s)
    slide_number(s, 3)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 04  — ARCHITECTURE OVERVIEW
# ══════════════════════════════════════════════════════════════════════════════
def slide_04():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s)
    section_tag(s, "Architecture")

    add_text(s, "System Stack", 0.45, 0.55, 4.0, 0.42,
             size=20, bold=True, color=TEXT)

    layers = [
        ("Linux 5.10 Kernel",         "allnoconfig · 2.3 MB · Hardware only",            RGBColor(0x1E, 0x29, 0x3B), MUTED),
        ("Wasmtime Runtime",           "WASI Preview 2 · Capability enforcement",          RGBColor(0x1A, 0x2A, 0x1A), GREEN),
        ("Rust Supervisor  (PID 1)",   "697 KB · Manifest parser · IPC broker · Scheduler",RGBColor(0x1A, 0x16, 0x2E), PURPLE_LT),
        ("WASM Applications",          "wasm32-wasip2 · Byte-identical · 1–10 KB each",    RGBColor(0x1A, 0x22, 0x2E), BLUE),
    ]

    for i, (title, sub, bg_col, col) in enumerate(layers):
        cy = 1.15 + i * 0.87
        card(s, 0.45, cy, 4.55, 0.75, fill=bg_col)
        add_rect(s, 0.45, cy, 0.06, 0.75, fill=col)
        add_text(s, title, 0.65, cy + 0.08, 3.5, 0.32, size=12, bold=True, color=col)
        add_text(s, sub,   0.65, cy + 0.38, 4.1, 0.28, size=9,  color=MUTED)
        # arrow
        if i < 3:
            add_text(s, "↕", 2.5, cy + 0.78, 0.4, 0.25,
                     size=10, color=MUTED, align=PP_ALIGN.CENTER)

    # Right side: key numbers
    stats = [
        ("697 KB",  "Supervisor binary", PURPLE),
        ("2.3 MB",  "Linux kernel",       BLUE),
        ("18 MB",   "Full initramfs",     GREEN),
        ("< 5 s",   "Boot time (QEMU)",   ORANGE),
        ("200+",    "WASM apps",          CYAN),
        ("43.0",    "Wasmtime version",   MUTED),
    ]

    for i, (val, lbl, col) in enumerate(stats):
        cy = 1.0 + i * 0.72
        cx = 5.5 + (i % 2) * 2.18
        cy2 = 1.0 + (i // 2) * 1.15
        card(s, cx, cy2, 1.96, 0.98)
        add_text(s, val, cx + 0.12, cy2 + 0.08, 1.72, 0.48,
                 size=20, bold=True, color=col, align=PP_ALIGN.CENTER)
        add_text(s, lbl, cx + 0.08, cy2 + 0.55, 1.8, 0.32,
                 size=8, color=MUTED, align=PP_ALIGN.CENTER)

    bottom_bar(s)
    slide_number(s, 4)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 05  — CAPABILITY SECURITY MODEL
# ══════════════════════════════════════════════════════════════════════════════
def slide_05():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, GREEN)
    section_tag(s, "Security Model", GREEN)

    add_text(s, "Declare. Enforce. Nothing else gets through.", 0.45, 0.56, 8.0, 0.45,
             size=22, bold=True, color=TEXT)

    add_text(s, "Capabilities not declared in vyoma.toml are never wired up — no filtering layer needed.",
             0.45, 1.1, 9.1, 0.32, size=11, color=MUTED)

    # Capability table
    caps = [
        ("stdio",       "stdin/stdout/stderr", GREEN),
        ("filesystem",  "Mount /data (9P, persistent)", BLUE),
        ("network",     "WASI sockets, TCP port 8080", CYAN),
        ("display",     "Framebuffer + VYOMA_DRAW protocol", PURPLE),
        ("shell",       "@supervisor: process management", ORANGE),
        ("mouse",       "VYOMA_INPUT:mouse: events", RED),
        ("watchdog",    "Kill if silent for N seconds", MUTED),
    ]

    for i, (cap, desc, col) in enumerate(caps):
        row = i // 2
        col_idx = i % 2
        cx = 0.45 + col_idx * 4.75
        cy = 1.55 + row * 0.62
        if i == 6:
            cx = 0.45 + 4.75 * 0
            cy = 1.55 + 3 * 0.62
        card(s, cx, cy, 4.55, 0.52)
        add_rect(s, cx, cy, 0.055, 0.52, fill=col)
        add_text(s, cap, cx + 0.18, cy + 0.06, 1.3, 0.25,
                 size=10, bold=True, color=col)
        add_text(s, "= true", cx + 1.38, cy + 0.06, 0.55, 0.25,
                 size=9, color=MUTED)
        add_text(s, desc, cx + 0.18, cy + 0.28, 4.1, 0.2,
                 size=8.5, color=MUTED)

    # Right panel — manifest snippet
    card(s, 5.25, 1.55, 4.3, 2.45, fill=RGBColor(0x0D, 0x11, 0x17))
    code_lines = [
        ("[app]",               MUTED,     False, 9),
        ('name    = "shell"',   TEXT,      False, 9),
        ("",                    TEXT,      False, 9),
        ("[capabilities]",      MUTED,     False, 9),
        ("stdio      = true",   GREEN_LT,  False, 9),
        ("filesystem = true",   BLUE,      False, 9),
        ("shell      = true",   ORANGE,    False, 9),
        ("mouse      = true",   RED,       False, 9),
        ("# network NOT declared", MUTED,  True,  8.5),
        ("# → no TCP interface",   MUTED,  True,  8.5),
    ]
    add_multiline(s, code_lines, 5.42, 1.68, 3.9, 2.2, size=9)

    bottom_bar(s, GREEN)
    slide_number(s, 5)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 06  — THE SUPERVISOR (PID 1)
# ══════════════════════════════════════════════════════════════════════════════
def slide_06():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, PURPLE)
    section_tag(s, "Supervisor")

    add_text(s, "Rust PID 1 — 697 KB static binary", 0.45, 0.55, 8.0, 0.44,
             size=21, bold=True, color=TEXT)

    modules = [
        ("Manifest Parser",   "Reads vyoma.toml per app\nEnforces capability schema",   PURPLE),
        ("Scheduler",         "One thread per app\nConcurrent spawn + lifecycle",       BLUE),
        ("IPC Broker",        "@<app>: message routing\nBroadcast, reply, unicast",      GREEN),
        ("Display Driver",    "DRM/virtio-gpu framebuffer\nVYOMA_DRAW protocol parser",  CYAN),
        ("Input Router",      "Raw TTY mode\nPer-keypress dispatch to focused app",      ORANGE),
        ("Process Manager",   "ps · kill · restart · reload\nlog · logf · watchdog",    RED),
    ]

    for i, (name, desc, col) in enumerate(modules):
        row = i // 3
        cidx = i % 3
        cx = 0.45 + cidx * 3.18
        cy = 1.18 + row * 1.72
        card(s, cx, cy, 2.98, 1.52)
        add_rect(s, cx, cy, 2.98, 0.065, fill=col)
        icon_box(s, cx + 0.18, cy + 0.2, 0.38, col, "")
        add_rect(s, cx + 0.18, cy + 0.2, 0.38, 0.38, fill=col)
        add_text(s, name, cx + 0.7, cy + 0.22, 2.1, 0.35,
                 size=11, bold=True, color=col)
        add_text(s, desc, cx + 0.18, cy + 0.72, 2.6, 0.65,
                 size=9, color=MUTED)

    bottom_bar(s)
    slide_number(s, 6)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 07  — DISPLAY SYSTEM
# ══════════════════════════════════════════════════════════════════════════════
def slide_07():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, CYAN)
    section_tag(s, "Display System", CYAN)

    add_text(s, "VYOMA_DRAW Protocol", 0.45, 0.55, 6.0, 0.44,
             size=21, bold=True, color=TEXT)
    add_text(s, "Line-oriented framebuffer commands from app stdout → supervisor renders to DRM/virtio-gpu",
             0.45, 1.08, 9.1, 0.3, size=10.5, color=MUTED)

    # Protocol commands
    cmds = [
        ("fill_rect",      "VYOMA_DRAW:fill_rect:<x>,<y>,<w>,<h>,<rgba>",           PURPLE),
        ("draw_text",      "VYOMA_DRAW:draw_text:<x>,<y>,<rgba>,<size>,<text>",      BLUE),
        ("draw_text_wrap", "VYOMA_DRAW:draw_text_wrap:<x>,<y>,<max_w>,<rgba>,<size>,<text>", CYAN),
        ("flush",          "VYOMA_DRAW:flush    — commit frame to framebuffer",      GREEN),
    ]

    for i, (cmd, proto, col) in enumerate(cmds):
        cy = 1.55 + i * 0.55
        card(s, 0.45, cy, 5.6, 0.46)
        add_rect(s, 0.45, cy, 0.055, 0.46, fill=col)
        add_text(s, cmd,   0.62, cy + 0.05, 1.2, 0.22, size=9.5, bold=True, color=col)
        add_text(s, proto, 0.62, cy + 0.24, 5.2, 0.18, size=8,   color=MUTED)

    # Code example
    card(s, 6.2, 1.42, 3.55, 2.95, fill=RGBColor(0x0D, 0x11, 0x17))
    add_text(s, "// Rust app example", 6.35, 1.52, 3.2, 0.25,
             size=8, italic=True, color=MUTED)
    code = [
        ("const WHITE: u32 = 0xFFFFFFFF;", GREEN_LT, False, 8),
        ("const BG:    u32 = 0x1E1E2EFF;", BLUE,     False, 8),
        ("",                               TEXT,      False, 8),
        ('println!("VYOMA_DRAW:fill_rect:', MUTED,    False, 8),
        ('  0,20,960,700,{BG}");',          TEXT,     False, 8),
        ('println!("VYOMA_DRAW:draw_text:', MUTED,    False, 8),
        ('  8,24,{WHITE},m,Hello");',        TEXT,    False, 8),
        ('println!("VYOMA_DRAW:flush");',   ORANGE,   False, 8),
    ]
    add_multiline(s, code, 6.35, 1.82, 3.3, 2.4, size=8)

    # Font sizes
    add_text(s, "Font sizes:  s = 4×8   m = 8×16   l = 16×32",
             0.45, 3.85, 5.8, 0.3, size=9, color=MUTED)

    # Window chrome features
    add_text(s, "Window Chrome (Phase 17)", 0.45, 4.2, 5.8, 0.3,
             size=10, bold=True, color=CYAN)
    feats = ["Focus border · Accent-colored title bars · Per-app FPS",
             "Status strip (app name + uptime) · Alt+? shortcut overlay"]
    for i, f in enumerate(feats):
        add_text(s, f"• {f}", 0.45, 4.52 + i * 0.25, 5.8, 0.22,
                 size=9, color=MUTED)

    bottom_bar(s, CYAN)
    slide_number(s, 7)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 08  — IPC BROKER
# ══════════════════════════════════════════════════════════════════════════════
def slide_08():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, GREEN)
    section_tag(s, "IPC Broker", GREEN)

    add_text(s, "Every message flows through the supervisor.", 0.45, 0.56, 8.0, 0.44,
             size=21, bold=True, color=TEXT)
    add_text(s, "Apps write @<target>: <msg> to stdout. Supervisor intercepts and routes.",
             0.45, 1.08, 9.1, 0.3, size=10.5, color=MUTED)

    # IPC types
    ipc_types = [
        ("@app-name: msg",    "Unicast",   "Route to one specific app",             GREEN),
        ("@broadcast: msg",   "Broadcast", "Deliver to ALL running apps",            PURPLE),
        ("@reply: msg",       "Reply",     "Route back to the last IPC sender",      BLUE),
        ("@supervisor: cmd",  "Control",   "Built-in commands: list, ping, uptime\nversion, apps, loglevel, focus, kill", ORANGE),
    ]

    for i, (fmt, kind, desc, col) in enumerate(ipc_types):
        row = i // 2
        cidx = i % 2
        cx = 0.45 + cidx * 4.75
        cy = 1.5 + row * 1.5
        card(s, cx, cy, 4.55, 1.3)
        add_rect(s, cx, cy, 0.055, 1.3, fill=col)
        add_text(s, kind, cx + 0.2, cy + 0.1, 1.4, 0.3, size=10, bold=True, color=col)
        card(s, cx + 0.2, cy + 0.45, 3.2, 0.35, fill=BG)
        add_text(s, fmt, cx + 0.3, cy + 0.49, 3.0, 0.27,
                 size=8.5, color=col)
        add_text(s, desc, cx + 0.2, cy + 0.88, 4.1, 0.36,
                 size=9, color=MUTED)

    bottom_bar(s, GREEN)
    slide_number(s, 8)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 09  — APP ECOSYSTEM
# ══════════════════════════════════════════════════════════════════════════════
def slide_09():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, ORANGE)
    section_tag(s, "App Ecosystem", ORANGE)

    add_text(s, "200+ WASM apps — all wasm32-wasip2", 0.45, 0.56, 9.0, 0.44,
             size=21, bold=True, color=TEXT)

    categories = [
        ("Productivity",  ["text-editor", "markdown-viewer", "notes", "kanban", "calendar", "spreadsheet"], BLUE),
        ("Developer",     ["code-editor", "code-runner", "hex-editor", "json-viewer", "diff-viewer", "terminal"], CYAN),
        ("System",        ["system-monitor", "process-inspector", "file-manager", "settings", "app-store", "dock"], GREEN),
        ("Games",         ["chess", "tetris", "snake", "minesweeper", "asteroids", "wordle"], ORANGE),
        ("Visualization", ["fractal", "fourier", "oscilloscope", "fluid", "ray-march", "bezier"], PURPLE),
        ("Creativity",    ["paint-pro", "music-composer", "pixel-art", "photo-editor", "draw-pad", "piano"], RED),
    ]

    for i, (cat, apps_list, col) in enumerate(categories):
        row = i // 3
        cidx = i % 3
        cx = 0.45 + cidx * 3.18
        cy = 1.22 + row * 1.82
        card(s, cx, cy, 2.98, 1.65)
        add_rect(s, cx, cy, 2.98, 0.055, fill=col)
        add_text(s, cat, cx + 0.18, cy + 0.12, 2.6, 0.3, size=10.5, bold=True, color=col)
        apps_str = "  ·  ".join(apps_list[:4])
        more = f"  + {len(apps_list) - 4} more" if len(apps_list) > 4 else ""
        add_text(s, apps_str, cx + 0.18, cy + 0.52, 2.6, 0.25,
                 size=8, color=MUTED)
        add_text(s, more, cx + 0.18, cy + 0.75, 2.6, 0.22,
                 size=8, color=col)
        add_text(s, f"1–10 KB each · Zero native deps", cx + 0.18, cy + 1.2, 2.6, 0.25,
                 size=7.5, color=RGBColor(0x4A, 0x55, 0x68), italic=True)

    bottom_bar(s, ORANGE)
    slide_number(s, 9)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 10  — BUILD SYSTEM
# ══════════════════════════════════════════════════════════════════════════════
def slide_10():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, BLUE)
    section_tag(s, "Build System", BLUE)

    add_text(s, "Docker-based hermetic builds.\nSHA-256 verified. Fully reproducible.", 0.45, 0.5, 7.0, 0.8,
             size=21, bold=True, color=TEXT)

    steps = [
        ("make kernel",     "Linux 5.10 allnoconfig\n→ out/bzImage (2.3 MB)",        MUTED),
        ("make supervisor", "Rust cross-compile\n→ x86_64-unknown-linux-musl",        MUTED),
        ("make apps",       "200+ WASM apps\nPer-stamp incremental builds",           MUTED),
        ("make rootfs",     "cpio.gz initramfs\nBusyBox + Wasmtime + supervisor",     MUTED),
        ("make run",        "QEMU boot\nSerial + virtio-gpu display",                 GREEN),
    ]

    arrow_x = [0.45, 2.35, 4.25, 6.15, 8.05]
    box_w = 1.72

    for i, (cmd, desc, col) in enumerate(steps):
        cx = arrow_x[i]
        card(s, cx, 1.55, box_w, 1.55)
        add_rect(s, cx, 1.55, box_w, 0.055,
                 fill=GREEN if i == 4 else BLUE)
        add_text(s, cmd, cx + 0.12, 1.65, 1.48, 0.32,
                 size=9.5, bold=True, color=GREEN if i == 4 else BLUE)
        add_text(s, desc, cx + 0.12, 2.05, 1.48, 0.92,
                 size=8.5, color=MUTED)
        if i < 4:
            add_text(s, "→", cx + box_w + 0.06, 2.15, 0.25, 0.4,
                     size=12, color=MUTED, align=PP_ALIGN.CENTER)

    # Incremental builds note
    card(s, 0.45, 3.35, 9.1, 0.75, fill=RGBColor(0x0D, 0x14, 0x0D))
    add_rect(s, 0.45, 3.35, 0.055, 0.75, fill=GREEN)
    add_text(s, "Incremental builds", 0.65, 3.42, 2.0, 0.3, size=10, bold=True, color=GREEN)
    add_text(s,
             "Per-app stamp files (out/.apps/<name>.stamp) — only the app whose source changed recompiles.\n"
             "touch apps/snake/src/main.rs && make apps  →  only snake recompiles.",
             0.65, 3.72, 8.7, 0.32, size=8.5, color=MUTED)

    # CI pipeline
    card(s, 0.45, 4.28, 9.1, 0.72, fill=BG2)
    add_text(s, "make test  =  make build  +  make unit-test  +  make smoke",
             0.65, 4.34, 8.7, 0.3, size=9.5, color=BLUE)
    add_text(s,
             "Smoke test: headless QEMU boot → wait for '[lifecycle] all apps spawned' → 30s timeout → SMOKE: PASS/FAIL",
             0.65, 4.65, 8.7, 0.28, size=8.5, color=MUTED)

    bottom_bar(s, BLUE)
    slide_number(s, 10)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 11  — PERFORMANCE & NUMBERS
# ══════════════════════════════════════════════════════════════════════════════
def slide_11():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, ORANGE)
    section_tag(s, "Performance", ORANGE)

    add_text(s, "Numbers that matter.", 0.45, 0.56, 8.0, 0.44,
             size=22, bold=True, color=TEXT)

    metrics = [
        ("< 5 s",    "Boot time in QEMU\n(kernel + supervisor + 10 apps)", ORANGE),
        ("2.3 MB",   "Linux kernel binary\n(allnoconfig + virtio + DRM)",  BLUE),
        ("697 KB",   "Supervisor binary\n(static musl, stripped)",          PURPLE),
        ("18 MB",    "Complete initramfs\n(Wasmtime dominates at 61 MB rootfs)", GREEN),
        ("1–10 KB",  "Average WASM app size\n(no native deps, no stdlib overhead)", CYAN),
        ("10+",      "Concurrent apps at boot\ngui-demo, shell, http-server, …", ORANGE),
    ]

    for i, (val, label, col) in enumerate(metrics):
        row = i // 3
        cidx = i % 3
        cx = 0.45 + cidx * 3.18
        cy = 1.25 + row * 1.9
        card(s, cx, cy, 2.98, 1.72)
        add_text(s, val, cx + 0.2, cy + 0.2, 2.6, 0.65,
                 size=32, bold=True, color=col, align=PP_ALIGN.CENTER)
        divider_line(s, cx + 0.3, cy + 0.88, 2.4, color=DIVIDER)
        add_text(s, label, cx + 0.18, cy + 0.98, 2.62, 0.65,
                 size=9, color=MUTED, align=PP_ALIGN.CENTER)

    bottom_bar(s, ORANGE)
    slide_number(s, 11)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 12  — COMPARISON MATRIX
# ══════════════════════════════════════════════════════════════════════════════
def slide_12():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, PURPLE)
    section_tag(s, "Comparison")

    add_text(s, "VyomaOS vs. the Field", 0.45, 0.54, 8.0, 0.44,
             size=21, bold=True, color=TEXT)

    headers = ["", "VyomaOS", "Alpine", "Docker/OCI", "MirageOS"]
    col_w   = [2.3, 1.72, 1.72, 1.72, 1.72]
    starts  = [0.45, 2.75, 4.47, 6.19, 7.91]

    rows = [
        ("App runtime",    "WASM/WASI P2",    "Native ELF",     "Native ELF",    "OCaml/native"),
        ("Capability",     "Manifest-declared","File perms",     "OCI (optional)","None"),
        ("Kernel size",    "2.3 MB",           "5–10 MB",        "Host kernel",   "N/A (hypervisor)"),
        ("App portability","Any lang→wasm32",  "Arch-specific",  "Arch-specific", "OCaml/C limited"),
        ("Attack surface", "Kernel+WT+Supv",   "Full userland",  "Full userland", "Hypervisor only"),
        ("Binary determ.", "Byte-identical",   "Varies",         "Layer hash",    "Varies"),
    ]

    header_y = 1.1
    for ci, (hdr, cw, cx) in enumerate(zip(headers, col_w, starts)):
        if ci == 0:
            add_text(s, hdr, cx, header_y, cw, 0.3, size=9, bold=True, color=MUTED)
        else:
            col_c = PURPLE_LT if ci == 1 else MUTED
            bg_c  = RGBColor(0x1A, 0x16, 0x2E) if ci == 1 else BG2
            add_rect(s, cx, header_y - 0.04, cw - 0.05, 0.32, fill=bg_c)
            add_text(s, hdr, cx + 0.05, header_y, cw - 0.1, 0.28,
                     size=9, bold=True, color=col_c, align=PP_ALIGN.CENTER)

    for ri, (row_label, *cells) in enumerate(rows):
        ry = 1.5 + ri * 0.62
        add_text(s, row_label, starts[0], ry + 0.04, col_w[0] - 0.1, 0.35,
                 size=9, bold=True, color=MUTED)
        for ci, (cell, cw, cx) in enumerate(zip(cells, col_w[1:], starts[1:])):
            bg_c = RGBColor(0x13, 0x10, 0x22) if ci == 0 else (
                   RGBColor(0x0F, 0x14, 0x18) if ri % 2 else BG)
            add_rect(s, cx, ry, cw - 0.05, 0.55, fill=bg_c)
            col_c = GREEN if ci == 0 else MUTED
            add_text(s, cell, cx + 0.06, ry + 0.1, cw - 0.12, 0.36,
                     size=8.5, color=col_c, align=PP_ALIGN.CENTER)

    bottom_bar(s)
    slide_number(s, 12)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 13  — PHASE ROADMAP
# ══════════════════════════════════════════════════════════════════════════════
def slide_13():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, CYAN)
    section_tag(s, "Roadmap", CYAN)

    add_text(s, "17 Phases Complete. Building Toward a Full Desktop OS.", 0.45, 0.56, 9.1, 0.44,
             size=19, bold=True, color=TEXT)

    phases = [
        ("P01–P04", "Kernel + Supervisor + Manifest + IPC",      True),
        ("P05–P08", "Seccomp + Storage + Unit Tests + CI",        True),
        ("P09–P10", "Display (DRM/virtio-gpu) + Bitmap Font",     True),
        ("P11–P12", "Networking + Interactive Shell + Keyboard",   True),
        ("P13–P15", "Process Mgmt + Package Mgr + Persistent Logs",True),
        ("P16–P17", "Real-time TTY + Windowing + Mouse Input",    True),
        ("P18–P19", "Font Scaling + GPU-Accelerated Display",     False),
        ("P20–P21", "Multi-user Auth + App Store",                False),
        ("P22+",    "Full Desktop Shell + Hardware Support",      False),
    ]

    for i, (num, desc, done) in enumerate(phases):
        row = i // 3
        cidx = i % 3
        cx = 0.45 + cidx * 3.18
        cy = 1.22 + row * 1.38
        col = GREEN if done else RGBColor(0x1E, 0x29, 0x3B)
        text_col = TEXT if done else MUTED
        card(s, cx, cy, 2.98, 1.2, fill=RGBColor(0x10, 0x1A, 0x10) if done else BG2)
        add_rect(s, cx, cy, 2.98, 0.055, fill=GREEN if done else MUTED)
        add_text(s, "✓ " + num if done else num, cx + 0.18, cy + 0.1, 2.6, 0.3,
                 size=9.5, bold=True, color=GREEN if done else MUTED)
        add_text(s, desc, cx + 0.18, cy + 0.48, 2.6, 0.62,
                 size=8.5, color=text_col)

    bottom_bar(s, CYAN)
    slide_number(s, 13)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 14  — WINDOWING SYSTEM (Current Work)
# ══════════════════════════════════════════════════════════════════════════════
def slide_14():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, PURPLE)
    section_tag(s, "Current: P17-18")

    add_text(s, "Windowing System — Live Today", 0.45, 0.56, 9.0, 0.44,
             size=21, bold=True, color=TEXT)
    add_text(s, "Supervisor manages window chrome, focus, per-app status strips, and keyboard routing.",
             0.45, 1.08, 9.1, 0.3, size=10.5, color=MUTED)

    features = [
        ("Focus Border",       "2px accent-color border around active window",          PURPLE),
        ("Title Bar Accent",   "Deterministic per-app color · Bright on focus, 80% on blur", BLUE),
        ("Status Strip",       "Per-window bar: app name + uptime, drawn by supervisor", GREEN),
        ("FPS Tracking",       "Per-app flush rate · Logged every 5s",                  CYAN),
        ("Click to Focus",     "Click app name in menu bar → raise & focus window",     ORANGE),
        ("Drag to Move",       "Mouse drag deltas logged · Window repositioning",       RED),
        ("Alt+? Overlay",      "Keyboard shortcut cheat sheet as toast overlay",        PURPLE_LT),
        ("IPC: @supervisor",   "focus · list · ping · uptime · version · apps · loglevel", MUTED),
    ]

    for i, (name, desc, col) in enumerate(features):
        row = i // 2
        cidx = i % 2
        cx = 0.45 + cidx * 4.75
        cy = 1.5 + row * 0.85
        card(s, cx, cy, 4.55, 0.72)
        add_rect(s, cx, cy, 0.055, 0.72, fill=col)
        add_text(s, name, cx + 0.2, cy + 0.08, 2.0, 0.28, size=10, bold=True, color=col)
        add_text(s, desc, cx + 0.2, cy + 0.38, 4.1, 0.28, size=8.5, color=MUTED)

    bottom_bar(s)
    slide_number(s, 14)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 15  — SECURITY DEEP DIVE
# ══════════════════════════════════════════════════════════════════════════════
def slide_15():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, RED)
    section_tag(s, "Security Deep Dive", RED)

    add_text(s, "Defense in depth — without the complexity.", 0.45, 0.56, 9.0, 0.44,
             size=21, bold=True, color=TEXT)

    layers_sec = [
        ("Layer 1: WASM Sandbox",
         "Every app runs inside Wasmtime's WASM sandbox. Memory is isolated. No raw pointers.\nStack overflows, buffer overruns, UAF — eliminated at the bytecode level.",
         RED),
        ("Layer 2: WASI Capability Model",
         "App declares capabilities in vyoma.toml. Supervisor wires ONLY declared interfaces.\nNo network capability → no TCP socket wired up. Not filtered — simply absent.",
         ORANGE),
        ("Layer 3: Seccomp BPF Denylist",
         "Supervisor applies seccomp BPF denylist to all Wasmtime child processes.\nBlocks dangerous syscalls even if Wasmtime were somehow compromised.",
         PURPLE),
        ("Layer 4: No C Userland",
         "No shell. No libc. No interpreters. No POSIX env vars. No /proc injection.\nAttack surface = kernel + Wasmtime + supervisor. That's it.",
         GREEN),
    ]

    for i, (title, body, col) in enumerate(layers_sec):
        cy = 1.22 + i * 0.95
        card(s, 0.45, cy, 9.1, 0.82)
        add_rect(s, 0.45, cy, 0.06, 0.82, fill=col)
        add_text(s, title, 0.65, cy + 0.06, 3.5, 0.28, size=10.5, bold=True, color=col)
        add_text(s, body,  0.65, cy + 0.35, 8.5, 0.42, size=8.8,  color=MUTED)

    bottom_bar(s, RED)
    slide_number(s, 15)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 16  — DEVELOPER EXPERIENCE
# ══════════════════════════════════════════════════════════════════════════════
def slide_16():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, BLUE)
    section_tag(s, "Dev Experience", BLUE)

    add_text(s, "Write Rust (or Go, C, Python). Get a sandboxed OS app.", 0.45, 0.56, 9.0, 0.44,
             size=20, bold=True, color=TEXT)

    # Steps
    steps_dev = [
        ("1", "mkdir apps/my-app\ncargo init --name my-app", "Create app", BLUE),
        ("2", "[capabilities]\nstdio = true\ndisplay = true", "Declare capabilities", PURPLE),
        ("3", "cargo build\n--target wasm32-wasip2", "Build to WASM", GREEN),
        ("4", "make rootfs && make run", "Boot & run in QEMU", ORANGE),
    ]

    for i, (num, code, label, col) in enumerate(steps_dev):
        cx = 0.45 + i * 2.4
        card(s, cx, 1.22, 2.28, 2.8)
        add_rect(s, cx, 1.22, 2.28, 0.055, fill=col)
        add_rect(s, cx + 0.15, 1.32, 0.45, 0.45, fill=col)
        add_text(s, num, cx + 0.16, 1.32, 0.43, 0.44,
                 size=18, bold=True, color=WHITE, align=PP_ALIGN.CENTER)
        add_text(s, label, cx + 0.75, 1.38, 1.4, 0.32, size=9.5, bold=True, color=col)
        card(s, cx + 0.15, 1.88, 1.98, 0.95, fill=BG)
        add_text(s, code, cx + 0.25, 1.92, 1.8, 0.88, size=8, color=GREEN_LT)

    # In-VM shell commands
    card(s, 0.45, 4.2, 9.1, 0.8, fill=RGBColor(0x0A, 0x0F, 0x0A))
    add_rect(s, 0.45, 4.2, 0.055, 0.8, fill=GREEN)
    add_text(s, "Inside the VM:", 0.65, 4.26, 1.6, 0.28, size=9.5, bold=True, color=GREEN)
    add_text(s,
             "ps  ·  log <name>  ·  logf <name>  ·  kill <name>  ·  restart <name>"
             "  ·  @supervisor: list  ·  @supervisor: focus <name>",
             0.65, 4.54, 8.7, 0.38, size=8.8, color=MUTED)

    bottom_bar(s, BLUE)
    slide_number(s, 16)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 17  — LONG-TERM VISION
# ══════════════════════════════════════════════════════════════════════════════
def slide_17():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    top_bar(s, PURPLE)
    section_tag(s, "Long-Term Vision")

    add_text(s, "Every feature a modern OS ships.\nDelivered as WASM apps.", 0.45, 0.45, 9.0, 0.82,
             size=26, bold=True, color=TEXT)

    vision_items = [
        ("App Store",       "Signed .wasm bundles · capability manifests\none-command install · sandboxed by default", PURPLE),
        ("Package Manager", "vyoma-pkg · WASM-native · no native binaries\ninstall, update, remove from signed registry", GREEN),
        ("Desktop Shell",   "App launcher · taskbar · notifications\nall WASM, rendered via VYOMA_DRAW", BLUE),
        ("Multi-user Auth", "Capability tokens · user-scoped filesystem\nno UNIX uid/gid dependency", ORANGE),
        ("Hardware Drivers","USB · audio · camera · sensors\neach as a typed WASI interface", CYAN),
        ("OTA Updates",     "Atomic supervisor + kernel updates\nrollback-capable · app updates via diff", RED),
    ]

    for i, (title, desc, col) in enumerate(vision_items):
        row = i // 3
        cidx = i % 3
        cx = 0.45 + cidx * 3.18
        cy = 1.55 + row * 1.68
        card(s, cx, cy, 2.98, 1.5)
        add_rect(s, cx, cy, 0.055, 1.5, fill=col)
        add_text(s, title, cx + 0.2, cy + 0.1, 2.6, 0.3, size=10.5, bold=True, color=col)
        add_text(s, desc,  cx + 0.2, cy + 0.5, 2.6, 0.85, size=8.8, color=MUTED)

    bottom_bar(s)
    slide_number(s, 17)


# ══════════════════════════════════════════════════════════════════════════════
# SLIDE 18  — CLOSING / CTA
# ══════════════════════════════════════════════════════════════════════════════
def slide_18():
    s = prs.slides.add_slide(BLANK)
    bg(s)
    add_rect(s, 0, 0, 4.5, SH, fill=RGBColor(0x11, 0x0D, 0x1F))
    add_rect(s, 0, 0, 4.5, 0.07, fill=PURPLE)
    add_rect(s, 4.5, 0, SW - 4.5, 0.07, fill=GREEN)

    logo_mark(s, 0.5, 0.9)
    add_text(s, "VyomaOS", 0.8, 0.9, 3.0, 0.5, size=30, bold=True, color=WHITE)
    add_text(s, "github.com/hbarve1/vyomaos", 0.5, 1.52, 3.5, 0.3,
             size=11, color=MUTED)

    divider_line(s, 0.5, 2.0, 3.5, color=PURPLE)

    taglines = [
        "Capability-secure by default",
        "200+ WASM apps · 18 MB",
        "Boots in < 5 seconds",
        "Built in public · PRs welcome",
    ]
    for i, tag in enumerate(taglines):
        add_text(s, f"→  {tag}", 0.55, 2.15 + i * 0.38, 3.4, 0.32,
                 size=10.5, color=GREEN_LT if i == 0 else MUTED)

    add_text(s, "Let's build the future of OS security together.",
             0.5, 3.85, 3.9, 0.45, size=11, bold=True, color=PURPLE_LT)

    # Right side
    add_text(s, "The Runtime\nIS the OS Boundary.", 4.9, 1.0, 4.7, 1.2,
             size=30, bold=True, color=RGBColor(0x2D, 0x1F, 0x52), align=PP_ALIGN.CENTER)

    milestones = [
        ("Phase 01–17", "Complete",                   GREEN),
        ("200+ Apps",   "wasm32-wasip2, 1–10 KB",     PURPLE_LT),
        ("< 5s Boot",   "QEMU, 10 concurrent apps",   BLUE),
        ("18 MB",       "Full OS image",               ORANGE),
    ]

    for i, (key, val, col) in enumerate(milestones):
        row = i // 2
        cidx = i % 2
        cx = 4.85 + cidx * 2.4
        cy = 2.6 + row * 1.08
        card(s, cx, cy, 2.28, 0.88)
        add_text(s, key, cx + 0.15, cy + 0.06, 2.0, 0.32, size=11, bold=True, color=col)
        add_text(s, val, cx + 0.15, cy + 0.42, 2.0, 0.38, size=9, color=MUTED)

    bottom_bar(s, GREEN)
    slide_number(s, 18)


# ── Build all slides ────────────────────────────────────────────────────────
slide_01(); print("✓ Slide 01 — Title")
slide_02(); print("✓ Slide 02 — The Problem")
slide_03(); print("✓ Slide 03 — Vision")
slide_04(); print("✓ Slide 04 — Architecture")
slide_05(); print("✓ Slide 05 — Capability Security")
slide_06(); print("✓ Slide 06 — Supervisor (PID 1)")
slide_07(); print("✓ Slide 07 — Display System")
slide_08(); print("✓ Slide 08 — IPC Broker")
slide_09(); print("✓ Slide 09 — App Ecosystem")
slide_10(); print("✓ Slide 10 — Build System")
slide_11(); print("✓ Slide 11 — Performance Numbers")
slide_12(); print("✓ Slide 12 — Comparison Matrix")
slide_13(); print("✓ Slide 13 — Roadmap")
slide_14(); print("✓ Slide 14 — Windowing (Current Work)")
slide_15(); print("✓ Slide 15 — Security Deep Dive")
slide_16(); print("✓ Slide 16 — Developer Experience")
slide_17(); print("✓ Slide 17 — Long-Term Vision")
slide_18(); print("✓ Slide 18 — Closing / CTA")

OUT = "/Users/hbarve1/codes/github/hbarve1/vyomaos/docs/presentation/VyomaOS.pptx"
prs.save(OUT)
print(f"\n✅  Saved → {OUT}")
