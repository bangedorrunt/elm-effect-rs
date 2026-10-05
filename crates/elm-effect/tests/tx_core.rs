//! STM core fence: transactional cells, queues, maps and friends.
//! Behaviors mirror packages/effect/test/Tx*.test.ts (composable,
//! atomic, isolated multi-op blocks; blocking-retry is a residual).

use elm_effect::{
    TxChunk, TxDeferred, TxHashMap, TxHashSet, TxPriorityQueue, TxPubSub, TxQueue, TxReentrantLock,
    TxRef, TxSemaphore, TxSubscriptionRef, atomically,
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
