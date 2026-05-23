use std::io::{self, BufRead, Write};

const W: u32 = 1100;
const H: u32 = 720;
const HDR: u32 = 40;
const SB_H: u32 = 28;
const DAY_HDR_H: u32 = 28;
const GRID_Y: u32 = HDR + DAY_HDR_H;
const GRID_H: u32 = H - GRID_Y - SB_H;
const MEAL_H: u32 = GRID_H / 3;
const LEFT_W: u32 = 700;
const RIGHT_X: u32 = LEFT_W + 1;
const RIGHT_W: u32 = W - RIGHT_X;
const DAY_W: u32 = LEFT_W / 7;

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
const C_PURPLE: u32 = 0xBC8CFFFF;
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

// (name, ingredients..., prep_mins, calories)
struct Recipe {
    name:  &'static str,
    ingr:  &'static [&'static str],
    prep:  u32,
    kcal:  u32,
    color: u32,
}

const BREAKFAST: &[Recipe] = &[
    Recipe { name: "Oatmeal & Berries",      ingr: &["oats","milk","blueberries","honey"],             prep: 10, kcal: 320, color: C_ORANGE },
    Recipe { name: "Avocado Toast",           ingr: &["bread","avocado","egg","salt","lemon"],           prep: 8,  kcal: 380, color: C_GREEN  },
    Recipe { name: "Greek Yogurt Parfait",    ingr: &["yogurt","granola","strawberries","honey"],        prep: 5,  kcal: 290, color: C_YELLOW },
    Recipe { name: "Scrambled Eggs",          ingr: &["eggs","butter","chives","salt","pepper"],         prep: 10, kcal: 280, color: C_ORANGE },
    Recipe { name: "Pancakes",                ingr: &["flour","eggs","milk","butter","maple syrup"],     prep: 20, kcal: 420, color: C_YELLOW },
    Recipe { name: "Smoothie Bowl",           ingr: &["banana","spinach","almond milk","chia seeds"],    prep: 8,  kcal: 310, color: C_GREEN  },
    Recipe { name: "French Toast",            ingr: &["bread","eggs","milk","cinnamon","vanilla"],       prep: 15, kcal: 390, color: C_ORANGE },
    Recipe { name: "Breakfast Burrito",       ingr: &["eggs","tortilla","cheese","beans","salsa"],       prep: 15, kcal: 460, color: C_RED    },
    Recipe { name: "Fruit Salad",             ingr: &["apple","orange","grapes","kiwi","mint"],          prep: 10, kcal: 180, color: C_GREEN  },
    Recipe { name: "Bagel & Cream Cheese",    ingr: &["bagel","cream cheese","lox","capers"],            prep: 5,  kcal: 400, color: C_YELLOW },
    Recipe { name: "Acai Bowl",               ingr: &["acai","banana","granola","coconut","honey"],      prep: 10, kcal: 350, color: C_PURPLE },
    Recipe { name: "Waffles",                 ingr: &["flour","eggs","butter","milk","vanilla"],         prep: 20, kcal: 410, color: C_YELLOW },
    Recipe { name: "Egg Muffins",             ingr: &["eggs","spinach","bell pepper","cheese"],          prep: 25, kcal: 240, color: C_ORANGE },
    Recipe { name: "Overnight Oats",          ingr: &["oats","milk","chia","banana","honey"],            prep: 5,  kcal: 360, color: C_SEL    },
    Recipe { name: "PB Banana Toast",         ingr: &["bread","peanut butter","banana","honey"],         prep: 5,  kcal: 380, color: C_YELLOW },
    Recipe { name: "Hash Browns",             ingr: &["potato","onion","butter","salt","pepper"],        prep: 20, kcal: 340, color: C_ORANGE },
    Recipe { name: "Chia Pudding",            ingr: &["chia seeds","coconut milk","mango","lime"],       prep: 5,  kcal: 290, color: C_GREEN  },
    Recipe { name: "Crepes",                  ingr: &["flour","eggs","milk","butter","jam"],             prep: 25, kcal: 380, color: C_YELLOW },
    Recipe { name: "Baked Oatmeal",           ingr: &["oats","banana","eggs","milk","blueberries"],      prep: 30, kcal: 320, color: C_ORANGE },
    Recipe { name: "Veggie Omelette",         ingr: &["eggs","mushrooms","onion","tomato","cheese"],     prep: 12, kcal: 300, color: C_GREEN  },
];

const LUNCH: &[Recipe] = &[
    Recipe { name: "Caesar Salad",            ingr: &["romaine","croutons","parmesan","caesar dressing"], prep: 15, kcal: 350, color: C_GREEN  },
    Recipe { name: "Turkey Sandwich",         ingr: &["turkey","lettuce","tomato","mayo","bread"],        prep: 10, kcal: 420, color: C_YELLOW },
    Recipe { name: "Tomato Soup + Grilled Cheese", ingr: &["tomato","cream","bread","cheddar","butter"], prep: 25, kcal: 480, color: C_RED    },
    Recipe { name: "Greek Salad",             ingr: &["cucumber","tomato","feta","olives","olive oil"],   prep: 10, kcal: 280, color: C_GREEN  },
    Recipe { name: "Chicken Wrap",            ingr: &["chicken","lettuce","tomato","hummus","tortilla"],  prep: 15, kcal: 450, color: C_ORANGE },
    Recipe { name: "Quinoa Bowl",             ingr: &["quinoa","avocado","corn","black beans","lime"],    prep: 20, kcal: 390, color: C_GREEN  },
    Recipe { name: "BLT Sandwich",            ingr: &["bacon","lettuce","tomato","mayo","toast"],         prep: 15, kcal: 470, color: C_RED    },
    Recipe { name: "Lentil Soup",             ingr: &["lentils","carrot","celery","onion","cumin"],       prep: 40, kcal: 310, color: C_ORANGE },
    Recipe { name: "Caprese Salad",           ingr: &["mozzarella","tomato","basil","balsamic","oil"],    prep: 8,  kcal: 320, color: C_GREEN  },
    Recipe { name: "Tuna Melt",               ingr: &["tuna","mayo","cheddar","bread","celery"],          prep: 15, kcal: 430, color: C_YELLOW },
    Recipe { name: "Veggie Stir-fry",         ingr: &["broccoli","bell pepper","tofu","soy sauce","rice"], prep: 20, kcal: 360, color: C_GREEN },
    Recipe { name: "Chicken Noodle Soup",     ingr: &["chicken","noodles","carrot","celery","broth"],     prep: 35, kcal: 340, color: C_ORANGE },
    Recipe { name: "Egg Salad Sandwich",      ingr: &["eggs","mayo","mustard","celery","bread"],          prep: 15, kcal: 400, color: C_YELLOW },
    Recipe { name: "Falafel Wrap",            ingr: &["falafel","hummus","tomato","lettuce","pita"],      prep: 20, kcal: 440, color: C_ORANGE },
    Recipe { name: "Pesto Pasta",             ingr: &["pasta","pesto","cherry tomatoes","parmesan"],      prep: 20, kcal: 480, color: C_GREEN  },
    Recipe { name: "Asian Noodle Salad",      ingr: &["noodles","edamame","sesame oil","ginger","soy"],   prep: 20, kcal: 390, color: C_YELLOW },
    Recipe { name: "Avocado Egg Toast",       ingr: &["avocado","eggs","toast","red pepper flakes"],      prep: 15, kcal: 430, color: C_GREEN  },
    Recipe { name: "Minestrone Soup",         ingr: &["pasta","beans","zucchini","tomato","basil"],       prep: 45, kcal: 290, color: C_RED    },
    Recipe { name: "Club Sandwich",           ingr: &["turkey","bacon","lettuce","tomato","swiss","bread"], prep: 15, kcal: 550, color: C_YELLOW },
    Recipe { name: "Grain Bowl",              ingr: &["farro","roasted veggies","tahini","lemon"],        prep: 30, kcal: 420, color: C_SEL    },
];

const DINNER: &[Recipe] = &[
    Recipe { name: "Spaghetti Bolognese",     ingr: &["pasta","beef","tomato sauce","parmesan","onion"],  prep: 40, kcal: 580, color: C_RED    },
    Recipe { name: "Grilled Salmon",          ingr: &["salmon","lemon","dill","olive oil","asparagus"],   prep: 25, kcal: 490, color: C_ORANGE },
    Recipe { name: "Chicken Stir-fry",        ingr: &["chicken","broccoli","soy sauce","ginger","rice"],  prep: 25, kcal: 520, color: C_YELLOW },
    Recipe { name: "Beef Tacos",              ingr: &["beef","taco shells","cheese","salsa","avocado"],   prep: 30, kcal: 560, color: C_RED    },
    Recipe { name: "Veggie Curry",            ingr: &["chickpeas","spinach","coconut milk","curry"],      prep: 35, kcal: 440, color: C_ORANGE },
    Recipe { name: "Roast Chicken",           ingr: &["chicken","garlic","rosemary","lemon","butter"],    prep: 90, kcal: 620, color: C_YELLOW },
    Recipe { name: "Shrimp Pasta",            ingr: &["shrimp","pasta","garlic","cherry tomatoes","basil"], prep: 25, kcal: 510, color: C_ORANGE },
    Recipe { name: "Veggie Lasagna",          ingr: &["lasagna","ricotta","spinach","tomato sauce","mozzarella"], prep: 60, kcal: 520, color: C_GREEN },
    Recipe { name: "Thai Green Curry",        ingr: &["chicken","green curry paste","coconut milk","rice"], prep: 30, kcal: 530, color: C_GREEN },
    Recipe { name: "Beef Burgers",            ingr: &["beef patty","bun","lettuce","tomato","cheddar"],   prep: 25, kcal: 620, color: C_RED    },
    Recipe { name: "Mushroom Risotto",        ingr: &["arborio","mushrooms","parmesan","white wine"],     prep: 45, kcal: 490, color: C_YELLOW },
    Recipe { name: "Grilled Tuna Steak",      ingr: &["tuna","soy sauce","ginger","sesame","edamame"],    prep: 20, kcal: 460, color: C_SEL    },
    Recipe { name: "Eggplant Parmesan",       ingr: &["eggplant","tomato sauce","mozzarella","parmesan"], prep: 50, kcal: 480, color: C_ORANGE },
    Recipe { name: "Chicken Tikka Masala",    ingr: &["chicken","tikka sauce","cream","rice","naan"],     prep: 40, kcal: 590, color: C_ORANGE },
    Recipe { name: "Pork Tenderloin",         ingr: &["pork","apple","thyme","mustard","potatoes"],       prep: 45, kcal: 550, color: C_YELLOW },
    Recipe { name: "Seafood Paella",          ingr: &["rice","shrimp","mussels","saffron","bell pepper"], prep: 60, kcal: 580, color: C_ORANGE },
    Recipe { name: "Cauliflower Steak",       ingr: &["cauliflower","tahini","pomegranate","herbs"],      prep: 30, kcal: 360, color: C_GREEN  },
    Recipe { name: "BBQ Pulled Pork",         ingr: &["pork","BBQ sauce","coleslaw","buns","pickle"],     prep: 120,kcal: 610, color: C_RED    },
    Recipe { name: "Lamb Stew",               ingr: &["lamb","potato","carrot","rosemary","red wine"],    prep: 90, kcal: 640, color: C_RED    },
    Recipe { name: "Tofu Pad Thai",           ingr: &["tofu","rice noodles","bean sprouts","tamarind","peanuts"], prep: 30, kcal: 500, color: C_YELLOW },
];

const DAYS: &[&str] = &["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MEALS: &[&str] = &["Breakfast", "Lunch", "Dinner"];

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

fn meal_recipes(m: usize) -> &'static [Recipe] {
    match m { 0 => BREAKFAST, 1 => LUNCH, _ => DINNER }
}

#[derive(PartialEq)]
enum View { Week, Shopping }

struct App {
    week:     [[Option<usize>; 3]; 7],
    sel_day:  usize,
    sel_meal: usize,
    view:     View,
    seed:     u64,
    status:   String,
}

impl App {
    fn new() -> Self {
        let mut app = App {
            week: [[None; 3]; 7],
            sel_day: 0, sel_meal: 0,
            view: View::Week,
            seed: 0xFEED_1234_5678_9ABCu64,
            status: String::from("←→=day  ↑↓=meal  Enter=assign  R=randomize  C=clear  S/Tab=shopping list  Ctrl+C=quit"),
        };
        app.randomize();
        app
    }

    fn randomize(&mut self) {
        for d in 0..7 {
            for m in 0..3 {
                self.seed = lcg(self.seed);
                let count = meal_recipes(m).len();
                self.week[d][m] = Some(self.seed as usize % count);
            }
        }
    }

    fn shopping_list(&self) -> Vec<String> {
        let mut ingr: Vec<&'static str> = Vec::new();
        for d in 0..7 {
            for m in 0..3 {
                if let Some(ri) = self.week[d][m] {
                    for &i in meal_recipes(m)[ri].ingr {
                        if !ingr.contains(&i) { ingr.push(i); }
                    }
                }
            }
        }
        ingr.sort_unstable();
        ingr.iter().map(|s| format!("• {}", s)).collect()
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HDR, C_HEADER);
    fill(0, HDR - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Recipe Planner");
    text(190, 12, C_HINT, "Weekly meal plan with shopping list");
    if app.view == View::Shopping {
        text(W - 200, 12, C_YELLOW, "[Tab=back to week]");
    }

    if app.view == View::Shopping {
        draw_shopping(app);
    } else {
        draw_week(app);
    }

    let sb_y = H - SB_H;
    fill(0, sb_y, W, SB_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
}

fn draw_shopping(app: &App) {
    let items = app.shopping_list();
    let col1 = items.len() / 2 + items.len() % 2;
    text(16, HDR + 8, C_SEL, &format!("Shopping List — {} ingredients:", items.len()));
    fill(16, HDR + 24, W - 32, 1, C_BORDER);

    for (i, item) in items.iter().enumerate() {
        let col = if i < col1 { 0 } else { 1 };
        let row = if i < col1 { i } else { i - col1 };
        let x = 24 + col as u32 * (W / 2 - 16);
        let y = HDR + 32 + row as u32 * 18;
        if y < H - SB_H - 4 {
            text(x, y, C_TEXT, item);
        }
    }

    // Meal counts
    let assigned: usize = (0..7).flat_map(|d| (0..3).filter(move |&m| app.week[d][m].is_some())).count();
    text(16, H - SB_H - 24, C_HINT, &format!("{}/21 meals planned this week", assigned));
}

fn draw_week(app: &App) {
    // Day header row
    fill(0, HDR, LEFT_W, DAY_HDR_H, C_CARD);
    for (d, &day) in DAYS.iter().enumerate() {
        let dx = d as u32 * DAY_W;
        let is_sel = d == app.sel_day;
        if is_sel { fill(dx, HDR, DAY_W, DAY_HDR_H, C_HEADER); }
        text(dx + DAY_W / 2 - 12, HDR + 6, if is_sel { C_SEL } else { C_HINT }, day);
        fill(dx + DAY_W - 1, HDR, 1, DAY_HDR_H, C_BORDER);
    }
    fill(0, HDR + DAY_HDR_H - 1, LEFT_W, 1, C_BORDER);

    // Meal label column + grid
    for m in 0..3usize {
        let my = GRID_Y + m as u32 * MEAL_H;
        // Meal type background stripe
        fill(0, my, LEFT_W, MEAL_H, if m % 2 == 0 { C_BG } else { 0x0A0E14FF });
        fill(0, my + MEAL_H - 1, LEFT_W, 1, C_BORDER);

        // Meal label on very left
        text(2, my + MEAL_H / 2 - 8, C_HINT, &MEALS[m][..1]);

        for d in 0..7usize {
            let dx = d as u32 * DAY_W;
            let is_sel = d == app.sel_day && m == app.sel_meal;

            if is_sel { fill(dx, my, DAY_W - 1, MEAL_H - 1, 0x1C2430FF); }

            if let Some(ri) = app.week[d][m] {
                let r = &meal_recipes(m)[ri];
                let col = r.color;
                // Colored indicator bar
                fill(dx, my, 3, MEAL_H - 1, col);
                // Name (truncated)
                let max_c = ((DAY_W - 8) / 8) as usize;
                let shown = if r.name.len() > max_c { &r.name[..max_c] } else { r.name };
                text(dx + 5, my + 4, if is_sel { C_TEXT } else { C_HINT }, shown);
                // kcal
                text(dx + 5, my + MEAL_H - 16, col, &format!("{}kcal", r.kcal));
            } else {
                text(dx + 5, my + MEAL_H / 2 - 6, C_BORDER, "empty");
            }

            // Vertical separator
            fill(dx + DAY_W - 1, my, 1, MEAL_H, C_BORDER);

            if is_sel { border(dx, my, DAY_W - 1, MEAL_H - 1, C_SEL); }
        }
    }

    // Right panel: recipe detail
    fill(RIGHT_X, HDR, RIGHT_W, H - HDR - SB_H, C_CARD);
    fill(RIGHT_X, HDR, 1, H - HDR - SB_H, C_BORDER);

    let d = app.sel_day;
    let m = app.sel_meal;
    let rx = RIGHT_X + 8;
    let mut ry = HDR + 8;

    text(rx, ry, C_HINT, &format!("{} — {}", DAYS[d], MEALS[m]));
    ry += 20;

    if let Some(ri) = app.week[d][m] {
        let r = &meal_recipes(m)[ri];
        text(rx, ry, r.color, r.name);
        ry += 20;
        fill(rx, ry, RIGHT_W - 16, 1, C_BORDER);
        ry += 8;
        text(rx, ry, C_HINT, &format!("Prep: {} min   Calories: {}", r.prep, r.kcal));
        ry += 20;
        text(rx, ry, C_HINT, "Ingredients:");
        ry += 16;
        for &ingr in r.ingr {
            text(rx + 8, ry, C_TEXT, &format!("• {}", ingr));
            ry += 16;
        }
    } else {
        text(rx, ry, C_BORDER, "(no recipe assigned)");
        ry += 20;
        text(rx, ry, C_HINT, "Press Enter to assign");
    }

    // Quick stats
    let assigned: usize = (0..7).flat_map(|d2| (0..3).filter(move |&m2| app.week[d2][m2].is_some())).count();
    let total_kcal: u32 = (0..7).flat_map(|d2| (0..3).filter_map(move |m2|
        app.week[d2][m2].map(|ri| meal_recipes(m2)[ri].kcal)
    )).sum();

    let sy = H - SB_H - 70;
    fill(RIGHT_X + 8, sy, RIGHT_W - 16, 1, C_BORDER);
    text(rx, sy + 6, C_HINT, &format!("Meals: {}/21", assigned));
    text(rx, sy + 22, C_HINT, &format!("Weekly: ~{}kcal", total_kcal));
    text(rx, sy + 38, C_HINT, &format!("Daily avg: ~{}kcal", total_kcal / 7));
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise recipe-planner");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }

            "\x1b[D" => { if app.sel_day > 0 { app.sel_day -= 1; } }
            "\x1b[C" => { if app.sel_day < 6 { app.sel_day += 1; } }
            "\x1b[A" => { if app.sel_meal > 0 { app.sel_meal -= 1; } }
            "\x1b[B" => { if app.sel_meal < 2 { app.sel_meal += 1; } }

            "" => {
                // Assign random recipe
                app.seed = lcg(app.seed);
                let count = meal_recipes(app.sel_meal).len();
                let ri = app.seed as usize % count;
                app.week[app.sel_day][app.sel_meal] = Some(ri);
                let r = &meal_recipes(app.sel_meal)[ri];
                app.status = format!("Assigned: {}", r.name);
            }

            "c" | "C" => {
                app.week[app.sel_day][app.sel_meal] = None;
                app.status = format!("Cleared {} {}", DAYS[app.sel_day], MEALS[app.sel_meal]);
            }

            "r" | "R" => {
                app.randomize();
                app.status = "Full week randomized.".to_string();
            }

            "s" | "S" | "\t" => {
                app.view = if app.view == View::Shopping { View::Week } else { View::Shopping };
            }

            _ => {}
        }

        draw(&app);
    }
}
