//! `TxReentrantLock.test.ts` fence: read/write reentrancy, scoped holds,
//! with-lock releases, counters, and cross-fiber blocking.
//!
//! Ownership is per thread (this crate's fiber is a thread), so the pin's
//! `fiberId` rows are written with real threads. The pin suspends a waiter
//! through `txRetry`; here a waiter parks on a condvar, which is why a blocked
//! acquire cannot be interrupted (noted on the type itself).

use elm_effect::{Effect, Scope, TxReentrantLock};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Wait for a sibling thread to reach the point just before a blocking
/// acquire, with a ceiling that fails instead of hanging the suite.
fn spin_until(condition: impl Fn() -> bool) -> bool {
    for _ in 0..2_000 {
        if condition() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    false
}

#[test]
fn tx_reentrant_lock_starts_unlocked() {
    // :6 make creates an unlocked lock.
    let lock = TxReentrantLock::new();
    assert!(!lock.locked());
    assert!(!lock.read_locked());
    assert!(!lock.write_locked());
    assert_eq!(lock.read_locks(), 0);
    assert_eq!(lock.write_locks(), 0);
}

#[test]
fn tx_reentrant_lock_read_acquires_and_releases() {
    // :18 acquireRead and releaseRead work correctly.
    let lock = TxReentrantLock::new();
    assert_eq!(lock.acquire_read(), 1);
    assert!(lock.read_locked());
    assert_eq!(lock.read_locks(), 1);
    assert_eq!(lock.release_read(), 0);
    assert!(!lock.read_locked());
}

#[test]
fn tx_reentrant_lock_read_is_reentrant() {
    // :31 multiple read acquires from same fiber are reentrant.
    let lock = TxReentrantLock::new();
    assert_eq!(lock.acquire_read(), 1);
    assert_eq!(lock.acquire_read(), 2);
    assert_eq!(lock.read_locks(), 2);
    assert_eq!(lock.release_read(), 1);
    assert_eq!(lock.read_locks(), 1);
    assert_eq!(lock.release_read(), 0);
    assert_eq!(lock.read_locks(), 0);
}

#[test]
fn tx_reentrant_lock_write_acquires_and_releases() {
    // :48 acquireWrite and releaseWrite work correctly.
    let lock = TxReentrantLock::new();
    assert_eq!(lock.acquire_write(), 1);
    assert!(lock.write_locked());
    assert_eq!(lock.write_locks(), 1);
    assert_eq!(lock.release_write(), 0);
    assert!(!lock.write_locked());
}

#[test]
fn tx_reentrant_lock_write_is_reentrant() {
    // :61 multiple write acquires from same fiber are reentrant.
    let lock = TxReentrantLock::new();
    assert_eq!(lock.acquire_write(), 1);
    assert_eq!(lock.acquire_write(), 2);
    assert_eq!(lock.write_locks(), 2);
    assert_eq!(lock.release_write(), 1);
    assert_eq!(lock.write_locks(), 1);
    assert_eq!(lock.release_write(), 0);
    assert_eq!(lock.write_locks(), 0);
}

#[test]
fn tx_reentrant_lock_writer_also_reads() {
    // :76 write lock holder can acquire read lock (reentrancy). The upgrade
    // keeps both counters: dropping the write does not drop the read.
    let lock = TxReentrantLock::new();
    assert_eq!(lock.acquire_write(), 1);
    assert_eq!(lock.acquire_read(), 1);
    assert!(lock.locked());
    assert_eq!(lock.release_read(), 0);
    assert_eq!(lock.release_write(), 0);
    assert!(!lock.locked());
}

#[test]
fn tx_reentrant_lock_scoped_read_releases_on_close() {
    // :91 readLock scoped acquires and releases.
    let lock = Arc::new(TxReentrantLock::new());
    let scope = Scope::new(1);
    assert_eq!(lock.read_lock(&scope).unwrap(), 1);
    assert!(lock.read_locked());
    scope.close();
    assert!(!lock.read_locked());
}

#[test]
fn tx_reentrant_lock_scoped_write_releases_on_close() {
    // :105 writeLock scoped acquires and releases.
    let lock = Arc::new(TxReentrantLock::new());
    let scope = Scope::new(2);
    assert_eq!(lock.write_lock(&scope).unwrap(), 1);
    assert!(lock.write_locked());
    scope.close();
    assert!(!lock.write_locked());
}

#[test]
fn tx_reentrant_lock_scoped_write_is_reentrant() {
    // :119 writeLock can be acquired reentrantly in the same scope.
    let lock = Arc::new(TxReentrantLock::new());
    let scope = Scope::new(3);
    assert_eq!(lock.write_lock(&scope).unwrap(), 1);
    assert_eq!(lock.write_lock(&scope).unwrap(), 2);
    assert_eq!(lock.write_locks(), 2);
    scope.close();
    assert_eq!(lock.write_locks(), 0);
}

#[test]
fn tx_reentrant_lock_with_read_runs_and_releases() {
    // :135 withReadLock runs effect and releases.
    let lock = TxReentrantLock::new();
    let seen = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&seen);
    let held = Arc::new(lock);
    let result = held.with_read_lock(Effect::<u32, String>::sync(move || {
        observed.store(true, Ordering::SeqCst);
        7
    }));
    assert_eq!(result.ok(), Some(7));
    assert!(seen.load(Ordering::SeqCst));
    assert!(!held.read_locked());
}

#[test]
fn tx_reentrant_lock_with_write_runs_and_releases() {
    // :151 withWriteLock runs effect and releases.
    let lock = TxReentrantLock::new();
    let seen = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&seen);
    let held = Arc::new(lock);
    let result = held.with_write_lock(Effect::<u32, String>::sync(move || {
        observed.store(true, Ordering::SeqCst);
        9
    }));
    assert_eq!(result.ok(), Some(9));
    assert!(seen.load(Ordering::SeqCst));
    assert!(!held.write_locked());
}

#[test]
fn tx_reentrant_lock_with_lock_aliases_write() {
    // :167 withLock is alias for withWriteLock.
    let lock = TxReentrantLock::new();
    let held = Arc::new(lock);
    let result = held.with_lock(Effect::<&str, String>::succeed("exclusive"));
    assert_eq!(result.ok(), Some("exclusive"));
    assert!(!held.locked());
}

#[test]
fn tx_reentrant_lock_with_read_releases_on_failure() {
    // :183 withReadLock releases on failure.
    let lock = TxReentrantLock::new();
    let result = lock.with_read_lock(Effect::<u32, String>::fail("boom".to_string()));
    assert!(result.is_err());
    assert!(!lock.read_locked());
}

#[test]
fn tx_reentrant_lock_with_write_releases_on_failure() {
    // :198 withWriteLock releases on failure.
    let lock = TxReentrantLock::new();
    let result = lock.with_write_lock(Effect::<u32, String>::fail("boom".to_string()));
    assert!(result.is_err());
    assert!(!lock.write_locked());
}

#[test]
fn tx_reentrant_lock_readers_run_concurrently() {
    // :239 multiple readers can proceed concurrently.
    let lock = Arc::new(TxReentrantLock::new());
    let results: Vec<u32> = std::thread::scope(|s| {
        let handles: Vec<_> = [1u32, 2, 3]
            .into_iter()
            .map(|value| {
                let lock = Arc::clone(&lock);
                s.spawn(move || {
                    lock.with_read_lock(Effect::<u32, String>::succeed(value))
                        .unwrap()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(results, vec![1, 2, 3]);
    assert!(!lock.locked());
}

#[test]
fn tx_reentrant_lock_a_writer_blocks_another_writer() {
    // :265 writer blocks other writers.
    let lock = Arc::new(TxReentrantLock::new());
    let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let started = Arc::new(AtomicBool::new(false));
    lock.acquire_write();
    std::thread::scope(|s| {
        let child_lock = Arc::clone(&lock);
        let child_order = Arc::clone(&order);
        let child_started = Arc::clone(&started);
        let child = s.spawn(move || {
            child_started.store(true, Ordering::SeqCst);
            child_lock
                .with_write_lock(Effect::<(), String>::succeed(()))
                .unwrap();
            child_order.lock().unwrap().push("child-wrote");
        });
        assert!(spin_until(|| started.load(Ordering::SeqCst)));
        order.lock().unwrap().push("parent-releasing");
        lock.release_write();
        child.join().unwrap();
    });
    assert_eq!(
        &*order.lock().unwrap(),
        &["parent-releasing", "child-wrote"]
    );
    assert!(!lock.locked());
}

#[test]
fn tx_reentrant_lock_a_writer_blocks_a_reader() {
    // :290 writer blocks readers from other fibers.
    let lock = Arc::new(TxReentrantLock::new());
    let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let started = Arc::new(AtomicBool::new(false));
    lock.acquire_write();
    std::thread::scope(|s| {
        let child_lock = Arc::clone(&lock);
        let child_order = Arc::clone(&order);
        let child_started = Arc::clone(&started);
        let child = s.spawn(move || {
            child_started.store(true, Ordering::SeqCst);
            child_lock
                .with_read_lock(Effect::<(), String>::succeed(()))
                .unwrap();
            child_order.lock().unwrap().push("child-read");
        });
        assert!(spin_until(|| started.load(Ordering::SeqCst)));
        order.lock().unwrap().push("parent-releasing");
        lock.release_write();
        child.join().unwrap();
    });
    assert_eq!(&*order.lock().unwrap(), &["parent-releasing", "child-read"]);
    assert!(!lock.locked());
}

#[test]
fn tx_reentrant_lock_a_reader_blocks_a_writer() {
    // :314 reader blocks writer from another fiber.
    let lock = Arc::new(TxReentrantLock::new());
    let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let started = Arc::new(AtomicBool::new(false));
    lock.acquire_read();
    std::thread::scope(|s| {
        let child_lock = Arc::clone(&lock);
        let child_order = Arc::clone(&order);
        let child_started = Arc::clone(&started);
        let child = s.spawn(move || {
            child_started.store(true, Ordering::SeqCst);
            child_lock
                .with_write_lock(Effect::<(), String>::succeed(()))
                .unwrap();
            child_order.lock().unwrap().push("child-wrote");
        });
        assert!(spin_until(|| started.load(Ordering::SeqCst)));
        order.lock().unwrap().push("parent-releasing");
        lock.release_read();
        child.join().unwrap();
    });
    assert_eq!(
        &*order.lock().unwrap(),
        &["parent-releasing", "child-wrote"]
    );
    assert!(!lock.locked());
}

#[test]
fn tx_reentrant_lock_read_locks_count_total() {
    // :340 readLocks counts total read locks.
    let lock = TxReentrantLock::new();
    lock.acquire_read();
    lock.acquire_read();
    assert_eq!(lock.read_locks(), 2);
    lock.release_read();
    assert_eq!(lock.read_locks(), 1);
    lock.release_read();
    assert_eq!(lock.read_locks(), 0);
}

#[test]
fn tx_reentrant_lock_write_locks_count_depth() {
    // :354 writeLocks counts write lock depth.
    let lock = TxReentrantLock::new();
    assert_eq!(lock.write_locks(), 0);
    lock.acquire_write();
    assert_eq!(lock.write_locks(), 1);
    lock.acquire_write();
    assert_eq!(lock.write_locks(), 2);
    lock.release_write();
    lock.release_write();
    assert_eq!(lock.write_locks(), 0);
}

#[test]
fn tx_reentrant_lock_locked_sees_either_kind() {
    // :370 locked returns true when any lock is held.
    let lock = TxReentrantLock::new();
    assert!(!lock.locked());
    lock.acquire_read();
    assert!(lock.locked());
    lock.release_read();
    assert!(!lock.locked());
    lock.acquire_write();
    assert!(lock.locked());
    lock.release_write();
    assert!(!lock.locked());
}

#[test]
fn tx_reentrant_lock_release_read_without_acquire_is_zero() {
    // :399 releaseRead without acquire returns 0.
    let lock = TxReentrantLock::new();
    assert_eq!(lock.release_read(), 0);
}

#[test]
fn tx_reentrant_lock_release_write_without_acquire_is_zero() {
    // :406 releaseWrite without acquire returns 0.
    let lock = TxReentrantLock::new();
    assert_eq!(lock.release_write(), 0);
}

#[test]
fn tx_reentrant_lock_a_scope_closed_elsewhere_releases_its_read() {
    // :414 releases a scoped read lock when another fiber closes the scope.
    // The finalizer carries the acquiring thread's id, so the closing thread
    // frees the entry the acquirer made, not one of its own.
    let lock = Arc::new(TxReentrantLock::new());
    let scope = Arc::new(Scope::new(4));
    std::thread::scope(|s| {
        let child_lock = Arc::clone(&lock);
        let child_scope = Arc::clone(&scope);
        s.spawn(move || {
            child_lock.read_lock(&child_scope).unwrap();
        })
        .join()
        .unwrap();
    });
    assert!(lock.read_locked());
    scope.close();
    assert!(!lock.read_locked());
}

#[test]
fn tx_reentrant_lock_a_scope_closed_elsewhere_releases_its_write() {
    // :427 releases a scoped write lock when another fiber closes the scope.
    let lock = Arc::new(TxReentrantLock::new());
    let scope = Arc::new(Scope::new(5));
    std::thread::scope(|s| {
        let child_lock = Arc::clone(&lock);
        let child_scope = Arc::clone(&scope);
        s.spawn(move || {
            child_lock.write_lock(&child_scope).unwrap();
        })
        .join()
        .unwrap();
    });
    assert!(lock.write_locked());
    scope.close();
    assert!(!lock.write_locked());
}
