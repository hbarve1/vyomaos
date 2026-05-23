use std::io::{self, BufRead, Write};

const W: u32 = 800;
const H: u32 = 600;
const HEADER_H: u32 = 40;
const CARD_X: u32 = 40;
const CARD_Y: u32 = HEADER_H + 12;
const CARD_W: u32 = W - 80;
const CARD_H: u32 = 240;
const STATS_Y: u32 = CARD_Y + CARD_H + 12;
const STATS_H: u32 = 120;
const KEYS_Y: u32 = STATS_Y + STATS_H + 8;
const STATUS_H: u32 = 28;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_YELLOW: u32 = 0xD29922FF;
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
enum Lang { En, Es, Fr, De }

impl Lang {
    fn name(self) -> &'static str {
        match self { Lang::En => "English", Lang::Es => "Spanish", Lang::Fr => "French", Lang::De => "German" }
    }
    fn code(self) -> &'static str {
        match self { Lang::En => "EN", Lang::Es => "ES", Lang::Fr => "FR", Lang::De => "DE" }
    }
    fn color(self) -> u32 {
        match self { Lang::En => C_SEL, Lang::Es => C_RED, Lang::Fr => C_ORANGE, Lang::De => C_YELLOW }
    }
}

// Each card: (English, Spanish, French, German, hint/category)
const VOCAB: &[(&str, &str, &str, &str, &str)] = &[
    ("house",       "casa",         "maison",      "Haus",        "Nouns: Home"),
    ("water",       "agua",         "eau",         "Wasser",      "Nouns: Nature"),
    ("fire",        "fuego",        "feu",         "Feuer",       "Nouns: Nature"),
    ("book",        "libro",        "livre",       "Buch",        "Nouns: Objects"),
    ("door",        "puerta",       "porte",       "Tür",         "Nouns: Home"),
    ("window",      "ventana",      "fenêtre",     "Fenster",     "Nouns: Home"),
    ("car",         "coche",        "voiture",     "Auto",        "Nouns: Transport"),
    ("tree",        "árbol",        "arbre",       "Baum",        "Nouns: Nature"),
    ("sun",         "sol",          "soleil",      "Sonne",       "Nouns: Nature"),
    ("moon",        "luna",         "lune",        "Mond",        "Nouns: Nature"),
    ("dog",         "perro",        "chien",       "Hund",        "Nouns: Animals"),
    ("cat",         "gato",         "chat",        "Katze",       "Nouns: Animals"),
    ("bread",       "pan",          "pain",        "Brot",        "Nouns: Food"),
    ("wine",        "vino",         "vin",         "Wein",        "Nouns: Food"),
    ("time",        "tiempo",       "temps",       "Zeit",        "Nouns: Abstract"),
    ("to eat",      "comer",        "manger",      "essen",       "Verbs: Basic"),
    ("to drink",    "beber",        "boire",       "trinken",     "Verbs: Basic"),
    ("to sleep",    "dormir",       "dormir",      "schlafen",    "Verbs: Basic"),
    ("to run",      "correr",       "courir",      "laufen",      "Verbs: Basic"),
    ("to read",     "leer",         "lire",        "lesen",       "Verbs: Activity"),
    ("to write",    "escribir",     "écrire",      "schreiben",   "Verbs: Activity"),
    ("to speak",    "hablar",       "parler",      "sprechen",    "Verbs: Communication"),
    ("to love",     "amar",         "aimer",       "lieben",      "Verbs: Emotion"),
    ("to know",     "saber",        "savoir",      "wissen",      "Verbs: Mind"),
    ("to go",       "ir",           "aller",       "gehen",       "Verbs: Movement"),
    ("big",         "grande",       "grand",       "groß",        "Adjectives"),
    ("small",       "pequeño",      "petit",       "klein",       "Adjectives"),
    ("beautiful",   "hermoso",      "beau",        "schön",       "Adjectives"),
    ("fast",        "rápido",       "rapide",      "schnell",     "Adjectives"),
    ("happy",       "feliz",        "heureux",     "glücklich",   "Adjectives"),
];

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

// Card weight for spaced repetition: starts at 1, doubles on wrong answer
struct CardState {
    weight: u32,   // 1 = normal, 2 = review soon
    correct: u32,
    wrong:   u32,
}

struct App {
    lang:       Lang,
    order:      Vec<usize>,   // shuffled indices into VOCAB
    pos:        usize,        // current position in order
    flipped:    bool,
    seed:       u64,
    states:     Vec<CardState>,
    streak:     u32,
    best_streak: u32,
    total:      u32,
    correct:    u32,
    status:     String,
}

impl App {
    fn new() -> Self {
        let mut seed = 0xF1A5CA_12345678u64;
        let mut order: Vec<usize> = (0..VOCAB.len()).collect();
        // Fisher-Yates shuffle with LCG
        for i in (1..order.len()).rev() {
            seed = lcg(seed);
            let j = (seed as usize) % (i + 1);
            order.swap(i, j);
        }
        let states = (0..VOCAB.len()).map(|_| CardState { weight: 1, correct: 0, wrong: 0 }).collect();
        App {
            lang: Lang::Es,
            order,
            pos: 0,
            flipped: false,
            seed,
            states,
            streak: 0,
            best_streak: 0,
            total: 0,
            correct: 0,
            status: String::from("Space=flip  ←→=prev/next  1-4=lang  Y=correct  N=wrong  R=reset  S=shuffle"),
        }
    }

    fn current_card(&self) -> usize {
        self.order[self.pos % self.order.len()]
    }

    fn shuffle(&mut self) {
        for i in (1..self.order.len()).rev() {
            self.seed = lcg(self.seed);
            let j = (self.seed as usize) % (i + 1);
            self.order.swap(i, j);
        }
        self.pos = 0;
        self.flipped = false;
    }

    fn translation(&self, idx: usize) -> &'static str {
        match self.lang {
            Lang::En => VOCAB[idx].0,
            Lang::Es => VOCAB[idx].1,
            Lang::Fr => VOCAB[idx].2,
            Lang::De => VOCAB[idx].3,
        }
    }
}


fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Language Flashcards");

    // Language selector
    let langs = [Lang::En, Lang::Es, Lang::Fr, Lang::De];
    let mut lx = 220u32;
    for (i, &l) in langs.iter().enumerate() {
        let active = app.lang == l;
        let col = if active { l.color() } else { C_HINT };
        let label = format!("[{}]{}", i + 1, l.code());
        if active { border(lx - 2, 8, label.len() as u32 * 8 + 4, 22, l.color()); }
        text(lx, 12, col, &label);
        lx += label.len() as u32 * 8 + 16;
    }

    // Progress indicator
    let total_cards = app.order.len();
    let pos_display = (app.pos % total_cards) + 1;
    text(W - 100, 12, C_HINT, &format!("{}/{}", pos_display, total_cards));

    // Card
    let card_idx = app.current_card();
    let front = VOCAB[card_idx].0;
    let back = app.translation(card_idx);
    let hint = VOCAB[card_idx].4;
    let lang_col = app.lang.color();

    fill(CARD_X, CARD_Y, CARD_W, CARD_H, C_CARD);
    border(CARD_X, CARD_Y, CARD_W, CARD_H, if app.flipped { lang_col } else { C_BORDER });

    if app.flipped {
        // Show answer
        text(CARD_X + 16, CARD_Y + 20, C_HINT, "Translation:");
        let back_x = CARD_X + CARD_W / 2 - back.len() as u32 * 8;
        text(back_x.max(CARD_X + 16), CARD_Y + 70, lang_col, back);
        text(CARD_X + 16, CARD_Y + 120, C_HINT, &format!("({} → {})", Lang::En.name(), app.lang.name()));
        text(CARD_X + 16, CARD_Y + 150, C_HINT, &format!("English: {}", front));
        text(CARD_X + 16, CARD_Y + 180, C_HINT, &format!("Category: {}", hint));

        // Correct/wrong buttons
        text(CARD_X + 16, CARD_Y + 210, C_GREEN, "[Y] Correct");
        text(CARD_X + 200, CARD_Y + 210, C_RED, "[N] Wrong");

        // Card stats
        let cs = &app.states[card_idx];
        let stat_str = format!("Card: {} correct, {} wrong", cs.correct, cs.wrong);
        text(CARD_X + CARD_W - stat_str.len() as u32 * 8 - 8, CARD_Y + 210, C_HINT, &stat_str);
    } else {
        // Show question
        text(CARD_X + 16, CARD_Y + 20, C_HINT, "English:");
        let front_x = CARD_X + CARD_W / 2 - front.len() as u32 * 8;
        text(front_x.max(CARD_X + 16), CARD_Y + 80, C_TEXT, front);
        text(CARD_X + 16, CARD_Y + 150, C_HINT, &format!("Translate to: {}", app.lang.name()));
        text(CARD_X + 16, CARD_Y + 180, C_HINT, &format!("Category: {}", hint));

        if app.states[card_idx].weight > 1 {
            text(CARD_X + CARD_W - 120, CARD_Y + 20, C_YELLOW, "★ Review");
        }

        text(CARD_X + CARD_W / 2 - 40, CARD_Y + 210, C_HINT, "Press Space to flip");
    }

    // Stats panel
    fill(CARD_X, STATS_Y, CARD_W, STATS_H, C_CARD);
    border(CARD_X, STATS_Y, CARD_W, STATS_H, C_BORDER);

    let pct = if app.total > 0 { app.correct * 100 / app.total } else { 0 };
    text(CARD_X + 16, STATS_Y + 8, C_HINT, "Session Stats:");

    // Progress bar
    let bar_w = CARD_W - 32;
    fill(CARD_X + 16, STATS_Y + 28, bar_w, 12, C_BORDER);
    if app.total > 0 {
        let filled = bar_w * app.correct / app.total.max(1);
        fill(CARD_X + 16, STATS_Y + 28, filled, 12, C_GREEN);
    }

    text(CARD_X + 16, STATS_Y + 48, C_GREEN,  &format!("Correct: {}  ({pct}%)", app.correct));
    text(CARD_X + 200, STATS_Y + 48, C_RED,   &format!("Wrong: {}", app.total.saturating_sub(app.correct)));
    text(CARD_X + 380, STATS_Y + 48, C_TEXT,  &format!("Total: {}", app.total));
    text(CARD_X + 16, STATS_Y + 68, C_YELLOW, &format!("Streak: {}  Best: {}", app.streak, app.best_streak));
    text(CARD_X + 300, STATS_Y + 68, C_HINT,  &format!("Language: {}", app.lang.name()));

    // Key hints
    if KEYS_Y + 20 < H - STATUS_H {
        text(CARD_X, KEYS_Y, C_HINT, "Space=flip  ←→=navigate  Y/N=score  R=reset scores  S=shuffle  1-4=language  Ctrl+C=quit");
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise lang-flash");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        let total_cards = app.order.len();

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }

            " " => {
                app.flipped = !app.flipped;
            }

            "\x1b[C" => {
                app.pos = (app.pos + 1) % total_cards;
                app.flipped = false;
            }
            "\x1b[D" => {
                app.pos = if app.pos == 0 { total_cards - 1 } else { app.pos - 1 };
                app.flipped = false;
            }

            "y" | "Y" => {
                if app.flipped {
                    let ci = app.current_card();
                    app.states[ci].correct += 1;
                    // Reduce weight on correct
                    if app.states[ci].weight > 1 { app.states[ci].weight -= 1; }
                    app.correct += 1;
                    app.total += 1;
                    app.streak += 1;
                    if app.streak > app.best_streak { app.best_streak = app.streak; }
                    app.status = format!("Correct! Streak: {}", app.streak);
                    app.pos = (app.pos + 1) % total_cards;
                    app.flipped = false;
                }
            }

            "n" | "N" => {
                if app.flipped {
                    let ci = app.current_card();
                    app.states[ci].wrong += 1;
                    app.states[ci].weight = (app.states[ci].weight * 2).min(4);
                    app.total += 1;
                    app.streak = 0;
                    app.status = format!("Wrong. Card marked for review. (streak reset)");
                    app.pos = (app.pos + 1) % total_cards;
                    app.flipped = false;
                }
            }

            // Language selection
            "1" => { app.lang = Lang::En; app.flipped = false; }
            "2" => { app.lang = Lang::Es; app.flipped = false; }
            "3" => { app.lang = Lang::Fr; app.flipped = false; }
            "4" => { app.lang = Lang::De; app.flipped = false; }

            // Shuffle
            "s" | "S" => {
                app.shuffle();
                app.status = "Deck shuffled.".to_string();
            }

            // Reset scores
            "r" | "R" => {
                for s in &mut app.states { s.correct = 0; s.wrong = 0; s.weight = 1; }
                app.streak = 0;
                app.best_streak = 0;
                app.total = 0;
                app.correct = 0;
                app.status = "Scores reset.".to_string();
            }

            _ => {}
        }

        draw(&app);
    }
}
