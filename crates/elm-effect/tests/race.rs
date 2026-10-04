//! Races: first success wins, losers are released, failures concatenate.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use elm_effect::{Cause, Effect, Exit, Reason, Runtime, race, race_first};

#[test]
fn race_failure_does_not_beat_later_success() {
    let rt = Runtime::new();
    let exit = race(
        &rt,
        |_| Exit::Failure(Cause::fail("early".to_string())),
        |_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            Exit::Success(7u32)
        },
    );
    assert!(matches!(exit, Exit::Success(7)));
}

#[test]
fn race_all_fail_concatenates_causes() {
    let rt = Runtime::new();
    let exit: Exit<u32, String> = race(
        &rt,
        |_| Exit::Failure(Cause::fail("left".to_string())),
        |_| Exit::Failure(Cause::fail("right".to_string())),
    );
    match exit {
        Exit::Failure(cause) => {
            let failures: Vec<&String> = cause
                .reasons
                .iter()
                .filter_map(|r| match r {
                    Reason::Fail(e) => Some(e),
                    _ => None,
                })
                .collect();
            assert_eq!(failures, vec!["left", "right"]);
        }
        Exit::Success(_) => panic!("expected failure"),
    }
}

#[test]
fn race_loser_is_released() {
    let rt = Runtime::new();
    let released = Arc::new(AtomicUsize::new(0));
    let exit = race::<u32, String>(
        &rt,
        |_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            Exit::Success(1u32)
        },
        {
            let released = Arc::clone(&released);
            move |ctx| {
                ctx.acquire_release(Effect::<(), String>::succeed(()), move |_| {
                    released.fetch_add(1, Ordering::SeqCst);
                    Effect::<(), String>::succeed(())
                })
                .expect("acquire cannot fail");
                while !ctx.is_interrupted() {
                    std::thread::park_timeout(std::time::Duration::from_millis(1));
                }
                Exit::Failure(Cause::interrupt(None))
            }
        },
    );
    assert!(matches!(exit, Exit::Success(1)));
    assert_eq!(released.load(Ordering::SeqCst), 1);
}

#[test]
fn race_first_takes_any_completion() {
    let rt = Runtime::new();
    let exit: Exit<u32, String> = race_first(
        &rt,
        |_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            Exit::Success(1)
        },
        |_| Exit::Failure(Cause::fail("fast".to_string())),
    );
    match exit {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"fast".to_string())),
        Exit::Success(_) => panic!("expected the fast failure"),
    }
}
