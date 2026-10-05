//! Queue behaviors: offers, draining takes, shutdown, clearing.
//! Ports packages/effect/test/Queue.test.ts (synchronous core; fiber
//! back-pressure and takeN-wakeup need the async runtime).

use elm_effect::Queue;

#[test]
fn fresh_queue_is_empty() {
    let queue: Queue<u32> = Queue::new(2);
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
}

#[test]
fn offers_and_polls_roundtrip() {
    let queue: Queue<String> = Queue::new(4);
    assert!(queue.try_offer("a".to_string()));
    assert!(queue.try_offer("b".to_string()));
    assert_eq!(queue.len(), 2);
    assert_eq!(queue.try_poll(), Some("a".to_string()));
    assert_eq!(queue.try_poll(), Some("b".to_string()));
    assert_eq!(queue.try_poll(), None);
    assert!(queue.is_empty());
}

#[test]
fn try_offer_refuses_when_full() {
    let queue = Queue::new(1);
    assert!(queue.try_offer(1));
    assert!(!queue.try_offer(2));
    assert_eq!(queue.len(), 1);
}

#[test]
fn peek_views_without_removing() {
    let queue = Queue::new(4);
    assert_eq!(queue.try_poll(), None);
    assert!(queue.try_offer("a".to_string()));
    assert_eq!(queue.peek(), Some("a".to_string()));
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.try_poll(), Some("a".to_string()));
}

#[test]
fn take_all_drains_in_order() {
    let queue = Queue::new(8);
    for n in [1, 2, 3] {
        assert!(queue.try_offer(n));
    }
    assert_eq!(queue.take_all(), vec![1, 2, 3]);
    assert!(queue.is_empty());
    assert!(queue.take_all().is_empty());
}

#[test]
fn shutdown_freezes_offers_but_drains() {
    let queue = Queue::new(8);
    assert!(queue.try_offer(1));
    assert!(!queue.is_shutdown());
    queue.shutdown();
    assert!(queue.is_shutdown());
    assert!(!queue.try_offer(2));
    assert_eq!(queue.try_poll(), Some(1));
    assert_eq!(queue.try_poll(), None);
}

#[test]
fn clear_drops_and_counts() {
    let queue = Queue::new(8);
    for n in [1, 2, 3] {
        assert!(queue.try_offer(n));
    }
    assert_eq!(queue.clear(), 3);
    assert!(queue.is_empty());
}

#[test]
fn capacity_reports_the_bound() {
    let queue: Queue<u32> = Queue::new(4);
    assert_eq!(queue.capacity(), 4);
}
