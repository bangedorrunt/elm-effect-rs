//! hi5 tail slice: Clock wall/monotonic cells.
//! Ports packages/effect/test/Clock.test.ts (virtual alignment plus the
//! monotonic source), left uncovered by the area beads.

use elm_effect::{ClockImpl, Duration, SystemClock, TestClock};

#[test]
fn test_clock_starts_aligned_and_advances_monotonically() {
    let clock = TestClock::new();
    assert_eq!(clock.now_millis(), 0, "virtual origin starts aligned");
    clock.adjust(Duration::from_millis(500));
    assert_eq!(clock.now_millis(), 500);
    clock.adjust(Duration::from_secs(2));
    assert_eq!(clock.now_millis(), 2500, "adjustments accumulate monotonically");
}

#[test]
fn system_clock_tracks_wall_time() {
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("wall clock readable")
        .as_millis() as u64;
    let read = SystemClock.now_millis();
    assert!(
        read >= wall && read - wall < 5_000,
        "system source stays wall-aligned (wall={wall}, read={read})"
    );
}

#[test]
fn virtual_adjustment_does_not_move_the_wall_source() {
    let clock = TestClock::new();
    let before = SystemClock.now_millis();
    clock.adjust(Duration::from_minutes(60));
    let after = SystemClock.now_millis();
    assert_eq!(clock.now_millis(), 3_600_000);
    assert!(
        after - before < 5_000,
        "the wall advanced only by real elapsed millis"
    );
}
