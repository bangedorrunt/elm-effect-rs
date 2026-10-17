//! `TxPriorityQueue` at the pin (`TxPriorityQueue.test.ts`): a sorted queue
//! under a supplied `Order`, with a galloping merge on the way in and a
//! blocking `take` where the pin retries the transaction.
//!
//! Rows bound here: :14 fromIterable creates a sorted queue, :21 the curried
//! call, :28 make from variadic args, :43 isEmpty/isNonEmpty, :53 peek,
//! :61/:68 peekOption, :80 offer+take order, :91 duplicate priorities,
//! :98 take on empty retries until offer, :107 takeOption, :114/:122 takeUpTo,
//! :130 offerAll inserts, :138/:156 offerAll does not rescan the queue,
//! :174 removeIf, :182 retainIf, :192 isTxPriorityQueue, :203 concurrent offer
//! and take, :220 reverse order (max-first).
#![cfg(feature = "stm")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use elm_effect::{Order, TxPriorityQueue, is_tx_priority_queue};

/// Ascending order over `i32`: the pin's `Order.Number`.
fn number_order() -> Order<i32> {
    Order::make(|a: &i32, b: &i32| a.cmp(b))
}

/// :14 — `fromIterable` yields the values in priority order.
#[test]
fn from_iterable_creates_a_sorted_queue() {
    let pq = TxPriorityQueue::from_iterable(number_order(), vec![3, 1, 2]);
    assert_eq!(pq.to_array(), vec![1, 2, 3]);
}

/// :21 — the curried call: the order first, the iterable second.
#[test]
fn from_iterable_supports_the_curried_call() {
    let pq = TxPriorityQueue::from_iterable_curried(number_order())(vec![3, 1, 2]);
    assert_eq!(pq.to_array(), vec![1, 2, 3]);
}

/// :28 — `make` from the values in hand.
#[test]
fn make_creates_from_the_values() {
    let pq = TxPriorityQueue::make(number_order(), vec![5, 3, 1, 4, 2]);
    assert_eq!(pq.to_array(), vec![1, 2, 3, 4, 5]);
}

/// :43 — `isEmpty` / `isNonEmpty` track the element count.
#[test]
fn is_empty_and_is_non_empty() {
    let pq = TxPriorityQueue::empty(number_order());
    assert!(pq.is_empty());
    assert!(!pq.is_non_empty());
    pq.offer(1);
    assert!(!pq.is_empty());
    assert!(pq.is_non_empty());
}

/// :53 — `peek` returns the smallest without removing it.
#[test]
fn peek_returns_the_smallest_without_removing() {
    let pq = TxPriorityQueue::from_iterable(number_order(), vec![3, 1, 2]);
    assert_eq!(pq.peek(), 1);
    assert_eq!(pq.size(), 3);
}

/// :61 — `peekOption` is `None` on an empty queue.
#[test]
fn peek_option_is_none_when_empty() {
    let pq = TxPriorityQueue::empty(number_order());
    assert_eq!(pq.peek_option(), None);
}

/// :68 — `peekOption` is `Some` on a non-empty queue.
#[test]
fn peek_option_is_some_when_non_empty() {
    let pq = TxPriorityQueue::from_iterable(number_order(), vec![3, 1]);
    assert_eq!(pq.peek_option(), Some(1));
}

/// :80 — offers come back in ascending priority order.
#[test]
fn offer_and_take_return_elements_in_priority_order() {
    let pq = TxPriorityQueue::empty(number_order());
    pq.offer(3);
    pq.offer(1);
    pq.offer(2);
    assert_eq!(pq.take(), 1);
    assert_eq!(pq.take(), 2);
    assert_eq!(pq.take(), 3);
}

/// :91 — duplicate priorities are all returned, ties in a stable order.
#[test]
fn duplicate_priorities_are_all_returned() {
    let pq = TxPriorityQueue::from_iterable(number_order(), vec![2, 1, 2, 1, 3]);
    assert_eq!(pq.take_all(), vec![1, 1, 2, 2, 3]);
}

/// :98 — `take` on an empty queue waits for an offer instead of failing. The
/// pin gets this from `Effect.txRetry`; here the wait is a condvar.
#[test]
fn take_on_empty_waits_until_offer() {
    let pq = Arc::new(TxPriorityQueue::empty(number_order()));
    let taker = {
        let pq = Arc::clone(&pq);
        std::thread::spawn(move || pq.take())
    };
    // The queue is empty by construction, so the taker is either already
    // parked or about to be; both paths must end with the offered value.
    assert!(pq.is_empty());
    pq.offer(42);
    assert_eq!(taker.join().expect("the taker must not panic"), 42);
}

/// :107 — `takeOption` is `None` on an empty queue and does not wait.
#[test]
fn take_option_is_none_when_empty() {
    let pq = TxPriorityQueue::empty(number_order());
    assert_eq!(pq.take_option(), None);
}

/// :114 — `takeUpTo` returns `min(n, size)` elements in order.
#[test]
fn take_up_to_returns_at_most_n_in_order() {
    let pq = TxPriorityQueue::from_iterable(number_order(), vec![5, 3, 1, 4, 2]);
    assert_eq!(pq.take_up_to(3), vec![1, 2, 3]);
    assert_eq!(pq.size(), 2);
}

/// :122 — `takeUpTo` past the size returns everything.
#[test]
fn take_up_to_past_the_size_returns_all() {
    let pq = TxPriorityQueue::from_iterable(number_order(), vec![2, 1]);
    assert_eq!(pq.take_up_to(10), vec![1, 2]);
    assert_eq!(pq.size(), 0);
}

/// :130 — `offerAll` inserts the whole batch.
#[test]
fn offer_all_inserts_multiple_elements() {
    let pq = TxPriorityQueue::empty(number_order());
    pq.offer_all(vec![3, 1, 2]);
    assert_eq!(pq.to_array(), vec![1, 2, 3]);
}

/// :138 — one offered value into a 128-element queue must not scan the queue:
/// the galloping merge brackets the insert point instead. The bound is the
/// row's own (< 32 comparisons), and the comparator counts them.
#[test]
fn offer_all_does_not_compare_every_queued_element() {
    let comparisons = Arc::new(AtomicUsize::new(0));
    let order = {
        let comparisons = Arc::clone(&comparisons);
        Order::make(move |a: &i32, b: &i32| {
            comparisons.fetch_add(1, AtomicOrdering::SeqCst);
            a.cmp(b)
        })
    };
    let queued: Vec<i32> = (0..128).map(|i| i * 2).collect();
    let pq = TxPriorityQueue::from_iterable(order, queued);
    comparisons.store(0, AtomicOrdering::SeqCst);
    pq.offer_all(vec![127]);
    let spent = comparisons.load(AtomicOrdering::SeqCst);
    assert!(spent < 32, "one offer cost {spent} comparisons, not < 32");

    let mut expected: Vec<i32> = (0..128).map(|i| i * 2).collect();
    expected.insert(64, 127);
    assert_eq!(pq.to_array(), expected);
}

/// :156 — a batch that interleaves with the whole queue is merged, not
/// re-searched per value. The bound is the row's own (< 900).
#[test]
fn offer_all_avoids_repeated_full_range_searches() {
    let comparisons = Arc::new(AtomicUsize::new(0));
    let order = {
        let comparisons = Arc::clone(&comparisons);
        Order::make(move |a: &i32, b: &i32| {
            comparisons.fetch_add(1, AtomicOrdering::SeqCst);
            a.cmp(b)
        })
    };
    let queued: Vec<i32> = (0..256).map(|i| i * 2).collect();
    let incoming: Vec<i32> = (0..256).map(|i| i * 2 + 1).collect();
    let pq = TxPriorityQueue::from_iterable(order, queued);
    comparisons.store(0, AtomicOrdering::SeqCst);
    pq.offer_all(incoming);
    let spent = comparisons.load(AtomicOrdering::SeqCst);
    assert!(
        spent < 900,
        "an interleaved 256-element batch cost {spent} comparisons, not < 900"
    );
    assert_eq!(pq.to_array(), (0..512).collect::<Vec<i32>>());
}

/// :174 — `removeIf` drops the matching elements.
#[test]
fn remove_if_removes_matching_elements() {
    let pq = TxPriorityQueue::from_iterable(number_order(), vec![1, 2, 3, 4, 5]);
    pq.remove_if(|n| n % 2 == 0);
    assert_eq!(pq.take_all(), vec![1, 3, 5]);
}

/// :182 — `retainIf` keeps only the matching elements.
#[test]
fn retain_if_keeps_only_matching_elements() {
    let pq = TxPriorityQueue::from_iterable(number_order(), vec![1, 2, 3, 4, 5]);
    pq.retain_if(|n| n % 2 == 0);
    assert_eq!(pq.take_all(), vec![2, 4]);
}

/// :192 — the guard. Foreign values are different Rust types, so the type
/// already narrows and the predicate is total over what can reach it.
#[test]
fn is_tx_priority_queue_narrows_the_type() {
    let pq = TxPriorityQueue::empty(number_order());
    assert!(is_tx_priority_queue(&pq));
}

/// :203 — three takes on one fiber against an offering producer: the takes
/// wait and then drain in priority order.
#[test]
fn concurrent_offer_and_take() {
    let pq = Arc::new(TxPriorityQueue::empty(number_order()));
    let taker = {
        let pq = Arc::clone(&pq);
        std::thread::spawn(move || vec![pq.take(), pq.take(), pq.take()])
    };
    pq.offer_all(vec![3, 1, 2]);
    assert_eq!(
        taker.join().expect("the taker must not panic"),
        vec![1, 2, 3]
    );
}

/// :220 — a custom order makes it a max-first queue.
#[test]
fn reverse_order_is_max_first() {
    let reverse = Order::make(|a: &i32, b: &i32| b.cmp(a));
    let pq = TxPriorityQueue::from_iterable(reverse, vec![1, 3, 2]);
    assert_eq!(pq.take(), 3);
    // Still max-first after one take: 2 precedes 1.
    assert_eq!(pq.to_array(), vec![2, 1]);
}
