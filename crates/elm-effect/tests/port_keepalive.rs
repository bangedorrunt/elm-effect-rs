//! hi5 tail slice: process keep-alive cells.
//! Ports packages/effect/test/EffectKeepAlive.test.ts (the process stays
//! alive until completion), left uncovered by the area beads.
//! `run_main` exits the process, so the completion case runs in a re-executed
//! child: the parent asserts the exit code and the elapsed floor.
#![cfg(feature = "tokio")]

use elm_effect::{Effect, Runtime};
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
