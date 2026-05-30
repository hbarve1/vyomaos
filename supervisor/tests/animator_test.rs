use supervisor::display::animator::{Animation, AnimKind};

#[test]
fn open_animation_starts_fast() {
    // With ease-out, at t=0.5 the alpha should be > 50% of the total range.
    // Linear t=0.5 → alpha = 127. Ease-out t=0.5 → t' = 1-(0.5)^3 = 0.875 → alpha = 223.
    let anim = Animation::new(AnimKind::Open, 0);
    let half_state = anim.sample(150); // 150ms = half of 300ms duration
    assert!(half_state.alpha > 127,
        "ease-out animation should be past halfway at t=0.5, got alpha={}", half_state.alpha);
}

#[test]
fn animation_completes_at_full_duration() {
    let anim = Animation::new(AnimKind::Open, 0);
    let done = anim.sample(300); // exactly at duration
    assert!(done.done, "animation must be done at full duration");
    assert_eq!(done.alpha, 255, "Open animation must end fully opaque");
}

#[test]
fn close_animation_starts_fast() {
    // Close: from_alpha=255, to_alpha=0. At t=0.5 ease-out gives t'=0.875, alpha = 255*(1-0.875) = 31.
    let anim = Animation::new(AnimKind::Close, 0);
    let half_state = anim.sample(125); // 125ms = half of 250ms
    assert!(half_state.alpha < 127,
        "ease-out close should be mostly transparent at midpoint, got alpha={}", half_state.alpha);
}

#[test]
fn close_alpha_decreases_monotonically() {
    let anim = Animation::new(AnimKind::Close, 0);
    let mut prev_alpha = 255u8;
    // Sample at 50ms intervals through the 250ms Close animation.
    for ms in (50..=250).step_by(50) {
        let state = anim.sample(ms);
        assert!(state.alpha <= prev_alpha,
            "Close alpha must decrease: at {}ms got {} but prev was {}",
            ms, state.alpha, prev_alpha);
        prev_alpha = state.alpha;
    }
    // Final sample must be fully transparent and done.
    let final_state = anim.sample(250);
    assert!(final_state.done);
    assert_eq!(final_state.alpha, 0);
}

#[test]
fn minimize_alpha_decreases_to_zero() {
    let anim = Animation::new(AnimKind::Minimize, 0);
    let mid = anim.sample(150);
    assert!(mid.alpha < 128, "Minimize should be mostly faded at midpoint, got {}", mid.alpha);
    let end = anim.sample(300);
    assert!(end.done);
    assert_eq!(end.alpha, 0);
}
