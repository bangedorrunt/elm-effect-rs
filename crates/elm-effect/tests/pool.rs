//! Pool.test.ts's borrowing families: idle reuse, reservation as removal, and
//! a defective item leaving circulation.
//!
//! The pool's `reserve` takes a value out of shared circulation permanently —
//! dropping the guard shrinks the pool rather than returning the value — so a
//! second reservation over one item is not something this API can express.

#[path = "../src/cache.rs"]
mod cache;

use std::sync::{Arc, Mutex};

use cache::{IdlePolicy, Pool};

/// A pool of counters. The maker counts its calls, so a row can prove a value
/// was reused rather than remade.
fn counting_pool(max: usize) -> (Arc<Pool<u32>>, Arc<Mutex<usize>>) {
    let made = Arc::new(Mutex::new(0usize));
    let counting = Arc::clone(&made);
    let pool = Arc::new(Pool::new(max, IdlePolicy::default(), move || {
        let mut calls = counting.lock().unwrap();
        *calls += 1;
        *calls as u32
    }));
    (pool, made)
}

#[test]
fn acquire_reuses_an_idle_value_before_making_another() {
    // :25 — a dropped borrower returns its value, and the next acquire reuses
    // it rather than making a new one.
    let (pool, made) = counting_pool(2);
    let first = pool.acquire();
    drop(first);
    let second = pool.acquire();
    // The maker returns its call count, so the first value it made is 1.
    assert_eq!(*second, 1, "the returned value was reused");
    assert_eq!(*made.lock().unwrap(), 1, "the maker ran once");
}

#[test]
fn reserve_takes_the_item_out_of_circulation_permanently() {
    // :442 — reserving a slot of a one-item pool takes the item outright, so
    // the pool reports it in use with nothing idle left behind, and dropping
    // the guard does not put it back.
    let (pool, made) = counting_pool(1);
    let reserved = pool.reserve();
    assert_eq!(pool.in_use(), 1);
    assert_eq!(pool.idle_count(), 0);
    drop(reserved);
    assert_eq!(pool.in_use(), 0, "the reserved item did not return");
    assert_eq!(pool.idle_count(), 0);
    let fresh = pool.acquire();
    assert_eq!(*made.lock().unwrap(), 2, "a new item was made");
    drop(fresh);
}

#[test]
fn invalidate_drops_a_broken_item_without_returning_it() {
    // :50 — a defective borrower discards its value instead of circulating it,
    // so the next acquire remakes rather than reusing.
    let (pool, made) = counting_pool(1);
    let broken = pool.acquire();
    broken.invalidate();
    assert_eq!(pool.in_use(), 0);
    let fresh = pool.acquire();
    assert_eq!(*made.lock().unwrap(), 2, "the invalidated item was remade");
    drop(fresh);
}
