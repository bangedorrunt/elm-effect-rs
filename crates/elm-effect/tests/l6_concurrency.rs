//! Law L13 (fan-out part): `all` defaults to sequential; bounds are explicit.
//!
//! Bodies are fiber closures (sync `Effect`s cannot suspend, so they are the
//! wrong seam for a suspension test). The sequential test observes from
//! another thread: while the first body is parked, the second must not have
//! started.

#![cfg(feature = "tokio")]

#[path = "../src/concurrency.rs"]
mod concurrency;

use concurrency::{Concurrency, all, for_each};
use elm_effect::{Cause, Ctx, Defect, Duration, Exit, FiberHandle, ForkOpts, Reason, Runtime};
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[test]
fn all_without_concurrency_is_sequential() {
    let rt = Runtime::new();
    let release = Arc::new(AtomicBool::new(false));
    let worker_release = release.clone();
    let started_second = Arc::new(AtomicBool::new(false));
    let probe_second = started_second.clone();

    // Run the fan-out on a worker thread so this thread can observe order.
    let worker = std::thread::spawn(move || {
        all(
            &rt,
            vec![
                Box::new(move |ctx: &elm_effect::Ctx| -> Exit<i32, String> {
                    while !worker_release.load(Ordering::SeqCst) {
                        if ctx.is_interrupted() {
                            return Exit::Failure(Cause::interrupt(None));
                        }
                        std::thread::park_timeout(std::time::Duration::from_millis(1));
                    }
                    Exit::succeed(1)
                }) as Box<dyn FnOnce(&elm_effect::Ctx) -> Exit<i32, String> + Send>,
                Box::new(move |_: &elm_effect::Ctx| -> Exit<i32, String> {
                    probe_second.store(true, Ordering::SeqCst);
                    Exit::succeed(2)
                }) as Box<dyn FnOnce(&elm_effect::Ctx) -> Exit<i32, String> + Send>,
            ],
            Concurrency::Sequential,
        )
    });
    // The first body is parked; the second must not have started.
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !started_second.load(Ordering::SeqCst),
        "second effect started while the first was suspended"
    );
    release.store(true, Ordering::SeqCst);
    let exit = worker.join().expect("worker thread");
    assert_eq!(exit, Exit::Success(vec![1, 2]));
    assert!(started_second.load(Ordering::SeqCst));
}

#[test]
fn all_with_bound_respects_bound() {
    let rt = Runtime::new();
    // Bound 1 behaves like sequential: completion order matches input order.
    let exit = all(
        &rt,
        vec![
            Box::new(|_: &elm_effect::Ctx| Exit::<i32, String>::succeed(1))
                as Box<dyn FnOnce(&elm_effect::Ctx) -> Exit<i32, String> + Send>,
            Box::new(|_: &elm_effect::Ctx| Exit::<i32, String>::succeed(2))
                as Box<dyn FnOnce(&elm_effect::Ctx) -> Exit<i32, String> + Send>,
        ],
        Concurrency::Bounded(1),
    );
    assert_eq!(exit, Exit::Success(vec![1, 2]));
}

#[test]
fn fanout_peak_in_flight() {
    let rt = Runtime::new();
    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut bodies = Vec::new();
    for _ in 0..5 {
        let cur = current.clone();
        let pk = peak.clone();
        bodies.push(Box::new(move |ctx: &elm_effect::Ctx| -> Exit<i32, String> {
            let now = cur.fetch_add(1, Ordering::SeqCst) + 1;
            pk.fetch_max(now, Ordering::SeqCst);
            let mut waited = 0;
            while waited < 20 && !ctx.is_interrupted() {
                std::thread::park_timeout(std::time::Duration::from_millis(1));
                waited += 1;
            }
            cur.fetch_sub(1, Ordering::SeqCst);
            Exit::succeed(1)
        })
            as Box<dyn FnOnce(&elm_effect::Ctx) -> Exit<i32, String> + Send>);
    }
    let exit = all(&rt, bodies, Concurrency::Bounded(2));
    assert_eq!(exit, Exit::Success(vec![1, 1, 1, 1, 1]));
    assert!(
        peak.load(Ordering::SeqCst) <= 2,
        "peak in flight exceeds bound"
    );
}

#[test]
fn all_fails_fast_and_releases_rest() {
    let rt = Runtime::new();
    let released = Arc::new(AtomicBool::new(false));
    let worker_released = released.clone();
    let sibling_up = Arc::new(AtomicBool::new(false));
    let fail_after_up = sibling_up.clone();
    let body_up = sibling_up.clone();
    let exit = all(
        &rt,
        vec![
            Box::new(move |ctx: &elm_effect::Ctx| -> Exit<i32, String> {
                // Wait until the sibling registered its finalizer: the
                // interrupt below must release something real, deterministically.
                while !fail_after_up.load(Ordering::SeqCst) {
                    if ctx.is_interrupted() {
                        return Exit::Failure(Cause::interrupt(None));
                    }
                    std::thread::park_timeout(std::time::Duration::from_millis(1));
                }
                Exit::Failure(Cause::fail("boom".to_string()))
            }) as Box<dyn FnOnce(&elm_effect::Ctx) -> Exit<i32, String> + Send>,
            Box::new(move |ctx: &elm_effect::Ctx| -> Exit<i32, String> {
                ctx.add_finalizer(move || worker_released.store(true, Ordering::SeqCst));
                body_up.store(true, Ordering::SeqCst);
                while !ctx.is_interrupted() {
                    std::thread::park_timeout(std::time::Duration::from_millis(1));
                }
                Exit::Failure(Cause::interrupt(None))
            }) as Box<dyn FnOnce(&elm_effect::Ctx) -> Exit<i32, String> + Send>,
        ],
        Concurrency::Unbounded,
    );
    // First failure wins; the sibling was interrupted and released.
    assert_eq!(exit, Exit::Failure(Cause::fail("boom".to_string())));
    assert!(released.load(Ordering::SeqCst));
}

#[test]
fn for_each_mirrors_bounds() {
    let rt = Runtime::new();
    let exit = for_each(&rt, vec![1, 2, 3], Concurrency::Sequential, |a: i32, _| {
        Exit::<i32, Infallible>::succeed(a * 10)
    });
    assert_eq!(exit, Exit::Success(vec![10, 20, 30]));
}

#[test]
fn for_each_sequential_preserves_input_order() {
    // Effect.forEach([1, 2, 3], succeed) === [1, 2, 3]
    let rt = Runtime::new();
    let exit = for_each(&rt, vec![1, 2, 3], Concurrency::Sequential, |a: i32, _| {
        Exit::<i32, Infallible>::succeed(a)
    });
    assert_eq!(exit, Exit::Success(vec![1, 2, 3]));
}

#[test]
fn for_each_unbounded_runs_every_item() {
    // concurrency "unbounded" starts every item; the result keeps input order.
    let rt = Runtime::new();
    let exit = for_each(&rt, vec![1, 2, 3], Concurrency::Unbounded, |a: i32, _| {
        Exit::<i32, Infallible>::succeed(a)
    });
    assert_eq!(exit, Exit::Success(vec![1, 2, 3]));
}

#[test]
fn for_each_bounded_respects_the_bound() {
    // concurrency 2 caps the workers without changing the collected order.
    use std::sync::atomic::AtomicUsize;
    let current = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let rt = Runtime::new();
    let exit = for_each(&rt, vec![1, 2, 3, 4, 5], Concurrency::Bounded(2), {
        let (current, peak) = (Arc::clone(&current), Arc::clone(&peak));
        move |a: i32, _ctx: &Ctx| {
            let (current, peak) = (Arc::clone(&current), Arc::clone(&peak));
            Exit::<i32, Infallible>::Success({
                let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(20));
                current.fetch_sub(1, Ordering::SeqCst);
                a
            })
        }
    });
    assert_eq!(exit, Exit::Success(vec![1, 2, 3, 4, 5]));
    assert!(
        peak.load(Ordering::SeqCst) <= 2,
        "a bound of 2 must never run more than two at once"
    );
}
