use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 720;
const HEADER_H: u32 = 40;
const RULER_Y: u32 = HEADER_H + 8;
const RULER_H: u32 = 24;
const TL_Y: u32 = RULER_Y + RULER_H + 4;    // timeline bar Y
const TL_H: u32 = 8;
const TICK_AREA_H: u32 = 200;                // space below timeline for ticks+labels
const INFO_Y: u32 = TL_Y + TL_H + TICK_AREA_H + 8;
const INFO_H: u32 = H - INFO_Y - 32;
const STATUS_H: u32 = 28;
const CONTENT_W: u32 = W - 32;
const TL_X: u32 = 16;
const TL_W: u32 = CONTENT_W;

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

#[derive(Clone, Copy, PartialEq)]
enum Cat { Science, Technology, Politics, Art, Space }

impl Cat {
    fn name(self) -> &'static str {
        match self {
            Cat::Science    => "Science",
            Cat::Technology => "Technology",
            Cat::Politics   => "Politics",
            Cat::Art        => "Art",
            Cat::Space      => "Space",
        }
    }
    fn color(self) -> u32 {
        match self {
            Cat::Science    => C_GREEN,
            Cat::Technology => C_SEL,
            Cat::Politics   => C_RED,
            Cat::Art        => C_PURPLE,
            Cat::Space      => C_ORANGE,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Cat::Science    => "1",
            Cat::Technology => "2",
            Cat::Politics   => "3",
            Cat::Art        => "4",
            Cat::Space      => "5",
        }
    }
}

struct Event {
    year:    i32,
    title:   &'static str,
    desc:    &'static str,
    cat:     Cat,
}

const EVENTS: &[Event] = &[
    Event { year: 1440, title: "Gutenberg Printing Press",    desc: "Johannes Gutenberg invents movable type printing, revolutionizing information distribution across Europe.", cat: Cat::Technology },
    Event { year: 1492, title: "Columbus reaches Americas",   desc: "Christopher Columbus sails from Spain and reaches the Caribbean, beginning the Age of Exploration.", cat: Cat::Politics },
    Event { year: 1543, title: "Heliocentric Model",          desc: "Copernicus publishes De revolutionibus, proposing the Sun at the center of the solar system.", cat: Cat::Science },
    Event { year: 1590, title: "Compound Microscope",         desc: "Zacharias Janssen invents the compound microscope, opening the world of microbiology.", cat: Cat::Science },
    Event { year: 1609, title: "Galileo's Telescope",         desc: "Galileo Galilei improves the telescope and observes moons of Jupiter, supporting heliocentrism.", cat: Cat::Science },
    Event { year: 1687, title: "Newton's Principia",          desc: "Isaac Newton publishes Principia Mathematica, establishing laws of motion and universal gravitation.", cat: Cat::Science },
    Event { year: 1712, title: "Steam Engine",                desc: "Thomas Newcomen builds the first practical steam engine, igniting the Industrial Revolution.", cat: Cat::Technology },
    Event { year: 1776, title: "American Independence",       desc: "The United States declares independence from Britain on July 4th, 1776.", cat: Cat::Politics },
    Event { year: 1789, title: "French Revolution",           desc: "The French Revolution begins with the storming of the Bastille, reshaping European politics.", cat: Cat::Politics },
    Event { year: 1800, title: "Volta's Battery",             desc: "Alessandro Volta invents the electric battery (voltaic pile), enabling electrical experiments.", cat: Cat::Science },
    Event { year: 1826, title: "First Photograph",            desc: "Joseph Nicéphore Niépce takes the oldest surviving photograph, View from the Window at Le Gras.", cat: Cat::Art },
    Event { year: 1844, title: "Telegraph Patented",          desc: "Samuel Morse patents the electric telegraph, enabling near-instant long-distance communication.", cat: Cat::Technology },
    Event { year: 1859, title: "Darwin's Origin of Species",  desc: "Charles Darwin publishes On the Origin of Species, establishing the theory of evolution.", cat: Cat::Science },
    Event { year: 1876, title: "Telephone Invented",          desc: "Alexander Graham Bell patents the telephone, transforming human communication forever.", cat: Cat::Technology },
    Event { year: 1879, title: "Edison's Light Bulb",         desc: "Thomas Edison demonstrates a practical incandescent light bulb, lighting up the modern world.", cat: Cat::Technology },
    Event { year: 1895, title: "X-Rays Discovered",           desc: "Wilhelm Röntgen discovers X-rays, revolutionizing medical diagnosis and physics research.", cat: Cat::Science },
    Event { year: 1903, title: "Wright Brothers Fly",         desc: "Orville and Wilbur Wright achieve the first powered aircraft flight at Kitty Hawk.", cat: Cat::Technology },
    Event { year: 1905, title: "Einstein's Relativity",       desc: "Albert Einstein publishes special relativity, transforming our understanding of space and time.", cat: Cat::Science },
    Event { year: 1914, title: "World War I Begins",          desc: "Assassination of Archduke Franz Ferdinand triggers the first global war (1914–1918).", cat: Cat::Politics },
    Event { year: 1928, title: "Penicillin Discovered",       desc: "Alexander Fleming discovers penicillin, founding the age of modern antibiotics.", cat: Cat::Science },
    Event { year: 1939, title: "World War II Begins",         desc: "Germany invades Poland; Britain and France declare war, beginning the deadliest conflict in history.", cat: Cat::Politics },
    Event { year: 1945, title: "Atomic Bomb / WWII End",      desc: "USA drops atomic bombs on Hiroshima and Nagasaki; World War II ends in August 1945.", cat: Cat::Politics },
    Event { year: 1947, title: "Transistor Invented",         desc: "Shockley, Bardeen, and Brattain invent the transistor at Bell Labs, enabling modern electronics.", cat: Cat::Technology },
    Event { year: 1953, title: "DNA Double Helix",            desc: "Watson and Crick describe the double-helix structure of DNA, founding molecular biology.", cat: Cat::Science },
    Event { year: 1957, title: "Sputnik Launched",            desc: "Soviet Union launches Sputnik 1, the first artificial satellite, starting the Space Age.", cat: Cat::Space },
    Event { year: 1961, title: "Yuri Gagarin in Space",       desc: "Yuri Gagarin becomes the first human in space aboard Vostok 1.", cat: Cat::Space },
    Event { year: 1969, title: "Moon Landing",                desc: "Apollo 11 lands on the Moon; Neil Armstrong and Buzz Aldrin walk on the lunar surface.", cat: Cat::Space },
    Event { year: 1971, title: "First Microprocessor",        desc: "Intel introduces the 4004 microprocessor, placing a computer on a single chip.", cat: Cat::Technology },
    Event { year: 1989, title: "World Wide Web",              desc: "Tim Berners-Lee proposes the World Wide Web at CERN, connecting documents via hyperlinks.", cat: Cat::Technology },
    Event { year: 1990, title: "Hubble Space Telescope",      desc: "NASA launches the Hubble Space Telescope, providing unprecedented views of the universe.", cat: Cat::Space },
    Event { year: 1991, title: "Soviet Union Dissolves",      desc: "The USSR officially dissolves on December 25, 1991, ending the Cold War era.", cat: Cat::Politics },
    Event { year: 2003, title: "Human Genome Sequenced",      desc: "The Human Genome Project completes sequencing the human genome, opening personalized medicine.", cat: Cat::Science },
    Event { year: 2007, title: "iPhone Announced",            desc: "Steve Jobs unveils the first iPhone, combining phone, iPod, and internet in one device.", cat: Cat::Technology },
    Event { year: 2012, title: "Higgs Boson Confirmed",       desc: "CERN announces confirmation of the Higgs boson, completing the Standard Model of physics.", cat: Cat::Science },
    Event { year: 2015, title: "Gravitational Waves Detected",desc: "LIGO detects gravitational waves from merging black holes, confirming Einstein's prediction.", cat: Cat::Science },
    Event { year: 2020, title: "COVID-19 Pandemic",           desc: "SARS-CoV-2 spreads globally; WHO declares a pandemic in March 2020.", cat: Cat::Science },
    Event { year: 2021, title: "Webb Telescope Launched",     desc: "James Webb Space Telescope launches on December 25, 2021, succeeding Hubble.", cat: Cat::Space },
    Event { year: 2023, title: "AI Inflection Point",         desc: "Large language models (GPT-4, Claude) reach widespread adoption, transforming knowledge work.", cat: Cat::Technology },
    Event { year: 2026, title: "VyomaOS 1.0 Released",        desc: "VyomaOS launches as the first WASM-first capability-secure OS built on Linux + Wasmtime.", cat: Cat::Technology },
];

struct App {
    view_start: i32,   // leftmost year in view
    view_span:  i32,   // total years visible
    sel:        usize,
    cat_filter: Option<Cat>,
    status:     String,
}

fn visible_events(app: &App) -> Vec<usize> {
    let view_end = app.view_start + app.view_span;
    EVENTS.iter().enumerate()
        .filter(|(_, e)| {
            e.year >= app.view_start && e.year <= view_end
                && app.cat_filter.map_or(true, |c| e.cat == c)
        })
        .map(|(i, _)| i)
        .collect()
}

fn year_to_x(year: i32, view_start: i32, view_span: i32) -> u32 {
    let frac = (year - view_start) as f32 / view_span as f32;
    TL_X + (frac * TL_W as f32) as u32
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Timeline Viewer");
    let span_label = format!("{} – {}", app.view_start, app.view_start + app.view_span);
    text(200, 12, C_TEXT, &span_label);
    text(400, 12, C_HINT, "←→ scroll · +/- zoom · 1-5 cat · Tab=all · ↑↓ select");

    // Category legend
    let cats = [Cat::Science, Cat::Technology, Cat::Politics, Cat::Art, Cat::Space];
    let mut lx = 16u32;
    for cat in cats {
        let active = app.cat_filter.map_or(false, |c| c == cat);
        let bg = if active { cat.color() & 0xFFFFFF40 | 0x00000040 } else { C_CARD };
        let _ = bg;
        let col = if active { cat.color() } else { C_HINT };
        let label = format!("[{}]{}", cat.key(), cat.name());
        fill(lx, HEADER_H + 2, label.len() as u32 * 8 + 8, 20, if active { C_CARD } else { C_BG });
        if active { border(lx, HEADER_H + 2, label.len() as u32 * 8 + 8, 20, cat.color()); }
        text(lx + 4, HEADER_H + 6, col, &label);
        lx += label.len() as u32 * 8 + 16;
    }

    // Decade ruler
    let view_end = app.view_start + app.view_span;
    let decade_step = if app.view_span <= 100 { 10 } else if app.view_span <= 300 { 50 } else { 100 };
    let first_decade = ((app.view_start + decade_step - 1) / decade_step) * decade_step;
    let mut yr = first_decade;
    while yr <= view_end {
        let x = year_to_x(yr, app.view_start, app.view_span);
        fill(x, RULER_Y, 1, RULER_H, C_BORDER);
        text(x.saturating_sub(16), RULER_Y + 6, C_HINT, &yr.to_string());
        yr += decade_step;
    }

    // Timeline bar
    fill(TL_X, TL_Y, TL_W, TL_H, C_BORDER);

    // Event ticks
    let vis = visible_events(app);
    let sel_idx = app.sel;

    // Alternate tick levels to avoid overlap
    for (slot, &ei) in vis.iter().enumerate() {
        let ev = &EVENTS[ei];
        let x = year_to_x(ev.year, app.view_start, app.view_span);
        let is_sel = ei == sel_idx;
        let col = if is_sel { C_TEXT } else { ev.cat.color() };

        // Tick height alternates: short/tall
        let tick_h = if slot % 2 == 0 { 40u32 } else { 80u32 };
        let tick_y = TL_Y + TL_H;
        fill(x, tick_y, if is_sel { 3 } else { 1 }, tick_h, col);

        // Dot at top of tick
        fill(x.saturating_sub(2), tick_y + tick_h, 5, 5, col);

        // Label below dot (truncated)
        let max_chars = 14usize;
        let label = if ev.title.len() > max_chars { &ev.title[..max_chars] } else { ev.title };
        let lx = x.saturating_sub(4);
        let ly = tick_y + tick_h + 8;
        if ly + 16 < INFO_Y {
            text(lx, ly, col, label);
            // Year under label
            text(lx, ly + 14, C_HINT, &ev.year.to_string());
        }
    }

    // Info panel
    fill(TL_X, INFO_Y, CONTENT_W, INFO_H, C_CARD);
    border(TL_X, INFO_Y, CONTENT_W, INFO_H, C_BORDER);

    if sel_idx < EVENTS.len() {
        let ev = &EVENTS[sel_idx];
        let in_view = ev.year >= app.view_start && ev.year <= view_end;
        let col = ev.cat.color();
        text(TL_X + 8, INFO_Y + 8, col, &format!("{} — {}", ev.year, ev.title));
        fill(TL_X + 8, INFO_Y + 28, CONTENT_W - 16, 1, C_BORDER);
        text(TL_X + 8, INFO_Y + 36, C_HINT, &format!("Category: {}", ev.cat.name()));
        if !in_view {
            text(TL_X + 200, INFO_Y + 36, C_YELLOW, "(scroll to see this event)");
        }
        // Wrap description manually at ~110 chars
        let desc = ev.desc;
        let wrap = 110usize;
        let mut dy = INFO_Y + 56;
        let mut start = 0;
        while start < desc.len() && dy < INFO_Y + INFO_H - 4 {
            let end = (start + wrap).min(desc.len());
            let chunk = &desc[start..end];
            text(TL_X + 8, dy, C_TEXT, chunk);
            start = end;
            dy += 18;
        }
    } else {
        text(TL_X + 8, INFO_Y + 20, C_HINT, "Select an event with ↑↓");
    }

    // Event count
    let total_vis = visible_events(app).len();
    let all = EVENTS.len();
    text(TL_X + CONTENT_W - 200, INFO_Y + 8, C_HINT, &format!("{}/{} events shown", total_vis, all));

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App {
        view_start: 1400,
        view_span:  640,   // 1400–2040
        sel:        0,
        cat_filter: None,
        status:     String::from("←→=scroll  +/-=zoom  1-5=category  Tab=all  ↑↓=select  R=reset  Ctrl+C=quit"),
    };

    println!("@supervisor: raise timeline");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }

            // Scroll left/right
            "\x1b[D" => {
                let step = app.view_span / 10;
                app.view_start -= step;
            }
            "\x1b[C" => {
                let step = app.view_span / 10;
                app.view_start += step;
            }

            // Zoom in/out
            "+" | "=" => {
                if app.view_span > 20 {
                    let center = app.view_start + app.view_span / 2;
                    app.view_span = (app.view_span * 2 / 3).max(20);
                    app.view_start = center - app.view_span / 2;
                }
            }
            "-" => {
                let center = app.view_start + app.view_span / 2;
                app.view_span = (app.view_span * 3 / 2).min(1000);
                app.view_start = center - app.view_span / 2;
            }

            // Navigate events
            "\x1b[A" => {
                if app.sel > 0 { app.sel -= 1; }
                // scroll view to selected event
                let ev_year = EVENTS[app.sel].year;
                let view_end = app.view_start + app.view_span;
                if ev_year < app.view_start {
                    app.view_start = ev_year - app.view_span / 10;
                } else if ev_year > view_end {
                    app.view_start = ev_year - app.view_span + app.view_span / 10;
                }
            }
            "\x1b[B" => {
                if app.sel + 1 < EVENTS.len() { app.sel += 1; }
                let ev_year = EVENTS[app.sel].year;
                let view_end = app.view_start + app.view_span;
                if ev_year < app.view_start {
                    app.view_start = ev_year - app.view_span / 10;
                } else if ev_year > view_end {
                    app.view_start = ev_year - app.view_span + app.view_span / 10;
                }
            }

            // Category filter
            "1" => { app.cat_filter = Some(Cat::Science); }
            "2" => { app.cat_filter = Some(Cat::Technology); }
            "3" => { app.cat_filter = Some(Cat::Politics); }
            "4" => { app.cat_filter = Some(Cat::Art); }
            "5" => { app.cat_filter = Some(Cat::Space); }
            "\t" => { app.cat_filter = None; }

            // Reset
            "r" | "R" => {
                app.view_start = 1400;
                app.view_span  = 640;
                app.sel        = 0;
                app.cat_filter = None;
                app.status = "View reset.".to_string();
            }

            _ => {}
        }

        draw(&app);
    }
}
