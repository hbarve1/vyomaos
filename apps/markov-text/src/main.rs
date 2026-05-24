use std::collections::HashMap;
use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const GY: i32 = 24;

const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_BG:     u32 = 0x0D1117FF;
const C_BORDER: u32 = 0x30363DFF;
const C_CARD:   u32 = 0x161B22FF;
const C_GREEN:  u32 = 0x3FB950FF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn text_wrap(x: i32, y: i32, mw: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text_wrap:{},{},{},{},m,{}", x, y, mw, c, s); }
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

const CORPORA: [(&str, &str); 3] = [
    ("Technology",
     "the compiler transforms source code into machine instructions. \
      algorithms solve complex problems with efficient steps. \
      data structures organize information inside memory. \
      networks transmit packets across connected systems. \
      functions encapsulate reusable logic in programs. \
      variables store state during program execution. \
      loops iterate over sequences of data efficiently. \
      recursion solves problems through self-referential calls. \
      abstractions hide implementation details from callers. \
      interfaces define contracts between software components. \
      testing ensures that code behaves correctly and reliably. \
      optimization improves performance of critical software paths. \
      concurrency allows multiple tasks to run simultaneously. \
      memory management prevents leaks and data corruption. \
      types constrain values and prevent runtime errors."),
    ("Nature",
     "rivers flow downward toward the distant sea. \
      birds migrate across continents each passing autumn. \
      forests breathe oxygen into the surrounding atmosphere. \
      mountains form slowly over millions of years. \
      seeds germinate in warm and moist fertile soil. \
      storms gather immense energy from warm ocean waters. \
      animals adapt to harsh environments over generations. \
      sunlight drives photosynthesis inside green living plants. \
      water cycles endlessly through clouds and falling rain. \
      roots anchor ancient trees in rocky barren soil. \
      seasons change as the earth tilts on its axis. \
      creatures hunt and hide beneath the forest shadows. \
      flowers bloom when warm conditions become favorable. \
      snow melts into clear streams during early spring. \
      wind carries pollen between distant flowering plants."),
    ("Philosophy",
     "existence precedes essence in all conscious beings. \
      reality consists of hidden forms and surface appearances. \
      the mind perceives patterns within direct experience. \
      truth emerges from careful observation and deep reason. \
      knowledge requires both strong evidence and honest reflection. \
      consciousness arises from complex physical brain processes. \
      language shapes and limits the boundaries of thought. \
      freedom demands full responsibility for each choice made. \
      identity persists despite continuous and gradual change. \
      beauty reveals hidden harmony within the natural world. \
      wisdom grows slowly through suffering and honest reflection. \
      meaning requires genuine relationship with other people. \
      time flows only forward in one direction always. \
      perception filters and constructs our experienced reality. \
      logic connects premises to valid and sound conclusions."),
];

fn build_chain(corpus: &str) -> (HashMap<String, Vec<String>>, Vec<String>) {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    let mut starts: Vec<String> = Vec::new();
    for sentence in corpus.split('.') {
        let words: Vec<&str> = sentence.split_whitespace().collect();
        if words.is_empty() { continue; }
        starts.push(words[0].to_lowercase());
        for i in 0..words.len().saturating_sub(1) {
            let from = words[i].to_lowercase();
            let to   = words[i + 1].to_lowercase();
            map.entry(from).or_default().push(to);
        }
    }
    (map, starts)
}

fn generate_sentence(map: &HashMap<String, Vec<String>>, starts: &[String], rng: &mut u64) -> String {
    if starts.is_empty() { return "No words found.".to_string(); }
    *rng = lcg(*rng);
    let start = &starts[(*rng >> 32) as usize % starts.len()];
    let mut word = start.clone();
    let mut words = vec![capitalize(&word)];
    for _ in 1..18 {
        *rng = lcg(*rng);
        match map.get(&word) {
            Some(nexts) if !nexts.is_empty() => {
                word = nexts[(*rng >> 32) as usize % nexts.len()].clone();
                words.push(word.clone());
            }
            _ => break,
        }
    }
    let mut s = words.join(" ");
    s.push('.');
    s
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().to_string() + c.as_str(),
    }
}

struct App {
    corpus:   usize,
    chain:    HashMap<String, Vec<String>>,
    starts:   Vec<String>,
    sentences: [String; 3],
    rng:      u64,
}

impl App {
    fn new() -> Self {
        let mut rng: u64 = 0xDEADBEEF_CAFEBABE;
        let (chain, starts) = build_chain(CORPORA[0].1);
        let sentences = [
            generate_sentence(&chain, &starts, &mut rng),
            generate_sentence(&chain, &starts, &mut rng),
            generate_sentence(&chain, &starts, &mut rng),
        ];
        App { corpus: 0, chain, starts, sentences, rng }
    }

    fn reload_corpus(&mut self) {
        let (chain, starts) = build_chain(CORPORA[self.corpus].1);
        self.chain  = chain;
        self.starts = starts;
        self.regenerate();
    }

    fn regenerate(&mut self) {
        for i in 0..3 {
            self.sentences[i] = generate_sentence(&self.chain, &self.starts, &mut self.rng);
        }
    }

    fn render(&self) {
        fill(0, 0, W, GY, C_HEADER);
        let name = CORPORA[self.corpus].0;
        text(12, 4, C_TEXT, &format!("Markov Text — {}", name));
        text(620, 4, C_HINT, "Space=generate  Tab=corpus  Q=quit");

        fill(0, GY, W, H - GY, C_BG);

        let card_h = 190;
        let card_pad = 16;
        for i in 0..3 {
            let cy = GY + 30 + i as i32 * (card_h + 12);
            fill(20, cy, W - 40, card_h, C_CARD);
            fill(20, cy, W - 40, 1, C_BORDER);
            fill(20, cy + card_h - 1, W - 40, 1, C_BORDER);
            fill(20, cy, 1, card_h, C_BORDER);
            fill(W - 21, cy, 1, card_h, C_BORDER);
            fill(20, cy, 3, card_h, C_GREEN);

            let label = ["Sentence 1", "Sentence 2", "Sentence 3"][i];
            text(30 + card_pad, cy + 10, C_HINT, label);
            text_wrap(30 + card_pad, cy + 32, W - 80, C_TEXT, &self.sentences[i]);
        }

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            " "           => { self.regenerate(); }
            "\t"          => { self.corpus = (self.corpus + 1) % 3; self.reload_corpus(); }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
        self.render();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.render();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
