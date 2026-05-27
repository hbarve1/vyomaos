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
