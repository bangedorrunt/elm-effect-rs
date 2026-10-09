//! STM core fence: transactional cells, queues, maps and friends.
//! Behaviors mirror packages/effect/test/Tx*.test.ts (composable,
//! atomic, isolated multi-op blocks; blocking-retry is a residual).

use elm_effect::{
    TxChunk, TxDeferred, TxHashMap, TxHashSet, TxPriorityQueue, TxPubSub, TxQueue, TxReentrantLock,
    TxRef, TxSemaphore, TxSubscriptionRef, atomically, is_tx_chunk, is_tx_hash_set, is_tx_semaphore,
};

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
