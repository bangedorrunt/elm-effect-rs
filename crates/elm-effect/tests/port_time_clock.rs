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
    assert_eq!(
        clock.now_millis(),
        2500,
        "adjustments accumulate monotonically"
    );
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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[test]
fn test_clock_sleep_does_not_require_wall_time() {
    let clock = Arc::new(TestClock::new());
    let clock2 = clock.clone();
    let done = Arc::new(AtomicBool::new(false));
    let probe = done.clone();
    let worker = std::thread::spawn(move || {
        let out = clock2.sleep(Duration::from_millis(10 * 3_600_000), &|| false);
        probe.store(true, Ordering::SeqCst);
        out
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while clock.pending_sleepers() < 1 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
    assert!(!done.load(Ordering::SeqCst));
    clock.adjust(Duration::from_millis(11 * 3_600_000));
    let outcome = worker.join().unwrap();
    assert_eq!(outcome, elm_effect::SleepOutcome::Elapsed);
    assert!(done.load(Ordering::SeqCst));
}

#[test]
fn test_clock_sleep_delays_until_adjusted() {
    let clock = Arc::new(TestClock::new());
    let clock2 = clock.clone();
    let done = Arc::new(AtomicBool::new(false));
    let probe = done.clone();
    let worker = std::thread::spawn(move || {
        let out = clock2.sleep(Duration::from_millis(10 * 3_600_000), &|| false);
        probe.store(true, Ordering::SeqCst);
        out
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while clock.pending_sleepers() < 1 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
    assert!(!done.load(Ordering::SeqCst));
    clock.adjust(Duration::from_millis(9 * 3_600_000));
    assert!(!done.load(Ordering::SeqCst));
    assert_eq!(clock.pending_sleepers(), 1);
    clock.adjust(Duration::from_millis(2 * 3_600_000));
    let outcome = worker.join().unwrap();
    assert_eq!(outcome, elm_effect::SleepOutcome::Elapsed);
    assert!(done.load(Ordering::SeqCst));
}

#[test]
fn test_clock_sleep_handles_multiple_sleeps() {
    let clock = Arc::new(TestClock::new());
    let message = Arc::new(Mutex::new(String::new()));
    let worker1_clock = clock.clone();
    let worker1_message = message.clone();
    let t1 = std::thread::spawn(move || {
        let _ = worker1_clock.sleep(Duration::from_millis(3 * 3_600_000), &|| false);
        worker1_message.lock().unwrap().push_str("World!");
    });
    let worker2_clock = clock.clone();
    let worker2_message = message.clone();
    let t2 = std::thread::spawn(move || {
        let _ = worker2_clock.sleep(Duration::from_millis(1 * 3_600_000), &|| false);
        worker2_message.lock().unwrap().push_str("Hello, ");
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut pending = 0usize;
    while pending < 2 {
        pending = clock.pending_sleepers();
        assert!(
            std::time::Instant::now() < deadline,
            "sleepers did not park"
        );
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
    clock.adjust(Duration::from_millis(1 * 3_600_000));
    let settle = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while std::time::Instant::now() < settle {
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
    assert_eq!(message.lock().unwrap().as_str(), "Hello, ");
    clock.adjust(Duration::from_millis(4 * 3_600_000));
    t1.join().unwrap();
    t2.join().unwrap();
    assert_eq!(message.lock().unwrap().as_str(), "Hello, World!");
}
