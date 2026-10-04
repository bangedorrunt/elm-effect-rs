//! `partition` and `validate` read from the pin: collect-versus-fail-fast
//! comes from `Effect.ts`, not the guides. Partition collects typed failures
//! and never fails on them; validate raises every collected error; defects
//! fail fast in both.

use elm_effect::{
    Cause, Concurrency, Defect, Exit, Runtime, partition, validate, validate_discard,
};

fn ok(n: u32) -> elm_effect::Body<u32, String> {
    Box::new(move |_: &elm_effect::Ctx| Exit::Success(n))
}

fn fail(err: &'static str) -> elm_effect::Body<u32, String> {
    Box::new(move |_: &elm_effect::Ctx| Exit::Failure(Cause::fail(err.to_string())))
}

#[test]
fn partition_collects_successes() {
    let rt = Runtime::new();
    let exit = partition(&rt, vec![ok(0), ok(1), ok(2)], Concurrency::Sequential);
    match exit {
        Exit::Success((passes, fails)) => {
            assert_eq!(passes, vec![0, 1, 2]);
            assert!(fails.is_empty());
        }
        Exit::Failure(_) => panic!("expected collection"),
    }
}

#[test]
fn partition_collects_mixed_outcomes() {
    let rt = Runtime::new();
    let exit = partition(
        &rt,
        vec![ok(1), fail("a"), ok(3), fail("b")],
        Concurrency::Sequential,
    );
    match exit {
        Exit::Success((passes, fails)) => {
            assert_eq!(passes, vec![1, 3]);
            assert_eq!(fails, vec!["a".to_string(), "b".to_string()]);
        }
        Exit::Failure(_) => panic!("expected collection"),
    }
}

#[test]
fn partition_defect_fails_fast() {
    let rt = Runtime::new();
    let exit: Exit<(Vec<u32>, Vec<String>), String> = partition(
        &rt,
        vec![
            ok(1),
            Box::new(|_: &elm_effect::Ctx| Exit::Failure(Cause::die(Defect::new("kaput"))))
                as Box<dyn FnOnce(&elm_effect::Ctx) -> Exit<u32, String> + Send>,
            fail("late"),
        ],
        Concurrency::Sequential,
    );
    match exit {
        Exit::Failure(cause) => assert_eq!(cause.defects().len(), 1),
        Exit::Success(_) => panic!("expected defect to win"),
    }
}

#[test]
fn validate_accumulates_all_failures() {
    let rt = Runtime::new();
    let exit = validate(
        &rt,
        vec![ok(0), fail("a"), ok(2), fail("b")],
        Concurrency::Sequential,
    );
    match exit {
        Exit::Failure(cause) => assert_eq!(
            cause.failure(),
            Some(&vec!["a".to_string(), "b".to_string()])
        ),
        Exit::Success(_) => panic!("expected accumulated failures"),
    }

    let exit = validate(&rt, vec![ok(1), ok(2)], Concurrency::Sequential);
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![1, 2]));

    let exit = validate_discard(&rt, vec![ok(1), ok(2)], Concurrency::Sequential);
    assert!(matches!(exit, Exit::Success(())));

    let exit: Exit<(), Vec<String>> =
        validate_discard(&rt, vec![ok(1), fail("x")], Concurrency::Sequential);
    assert!(matches!(exit, Exit::Failure(_)));
}

#[test]
fn partition_keeps_input_order_when_bounded() {
    let rt = Runtime::new();
    let exit = partition(
        &rt,
        vec![
            Box::new(|_: &elm_effect::Ctx| {
                std::thread::sleep(std::time::Duration::from_millis(30));
                Exit::Success(1u32)
            }) as Box<dyn FnOnce(&elm_effect::Ctx) -> Exit<u32, String> + Send>,
            Box::new(|_: &elm_effect::Ctx| Exit::Success(2u32))
                as Box<dyn FnOnce(&elm_effect::Ctx) -> Exit<u32, String> + Send>,
            Box::new(|_: &elm_effect::Ctx| Exit::Failure(Cause::fail("e".to_string())))
                as Box<dyn FnOnce(&elm_effect::Ctx) -> Exit<u32, String> + Send>,
        ],
        Concurrency::Bounded(3),
    );
    match exit {
        Exit::Success((passes, fails)) => {
            assert_eq!(passes, vec![1, 2]);
            assert_eq!(fails, vec!["e".to_string()]);
        }
        Exit::Failure(_) => panic!("expected collection"),
    }
}
