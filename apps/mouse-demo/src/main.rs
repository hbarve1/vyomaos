use std::io::{self, BufRead};

const BG:    u32 = 0x1E1E2EFF;
const WHITE: u32 = 0xFFFFFFFF;
const DIM:   u32 = 0x6C7086FF;
const GREEN: u32 = 0xA6E3A1FF;
const PINK:  u32 = 0xF38BA8FF;

struct State {
    last_move:  String,
    last_click: String,
    move_count: u32,
    click_count: u32,
}

fn draw_frame(s: &State) {
    println!("VYOMA_DRAW:fill_rect:0,0,480,400,{BG}");
    println!("VYOMA_DRAW:draw_text:8,8,{WHITE},m,mouse-demo");
    println!("VYOMA_DRAW:draw_text:8,32,{DIM},m,Move events: {}", s.move_count);
    if !s.last_move.is_empty() {
        println!("VYOMA_DRAW:draw_text:8,56,{WHITE},m,pos {}", s.last_move);
    }
    println!("VYOMA_DRAW:draw_text:8,88,{DIM},m,Click events: {}", s.click_count);
    if !s.last_click.is_empty() {
        println!("VYOMA_DRAW:draw_text:8,112,{PINK},m,click {}", s.last_click);
    }
    println!("VYOMA_DRAW:draw_text:8,144,{GREEN},m,Move cursor or click to test");
    println!("VYOMA_DRAW:flush");
}

fn main() {
    eprintln!("mouse-demo started");
    let mut state = State { last_move: String::new(), last_click: String::new(), move_count: 0, click_count: 0 };
    draw_frame(&state);

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        if let Some(rest) = line.strip_prefix("VYOMA_INPUT:mouse:") {
            if let Some(coords) = rest.strip_prefix("move:") {
                state.move_count += 1;
                state.last_move = coords.to_string();
            } else if let Some(payload) = rest.strip_prefix("click:") {
                state.click_count += 1;
                state.last_click = payload.to_string();
            }
            draw_frame(&state);
        }
    }
}
