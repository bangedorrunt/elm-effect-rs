//! Law L9: the shared-state ladder. Immutable first; pure updates may retry
//! but never perform I/O; one value per fact; bounded queues between tasks.
#![cfg(feature = "tokio")]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use elm_effect::{
    Deferred, Effect, Exit, MutableRef, Queue, Ref, Runtime, Semaphore, SynchronizedRef,
};

#[test]
fn atomic_update_retries_without_io() {
    // Purity is by type: `update` takes a plain `Fn(&A) -> A`, never an
    // effect. An effect-returning closure does not compile (see the
    // `compile_fail` doctest on `Ref::update`).
    let cell = Arc::new(Ref::new(0u32));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let (cell, calls) = (Arc::clone(&cell), Arc::clone(&calls));
        handles.push(std::thread::spawn(move || {
            for _ in 0..1000 {
                cell.update(|x| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    x + 1
                });
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    // Every increment applied exactly once; the closure ran at least once
    // per update (retries stay transparent).
    assert_eq!(cell.get(), 8000);
    assert!(calls.load(Ordering::Relaxed) >= 8000);
}

#[test]
fn synchronized_ref_runs_once() {
    let cell = SynchronizedRef::new(10u32);
    let decided = Arc::new(AtomicUsize::new(0));
    let ran = Arc::new(AtomicUsize::new(0));
    let effect: Effect<(), String, ()> = cell.update_effect({
        let (decided, ran) = (Arc::clone(&decided), Arc::clone(&ran));
        move |current: &u32| {
            decided.fetch_add(1, Ordering::SeqCst);
            let next = current + 1;
            let inner = Effect::sync({
                let ran = Arc::clone(&ran);
                move || {
                    ran.fetch_add(1, Ordering::SeqCst);
                }
            });
            (next, inner)
        }
    });
    let exit = Runtime::run(effect);
    assert!(matches!(exit, Exit::Success(())));
    assert_eq!(cell.get(), 11);
    assert_eq!(decided.load(Ordering::SeqCst), 1);
    assert_eq!(ran.load(Ordering::SeqCst), 1);
}

#[test]
fn queue_is_bounded() {
    let q = Queue::new(2);
    assert!(q.try_offer(1));
    assert!(q.try_offer(2));
    assert!(!q.try_offer(3));
    assert_eq!(q.len(), 2);
    assert_eq!(q.try_poll(), Some(1));
    assert_eq!(q.try_poll(), Some(2));
    assert_eq!(q.try_poll(), None);

    // Blocking offer waits for a poll.
    let q = Arc::new(Queue::new(1));
    let producer = {
        let q = Arc::clone(&q);
        std::thread::spawn(move || {
            q.offer("a");
            q.offer("b");
            q.offer("c");
        })
    };
    assert_eq!(q.poll(), "a");
    assert_eq!(q.poll(), "b");
    assert_eq!(q.poll(), "c");
    producer.join().unwrap();
    assert!(q.is_empty());
}

#[test]
fn semaphore_permits_match() {
    let sem = Semaphore::new(2);
    assert_eq!(sem.available(), 2);
    assert!(sem.try_acquire());
    assert!(sem.try_acquire());
    assert_eq!(sem.available(), 0);
    assert!(!sem.try_acquire());
    sem.release();
    assert_eq!(sem.available(), 1);
    assert!(sem.try_acquire());
    sem.release();
    sem.release();
    assert_eq!(sem.available(), 2);
}

#[test]
fn mutable_ref_and_deferred_handoff() {
    let cell = MutableRef::new(0u32);
    cell.set(41);
    cell.update(|v| *v += 1);
    assert_eq!(cell.get(), 42);

    let done: Arc<Deferred<&str, std::convert::Infallible>> = Arc::new(Deferred::new());
    let waiter = {
        let done = Arc::clone(&done);
        std::thread::spawn(move || done.wait())
    };
    assert!(done.complete("go"));
    assert!(!done.complete("again"));
    assert_eq!(waiter.join().unwrap(), "go");
}
