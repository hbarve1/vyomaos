// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 920;
const H: u32 = 760;
const HEADER_H: u32 = 48;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_CARD: u32    = 0x161B22FF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_SEL_BG: u32  = 0x1F4068FF;

struct Recipe {
    name:        &'static str,
    prep:        &'static str,
    servings:    &'static str,
    ingredients: &'static [&'static str],
    steps:       &'static [&'static str],
}

static RECIPES: &[Recipe] = &[
    Recipe {
        name: "Spaghetti Bolognese",
        prep: "30 min",
        servings: "4",
        ingredients: &["400g spaghetti", "300g ground beef", "1 onion", "2 garlic cloves",
                       "400g crushed tomatoes", "2 tbsp olive oil", "Salt & pepper", "Parmesan"],
        steps: &["Boil salted water and cook spaghetti until al dente.",
                 "Sauté diced onion and garlic in olive oil for 3 minutes.",
                 "Add ground beef and brown for 5 minutes.",
                 "Add crushed tomatoes, season with salt and pepper.",
                 "Simmer for 20 minutes on low heat.",
                 "Serve sauce over drained pasta with Parmesan."],
    },
    Recipe {
        name: "Chicken Stir Fry",
        prep: "20 min",
        servings: "2",
        ingredients: &["300g chicken breast", "2 bell peppers", "1 onion", "3 tbsp soy sauce",
                       "1 tbsp sesame oil", "2 garlic cloves", "Rice for serving"],
        steps: &["Slice chicken and vegetables into thin strips.",
                 "Heat sesame oil in a wok over high heat.",
                 "Stir fry chicken until golden, about 5 minutes.",
                 "Add vegetables and garlic, stir fry 3 more minutes.",
                 "Pour in soy sauce and toss to coat.",
                 "Serve over steamed rice."],
    },
    Recipe {
        name: "Avocado Toast",
        prep: "10 min",
        servings: "2",
        ingredients: &["4 slices sourdough", "2 ripe avocados", "1 lemon", "Red pepper flakes",
                       "Salt & pepper", "2 eggs (optional)", "Olive oil"],
        steps: &["Toast the bread until golden and crispy.",
                 "Mash avocados with lemon juice, salt and pepper.",
                 "Spread avocado mixture on toast.",
                 "Top with red pepper flakes and a drizzle of olive oil.",
                 "Optionally add a poached or fried egg on top."],
    },
    Recipe {
        name: "Vegetable Curry",
        prep: "35 min",
        servings: "4",
        ingredients: &["2 potatoes", "1 cauliflower", "400ml coconut milk", "2 tbsp curry powder",
                       "1 onion", "3 garlic cloves", "400g chickpeas", "Fresh coriander"],
        steps: &["Dice potatoes and cut cauliflower into florets.",
                 "Sauté onion and garlic until soft.",
                 "Add curry powder and cook 1 minute until fragrant.",
                 "Add vegetables, chickpeas and coconut milk.",
                 "Simmer 25 minutes until potatoes are tender.",
                 "Garnish with fresh coriander and serve with rice."],
    },
    Recipe {
        name: "Pancakes",
        prep: "15 min",
        servings: "3",
        ingredients: &["200g flour", "2 eggs", "300ml milk", "2 tbsp butter", "1 tsp baking powder",
                       "2 tbsp sugar", "Pinch of salt", "Maple syrup to serve"],
        steps: &["Mix flour, sugar, baking powder and salt in a bowl.",
                 "Whisk eggs and milk together, add melted butter.",
                 "Combine wet and dry ingredients until just smooth.",
                 "Heat a lightly oiled pan over medium heat.",
                 "Pour ¼ cup batter per pancake, cook until bubbles form.",
                 "Flip and cook 1 more minute. Serve with maple syrup."],
    },
    Recipe {
        name: "Greek Salad",
        prep: "10 min",
        servings: "2",
        ingredients: &["3 tomatoes", "1 cucumber", "1 red onion", "200g feta cheese",
                       "80g olives", "3 tbsp olive oil", "1 tbsp red wine vinegar", "Oregano"],
        steps: &["Chop tomatoes, cucumber and onion into chunks.",
                 "Combine vegetables in a large bowl.",
                 "Add olives and crumbled feta cheese.",
                 "Drizzle olive oil and vinegar over the salad.",
                 "Season with oregano, salt and pepper.",
                 "Toss gently and serve immediately."],
    },
    Recipe {
        name: "Banana Smoothie",
        prep: "5 min",
        servings: "1",
        ingredients: &["2 bananas", "200ml milk", "2 tbsp honey", "1 tsp vanilla extract",
                       "Ice cubes", "Pinch of cinnamon"],
        steps: &["Peel and slice bananas.",
                 "Add all ingredients to a blender.",
                 "Blend on high until smooth and creamy.",
                 "Adjust sweetness with more honey if needed.",
                 "Pour into a glass and sprinkle cinnamon on top."],
    },
    Recipe {
        name: "Tomato Soup",
        prep: "25 min",
        servings: "4",
        ingredients: &["800g canned tomatoes", "1 onion", "3 garlic cloves", "500ml vegetable stock",
                       "2 tbsp olive oil", "1 tsp sugar", "Fresh basil", "Cream to finish"],
        steps: &["Sauté diced onion and garlic in olive oil until soft.",
                 "Add canned tomatoes and vegetable stock.",
                 "Season with sugar, salt and pepper.",
                 "Simmer for 15 minutes.",
                 "Blend until smooth using an immersion blender.",
                 "Serve with a swirl of cream and fresh basil."],
    },
    Recipe {
        name: "Omelette",
        prep: "10 min",
        servings: "1",
        ingredients: &["3 eggs", "1 tbsp butter", "30g cheese", "2 mushrooms",
                       "Salt & pepper", "Fresh herbs (chives, parsley)"],
        steps: &["Beat eggs with salt and pepper until combined.",
                 "Heat butter in a non-stick pan over medium heat.",
                 "Sauté sliced mushrooms for 2 minutes, remove.",
                 "Pour egg mixture into pan and let it set slightly.",
                 "Add mushrooms and cheese to one half.",
                 "Fold omelette in half and slide onto plate. Top with herbs."],
    },
    Recipe {
        name: "Chocolate Brownies",
        prep: "40 min",
        servings: "16",
        ingredients: &["200g dark chocolate", "150g butter", "200g sugar", "3 eggs",
                       "100g flour", "2 tbsp cocoa powder", "1 tsp vanilla", "Pinch of salt"],
        steps: &["Preheat oven to 180C. Grease a square baking tin.",
                 "Melt chocolate and butter together over low heat.",
                 "Whisk in sugar, then eggs one at a time.",
                 "Stir in vanilla, flour, cocoa and salt.",
                 "Pour into tin and bake 25-30 minutes.",
                 "Cool completely before cutting into squares."],
    },
];

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

const LIST_W: u32 = 240;
const LIST_X: u32 = 12;
const DETAIL_X: u32 = LIST_X + LIST_W + 12;
const DETAIL_W: u32 = W - DETAIL_X - 12;
const CONTENT_Y: u32 = HEADER_H + 8;
const ROW_H: u32 = 36;

struct App {
    sel:         usize,
    scroll:      usize,
    search:      [u8; 20],
    search_len:  usize,
    filtered:    Vec<usize>,
}

impl App {
    fn new() -> Self {
        let mut a = App {
            sel: 0,
            scroll: 0,
            search: [0; 20],
            search_len: 0,
            filtered: Vec::new(),
        };
        a.rebuild_filter();
        a
    }

    fn rebuild_filter(&mut self) {
        let q = std::str::from_utf8(&self.search[..self.search_len]).unwrap_or("").to_lowercase();
        self.filtered = (0..RECIPES.len())
            .filter(|&i| q.is_empty() || RECIPES[i].name.to_lowercase().contains(&q))
            .collect();
        if self.sel >= self.filtered.len() && !self.filtered.is_empty() {
            self.sel = self.filtered.len() - 1;
        }
        self.scroll = 0;
    }

    fn recipe(&self) -> Option<&Recipe> {
        self.filtered.get(self.sel).map(|&i| &RECIPES[i])
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Recipe Browser");
    let q = std::str::from_utf8(&app.search[..app.search_len]).unwrap_or("");
    if app.search_len > 0 {
        text(180, 16, C_SEL, &format!("Search: {}|", q));
    } else {
        text(180, 16, C_HINT, "Type to search");
    }
    text(560, 16, C_HINT, "↑↓:nav  Bsp:clear search");

    // Left panel — recipe list
    fill(LIST_X, CONTENT_Y, LIST_W, H - CONTENT_Y, C_CARD);
    border(LIST_X, CONTENT_Y, LIST_W, H - CONTENT_Y, C_BORDER);

    let visible = ((H - CONTENT_Y) / ROW_H) as usize;
    for (i, &ri) in app.filtered.iter().enumerate().skip(app.scroll).take(visible) {
        let ry = CONTENT_Y + (i - app.scroll) as u32 * ROW_H;
        let is_sel = i == app.sel;
        fill(LIST_X + 1, ry, LIST_W - 2, ROW_H - 1, if is_sel { C_SEL_BG } else { C_CARD });
        let tc = if is_sel { C_TEXT } else { C_HINT };
        let name = RECIPES[ri].name;
        let short = if name.len() > 24 { &name[..24] } else { name };
        text(LIST_X + 8, ry + 10, tc, short);
    }

    // Right panel — recipe detail
    if let Some(r) = app.recipe() {
        fill(DETAIL_X, CONTENT_Y, DETAIL_W, H - CONTENT_Y, C_CARD);
        border(DETAIL_X, CONTENT_Y, DETAIL_W, H - CONTENT_Y, C_BORDER);

        let mut dy = CONTENT_Y + 16;
        text(DETAIL_X + 16, dy, C_TEXT, r.name);
        dy += 24;
        text(DETAIL_X + 16, dy, C_HINT, &format!("Prep: {}  |  Servings: {}", r.prep, r.servings));
        dy += 20;
        fill(DETAIL_X + 16, dy, DETAIL_W - 32, 1, C_BORDER);
        dy += 10;

        // Ingredients
        text(DETAIL_X + 16, dy, C_ORANGE, "Ingredients");
        dy += 20;
        for &ing in r.ingredients {
            text(DETAIL_X + 24, dy, C_GREEN, "•");
            text(DETAIL_X + 36, dy, C_TEXT, ing);
            dy += 18;
        }

        dy += 8;
        fill(DETAIL_X + 16, dy, DETAIL_W - 32, 1, C_BORDER);
        dy += 10;

        // Steps
        text(DETAIL_X + 16, dy, C_ORANGE, "Steps");
        dy += 20;
        for (i, &step) in r.steps.iter().enumerate().skip(app.scroll) {
            if dy + 20 > H - 12 { break; }
            let num = format!("{}.", i + 1);
            text(DETAIL_X + 16, dy, C_SEL, &num);
            // Word wrap step text at ~55 chars
            let chars_per_line = 55usize;
            let words: Vec<&str> = step.split_whitespace().collect();
            let mut line = String::new();
            let mut first = true;
            for word in &words {
                if line.len() + word.len() + 1 > chars_per_line {
                    let lx = if first { DETAIL_X + 36 } else { DETAIL_X + 36 };
                    text(lx, dy, C_HINT, &line);
                    dy += 18;
                    line.clear();
                    first = false;
                }
                if !line.is_empty() { line.push(' '); }
                line.push_str(word);
            }
            if !line.is_empty() {
                text(DETAIL_X + 36, dy, C_HINT, &line);
                dy += 18;
            }
            dy += 4;
        }
    } else {
        text(DETAIL_X + 80, H / 2, C_HINT, "No recipes match search");
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise recipe");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => {
                if app.sel > 0 { app.sel -= 1; app.scroll = 0; }
            }
            "\x1b[B" => {
                if app.sel + 1 < app.filtered.len() { app.sel += 1; app.scroll = 0; }
            }
            "\x7f" => {
                if app.search_len > 0 { app.search_len -= 1; app.rebuild_filter(); }
            }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= 0x20 && b < 0x7F && app.search_len < 20 {
                    app.search[app.search_len] = b;
                    app.search_len += 1;
                    app.rebuild_filter();
                }
            }
            _ => {}
        }
        draw(&app);
    }
}
