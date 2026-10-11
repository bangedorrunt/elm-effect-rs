//! STM core fence: transactional cells, queues, maps and friends.
//! Behaviors mirror packages/effect/test/Tx*.test.ts (composable,
//! atomic, isolated multi-op blocks; blocking-retry is a residual).

use elm_effect::{
    Deferred, Effect, Exit, ForkOpts, Runtime, TxChunk, TxDeferred, TxHashMap, TxHashSet,
    TxPriorityQueue, TxPubSub, TxQueue, TxReentrantLock, TxRef, TxSemaphore, TxSubscriptionRef,
    atomically, is_tx_chunk, is_tx_hash_set, is_tx_semaphore,
};

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Bounded wait for a condition, the port's stand-in for the pin's
/// cooperative `Effect.repeat(Effect.yieldNow, { times: n })`: the pin
/// yields to let the other fiber advance, this port's fibers are threads,
/// so the test waits on the row's own counter with a ceiling that fails
/// loudly instead of hanging.
fn spin_until(condition: impl Fn() -> bool) -> bool {
    for _ in 0..50_000 {
        if condition() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    false
}

#[test]
fn tx_ref_read_write_update() {
    let cell = TxRef::new(1);
    assert_eq!(cell.read(), 1);
    cell.write(2);
    assert_eq!(cell.read(), 2);
    let doubled = cell.modify(|n| {
        *n *= 2;
        *n
    });
    assert_eq!(doubled, 4);
    assert_eq!(cell.read(), 4);
}

#[test]
fn atomically_composes_two_refs() {
    let a = TxRef::new(1);
    let b = TxRef::new(10);
    atomically(|| {
        let x = a.read();
        let y = b.read();
        a.write(x + y);
        b.write(x * y);
    });
    assert_eq!(a.read(), 11);
    assert_eq!(b.read(), 10);
}

#[test]
fn tx_deferred_complete_and_poll() {
    let gate = TxDeferred::<i32>::new();
    assert_eq!(gate.poll(), None);
    assert!(gate.complete(7));
    assert!(!gate.complete(8));
    assert_eq!(gate.poll(), Some(7));
}

#[test]
fn tx_queue_bounded_offer_take() {
    let queue = TxQueue::bounded(2);
    assert!(queue.offer(1));
    assert!(queue.offer(2));
    assert!(!queue.offer(3));
    assert_eq!(queue.size(), 2);
    assert_eq!(queue.take(), Some(1));
    assert_eq!(queue.take(), Some(2));
    assert_eq!(queue.take(), None);
}

#[test]
fn tx_queue_unbounded_never_full() {
    let queue = TxQueue::<i32>::unbounded();
    for i in 0..100 {
        assert!(queue.offer(i));
    }
    assert_eq!(queue.size(), 100);
}

#[test]
fn tx_semaphore_permits() {
    let sem = TxSemaphore::new(2);
    assert_eq!(sem.available(), 2);
    assert!(sem.try_acquire());
    assert!(sem.try_acquire());
    assert!(!sem.try_acquire());
    sem.release();
    assert_eq!(sem.available(), 1);
}

#[test]
fn tx_lock_is_reentrant() {
    let lock = TxReentrantLock::new();
    lock.lock();
    lock.lock();
    assert_eq!(lock.holds(), 2);
    lock.unlock();
    lock.unlock();
    assert_eq!(lock.holds(), 0);
}

#[test]
fn tx_chunk_push_get_extend() {
    let chunk = TxChunk::new();
    assert!(chunk.is_empty());
    chunk.push(1);
    chunk.extend(vec![2, 3]);
    assert_eq!(chunk.len(), 3);
    assert_eq!(chunk.get(0), Some(1));
    assert_eq!(chunk.get(5), None);
}

#[test]
fn tx_hash_map_crud() {
    let map = TxHashMap::new();
    map.insert("a".to_string(), 1);
    assert_eq!(map.get(&"a".to_string()), Some(1));
    assert_eq!(map.len(), 1);
    assert_eq!(map.remove(&"a".to_string()), Some(1));
    assert_eq!(map.len(), 0);
}

#[test]
fn tx_hash_set_add_contains() {
    let set = TxHashSet::new();
    assert!(set.insert(1));
    assert!(!set.insert(1));
    assert!(set.contains(&1));
    assert_eq!(set.len(), 1);
}

#[test]
fn tx_hash_set_make_remove_clear_size_flags() {
    // TxHashSet.test.ts:32/44/52/63 make/fromIterable/fromHashSet,
    // 92 add, 105 remove, 120 has, 128 size, 140 isEmpty/isNonEmpty,
    // 151 clear, 77 isTxHashSet.
    use std::collections::HashSet;
    let set = TxHashSet::from_iter([1, 2, 2, 3]);
    assert_eq!(set.len(), 3);
    assert!(set.contains(&1));
    assert!(!set.contains(&9));
    assert!(set.remove(&2));
    assert!(!set.remove(&2));
    assert!(!set.is_empty());
    set.clear();
    assert_eq!(set.len(), 0);
    assert!(set.is_empty());
    let std_set: HashSet<i32> = HashSet::from([4, 5]);
    let from_std = TxHashSet::from_iter(std_set);
    assert_eq!(from_std.len(), 2);
    assert!(is_tx_hash_set(&from_std));
    assert!(is_tx_hash_set(&TxHashSet::<i32>::new()));
}

#[test]
fn tx_priority_queue_orders() {
    let pq = TxPriorityQueue::new();
    pq.push(3);
    pq.push(1);
    pq.push(2);
    assert_eq!(pq.pop(), Some(3));
    assert_eq!(pq.pop(), Some(2));
    assert_eq!(pq.len(), 1);
}

#[test]
fn tx_pubsub_broadcasts() {
    let bus = TxPubSub::<i32>::new(8);
    bus.publish(1);
    bus.publish(2);
    assert_eq!(bus.drain(), vec![1, 2]);
}

#[test]
fn tx_pubsub_constructors_set_capacity_and_strategy() {
    // TxPubSub.test.ts:13/19/25 — unbounded/dropping/sliding.
    assert_eq!(TxPubSub::<i32>::unbounded().capacity(), None);
    assert_eq!(TxPubSub::<i32>::dropping(4).capacity(), Some(4));
    assert_eq!(TxPubSub::<i32>::sliding(4).capacity(), Some(4));
    assert_eq!(TxPubSub::<i32>::bounded(16).capacity(), Some(16));
    let sliding = TxPubSub::sliding(2);
    sliding.publish(1);
    sliding.publish(2);
    sliding.publish(3);
    assert_eq!(sliding.drain(), vec![2, 3]);
    let dropping = TxPubSub::dropping(2);
    dropping.publish(1);
    dropping.publish(2);
    dropping.publish(3);
    assert_eq!(dropping.drain(), vec![1, 2]);
    let open = TxPubSub::<i32>::unbounded();
    open.publish(1);
    open.publish(2);
    assert_eq!(open.drain(), vec![1, 2]);
}

#[test]
fn tx_subscription_ref_notifies() {
    let watched = TxSubscriptionRef::new(0);
    assert_eq!(watched.get(), 0);
    watched.set(5);
    assert_eq!(watched.get(), 5);
    assert_eq!(watched.changes(), vec![0, 5]);
}

#[test]
fn tx_hash_map_contains_and_from_iter() {
    let map = TxHashMap::from_iter([("a", 1), ("b", 2)]);
    assert!(map.contains(&"a"));
    assert!(!map.contains(&"z"));
    assert_eq!(map.insert("a", 10), Some(1));
    assert_eq!(map.remove(&"b"), Some(2));
    assert_eq!(map.remove(&"b"), None);
}

#[test]
fn tx_hash_map_has_set_remove_clear_size_flags() {
    // TxHashMap.test.ts:65/75 has, 83/95 set, 107/120 remove,
    // 131 clear, 145 size, 156/165 isEmpty/isNonEmpty, 30 fromIterable.
    let map = TxHashMap::from_iter([("a", 1), ("b", 2)]);
    assert!(map.contains(&"a"));
    assert!(!map.contains(&"z"));
    assert_eq!(map.insert("c", 3), None);
    assert_eq!(map.insert("a", 10), Some(1));
    assert_eq!(map.len(), 3);
    assert!(!map.is_empty());
    assert_eq!(map.remove(&"b"), Some(2));
    assert_eq!(map.remove(&"b"), None);
    map.clear();
    assert_eq!(map.len(), 0);
    assert!(map.is_empty());
    assert!(!map.contains(&"a"));
}

#[test]
fn tx_hash_set_from_iter_dedups() {
    let set = TxHashSet::from_iter([1, 2, 2, 3]);
    assert_eq!(set.len(), 3);
    assert!(set.contains(&2));
}

#[test]
fn tx_chunk_from_iter_prepend_set_all() {
    let chunk = TxChunk::from_iter([2, 3]);
    chunk.prepend(1);
    assert_eq!(chunk.get(0), Some(1));
    assert_eq!(chunk.len(), 3);
    chunk.set_all(vec![9]);
    assert_eq!(chunk.get(0), Some(9));
    assert_eq!(chunk.len(), 1);
}

#[test]
fn tx_chunk_guard_identifies_chunks() {
    // TxChunk.test.ts:344 — the guard matches chunk instances.
    // Foreign values are different types and cannot reach the predicate.
    assert!(is_tx_chunk(&TxChunk::from_iter([1, 2])));
    assert!(is_tx_chunk(&TxChunk::<i32>::new()));
}

#[test]
fn tx_semaphore_guard_identifies_semaphores() {
    // TxSemaphore.test.ts:224 — the guard matches semaphore instances.
    assert!(is_tx_semaphore(&TxSemaphore::new(1)));
    assert!(is_tx_semaphore(&TxSemaphore::new(0)));
}

#[test]
fn tx_queue_poll_matches_take() {
    let queue = TxQueue::unbounded();
    assert_eq!(queue.poll(), None);
    queue.offer(1);
    assert_eq!(queue.poll(), Some(1));
    assert!(queue.is_empty());
}

#[test]
fn tx_queue_dropping_keeps_first_when_full() {
    let queue = TxQueue::dropping(1);
    assert!(queue.offer(1));
    assert!(queue.offer(2));
    assert_eq!(queue.take(), Some(1));
    assert_eq!(queue.take(), None);
}

#[test]
fn tx_queue_sliding_evicts_oldest_when_full() {
    let queue = TxQueue::sliding(1);
    queue.offer(1);
    queue.offer(2);
    assert_eq!(queue.take(), Some(2));
    assert_eq!(queue.take(), None);
}

#[test]
fn tx_queue_close_drains_then_stops() {
    let queue = TxQueue::unbounded();
    queue.offer(1);
    queue.close();
    assert!(!queue.offer(2));
    assert_eq!(queue.take(), Some(1));
    assert_eq!(queue.take(), None);
    assert_eq!(queue.poll(), None);
}

#[test]
fn tx_queue_offer_all_then_take_all() {
    let queue = TxQueue::unbounded();
    queue.offer_all(vec![1, 2, 3]);
    assert_eq!(queue.take_all(), vec![1, 2, 3]);
    assert!(queue.take_all().is_empty());
}

#[test]
fn tx_queue_take_n_takes_up_to_n() {
    let queue = TxQueue::unbounded();
    queue.offer_all(vec![1, 2, 3]);
    assert_eq!(queue.take_n(2), vec![1, 2]);
    assert_eq!(queue.take_n(5), vec![3]);
    assert_eq!(queue.take_n(0), Vec::<i32>::new());
}

#[test]
fn tx_queue_peek_does_not_remove() {
    let queue = TxQueue::unbounded();
    assert_eq!(queue.peek(), None);
    queue.offer(1);
    assert_eq!(queue.peek(), Some(1));
    assert_eq!(queue.take(), Some(1));
}

#[test]
fn tx_queue_take_between_min_and_max() {
    let queue = TxQueue::unbounded();
    queue.offer_all(vec![1, 2, 3]);
    assert_eq!(queue.take_between(1, 2), vec![1, 2]);
    assert!(queue.take_between(5, 9).is_empty());
    assert!(queue.take_between(2, 1).is_empty());
}

#[test]
fn tx_enqueue_provides_write_only_interface() {
    let queue = TxQueue::bounded(5);
    let enqueue = queue.as_enqueue();
    assert!(enqueue.offer(42));
    assert_eq!(enqueue.offer_all([1, 2, 3]), Vec::<i32>::new());
    assert_eq!(queue.take_all(), vec![42, 1, 2, 3]);
}

#[test]
fn tx_dequeue_provides_read_only_interface() {
    let queue = TxQueue::bounded(5);
    queue.offer_all([1, 2, 3]);
    let dequeue = queue.as_dequeue();
    assert_eq!(dequeue.take(), Some(1));
    assert_eq!(dequeue.poll(), Some(2));
    assert_eq!(queue.take(), Some(3));
}

#[test]
fn tx_queue_serves_both_faces() {
    let queue = TxQueue::bounded(5);
    queue.as_enqueue().offer(42);
    assert_eq!(queue.as_dequeue().take(), Some(42));
    assert!(queue.is_open());
}

#[test]
fn tx_enqueue_shutdown_ops() {
    let queue = TxQueue::<i32>::bounded(5);
    let enqueue = queue.as_enqueue();
    assert!(enqueue.offer(42));
    assert_eq!(enqueue.offer_all([1, 2, 3]), Vec::<i32>::new());
    assert!(enqueue.fail());
    assert!(!enqueue.end());
    assert!(!queue.is_open());
}

#[test]
fn tx_dequeue_inspection_ops() {
    let queue = TxQueue::bounded(5);
    queue.offer_all([1, 2, 3]);
    let dequeue = queue.as_dequeue();
    assert_eq!(dequeue.take(), Some(1));
    assert_eq!(dequeue.poll(), Some(2));
    assert_eq!(dequeue.peek(), Some(3));
    assert_eq!(dequeue.size(), 1);
    assert!(!dequeue.is_empty());
    assert!(dequeue.is_open());
}

#[test]
fn tx_semaphore_zero_permits_starts_empty() {
    let sem = TxSemaphore::new(0);
    assert_eq!(sem.available(), 0);
    assert!(!sem.try_acquire());
}

#[test]
fn tx_semaphore_acquire_n_and_release_n() {
    let sem = TxSemaphore::new(3);
    assert!(sem.acquire_n(2));
    assert_eq!(sem.available(), 1);
    sem.release_n(2);
    assert_eq!(sem.available(), 3);
}

#[test]
fn tx_semaphore_try_acquire_n_needs_enough() {
    let sem = TxSemaphore::new(2);
    assert!(sem.try_acquire_n(2));
    assert!(!sem.try_acquire_n(1));
}

#[test]
fn tx_semaphore_with_permit_manages_lifecycle() {
    let sem = TxSemaphore::new(1);
    let ran = sem.with_permit(|| 42);
    assert_eq!(ran, Some(42));
    assert_eq!(sem.available(), 1);
    sem.try_acquire();
    assert_eq!(sem.with_permit(|| 1), None);
    assert_eq!(sem.available(), 0);
}

#[test]
fn tx_semaphore_with_permits_manages_many() {
    let sem = TxSemaphore::new(3);
    assert_eq!(sem.with_permits(2, || "held"), Some("held"));
    assert_eq!(sem.available(), 3);
}

#[test]
#[should_panic(expected = "positive count")]
fn tx_semaphore_acquire_n_zero_defects() {
    TxSemaphore::new(1).acquire_n(0);
}

#[test]
#[should_panic(expected = "positive count")]
fn tx_semaphore_release_n_zero_defects() {
    TxSemaphore::new(1).release_n(0);
}

#[test]
fn tx_semaphore_release_caps_at_capacity() {
    let sem = TxSemaphore::new(2);
    sem.release();
    assert_eq!(sem.available(), 2);
    sem.release_n(5);
    assert_eq!(sem.available(), 2);
    assert!(sem.acquire_n(2));
    assert_eq!(sem.available(), 0);
}

#[test]
fn tx_semaphore_ops_stay_atomic_in_transactions() {
    let sem = TxSemaphore::new(2);
    atomically(|| {
        assert!(sem.acquire_n(2));
        assert_eq!(sem.available(), 0);
        sem.release_n(2);
    });
    assert_eq!(sem.available(), 2);
}

#[test]
fn tx_chunk_from_iterable_and_size_flags() {
    let chunk = TxChunk::from_iterable([1, 2, 3]);
    assert_eq!(chunk.to_vec(), vec![1, 2, 3]);
    assert_eq!(chunk.len(), 3);
    assert!(!chunk.is_empty());
    assert!(chunk.is_non_empty());
    let empty: TxChunk<i32> = TxChunk::new();
    assert!(!empty.is_non_empty());
}

#[test]
fn tx_chunk_modify_returns_previous_size() {
    let chunk = TxChunk::from_iterable([1, 2, 3]);
    let old_size = chunk.modify(|values| {
        let size = values.len();
        values.push(4);
        size
    });
    assert_eq!(old_size, 3);
    assert_eq!(chunk.to_vec(), vec![1, 2, 3, 4]);
}

#[test]
fn tx_chunk_update_maps_values() {
    let chunk = TxChunk::from_iterable([1, 2, 3]);
    chunk.update(|values| values.iter().map(|x| x * 2).collect());
    assert_eq!(chunk.to_vec(), vec![2, 4, 6]);
}

#[test]
fn tx_chunk_slice_operations() {
    let chunk = TxChunk::from_iterable([1, 2, 3, 4, 5]);
    chunk.take(3);
    assert_eq!(chunk.to_vec(), vec![1, 2, 3]);
    let chunk = TxChunk::from_iterable([1, 2, 3, 4, 5]);
    chunk.drop(2);
    assert_eq!(chunk.to_vec(), vec![3, 4, 5]);
    let chunk = TxChunk::from_iterable([1, 2, 3, 4, 5]);
    chunk.slice(1, 4);
    assert_eq!(chunk.to_vec(), vec![2, 3, 4]);
}

#[test]
fn tx_chunk_map_and_filter() {
    let chunk = TxChunk::from_iterable([1, 2, 3]);
    chunk.map(|x| x + 10);
    assert_eq!(chunk.to_vec(), vec![11, 12, 13]);
    chunk.filter(|x| *x % 2 == 1);
    assert_eq!(chunk.to_vec(), vec![11, 13]);
}

#[test]
fn tx_chunk_concat_appends() {
    let left = TxChunk::from_iterable([1, 2]);
    let right = TxChunk::from_iterable([3, 4]);
    left.concat(&right);
    assert_eq!(left.to_vec(), vec![1, 2, 3, 4]);
    assert_eq!(right.to_vec(), vec![3, 4]);
}

#[test]
fn tx_chunk_append_and_prepend_all() {
    let left = TxChunk::from_iterable([3]);
    let other = TxChunk::from_iterable([1, 2]);
    left.append_all(&other);
    assert_eq!(left.to_vec(), vec![3, 1, 2]);
    let head = TxChunk::from_iterable([1, 2]);
    head.prepend_all(&TxChunk::from_iterable([9]));
    assert_eq!(head.to_vec(), vec![9, 1, 2]);
}

#[test]
fn tx_chunk_ops_stay_atomic_in_a_transaction() {
    let chunk = TxChunk::from_iterable([1, 2, 3]);
    atomically(|| {
        chunk.update(|values| values.iter().map(|x| x * 3).collect());
        chunk.prepend(0);
        assert_eq!(chunk.to_vec(), vec![0, 3, 6, 9]);
    });
    assert_eq!(chunk.to_vec(), vec![0, 3, 6, 9]);
}

// ---------------------------------------------------------------------------
// TxSubscriptionRef read-then-write at the pin
// ---------------------------------------------------------------------------

#[test]
fn tx_subscription_ref_modify_returns_value_and_updates() {
    let cell = TxSubscriptionRef::new(10u32);
    let result = cell.modify(|current| (format!("was {current}"), current + 1));
    assert_eq!(result, "was 10");
    assert_eq!(cell.get(), 11);
}

#[test]
fn tx_subscription_ref_get_and_set_returns_old_value() {
    let cell = TxSubscriptionRef::new("a");
    assert_eq!(cell.get_and_set("b"), "a");
    assert_eq!(cell.get(), "b");
}

#[test]
fn tx_subscription_ref_get_and_update_returns_old_and_applies() {
    let cell = TxSubscriptionRef::new(1u32);
    assert_eq!(cell.get_and_update(|n| n + 10), 1);
    assert_eq!(cell.get(), 11);
}

#[test]
fn tx_subscription_ref_update_and_get_returns_new() {
    let cell = TxSubscriptionRef::new(1u32);
    assert_eq!(cell.update_and_get(|n| n + 10), 11);
    assert_eq!(cell.get(), 11);
}

// ---------------------------------------------------------------------------
// Effect.tx at the pin (Effect.test.ts describe "transaction
// (composable transactions)", basic behavior / nesting / composition:
// :3779-:3918)
//
// The outer boundary stages writes on each touched ref; they publish only
// when the boundary's body succeeds. A nested `Effect.tx` composes into
// the running boundary (no inner boundary), so a failure caught inside
// the body is an ordinary value, and only a failure that escapes the
// boundary discards the journal.
// ---------------------------------------------------------------------------

#[test]
fn separate_tx_boundaries_each_commit_their_own_writes() {
    // Effect.test.ts :3779 "should create transaction boundaries when no
    // transaction is active" — three top-level `Effect.tx` calls are three
    // boundaries; each commits, and later reads see committed values.
    let ref1 = TxRef::new(0);
    let ref2 = TxRef::new(100);
    // One handle per `tx` body: descriptions are single-shot, so the
    // boundary's factory hands each attempt its own handle.
    // (`TxRef::clone` shares the cell rather than copying it.)
    let (outer, set2, read1, read2) = (ref1.clone(), ref2.clone(), ref1.clone(), ref2.clone());
    let program: Effect<(i32, i32), std::convert::Infallible> = Effect::tx(move || {
        let (set2, read1, read2) = (set2.clone(), read1.clone(), read2.clone());
        outer
            .set_effect(10)
            .flat_map(move |_| Effect::tx(move || set2.set_effect(200)))
            .flat_map(move |_| Effect::tx(move || read1.get_effect()))
            .flat_map(move |v1| Effect::tx(move || read2.get_effect()).map(move |v2| (v1, v2)))
    });
    assert_eq!(Runtime::run(program), Exit::Success((10, 200)));
}

#[test]
fn tx_ref_modify_outside_a_transaction_is_one_atomic_step() {
    // Effect.test.ts :3795 "should allow TxRef.modify outside an existing
    // transaction" — the pin's pair form returns the computed result and
    // stores the successor. With no boundary open the pin wraps a bare op
    // in its own `Effect.tx`, so the step is atomic and immediate.
    let cell = TxRef::new(0);
    let read_back = cell.clone();
    let program: Effect<(i32, i32), std::convert::Infallible> = cell
        .modify_effect(|current| (current + 1, current + 10))
        .flat_map(move |result| {
            Effect::tx(move || read_back.get_effect()).map(move |value| (result, value))
        });
    assert_eq!(Runtime::run(program), Exit::Success((1, 10)));
}

#[test]
fn failure_of_the_outer_boundary_rolls_back_the_nested_writes() {
    // Effect.test.ts :3806 "should roll back nested changes when the outer
    // transaction fails" — the nested `tx` composes into the outer
    // boundary, so the escaping failure discards both staged writes.
    let ref1 = TxRef::new(0);
    let ref2 = TxRef::new(100);
    let (outer, nested) = (ref1.clone(), ref2.clone());
    let program: Effect<(), String> = Effect::tx(move || {
        let nested = nested.clone();
        outer
            .set_effect(10)
            .flat_map(move |_| Effect::tx(move || nested.set_effect(200)))
            .flat_map(|_| Effect::fail("parent failed".to_string()))
    });
    match Runtime::run(program) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"parent failed".to_string())),
        Exit::Success(_) => panic!("expected the parent failure"),
    }
    assert_eq!(ref1.read(), 0, "the outer write rolled back");
    assert_eq!(ref2.read(), 100, "the nested write rolled back with it");
}

#[test]
fn catching_the_nested_failure_inside_the_body_keeps_both_writes() {
    // Effect.test.ts :3827 "should allow catching nested failures in the
    // same transaction" — the child failure is caught inside the parent
    // body, so the boundary never sees it and commits everything. The pin
    // observes the catch with `Effect.result`; the port's `catch` is the
    // same recovery with the error handed to the handler, which asserts
    // the row's `Result.isFailure(childResult)` claim directly.
    let ref1 = TxRef::new(0);
    let ref2 = TxRef::new(100);
    let (outer, in_child, in_parent) = (ref1.clone(), ref2.clone(), ref1.clone());
    let program: Effect<(), String> = Effect::tx(move || {
        let (in_child, in_parent) = (in_child.clone(), in_parent.clone());
        outer.set_effect(10).flat_map(move |_| {
            Effect::tx(move || {
                in_child
                    .set_effect(200)
                    .flat_map(|_| Effect::fail("child failed".to_string()))
            })
            .catch(|child_error| {
                assert_eq!(child_error, "child failed");
                Effect::succeed(())
            })
        })
        .flat_map(move |_| in_parent.set_effect(20))
    });
    assert!(matches!(Runtime::run(program), Exit::Success(())));
    assert_eq!(ref1.read(), 20);
    assert_eq!(ref2.read(), 200, "the caught child's write survived");
}

#[test]
fn three_levels_of_nested_transactions_compose_into_one() {
    // Effect.test.ts :3854 "should handle multiple levels of nested
    // composed transactions" — every level joins the same boundary.
    let ref1 = TxRef::new(0);
    let ref2 = TxRef::new(0);
    let ref3 = TxRef::new(0);
    let (outer, mid, deep) = (ref1.clone(), ref2.clone(), ref3.clone());
    let program: Effect<(), std::convert::Infallible> = Effect::tx(move || {
        let (mid, deep) = (mid.clone(), deep.clone());
        outer.set_effect(1).flat_map(move |_| {
            Effect::tx(move || {
                let deep = deep.clone();
                mid.set_effect(2)
                    .flat_map(move |_| Effect::tx(move || deep.set_effect(3)))
            })
        })
    });
    assert!(matches!(Runtime::run(program), Exit::Success(())));
    assert_eq!((ref1.read(), ref2.read(), ref3.read()), (1, 2, 3));
}

#[test]
fn nested_tx_calls_join_the_same_transaction_and_read_their_writes() {
    // Effect.test.ts :3881 "should compose nested tx calls into the same
    // transaction" — one boundary with a nested tx inside: the reads
    // happen inside the boundary, so they see the staged writes.
    let ref1 = TxRef::new(0);
    let ref2 = TxRef::new(0);
    let (outer, set2, read1, read2) = (ref1.clone(), ref2.clone(), ref1.clone(), ref2.clone());
    let program: Effect<(i32, i32), std::convert::Infallible> = Effect::tx(move || {
        let (set2, read1, read2) = (set2.clone(), read1.clone(), read2.clone());
        outer
            .set_effect(10)
            .flat_map(move |_| Effect::tx(move || set2.set_effect(20)))
            .flat_map(move |_| Effect::tx(move || read1.get_effect()))
            .flat_map(move |v1| Effect::tx(move || read2.get_effect()).map(move |v2| (v1, v2)))
    });
    assert_eq!(Runtime::run(program), Exit::Success((10, 20)));
}

#[test]
fn an_escaping_nested_failure_rolls_back_the_composed_transaction() {
    // Effect.test.ts :3899 "should rollback the entire composed
    // transaction on failure" — the nested failure is not caught, so the
    // whole composed boundary fails and no write survives.
    let cell = TxRef::new(0);
    let (outer, staged) = (cell.clone(), cell.clone());
    let program: Effect<(), String> = Effect::tx(move || {
        let staged = staged.clone();
        outer.set_effect(10).flat_map(move |_| {
            Effect::tx(move || {
                staged
                    .set_effect(20)
                    .flat_map(|_| Effect::fail("nested failure".to_string()))
            })
        })
    });
    match Runtime::run(program) {
        Exit::Failure(cause) => assert_eq!(cause.failure(), Some(&"nested failure".to_string())),
        Exit::Success(_) => panic!("expected the nested failure to escape"),
    }
    assert_eq!(cell.read(), 0, "the composed transaction rolled back whole");
}

#[test]
fn a_caught_nested_failure_preserves_the_nested_write() {
    // Effect.test.ts :3918 "should preserve nested writes when nested
    // failure is caught" — the child failure is caught by the parent body,
    // so the child's write survives in the same boundary.
    let cell = TxRef::new(0);
    let (outer, staged) = (cell.clone(), cell.clone());
    let program: Effect<(), String> = Effect::tx(move || {
        let staged = staged.clone();
        outer.set_effect(10).flat_map(move |_| {
            Effect::tx(move || {
                staged
                    .set_effect(20)
                    .flat_map(|_| Effect::fail("transaction nested failure".to_string()))
            })
            .catch(|child_error| {
                assert_eq!(child_error, "transaction nested failure");
                Effect::succeed(())
            })
        })
    });
    assert!(matches!(Runtime::run(program), Exit::Success(())));
    assert_eq!(cell.read(), 20, "the caught nested write survived");
}

#[test]
fn each_interpretation_of_a_transaction_description_opens_its_own_boundary() {
    // Beyond the rows: the pin opens its boundary per `tx` invocation, so
    // re-running one description stages twice and commits twice. A shared
    // boundary would leave the second run reading the first run's journal.
    let cell = TxRef::new(0);
    // A fresh build per run: running consumes a description (L1), and each
    // interpretation opens its own boundary.
    let build = || -> Effect<i32, std::convert::Infallible, ()> {
        let staged = cell.clone();
        Effect::tx(move || {
            let staged = staged.clone();
            staged.set_effect(1).flat_map(move |_| Effect::tx(move || staged.get_effect()))
        })
    };
    assert_eq!(Runtime::run(build()), Exit::Success(1));
    assert_eq!(Runtime::run(build()), Exit::Success(1), "a second boundary over the same cell");
    assert_eq!(cell.read(), 1);
}

#[test]
fn a_defect_inside_the_body_also_discards_the_staged_writes() {
    // Beyond the rows: only a success publishes. A defect escapes the
    // boundary like a typed failure, so nothing it staged survives.
    let cell = TxRef::new(0);
    let staged = cell.clone();
    let program: Effect<(), std::convert::Infallible> = Effect::tx(move || {
        let staged = staged.clone();
        staged
            .set_effect(10)
            .flat_map(|_| Effect::<(), std::convert::Infallible>::die(elm_effect::Defect::new("boom")))
    });
    match Runtime::run(program) {
        Exit::Failure(cause) => assert_eq!(cause.defects()[0].message, "boom"),
        Exit::Success(_) => panic!("expected the defect"),
    }
    assert_eq!(cell.read(), 0, "the defect discarded the staged write");
}

#[test]
fn tx_retry_outside_a_transaction_is_a_defect() {
    // Beyond the rows: the pin's `txRetry` requires the transaction
    // service, so calling it with no boundary open cannot succeed. The
    // port's fail-closed answer is a defect naming the missing context.
    let program: Effect<std::convert::Infallible, std::convert::Infallible> =
        Effect::tx_retry();
    match Runtime::run(program) {
        Exit::Failure(cause) => {
            assert_eq!(cause.defects()[0].message, "txRetry outside a transaction")
        }
        Exit::Success(never) => match never {},
    }
}

// ---------------------------------------------------------------------------
// transaction retry (Effect.test.ts describe "retry": :3940-:3996)
//
// `Effect.txRetry` marks the running boundary for another attempt and
// interrupts the body; the boundary parks on the one-shot every ref in
// its read set shares with the committer, and a commit wakes the waiters
// of the refs it WRITES. Registration happens before the consistency
// check, so a commit that lands first leaves a stale read set and an
// immediate rerun rather than a lost wake.
// ---------------------------------------------------------------------------

#[test]
fn a_read_ref_changed_while_the_transaction_was_suspended_reruns_it() {
    // Effect.test.ts :3940 "should rerun when a read ref changed while the
    // transaction was suspended". The pin's `Deferred.await` is a blocking
    // wait here and `TestClock` has no role; the observable is the rerun's
    // result. The commit lands while the body is still suspended, so the
    // parked attempt finds a stale read set and reruns at once.
    let rt = Runtime::new();
    let cell = TxRef::new(0);
    let read = Arc::new(Deferred::<(), std::convert::Infallible>::new());
    let committed = Arc::new(Deferred::<(), std::convert::Infallible>::new());
    let waiter = {
        let (cell, read, committed) = (cell.clone(), Arc::clone(&read), Arc::clone(&committed));
        rt.fork_detach_with(
            move |_ctx| {
                Runtime::run(Effect::tx(move || {
                    let (cell, read, committed) =
                        (cell.clone(), read.clone(), committed.clone());
                    cell.get_effect().flat_map(move |value| {
                        if value == 0 {
                            read.complete(());
                            committed.wait();
                            Effect::<i32, std::convert::Infallible>::tx_retry()
                        } else {
                            Effect::succeed(value)
                        }
                    })
                }))
            },
            ForkOpts::immediate(),
        )
    };
    read.wait();
    let main: Effect<(), std::convert::Infallible> = Effect::tx(move || cell.set_effect(1));
    assert!(matches!(Runtime::run(main), Exit::Success(())));
    committed.complete(());
    assert_eq!(waiter.join(), Exit::Success(1));
}

#[test]
fn commits_wake_the_waiters_of_refs_they_write_not_refs_they_read() {
    // Effect.test.ts :3969. The pin sequences with cooperative `yieldNow`;
    // this port's fibers are threads, so the row's own run counter is the
    // synchronizer. The claims, in the row's order: a read-only commit does
    // not wake the waiter, a same-value WRITE does, and a value-changing
    // write both wakes it and delivers the new value.
    let rt = Runtime::new();
    let gate = TxRef::new(0);
    let other = TxRef::new(0);
    let runs = Arc::new(AtomicUsize::new(0));
    let waiter = {
        let (gate, runs) = (gate.clone(), Arc::clone(&runs));
        rt.fork_detach_with(
            move |_ctx| {
                Runtime::run(Effect::tx(move || {
                    let (gate, runs) = (gate.clone(), Arc::clone(&runs));
                    Effect::sync(move || {
                        runs.fetch_add(1, Ordering::SeqCst);
                    })
                    .flat_map(move |_| gate.get_effect())
                    .flat_map(|value| {
                        if value == 0 {
                            Effect::<i32, std::convert::Infallible>::tx_retry()
                        } else {
                            Effect::succeed(value)
                        }
                    })
                }))
            },
            ForkOpts::immediate(),
        )
    };
    assert!(spin_until(|| runs.load(Ordering::SeqCst) >= 1), "the waiter never started");

    // A commit that READS the gate and writes another ref: the waiter's
    // read set includes the gate, but nothing it read changed.
    let reader: Effect<(), std::convert::Infallible> =
        Effect::tx({ let gate = gate.clone(); move || {
            let other = other.clone();
            gate.get_effect().flat_map(move |_| other.set_effect(1))
        } });
    assert!(matches!(Runtime::run(reader), Exit::Success(())));
    assert_eq!(runs.load(Ordering::SeqCst), 1, "reading a ref should not wake its waiters");

    // Writing the SAME value still wakes: the wake follows the write, not
    // the change.
    let same: Effect<(), std::convert::Infallible> =
        Effect::tx({ let gate = gate.clone(); move || gate.set_effect(0) });
    assert!(matches!(Runtime::run(same), Exit::Success(())));
    assert!(spin_until(|| runs.load(Ordering::SeqCst) >= 2), "the same-value write never woke the waiter");
    assert_eq!(runs.load(Ordering::SeqCst), 2);

    // Writing a new value wakes the waiter and delivers it.
    let change: Effect<(), std::convert::Infallible> =
        Effect::tx({ let gate = gate.clone(); move || gate.set_effect(1) });
    assert!(matches!(Runtime::run(change), Exit::Success(())));
    assert_eq!(waiter.join(), Exit::Success(1));
}

#[test]
fn a_write_of_negative_zero_over_zero_is_published_and_wakes_its_waiters() {
    // Effect.test.ts :3996. The pin's `Object.is(value, 0)` tells -0 from
    // 0 and Rust's `==` does not, so the attempt compares bits; the write
    // publishes, and the woken rerun divides by the published -0.
    let rt = Runtime::new();
    let zero = TxRef::new(0.0f64);
    let runs = Arc::new(AtomicUsize::new(0));
    let waiter = {
        let (zero, runs) = (zero.clone(), Arc::clone(&runs));
        rt.fork_detach_with(
            move |_ctx| {
                Runtime::run(Effect::tx(move || {
                    let (zero, runs) = (zero.clone(), Arc::clone(&runs));
                    Effect::sync(move || {
                        runs.fetch_add(1, Ordering::SeqCst);
                    })
                    .flat_map(move |_| zero.get_effect())
                    .flat_map(|value| {
                        if value.to_bits() == 0.0f64.to_bits() {
                            Effect::<f64, std::convert::Infallible>::tx_retry()
                        } else {
                            Effect::succeed(1.0 / value)
                        }
                    })
                }))
            },
            ForkOpts::immediate(),
        )
    };
    assert!(spin_until(|| runs.load(Ordering::SeqCst) >= 1), "the waiter never started");
    let negative_zero: Effect<(), std::convert::Infallible> = Effect::tx(move || zero.set_effect(-0.0));
    assert!(matches!(Runtime::run(negative_zero), Exit::Success(())));
    assert_eq!(waiter.join(), Exit::Success(f64::NEG_INFINITY));
}

#[test]
fn tx_subscription_ref_updates_publish_to_the_change_log() {
    // The whole reason these sit on a SubscriptionRef rather than a plain
    // TxRef: every assigned value lands in the change log, so a subscriber
    // replaying `changes` sees the full sequence.
    let cell = TxSubscriptionRef::new(0u32);
    cell.get_and_set(1);
    cell.get_and_update(|n| n + 1);
    cell.update_and_get(|n| n * 3);
    assert_eq!(cell.changes(), vec![0, 1, 2, 6]);
}

#[test]
fn tx_queue_strategies_and_inspection() {
    // TxQueue.test.ts:172, :182 — the two bounded strategies.
    let dropping = TxQueue::dropping(2);
    assert!(dropping.offer(1));
    assert!(dropping.offer(2));
    assert!(dropping.offer(3), "dropping stays true when full");
    assert_eq!(dropping.size(), 2, "the newest value is dropped");
    assert_eq!(dropping.take(), Some(1));

    let sliding = TxQueue::sliding(2);
    assert!(sliding.offer(1));
    assert!(sliding.offer(2));
    assert!(sliding.offer(3), "sliding stays true when full");
    assert_eq!(sliding.size(), 2);
    assert_eq!(sliding.take(), Some(2), "the oldest value was evicted");

    // :205, :213 — poll answers Option both ways.
    let queue: TxQueue<i32> = TxQueue::bounded(4);
    assert_eq!(queue.poll(), None);
    assert!(queue.offer(7));
    assert_eq!(queue.poll(), Some(7));

    // :587, :596 — size and the emptiness pair.
    assert_eq!(queue.size(), 0);
    assert!(queue.is_empty());
    assert!(!queue.is_non_empty());
    assert!(queue.offer(1));
    assert!(!queue.is_empty());
    assert!(queue.is_non_empty());

    // :608, :620 — is_full distinguishes bounded from unbounded.
    let full = TxQueue::bounded(1);
    assert!(!full.is_full());
    assert!(full.offer(1));
    assert!(full.is_full());
    let never = TxQueue::unbounded();
    assert!(never.offer(1));
    assert!(never.offer(2));
    assert!(!never.is_full());
}

#[test]
fn tx_queue_drain_family() {
    // :242, :272 — offer_all pushes everything, take_all drains everything.
    let queue = TxQueue::bounded(5);
    assert!(queue.offer_all([1, 2, 3]).is_empty());
    assert_eq!(queue.take_all(), vec![1, 2, 3]);

    // :306, :414 — take_n honours its count, and zero takes nothing.
    assert!(queue.offer_all([1, 2, 3, 4]).is_empty());
    assert_eq!(queue.take_n(2), vec![1, 2]);
    assert_eq!(queue.take_n(0), vec![]);
    assert_eq!(queue.take_n(9), vec![3, 4], "more than held drains all");

    // :572 — peek leaves the item in place.
    assert_eq!(queue.peek(), None);
    assert!(queue.offer(9));
    assert_eq!(queue.peek(), Some(9));
    assert_eq!(queue.size(), 1);

    // :457, :477, :543 — take_between waits for the minimum.
    let bounded = TxQueue::bounded(10);
    assert!(bounded.offer_all([1, 2, 3]).is_empty());
    assert_eq!(bounded.take_between(5, 8), vec![], "fewer than min waits");
    assert_eq!(bounded.take_between(2, 8), vec![1, 2, 3]);
    assert_eq!(bounded.take_between(0, 0), vec![]);
    // :492 — an inverted range takes nothing rather than everything.
    assert_eq!(bounded.take_between(4, 1), vec![]);
}

#[test]
fn tx_queue_clear_leaves_state_alone() {
    // :631 — clear hands back every item and empties the queue.
    let queue: TxQueue<i32> = TxQueue::bounded(10);
    assert!(queue.offer_all([1, 2, 3, 4, 5]).is_empty());
    assert_eq!(queue.clear(), vec![1, 2, 3, 4, 5]);
    assert_eq!(queue.size(), 0);
    assert!(queue.is_empty());

    // :649 — clearing does not close the queue, so offers still land.
    assert!(queue.offer_all([1, 2, 3]).is_empty());
    assert_eq!(queue.clear(), vec![1, 2, 3]);
    assert!(queue.is_open());
    assert!(queue.offer(42));
    assert_eq!(queue.size(), 1);

    // :669 — clearing an empty queue returns empty.
    let empty: TxQueue<i32> = TxQueue::bounded(10);
    assert_eq!(empty.clear(), vec![]);

    // :683 — clearing a halted, drained queue also returns empty and stays done.
    let ended: TxQueue<i32> = TxQueue::bounded(10);
    assert!(ended.offer_all([1, 2, 3]).is_empty());
    assert!(ended.end());
    assert_eq!(ended.take_all(), vec![1, 2, 3]);
    assert!(!ended.is_open());
    assert_eq!(ended.clear(), vec![]);
    assert!(!ended.is_open());
}

#[test]
fn tx_queue_shutdown_halts_and_empties() {
    // :710 — isShutdown is false before and true after.
    let queue = TxQueue::bounded(10);
    assert!(!queue.is_shutdown());
    assert!(queue.shutdown());
    assert!(queue.is_shutdown());

    // :722 — offers are refused once halted.
    assert!(!queue.offer(42));

    // :731 — a poll finds nothing, even though an item had been offered.
    let polled: TxQueue<i32> = TxQueue::bounded(10);
    assert!(polled.offer(42));
    assert!(polled.shutdown());
    assert_eq!(polled.poll(), None, "shutdown drops what was queued");

    // :1028, :1045 — shutdown is idempotent and stays a shutdown.
    assert!(!polled.shutdown());
    assert!(polled.is_shutdown());
    assert_eq!(polled.size(), 0);
}

#[test]
fn tx_queue_state_transitions() {
    // :808 — a fresh queue is open and not closing or done.
    let queue: TxQueue<i32> = TxQueue::bounded(5);
    assert!(queue.is_open());
    assert!(!queue.is_shutdown());

    // :821 — interrupt on an empty queue halts it outright.
    let empty: TxQueue<i32> = TxQueue::bounded(5);
    assert!(empty.interrupt());
    assert!(!empty.is_open());
    assert!(empty.is_shutdown());

    // :867, :909 — fail halts too, and reports the first halt only.
    let failed: TxQueue<i32> = TxQueue::bounded(5);
    assert!(failed.fail());
    assert!(!failed.is_open());
    assert!(!failed.offer(1));
    assert!(!failed.fail());

    // :884, :898 — done completes the queue, once.
    let done: TxQueue<i32> = TxQueue::bounded(5);
    assert!(done.done());
    assert!(!done.done());
    assert!(!done.is_open());

    // :1061, :1071 — once halted, a drained queue yields nothing and an offer
    // is refused, so take cannot resurrect the queue.
    let drained: TxQueue<i32> = TxQueue::bounded(5);
    assert!(drained.offer(1));
    assert!(drained.interrupt());
    assert_eq!(drained.take_all(), vec![1]);
    assert_eq!(drained.take(), None);
    assert!(!drained.offer(2));
}

#[test]
fn tx_queue_closing_and_full_behavior() {
    // :743, :763 — at capacity, dropping discards the newest and sliding
    // evicts the oldest, and neither refuses the offer.
    let dropping: TxQueue<i32> = TxQueue::dropping(2);
    assert!(dropping.offer_all([1, 2]).is_empty());
    assert!(dropping.offer(3));
    assert_eq!(dropping.size(), 2);
    assert_eq!(dropping.take_all(), vec![1, 2]);

    let sliding: TxQueue<i32> = TxQueue::sliding(2);
    assert!(sliding.offer_all([1, 2]).is_empty());
    assert!(sliding.offer(3));
    assert_eq!(sliding.size(), 2);
    assert_eq!(sliding.take_all(), vec![2, 3]);

    // :222, :232 — a closing queue still drains what it holds, then settles
    // on empty. The port has no separate Closing state, so "drains then done"
    // is the observable the row asserts.
    let closing: TxQueue<i32> = TxQueue::bounded(10);
    assert!(closing.offer_all([1, 2, 3]).is_empty());
    assert!(closing.end());
    assert_eq!(closing.poll(), Some(1));
    assert_eq!(closing.poll(), Some(2));
    assert_eq!(closing.poll(), Some(3));
    assert_eq!(closing.poll(), None, "the last item completes the queue");
    assert!(!closing.offer(4), "a closed queue takes no more offers");

    // :837 — interrupt on a non-empty queue halts it while the items drain.
    let partial: TxQueue<i32> = TxQueue::bounded(5);
    assert!(partial.offer_all([1, 2, 3]).is_empty());
    assert!(partial.interrupt());
    assert!(!partial.is_open());
    assert_eq!(partial.take_all(), vec![1, 2, 3]);

    // :931 — every entry point on a halted queue answers without working.
    let halted: TxQueue<i32> = TxQueue::bounded(5);
    assert!(halted.offer(1));
    assert!(halted.shutdown());
    assert!(!halted.offer(2));
    assert_eq!(halted.poll(), None);
    assert_eq!(halted.take(), None);
    assert_eq!(halted.take_all(), vec![]);
    assert_eq!(halted.take_n(3), vec![]);
    assert_eq!(halted.clear(), vec![]);
    assert_eq!(halted.size(), 0);
    assert!(halted.is_shutdown());
}
