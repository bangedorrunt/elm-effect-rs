//! Process keep-alive: the process stays alive until the effect completes.
//! `run_main` exits the process, so the completion case runs in a re-executed
//! child: the parent asserts the exit code and the elapsed floor.
#![cfg(feature = "tokio")]

use elm_effect::{ClockImpl, Duration, Effect, Runtime, SleepOutcome, TestClock};
use std::sync::Arc;
use std::time::Instant;

// EffectKeepAlive.test.ts:4 — the process stays alive until the effect
// completes instead of exiting early.
#[test]
fn keepalive_child_completes_slow_effect() {
    if std::env::var("ELM_KEEPALIVE_CHILD").is_ok() {
        let effect = Effect::<String, String>::sync(|| {
            std::thread::sleep(std::time::Duration::from_millis(200));
            "done".to_string()
        });
        Runtime::run_main(effect);
    }
    let start = Instant::now();
    let status = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .env("ELM_KEEPALIVE_CHILD", "1")
        .status()
        .expect("child runs");
    let elapsed = start.elapsed();
    assert_eq!(status.code(), Some(0), "slow effect exits clean");
    assert!(
        elapsed.as_millis() >= 200,
        "process stayed alive for completion ({}ms)",
        elapsed.as_millis()
    );
}

// EffectKeepAlive.test.ts:41 — a blocked interval (a virtual sleeper that
// never advances on its own) does not hold the process open: the run
// returns when the main effect completes, and the sleeper drains after.
#[test]
fn blocked_interval_does_not_hold_exit() {
    let clock = Arc::new(TestClock::new());
    let parked = clock.clone();
    let sleeper = std::thread::spawn(move || parked.sleep(Duration::from_secs(3600), &|| false));
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while clock.pending_sleepers() != 1 {
        assert!(Instant::now() < deadline, "sleeper parked");
        std::thread::park_timeout(std::time::Duration::from_millis(1));
    }
    let start = Instant::now();
    let exit = Runtime::run(Effect::<String, String>::succeed("done".to_string()));
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert!(matches!(exit, elm_effect::Exit::Success(ref s) if s == "done"));
    clock.adjust(Duration::from_secs(7200));
    assert_eq!(sleeper.join().expect("sleeper ends"), SleepOutcome::Elapsed);
}
