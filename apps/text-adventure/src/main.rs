use std::io::{self, BufRead, Write};

const W: u32 = 1100;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const INPUT_H: u32 = 36;
const LINE_H: u32 = 18;
const CHAR_W: u32 = 8;
const CONTENT_Y: u32 = HEADER_H;
const INPUT_Y: u32 = H - INPUT_H;
const VISIBLE_LINES: usize = ((INPUT_Y - CONTENT_Y) / LINE_H) as usize;
const MAX_HISTORY: usize = 300;

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
const C_YELLOW: u32 = 0xD29922FF;

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

// ── World data ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Dir { N, S, E, W }

struct Room {
    name:  &'static str,
    desc:  &'static str,
    exits: &'static [(Dir, usize)], // (direction, room_id)
}

const ROOMS: &[Room] = &[
    Room { // 0: Entrance Hall
        name: "Entrance Hall",
        desc: "A grand hall with stone floors. Torches flicker on the walls. Exits lead north and east.",
        exits: &[(Dir::N, 1), (Dir::E, 2)],
    },
    Room { // 1: Library
        name: "Library",
        desc: "Rows of dusty books line the shelves. A faint smell of old parchment fills the air.",
        exits: &[(Dir::S, 0), (Dir::E, 3)],
    },
    Room { // 2: Armory
        name: "Armory",
        desc: "Weapon racks line the walls, most empty. A locked door blocks passage to the east.",
        exits: &[(Dir::W, 0), (Dir::N, 3)],
    },
    Room { // 3: Guard Room
        name: "Guard Room",
        desc: "A small room with a table and two chairs. A sleeping guard sits slumped in the corner.",
        exits: &[(Dir::W, 1), (Dir::S, 2), (Dir::N, 4)],
    },
    Room { // 4: Courtyard
        name: "Courtyard",
        desc: "An open stone courtyard under a grey sky. A fountain stands dry in the center.",
        exits: &[(Dir::S, 3), (Dir::N, 5), (Dir::E, 6)],
    },
    Room { // 5: Chapel
        name: "Chapel",
        desc: "A small chapel with a stone altar and stained glass. A sense of calm pervades.",
        exits: &[(Dir::S, 4)],
    },
    Room { // 6: Dungeon Stairs
        name: "Dungeon Stairs",
        desc: "Stone steps descend into darkness. You can hear water dripping far below.",
        exits: &[(Dir::W, 4), (Dir::N, 7)],
    },
    Room { // 7: Dungeon Cell
        name: "Dungeon Cell",
        desc: "A damp cell with iron bars. A rusted drain marks the floor. Something glints in the corner.",
        exits: &[(Dir::S, 6), (Dir::E, 8)],
    },
    Room { // 8: Monster Lair
        name: "Monster Lair",
        desc: "The air is thick and foul. A large shadow moves at the back of the cave.",
        exits: &[(Dir::W, 7), (Dir::N, 9)],
    },
    Room { // 9: Treasure Chamber
        name: "Treasure Chamber",
        desc: "Gold coins and jewels are piled high. You have found the treasure! The castle is yours.",
        exits: &[(Dir::S, 8)],
    },
];

#[derive(Clone, Copy, PartialEq)]
enum Item {
    Key, Torch, Sword, Potion, Map, Coin, Book, Candle,
}

impl Item {
    fn name(self) -> &'static str {
        match self {
            Item::Key    => "key",
            Item::Torch  => "torch",
            Item::Sword  => "sword",
            Item::Potion => "potion",
            Item::Map    => "map",
            Item::Coin   => "coin",
            Item::Book   => "book",
            Item::Candle => "candle",
        }
    }
    fn from_str(s: &str) -> Option<Item> {
        match s.trim().to_lowercase().as_str() {
            "key"    => Some(Item::Key),
            "torch"  => Some(Item::Torch),
            "sword"  => Some(Item::Sword),
            "potion" => Some(Item::Potion),
            "map"    => Some(Item::Map),
            "coin"   => Some(Item::Coin),
            "book"   => Some(Item::Book),
            "candle" => Some(Item::Candle),
            _ => None,
        }
    }
}

// Items placed in rooms initially
const ROOM_ITEMS: &[(usize, Item)] = &[
    (0, Item::Candle),
    (1, Item::Book),
    (2, Item::Key),
    (3, Item::Coin),
    (4, Item::Map),
    (5, Item::Potion),
    (7, Item::Torch),
    (8, Item::Sword),
];

struct World {
    room_items: Vec<Vec<Item>>, // items in each room
    inventory:  Vec<Item>,
    player_room: usize,
    monster_alive: bool,
    won: bool,
}

impl World {
    fn new() -> Self {
        let mut room_items: Vec<Vec<Item>> = (0..ROOMS.len()).map(|_| Vec::new()).collect();
        for &(r, item) in ROOM_ITEMS {
            room_items[r].push(item);
        }
        World {
            room_items,
            inventory: Vec::new(),
            player_room: 0,
            monster_alive: true,
            won: false,
        }
    }

    fn items_here(&self) -> &[Item] { &self.room_items[self.player_room] }

    fn look(&self) -> Vec<(&'static str, u32)> {
        let rm = &ROOMS[self.player_room];
        let mut lines = vec![
            (rm.name, C_ORANGE),
            (rm.desc, C_TEXT),
        ];
        // Exits
        let exit_strs: Vec<&str> = rm.exits.iter().map(|(d, _)| match d {
            Dir::N => "north", Dir::S => "south", Dir::E => "east", Dir::W => "west",
        }).collect();
        // We return static-lifetime strings so we need to leak or use a different approach
        // For simplicity, use a fixed buffer — we'll do this via the caller
        let _ = exit_strs;
        lines
    }
}

// ── Game ─────────────────────────────────────────────────────────────────────

struct Game {
    world:   World,
    history: Vec<(String, u32)>,
    input:   String,
    scroll:  usize,
}

impl Game {
    fn new() -> Self {
        let mut g = Game {
            world: World::new(),
            history: Vec::new(),
            input: String::new(),
            scroll: 0,
        };
        g.push("=== CASTLE QUEST ===", C_ORANGE);
        g.push("A text adventure. Find the treasure deep in the castle.", C_HINT);
        g.push("Commands: go/n/s/e/w  look  take <item>  drop <item>  use <item>  inventory/i  help  quit", C_HINT);
        g.push("", C_TEXT);
        g.look_room();
        g
    }

    fn push(&mut self, s: &str, color: u32) {
        if self.history.len() >= MAX_HISTORY { self.history.remove(0); }
        self.history.push((s.to_string(), color));
        self.scroll = 0;
    }

    fn look_room(&mut self) {
        let rm = &ROOMS[self.world.player_room];
        self.push(rm.name, C_ORANGE);
        self.push(rm.desc, C_TEXT);

        let items = self.world.items_here();
        if !items.is_empty() {
            let names: Vec<&str> = items.iter().map(|i| i.name()).collect();
            let s = format!("You see: {}.", names.join(", "));
            self.push(&s, C_GREEN);
        }

        let exits: Vec<&str> = ROOMS[self.world.player_room].exits.iter().map(|(d, _)| match d {
            Dir::N => "north", Dir::S => "south", Dir::E => "east", Dir::W => "west",
        }).collect();
        let s = format!("Exits: {}.", exits.join(", "));
        self.push(&s, C_HINT);

        if self.world.player_room == 8 && self.world.monster_alive {
            self.push("A monster blocks the path to the north! You need a weapon.", C_RED);
        }
        if self.world.player_room == 9 && !self.world.won {
            self.world.won = true;
            self.push("*** You found the Treasure Chamber! You win! ***", C_YELLOW);
        }
    }

    fn exec(&mut self, raw: &str) {
        let raw = raw.trim();
        if raw.is_empty() { return; }
        self.push(&format!("> {}", raw), C_HINT);

        let parts: Vec<&str> = raw.splitn(2, ' ').collect();
        let cmd = parts[0].to_lowercase();
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("").to_lowercase();

        match cmd.as_str() {
            "quit" | "exit" => {
                self.push("Farewell, adventurer.", C_HINT);
                draw(self);
                std::process::exit(0);
            }
            "help" => {
                self.push("Commands:", C_SEL);
                self.push("  go <dir> / n s e w — move", C_TEXT);
                self.push("  look / l            — describe room", C_TEXT);
                self.push("  take <item>         — pick up item", C_TEXT);
                self.push("  drop <item>         — drop item", C_TEXT);
                self.push("  use <item>          — use item", C_TEXT);
                self.push("  inventory / i       — list items", C_TEXT);
                self.push("  quit                — exit game", C_TEXT);
            }
            "look" | "l" => { self.look_room(); }
            "inventory" | "i" | "inv" => {
                if self.world.inventory.is_empty() {
                    self.push("You carry nothing.", C_HINT);
                } else {
                    let names: Vec<&str> = self.world.inventory.iter().map(|i| i.name()).collect();
                    self.push(&format!("Carrying: {}.", names.join(", ")), C_GREEN);
                }
            }
            "take" | "get" | "pick" => {
                if arg.is_empty() { self.push("Take what?", C_RED); return; }
                let item = Item::from_str(&arg);
                if let Some(it) = item {
                    let pos = self.world.room_items[self.world.player_room].iter().position(|&x| x == it);
                    if let Some(i) = pos {
                        self.world.room_items[self.world.player_room].remove(i);
                        self.world.inventory.push(it);
                        self.push(&format!("You pick up the {}.", it.name()), C_GREEN);
                    } else {
                        self.push(&format!("There is no {} here.", arg), C_RED);
                    }
                } else {
                    self.push(&format!("You don't see any {} here.", arg), C_RED);
                }
            }
            "drop" => {
                if arg.is_empty() { self.push("Drop what?", C_RED); return; }
                let item = Item::from_str(&arg);
                if let Some(it) = item {
                    let pos = self.world.inventory.iter().position(|&x| x == it);
                    if let Some(i) = pos {
                        self.world.inventory.remove(i);
                        self.world.room_items[self.world.player_room].push(it);
                        self.push(&format!("You drop the {}.", it.name()), C_HINT);
                    } else {
                        self.push(&format!("You don't have a {}.", arg), C_RED);
                    }
                } else {
                    self.push(&format!("You don't have a {}.", arg), C_RED);
                }
            }
            "use" => {
                if arg.is_empty() { self.push("Use what?", C_RED); return; }
                let item = Item::from_str(&arg);
                if let Some(it) = item {
                    if !self.world.inventory.contains(&it) {
                        self.push(&format!("You don't have a {}.", arg), C_RED);
                        return;
                    }
                    match it {
                        Item::Potion => {
                            let pos = self.world.inventory.iter().position(|&x| x == Item::Potion).unwrap();
                            self.world.inventory.remove(pos);
                            self.push("You drink the potion. A warm glow restores your strength!", C_GREEN);
                        }
                        Item::Key => {
                            if self.world.player_room == 2 {
                                self.push("You unlock the door to the east!", C_GREEN);
                                // Unlock room 2 → east → room 9 shortcut (add dynamic exit?)
                                // For simplicity, just narrate — exits are static
                                self.push("(The armory's east passage is now open — but you find it leads back to the courtyard.)", C_HINT);
                            } else {
                                self.push("You find no lock to use the key on here.", C_HINT);
                            }
                        }
                        Item::Sword => {
                            if self.world.player_room == 8 && self.world.monster_alive {
                                self.world.monster_alive = false;
                                self.push("You brandish the sword! The monster recoils and flees!", C_YELLOW);
                                self.push("The path to the north is clear.", C_GREEN);
                            } else {
                                self.push("You swipe the sword through the air. Nothing to fight here.", C_HINT);
                            }
                        }
                        Item::Torch | Item::Candle => {
                            self.push("You hold up the light. The shadows retreat slightly.", C_YELLOW);
                        }
                        Item::Map => {
                            self.push("The map shows a castle layout. You trace your path through the rooms.", C_TEXT);
                            self.push("Entrance→Library→Guard Room→Courtyard→Dungeon→Treasure Chamber.", C_HINT);
                        }
                        Item::Book => {
                            self.push("You open the book. It reads: 'The beast fears the blade. The door fears the key.'", C_TEXT);
                        }
                        Item::Coin => {
                            self.push("You flip the coin. It lands heads. Lucky!", C_HINT);
                        }
                    }
                } else {
                    self.push(&format!("You don't know how to use '{}'.", arg), C_RED);
                }
            }
            "go" | "move" | "walk" | "n" | "s" | "e" | "w" |
            "north" | "south" | "east" | "west" => {
                let dir_str = if cmd == "go" || cmd == "move" || cmd == "walk" { arg.as_str() } else { cmd.as_str() };
                let dir = match dir_str {
                    "n" | "north" => Some(Dir::N),
                    "s" | "south" => Some(Dir::S),
                    "e" | "east"  => Some(Dir::E),
                    "w" | "west"  => Some(Dir::W),
                    _ => None,
                };
                if let Some(d) = dir {
                    let rm = &ROOMS[self.world.player_room];
                    let dest = rm.exits.iter().find(|(ed, _)| *ed == d).map(|(_, id)| *id);
                    if let Some(next_room) = dest {
                        if next_room == 9 && self.world.player_room == 8 && self.world.monster_alive {
                            self.push("The monster blocks your path! Defeat it first.", C_RED);
                        } else {
                            self.world.player_room = next_room;
                            self.look_room();
                        }
                    } else {
                        self.push("You can't go that way.", C_RED);
                    }
                } else {
                    self.push("Go where? (n/s/e/w)", C_RED);
                }
            }
            _ => {
                self.push(&format!("Unknown command '{}'. Type 'help' for a list.", cmd), C_RED);
            }
        }
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Castle Quest");
    text(180, 16, C_HINT, "Text Adventure");
    text(430, 16, C_HINT, "help  quit  n/s/e/w  look  take/drop/use  i");

    let total = g.history.len();
    let scroll_clamped = g.scroll.min(total.saturating_sub(VISIBLE_LINES));
    let start = total.saturating_sub(VISIBLE_LINES + scroll_clamped);
    let end = (start + VISIBLE_LINES).min(total);
    let max_chars = (W as usize - 16) / CHAR_W as usize;

    for (i, idx) in (start..end).enumerate() {
        let (ref line, color) = g.history[idx];
        let y = CONTENT_Y + i as u32 * LINE_H + 2;
        let display = if line.len() > max_chars { &line[..max_chars] } else { line.as_str() };
        text(8, y, color, display);
    }

    if g.scroll > 0 {
        text(W - 120, CONTENT_Y + 4, C_HINT, &format!("[+{} lines]", g.scroll));
    }

    fill(0, INPUT_Y, W, INPUT_H, C_CARD);
    fill(0, INPUT_Y, W, 1, C_BORDER);
    let prompt = "> ";
    text(8, INPUT_Y + 10, C_GREEN, prompt);
    let cx = 8 + (prompt.len() + g.input.len()) as u32 * CHAR_W;
    text(8 + prompt.len() as u32 * CHAR_W, INPUT_Y + 10, C_TEXT, &g.input);
    fill(cx, INPUT_Y + 8, 2, LINE_H, C_SEL);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut game = Game::new();

    println!("@supervisor: raise text-adventure");
    let _ = io::stdout().flush();
    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b" => {}
            "\x1b[5~" => { game.scroll += VISIBLE_LINES / 2; }
            "\x1b[6~" => { game.scroll = game.scroll.saturating_sub(VISIBLE_LINES / 2); }
            "\x1b[H" => { game.scroll = MAX_HISTORY; }
            "\x1b[F" => { game.scroll = 0; }
            "\x7f" => { game.input.pop(); }
            "" => {
                let cmd = game.input.clone();
                game.input.clear();
                game.exec(&cmd);
            }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= 0x20 && b < 0x7f { game.input.push(b as char); }
            }
            _ => {}
        }
        draw(&game);
    }
}
