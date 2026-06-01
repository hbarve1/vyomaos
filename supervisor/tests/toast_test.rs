// Unit tests for supervisor/src/toast.rs — notification banner queue logic.
//
// The actual enqueue/render functions use static OnceLock and display:: imports.
// We replicate the pure queue logic here to test correctness.

use std::collections::VecDeque;

struct Banner {
    #[allow(dead_code)]
    app_name: String,
    title: String,
    body: String,
    created_ms: u64,
    dismiss_after_ms: u32,
}

struct BannerQueue {
    q: VecDeque<Banner>,
}

impl BannerQueue {
    fn new() -> Self { Self { q: VecDeque::new() } }

    fn enqueue(&mut self, app_name: &str, title: &str, body: &str, now_ms: u64) {
        // Max 3 concurrent banners; evict oldest if full.
        if self.q.len() >= 3 { self.q.pop_front(); }
        self.q.push_back(Banner {
            app_name: app_name.to_string(),
            title: title.to_string(),
            body: body[..body.len().min(120)].to_string(),
            created_ms: now_ms,
            dismiss_after_ms: 4000,
        });
    }

    fn dismiss_expired(&mut self, now_ms: u64) {
        self.q.retain(|b| now_ms.saturating_sub(b.created_ms) < b.dismiss_after_ms as u64);
    }

    fn len(&self) -> usize { self.q.len() }
}

// ── Enqueue tests ───────────────────────────────────────────────────────────

#[test]
fn test_enqueue_single_banner() {
    let mut q = BannerQueue::new();
    q.enqueue("calc", "Hello", "World", 1000);
    assert_eq!(q.len(), 1);
    assert_eq!(q.q[0].title, "Hello");
}

#[test]
fn test_enqueue_max_3_banners() {
    let mut q = BannerQueue::new();
    q.enqueue("a", "1", "body1", 1000);
    q.enqueue("b", "2", "body2", 2000);
    q.enqueue("c", "3", "body3", 3000);
    assert_eq!(q.len(), 3);
}

#[test]
fn test_enqueue_evicts_oldest_when_full() {
    let mut q = BannerQueue::new();
    q.enqueue("a", "1", "body1", 1000);
    q.enqueue("b", "2", "body2", 2000);
    q.enqueue("c", "3", "body3", 3000);
    q.enqueue("d", "4", "body4", 4000); // evicts "1"
    assert_eq!(q.len(), 3);
    assert_eq!(q.q[0].title, "2"); // oldest is now "2"
    assert_eq!(q.q[2].title, "4"); // newest is "4"
}

#[test]
fn test_enqueue_evicts_repeatedly() {
    let mut q = BannerQueue::new();
    for i in 0..10 {
        q.enqueue("app", &format!("T{i}"), "body", i * 1000);
    }
    assert_eq!(q.len(), 3);
    assert_eq!(q.q[0].title, "T7");
    assert_eq!(q.q[1].title, "T8");
    assert_eq!(q.q[2].title, "T9");
}

// ── Auto-dismiss timing ─────────────────────────────────────────────────────

#[test]
fn test_dismiss_not_expired() {
    let mut q = BannerQueue::new();
    q.enqueue("a", "title", "body", 1000);
    q.dismiss_expired(3000); // 2000ms < 4000ms dismiss_after
    assert_eq!(q.len(), 1);
}

#[test]
fn test_dismiss_expired() {
    let mut q = BannerQueue::new();
    q.enqueue("a", "title", "body", 1000);
    q.dismiss_expired(5001); // 4001ms >= 4000ms dismiss_after
    assert_eq!(q.len(), 0);
}

#[test]
fn test_dismiss_exactly_at_threshold() {
    let mut q = BannerQueue::new();
    q.enqueue("a", "title", "body", 1000);
    // At exactly 4000ms elapsed: 5000 - 1000 = 4000, NOT < 4000 -> dismissed
    q.dismiss_expired(5000);
    assert_eq!(q.len(), 0);
}

#[test]
fn test_dismiss_partial_expiration() {
    let mut q = BannerQueue::new();
    q.enqueue("a", "old", "body", 1000);
    q.enqueue("b", "new", "body", 4000);
    q.dismiss_expired(5500); // "old" expired (4500ms), "new" not expired (1500ms)
    assert_eq!(q.len(), 1);
    assert_eq!(q.q[0].title, "new");
}

// ── Body truncation ─────────────────────────────────────────────────────────

#[test]
fn test_body_truncation_short() {
    let mut q = BannerQueue::new();
    q.enqueue("a", "t", "short body", 0);
    assert_eq!(q.q[0].body, "short body");
}

#[test]
fn test_body_truncation_long() {
    let mut q = BannerQueue::new();
    let long_body = "x".repeat(200);
    q.enqueue("a", "t", &long_body, 0);
    assert_eq!(q.q[0].body.len(), 120);
}

// ── Empty queue operations ──────────────────────────────────────────────────

#[test]
fn test_dismiss_empty_queue() {
    let mut q = BannerQueue::new();
    q.dismiss_expired(10000); // no panic
    assert_eq!(q.len(), 0);
}

// ── Focus transfer logic (auto_transfer_focus) ──────────────────────────────
// This is pure logic that the toast module implements.  We test the algorithm.

#[test]
fn test_focus_transfer_finds_next_display_app() {
    // Simulate: 3 apps, "calc" is exiting, "shell" and "monitor" are display+running
    let apps = vec![
        ("calc", true, true),     // display, running but exiting
        ("monitor", true, true),  // display, running
        ("shell", true, true),    // display, running
    ];
    let exiting = "calc";
    let mut candidates: Vec<&str> = apps.iter()
        .filter(|&&(name, has_display, running)| name != exiting && has_display && running)
        .map(|&(name, _, _)| name)
        .collect();
    candidates.sort();
    assert_eq!(candidates.first(), Some(&"monitor"));
}

#[test]
fn test_focus_transfer_no_other_apps() {
    let apps: Vec<(&str, bool, bool)> = vec![
        ("calc", true, true),
    ];
    let exiting = "calc";
    let candidates: Vec<&str> = apps.iter()
        .filter(|&&(name, has_display, running)| name != exiting && has_display && running)
        .map(|&(name, _, _)| name)
        .collect();
    assert!(candidates.is_empty());
}
