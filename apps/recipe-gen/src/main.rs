// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 700;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xD29922FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_CARD: u32   = 0x161B22FF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

// ── Ingredient data (40 total, 8 per category) ────────────────────────────────

static PROTEINS:   &[&str] = &["Chicken", "Beef", "Salmon", "Tofu", "Eggs", "Turkey", "Shrimp", "Lentils"];
static VEGETABLES: &[&str] = &["Broccoli", "Carrot", "Spinach", "Bell Pepper", "Tomato", "Onion", "Zucchini", "Mushroom"];
static GRAINS:     &[&str] = &["Rice", "Pasta", "Quinoa", "Bread", "Oats", "Couscous", "Barley", "Noodles"];
static DAIRY:      &[&str] = &["Cheese", "Butter", "Cream", "Yogurt", "Milk", "Parmesan", "Mozzarella", "Ricotta"];
static SPICES:     &[&str] = &["Salt", "Pepper", "Garlic", "Cumin", "Paprika", "Oregano", "Thyme", "Ginger"];

const CAT_NAMES:   [&str; 5]  = ["Protein", "Vegetable", "Grain", "Dairy", "Spice"];
const CAT_COLORS:  [u32; 5]   = [C_RED, C_GREEN, C_YELLOW, C_SEL, C_ORANGE];

// ── Recipe data ───────────────────────────────────────────────────────────────

struct Recipe {
    name:        String,
    servings:    u8,
    prep_mins:   u8,
    ingredients: Vec<(String, u8)>, // (name, category_idx 0-4)
    steps:       Vec<String>,
}

fn generate_recipe(seed: u64) -> Recipe {
    let mut s = seed;

    s = lcg(s); let protein_idx = ((s >> 33) % 8) as usize;
    s = lcg(s); let veg1_idx    = ((s >> 33) % 8) as usize;
    s = lcg(s); let veg2_raw    = ((s >> 33) % 8) as usize;
    let veg2_idx = if veg2_raw == veg1_idx { (veg2_raw + 1) % 8 } else { veg2_raw };
    s = lcg(s); let grain_idx   = ((s >> 33) % 8) as usize;
    s = lcg(s); let has_dairy   = (s >> 33) % 3 != 0;
    s = lcg(s); let dairy_idx   = ((s >> 33) % 8) as usize;
    s = lcg(s); let spice1_idx  = ((s >> 33) % 8) as usize;
    s = lcg(s); let spice2_raw  = ((s >> 33) % 8) as usize;
    let spice2_idx = if spice2_raw == spice1_idx { (spice2_raw + 1) % 8 } else { spice2_raw };
    s = lcg(s); let servings    = 2u8 + ((s >> 33) % 3) as u8;
    s = lcg(s); let prep_extra  = ((s >> 33) % 15) as u8;

    let protein = PROTEINS[protein_idx];
    let veg1    = VEGETABLES[veg1_idx];
    let veg2    = VEGETABLES[veg2_idx];
    let grain   = GRAINS[grain_idx];
    let spice1  = SPICES[spice1_idx];
    let spice2  = SPICES[spice2_idx];
    let dairy   = if has_dairy { Some(DAIRY[dairy_idx]) } else { None };

    let name = format!("{} {} with {}", protein, grain, veg1);

    let mut ingredients: Vec<(String, u8)> = vec![
        (protein.to_string(), 0),
        (veg1.to_string(),    1),
        (veg2.to_string(),    1),
        (grain.to_string(),   2),
        (spice1.to_string(),  4),
        (spice2.to_string(),  4),
    ];
    if let Some(d) = dairy {
        ingredients.push((d.to_string(), 3));
    }

    let prep_mins = 20 + prep_extra + (ingredients.len() as u8) * 2;

    let step6 = match dairy {
        Some(d) => format!("Stir in {}. Cook {} per package directions.", d, grain),
        None    => format!("Cook {} separately per package directions.", grain),
    };

    let steps = vec![
        format!("Gather: dice {} and {}, slice {} into portions.", veg1, veg2, protein),
        format!("Season {} with {} and {}. Let rest 5 min.", protein, spice1, spice2),
        format!("Heat 2 tbsp oil in a large pan over medium-high heat."),
        format!("Cook {} for 6-8 min until done. Remove and set aside.", protein),
        format!("In same pan, saute {} for 3 min. Add {}, cook 2 min.", veg1, veg2),
        step6,
        format!("Plate {} first. Top with {} and vegetables. Serve hot.", grain, protein),
    ];

    Recipe { name, servings, prep_mins, ingredients, steps }
}

// ── App ───────────────────────────────────────────────────────────────────────

struct App {
    history:  Vec<Recipe>,
    hist_idx: usize,
    seed:     u64,
}

impl App {
    fn new() -> Self {
        let seed = 0x5ECE_1234_5678u64;
        let mut app = App { history: Vec::new(), hist_idx: 0, seed };
        app.generate();
        app
    }

    fn generate(&mut self) {
        self.seed = lcg(self.seed);
        let r = generate_recipe(self.seed);
        self.history.insert(0, r);
        if self.history.len() > 10 { self.history.pop(); }
        self.hist_idx = 0;
    }

    fn current(&self) -> &Recipe {
        &self.history[self.hist_idx]
    }

    fn draw(&self) {
        let recipe = self.current();

        fill(0, 0, W, H, C_BG);

        // Header
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Recipe Generator");
        text(W - 336, 8, C_HINT, "R=new  \u{2190}\u{2192}=history  Q=quit");

        // Left panel — ingredient list
        let lw = 272;
        fill(8, 38, lw, H - 54, C_CARD);
        println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", 8, 38, lw, H - 54, C_BORDER);
        text(16, 46, C_HINT, &format!("INGREDIENTS ({})", recipe.ingredients.len()));

        let mut iy = 66i32;
        for cat in 0..5u8 {
            let items: Vec<&str> = recipe.ingredients.iter()
                .filter(|(_, c)| *c == cat)
                .map(|(n, _)| n.as_str())
                .collect();
            if items.is_empty() { continue; }
            let cc = CAT_COLORS[cat as usize];
            fill(16, iy + 2, 8, 8, cc);
            text(28, iy, cc, CAT_NAMES[cat as usize]);
            iy += 18;
            for item in &items {
                text(22, iy, C_TEXT, &format!("  {}", item));
                iy += 16;
            }
            iy += 6;
        }

        // Right panel — recipe details
        let rx = 288;
        let rw = W - rx - 8;
        fill(rx, 38, rw, H - 54, C_CARD);
        println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", rx, 38, rw, H - 54, C_BORDER);

        // Recipe name
        text(rx + 12, 50, C_ORANGE, &recipe.name);

        // Meta line
        let diff = if recipe.ingredients.len() >= 7 { "Medium" } else { "Easy" };
        text(rx + 12, 72, C_HINT,
             &format!("Serves {}  |  Prep {} min  |  Difficulty: {}",
                 recipe.servings, recipe.prep_mins, diff));

        // Separator
        fill(rx + 12, 90, rw - 24, 1, C_BORDER);

        // Steps
        let step_y_start = 100i32;
        let step_h = 72i32;
        for (i, step) in recipe.steps.iter().enumerate() {
            let sy = step_y_start + i as i32 * step_h;
            // Step number badge
            fill(rx + 12, sy + 2, 20, 16, C_SEL);
            text(rx + 14, sy + 2, C_BG, &format!("{}", i + 1));
            // Step text (wrap at ~68 chars)
            let max_w = rw - 48;
            let chars_per_line = (max_w / 8) as usize;
            text(rx + 36, sy + 2, C_TEXT, &step[..step.len().min(chars_per_line)]);
            if step.len() > chars_per_line {
                text(rx + 36, sy + 18, C_HINT, &step[chars_per_line..step.len().min(chars_per_line * 2)]);
            }
        }

        // Bottom bar
        fill(0, H - 16, W, 16, C_HEADER);
        text(12, H - 14, C_HINT,
             &format!("Recipe {}/{} history  |  {} ingredients  |  {} steps",
                 self.hist_idx + 1, self.history.len(),
                 recipe.ingredients.len(), recipe.steps.len()));

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "r" | "R" => { self.generate(); self.draw(); }
            "\x1b[C" => {
                if self.hist_idx + 1 < self.history.len() {
                    self.hist_idx += 1;
                    self.draw();
                }
            }
            "\x1b[D" => {
                if self.hist_idx > 0 {
                    self.hist_idx -= 1;
                    self.draw();
                }
            }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
        let _ = io::stdout().flush();
    }
}
