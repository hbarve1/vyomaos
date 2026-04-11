/// Compute the next backoff after `restarts` watchdog-triggered restarts.
/// Doubles each time, capped at 300 seconds.
fn next_backoff(watchdog_secs: u32, restarts: u32) -> u64 {
    let base = watchdog_secs as u64;
    let factor = 1u64 << restarts.min(8); // 2^restarts, max 256
    (base * factor).min(300)
}

#[test]
fn first_restart_equals_watchdog_secs() {
    assert_eq!(next_backoff(5, 0), 5);
}

#[test]
fn second_restart_doubles() {
    assert_eq!(next_backoff(5, 1), 10);
    assert_eq!(next_backoff(5, 2), 20);
    assert_eq!(next_backoff(5, 3), 40);
}

#[test]
fn caps_at_300() {
    // 5 * 2^7 = 640, but capped at 300
    assert_eq!(next_backoff(5, 7),  300);
    assert_eq!(next_backoff(5, 20), 300);
}

#[test]
fn watchdog_disabled_at_zero() {
    // watchdog_secs=0 means disabled; backoff is meaningless but shouldn't panic
    assert_eq!(next_backoff(0, 0), 0);
    assert_eq!(next_backoff(0, 5), 0);
}
