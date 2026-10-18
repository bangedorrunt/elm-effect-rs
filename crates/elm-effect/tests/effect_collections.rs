//! `Effect` collection combinators at the pin
//! (`packages/effect/test/Effect.test.ts`, describe blocks `partition`,
//! `reduce`, `validate`, `findFirst`, `findFirstFilter`).
//!
//! The item-mapping forms (`items, f`) are what the rows exercise. This
//! port's collection combinators interpret at the runtime, so they return
//! `Exit` directly rather than `Effect`: the ADR's `ManagedRuntime::run`
//! bridge is for a description, and these are already driven bodies.

use std::sync::Arc;
use std::sync::Mutex;

use elm_effect::{
    Concurrency, Exit, Runtime, find_first, find_first_filter, partition_items, reduce,
    validate_discard_items, validate_items,
};

fn items(values: &[u32]) -> Vec<u32> {
    values.to_vec()
}

fn even_fails(n: u32) -> Exit<u32, u32> {
    if n % 2 == 0 {
        Exit::fail(n)
    } else {
        Exit::succeed(n)
    }
}

// ---------------------------------------------------------------------------
// partition
// ---------------------------------------------------------------------------

#[test]
fn partition_collects_only_successes() {
    // Effect.partition([0..4], Effect.succeed) === [[0,1,2,3,4], []]
    let rt = Runtime::new();
    let exit = partition_items(
        &rt,
        items(&[0, 1, 2, 3, 4]),
        Concurrency::Sequential,
        |n, _c| Exit::<u32, u32>::succeed(n),
    );
    match exit {
        Exit::Success((passes, fails)) => {
            assert_eq!(passes, vec![0, 1, 2, 3, 4]);
            assert!(fails.is_empty());
        }
        Exit::Failure(_) => panic!("partition must not fail on typed failures"),
    }
}

#[test]
fn partition_collects_only_failures() {
    // Effect.partition([0..4], Effect.fail) === [[], [0,1,2,3,4]]
    let rt = Runtime::new();
    let exit = partition_items(
        &rt,
        items(&[0, 1, 2, 3, 4]),
        Concurrency::Sequential,
        |n, _c| Exit::<u32, u32>::fail(n),
    );
    match exit {
        Exit::Success((passes, fails)) => {
            assert!(passes.is_empty());
            assert_eq!(fails, vec![0, 1, 2, 3, 4]);
        }
        Exit::Failure(_) => panic!("partition must not fail fast on typed errors"),
    }
}

#[test]
fn partition_collects_successes_and_failures() {
    let rt = Runtime::new();
    let exit = partition_items(
        &rt,
        items(&[0, 1, 2, 3, 4, 5]),
        Concurrency::Sequential,
        |n, _c| even_fails(n),
    );
    match exit {
        Exit::Success((passes, fails)) => {
            assert_eq!(passes, vec![1, 3, 5]);
            assert_eq!(fails, vec![0, 2, 4]);
        }
        Exit::Failure(_) => panic!("partition must not fail"),
    }
}

#[test]
fn partition_supports_concurrency_option() {
    // concurrency "unbounded" keeps the input order and the same split.
    let rt = Runtime::new();
    let exit = partition_items(
        &rt,
        items(&[0, 1, 2, 3, 4, 5]),
        Concurrency::Unbounded,
        |n, _c| even_fails(n),
    );
    match exit {
        Exit::Success((passes, fails)) => {
            assert_eq!(passes, vec![1, 3, 5]);
            assert_eq!(fails, vec![0, 2, 4]);
        }
        Exit::Failure(_) => panic!("partition must not fail"),
    }
}

// ---------------------------------------------------------------------------
// reduce
// ---------------------------------------------------------------------------

#[test]
fn reduce_runs_sequentially_and_passes_the_index() {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let rt = Runtime::new();
    let probe = Arc::clone(&seen);
    let exit = reduce(
        &rt,
        items(&[1, 2, 3, 4, 5]),
        || 0u32,
        move |acc, value, index| {
            probe.lock().unwrap().push(format!("{index}:{value}"));
            Exit::<u32, u32>::succeed(acc + value)
        },
    );
    assert!(matches!(exit, Exit::Success(15)));
    assert_eq!(
        *seen.lock().unwrap(),
        vec!["0:1", "1:2", "2:3", "3:4", "4:5"]
    );
}

#[test]
fn reduce_stops_at_the_first_failure() {
    let seen = Arc::new(Mutex::new(Vec::<u32>::new()));
    let rt = Runtime::new();
    let probe = Arc::clone(&seen);
    let exit = reduce(
        &rt,
        items(&[1, 2, 3, 4, 5]),
        || 0u32,
        move |acc, value, _index| {
            probe.lock().unwrap().push(value);
            if value == 3 {
                Exit::<u32, String>::fail("fail".to_string())
            } else {
                Exit::<u32, String>::succeed(acc + value)
            }
        },
    );
    match exit {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"fail".to_string())),
        Exit::Success(_) => panic!("expected the typed failure"),
    }
    assert_eq!(*seen.lock().unwrap(), vec![1, 2, 3]);
}

#[test]
fn reduce_returns_an_empty_iterable_initial_value() {
    let rt = Runtime::new();
    let exit = reduce(
        &rt,
        Vec::<u32>::new(),
        || 42u32,
        |acc, value, _index| Exit::<u32, u32>::succeed(acc + value),
    );
    assert!(matches!(exit, Exit::Success(42)));
}

#[test]
fn validate_collects_successes_when_all_succeed() {
    let rt = Runtime::new();
    let exit = validate_items(
        &rt,
        items(&[0, 1, 2, 3, 4]),
        Concurrency::Sequential,
        |n, _c| Exit::<u32, u32>::succeed(n),
    );
    assert!(matches!(exit, Exit::Success(ref v) if *v == vec![0, 1, 2, 3, 4]));
}

#[test]
fn validate_accumulates_all_failures() {
    let rt = Runtime::new();
    let exit = validate_items(
        &rt,
        items(&[0, 1, 2, 3, 4, 5]),
        Concurrency::Sequential,
        |n, _c| even_fails(n),
    );
    match exit {
        // The error channel is the collected list: one Fail reason holding [0, 2, 4].
        Exit::Failure(cause) => assert_eq!(cause.failures(), vec![&vec![0u32, 2, 4]]),
        Exit::Success(_) => panic!("expected the accumulated failures"),
    }
}

#[test]
fn validate_supports_discard_option() {
    let rt = Runtime::new();
    let exit = validate_discard_items(&rt, items(&[1, 2, 3]), Concurrency::Sequential, |n, _c| {
        Exit::<u32, u32>::succeed(n)
    });
    assert!(matches!(exit, Exit::Success(())));
}

// ---------------------------------------------------------------------------
// findFirst / findFirstFilter
// ---------------------------------------------------------------------------

#[test]
fn find_first_returns_first_matching_element() {
    let rt = Runtime::new();
    let exit = find_first(&rt, items(&[1, 2, 3, 4]), |n, _index| {
        Exit::<bool, u32>::succeed(n > 2)
    });
    assert!(matches!(exit, Exit::Success(Some(3))));
}

#[test]
fn find_first_returns_none_when_no_element_matches() {
    let rt = Runtime::new();
    let exit = find_first(&rt, items(&[1, 2, 3, 4]), |n, _index| {
        Exit::<bool, u32>::succeed(n > 10)
    });
    assert!(matches!(exit, Exit::Success(None)));
}

#[test]
fn find_first_short_circuits_on_first_match() {
    let seen = Arc::new(Mutex::new(Vec::<u32>::new()));
    let rt = Runtime::new();
    let probe = Arc::clone(&seen);
    let exit = find_first(&rt, items(&[1, 2, 3, 4]), move |n, _index| {
        probe.lock().unwrap().push(n);
        Exit::<bool, u32>::succeed(n > 2)
    });
    assert!(matches!(exit, Exit::Success(Some(3))));
    assert_eq!(*seen.lock().unwrap(), vec![1, 2, 3]);
}

#[test]
fn find_first_filter_returns_first_successful_transformed_value() {
    let rt = Runtime::new();
    let exit = find_first_filter::<u32, String, u32, _>(&rt, items(&[1, 2, 3, 4]), |n, _index| {
        (n % 2 == 0).then(|| format!("n={n}"))
    });
    match exit {
        Exit::Success(Some(value)) => assert_eq!(value, "n=2"),
        other => panic!("expected n=2, got {other:?}"),
    }
}

#[test]
fn find_first_filter_passes_index_to_filter() {
    let rt = Runtime::new();
    let exit = find_first_filter::<u32, String, u32, _>(&rt, items(&[10, 11, 12]), |n, index| {
        (index == 2).then(|| format!("{n}:{index}"))
    });
    match exit {
        Exit::Success(Some(value)) => assert_eq!(value, "12:2"),
        other => panic!("expected 12:2, got {other:?}"),
    }
}

#[test]
fn find_first_filter_returns_none_when_filter_never_succeeds() {
    let rt = Runtime::new();
    let exit =
        find_first_filter::<u32, String, u32, _>(
            &rt,
            items(&[1, 2, 3]),
            |_n, _index| None::<String>,
        );
    assert!(matches!(exit, Exit::Success(None)));
}

// ---------------------------------------------------------------------------
// Effect.test.ts :1143 — the zero is evaluated once per run, not at
// construction. This port's `reduce` takes the zero as a closure, so the
// counter is the same probe the pin uses.
// ---------------------------------------------------------------------------

#[test]
fn reduce_evaluates_its_zero_lazily_for_each_run() {
    let evaluations = Arc::new(Mutex::new(0usize));
    let counter = evaluations.clone();
    let mut run = || {
        let counter = counter.clone();
        reduce(
            &Runtime::new(),
            items(&[1]),
            move || {
                *counter.lock().unwrap() += 1;
                Vec::<u32>::new()
            },
            |acc: Vec<u32>, value: u32, _index| -> Exit<Vec<u32>, u32> {
                Exit::succeed({
                    let mut acc = acc;
                    acc.push(value);
                    acc
                })
            },
        )
    };

    assert_eq!(*evaluations.lock().unwrap(), 0, "the zero is not evaluated yet");
    assert!(matches!(run(), Exit::Success(ref values) if values == &vec![1]));
    assert!(matches!(run(), Exit::Success(ref values) if values == &vec![1]));
    assert_eq!(*evaluations.lock().unwrap(), 2, "once per run");
}

// ---------------------------------------------------------------------------
// Effect.test.ts :1231 / :1237 — filter keeps the accepted elements in
// iteration order, for a list and for another iterable.
// ---------------------------------------------------------------------------

#[test]
fn filter_keeps_the_odd_numbers() {
    let exit: Exit<Vec<u32>, std::convert::Infallible> = Runtime::run(
        elm_effect::Effect::<u32, std::convert::Infallible, ()>::filter(
            items(&[1, 2, 3, 4, 5]),
            |value: &u32| value % 2 == 1,
        ),
    );
    assert_eq!(exit, Exit::Success(vec![1, 3, 5]));
}

#[test]
fn filter_accepts_any_iterable() {
    // A `BTreeSet` is this port's ordered analogue of the pin's `Set`
    // argument: the combinator takes `IntoIterator`, not a slice.
    let set: std::collections::BTreeSet<u32> = [1, 2, 3, 4, 5].into_iter().collect();
    let exit: Exit<Vec<u32>, std::convert::Infallible> = Runtime::run(
        elm_effect::Effect::<u32, std::convert::Infallible, ()>::filter(set, |value: &u32| {
            value % 2 == 1
        }),
    );
    assert_eq!(exit, Exit::Success(vec![1, 3, 5]));
}

/// Effect.test.ts :901 — `forEach` over nothing runs nothing and succeeds
/// with an empty collection.
#[test]
fn for_each_over_no_items_yields_an_empty_result() {
    let rt = Runtime::new();
    let exit = elm_effect::for_each::<u32, u32, u32>(&rt, items(&[]), Concurrency::Sequential, |item, _ctx| {
        Exit::succeed(item)
    });
    assert_eq!(exit, Exit::Success(Vec::<u32>::new()));
}
