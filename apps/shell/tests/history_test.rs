// Tests for command history navigation.
// We define the logic as pure functions here to verify behavior before wiring into main.rs.

const HISTORY_MAX: usize = 50;

/// Push a command to history. Caps at HISTORY_MAX, dropping oldest.
fn push_history(history: &mut Vec<String>, cmd: &str) {
    if cmd.is_empty() { return; }
    // Avoid consecutive duplicates
    if history.last().map(|s| s.as_str()) == Some(cmd) { return; }
    if history.len() >= HISTORY_MAX {
        history.remove(0);
    }
    history.push(cmd.to_string());
}

/// Navigate history. idx is 0 = "past end" (empty/current), 1 = most recent, 2 = second most recent.
/// Returns the input to show, and the new idx.
fn history_up(history: &[String], current_input: &str, idx: usize) -> (String, usize) {
    if history.is_empty() { return (current_input.to_string(), idx); }
    let new_idx = (idx + 1).min(history.len());
    let entry = history[history.len() - new_idx].clone();
    (entry, new_idx)
}

fn history_down(history: &[String], saved_input: &str, idx: usize) -> (String, usize) {
    if idx == 0 { return (saved_input.to_string(), 0); }
    let new_idx = idx - 1;
    let entry = if new_idx == 0 {
        saved_input.to_string()
    } else {
        history[history.len() - new_idx].clone()
    };
    (entry, new_idx)
}

#[test]
fn push_adds_to_history() {
    let mut h = vec![];
    push_history(&mut h, "ps");
    push_history(&mut h, "ls");
    assert_eq!(h, vec!["ps", "ls"]);
}

#[test]
fn push_ignores_empty() {
    let mut h = vec![];
    push_history(&mut h, "");
    assert!(h.is_empty());
}

#[test]
fn push_ignores_consecutive_duplicate() {
    let mut h = vec![];
    push_history(&mut h, "ps");
    push_history(&mut h, "ps");
    assert_eq!(h.len(), 1);
}

#[test]
fn push_caps_at_50() {
    let mut h = vec![];
    for i in 0..55 {
        push_history(&mut h, &format!("cmd{i}"));
    }
    assert_eq!(h.len(), 50);
    assert_eq!(h[0], "cmd5"); // oldest 5 dropped
}

#[test]
fn up_recalls_most_recent() {
    let h = vec!["ps".to_string(), "ls".to_string()];
    let (input, idx) = history_up(&h, "", 0);
    assert_eq!(input, "ls");
    assert_eq!(idx, 1);
}

#[test]
fn up_up_goes_further_back() {
    let h = vec!["ps".to_string(), "ls".to_string()];
    let (_, idx) = history_up(&h, "", 0);
    let (input, idx2) = history_up(&h, "", idx);
    assert_eq!(input, "ps");
    assert_eq!(idx2, 2);
}

#[test]
fn up_clamps_at_oldest() {
    let h = vec!["ps".to_string()];
    let (_, idx) = history_up(&h, "", 0);
    let (input, idx2) = history_up(&h, "", idx); // already at oldest
    assert_eq!(input, "ps");
    assert_eq!(idx2, 1); // doesn't go past 1
}

#[test]
fn down_returns_to_saved_input() {
    let h = vec!["ps".to_string(), "ls".to_string()];
    let (_, idx) = history_up(&h, "partial", 0); // idx=1, showing "ls"
    let (input, idx2) = history_down(&h, "partial", idx);
    assert_eq!(input, "partial"); // restores saved input
    assert_eq!(idx2, 0);
}

#[test]
fn down_at_zero_no_op() {
    let h = vec!["ps".to_string()];
    let (input, idx) = history_down(&h, "hello", 0);
    assert_eq!(input, "hello");
    assert_eq!(idx, 0);
}
