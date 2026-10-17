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

// ---------------------------------------------------------------------------
// Queue rows carried on the synchronous core (Queue.test.ts)
// ---------------------------------------------------------------------------

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use elm_effect::QueueError;

/// Wait for a spawned party to reach its parked state.
fn park_settle() {
    std::thread::sleep(Duration::from_millis(30));
}

#[test]
fn queue_guards_recognize_the_faces() {
    // Queue.test.ts :38/:47 — the type guards answer true for the queue and
    // false for a foreign value.
    let queue = Arc::new(Queue::<i32>::new(4));
    assert!(elm_effect::is_enqueue::<i32>(
        queue.as_ref() as &dyn std::any::Any
    ));
    assert!(elm_effect::is_dequeue::<i32>(
        queue.as_ref() as &dyn std::any::Any
    ));
    assert!(elm_effect::is_queue::<i32>(
        queue.as_ref() as &dyn std::any::Any
    ));
    assert!(!elm_effect::is_enqueue::<i32>(&7i32 as &dyn std::any::Any));
    assert!(!elm_effect::is_dequeue::<i32>(&7i32 as &dyn std::any::Any));
    assert!(!elm_effect::is_queue::<i32>(&7i32 as &dyn std::any::Any));
}

#[test]
fn queue_views_expose_only_their_face() {
    // Queue.test.ts :56/:68 — asEnqueue/asDequeue hand back the write-only
    // and read-only faces; each is still recognized by its guard, and the
    // operations work through the view.
    let queue = Arc::new(Queue::<i32>::new(4));
    let enqueue = Queue::as_enqueue(&queue);
    assert!(elm_effect::is_enqueue::<i32>(
        enqueue.as_ref() as &dyn std::any::Any
    ));
    assert!(elm_effect::is_queue::<i32>(
        enqueue.as_ref() as &dyn std::any::Any
    ));
    enqueue.offer(1);
    enqueue.offer_all([2, 3]);
    let dequeue = Queue::as_dequeue(&queue);
    assert!(elm_effect::is_dequeue::<i32>(
        dequeue.as_ref() as &dyn std::any::Any
    ));
    assert_eq!(dequeue.take(), Ok(1));
    assert_eq!(dequeue.take_n(2), Ok(vec![2, 3]));
}

#[test]
fn take_n_returns_consecutive_batches() {
    // Queue.test.ts :222 — takeN hands back consecutive batches up to n.
    let queue = Queue::unbounded();
    queue.offer_all([1, 2, 3, 4]);
    assert_eq!(queue.take_n(2), Ok(vec![1, 2]));
    assert_eq!(queue.take_n(2), Ok(vec![3, 4]));
}

#[test]
fn take_n_drains_an_insufficient_batch_after_end() {
    // Queue.test.ts :319 — after end, takeN drains what is left even below
    // its count, and the next takeN reports Done.
    let queue: Queue<i32> = Queue::unbounded();
    queue.offer_all([1, 2]);
    queue.end();
    assert_eq!(queue.take_n(5), Ok(vec![1, 2]));
    assert_eq!(queue.take_n(5), Err(QueueError::Done));
}

#[test]
fn take_n_and_take_between_normalize_counts() {
    // Queue.test.ts :379 — counts arrive through the pin's Count.normalize:
    // 1.9 floors to 1, 2.9 floors to 2, and NaN/non-positive become 0. A
    // zero count takes nothing and leaves the message buffered.
    let take_n_queue = Queue::unbounded();
    take_n_queue.offer(1);
    assert_eq!(take_n_queue.take_n(1), Ok(vec![1]));
    let take_between_queue = Queue::unbounded();
    take_between_queue.offer(1);
    assert_eq!(take_between_queue.take_between(1, 2), Ok(vec![1]));
    let invalid_queue = Queue::unbounded();
    invalid_queue.offer(1);
    assert_eq!(invalid_queue.take_n(0), Ok(Vec::new()));
    assert_eq!(invalid_queue.take_all(), vec![1]);
}

#[test]
fn take_n_and_take_between_keep_waiting_on_a_partial_batch() {
    // Queue.test.ts :298 — a taker whose minimum is not met keeps waiting,
    // and the rest of the batch completing it delivers the whole set.
    let take_n_queue = Arc::new(Queue::unbounded());
    let waiting = {
        let queue = Arc::clone(&take_n_queue);
        std::thread::spawn(move || queue.take_n(3))
    };
    take_n_queue.offer(1);
    park_settle();
    take_n_queue.offer_all([2, 3]);
    assert_eq!(waiting.join().unwrap(), Ok(vec![1, 2, 3]));

    let take_between_queue = Arc::new(Queue::unbounded());
    let waiting = {
        let queue = Arc::clone(&take_between_queue);
        std::thread::spawn(move || queue.take_between(2, 5))
    };
    take_between_queue.offer(1);
    park_settle();
    take_between_queue.offer_all([2, 3]);
    assert_eq!(waiting.join().unwrap(), Ok(vec![1, 2, 3]));
}

#[test]
fn collect_does_not_duplicate_messages() {
    // Queue.test.ts :395 — collect drains the buffered items once.
    let queue: Queue<i32> = Queue::new(10);
    queue.offer(0);
    queue.end();
    assert_eq!(queue.collect(), vec![0]);
    assert!(queue.collect().is_empty());
}

#[test]
fn offer_dropping_drops_new_items_when_full() {
    // Queue.test.ts :405 — a full dropping queue refuses both a batch and a
    // single offer, returning the unaccepted remainder.
    let queue: Queue<i32> = Queue::with_strategy(2, elm_effect::Strategy::Dropping);
    assert_eq!(queue.offer_all([1, 2, 3, 4]), vec![3, 4]);
    assert!(!queue.try_offer(5));
    assert_eq!(queue.take_all(), vec![1, 2]);
}

#[test]
fn offer_sliding_drops_the_oldest_when_full() {
    // Queue.test.ts :415 — a full sliding queue drops its oldest to make
    // room, so the batch leaves no remainder and the tail survives.
    let queue: Queue<i32> = Queue::with_strategy(2, elm_effect::Strategy::Sliding);
    assert_eq!(queue.offer_all([1, 2, 3, 4]), Vec::<i32>::new());
    assert!(queue.try_offer(5));
    assert_eq!(queue.take_all(), vec![4, 5]);
}

#[test]
fn offer_all_can_be_interrupted() {
    // Queue.test.ts :425 — an offerAll parked on a full queue withdraws its
    // unaccepted items when interrupted, and the capacity it vacated is
    // reusable afterwards.
    let queue = Arc::new(Queue::new(2));
    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&interrupted);
    let producer = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || {
            queue.offer_all_interruptible([1, 2, 3, 4], &|| flag.load(Ordering::SeqCst))
        })
    };
    park_settle();
    interrupted.store(true, Ordering::SeqCst);
    let withdrawn = producer.join().unwrap();
    assert_eq!(
        withdrawn,
        Err(()),
        "an interrupted batch withdraws its suffix"
    );
    assert_eq!(queue.take_all(), vec![1, 2]);
    queue.offer(5);
    assert_eq!(queue.take_all(), vec![5]);
}

#[test]
fn done_completes_takes() {
    // Queue.test.ts :462 — a takeAll parked on an empty queue completes
    // with Done when the queue ends.
    let queue = Arc::new(Queue::<i32>::new(2));
    let taker = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.take_all_blocking())
    };
    park_settle();
    queue.end();
    assert_eq!(taker.join().unwrap(), Err(QueueError::Done));
}

#[test]
fn interrupt_allows_draining() {
    // Queue.test.ts :500 — interrupt closes the offer side but the buffered
    // items still drain, and the empty queue then reports the interrupt.
    let queue = Queue::new(10);
    queue.offer_all([1, 2, 3, 4, 5]);
    assert!(queue.interrupt());
    assert!(!queue.try_offer(6));
    for expected in 1..=5 {
        assert_eq!(queue.take(), Ok(expected));
    }
    assert_eq!(queue.take(), Err(QueueError::Interrupted));
}

#[test]
fn shutdown_unsafe_interrupts_waiting_takers() {
    // Queue.test.ts :603 — shutdownUnsafe interrupts a parked taker
    // synchronously: by the time it returns, the take has been interrupted.
    let queue = Arc::new(Queue::<i32>::unbounded());
    let taker = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.take())
    };
    park_settle();
    queue.shutdown_unsafe();
    assert_eq!(taker.join().unwrap(), Err(QueueError::Interrupted));
}

#[test]
fn shutdown_unsafe_clears_the_buffer_and_settles_pending_offers() {
    // Queue.test.ts :613 — shutdownUnsafe drops the buffered items and
    // withdraws the offer parked behind them, leaving the queue empty.
    let queue = Arc::new(Queue::new(1));
    queue.offer(1);
    let producer = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.offer(2))
    };
    park_settle();
    queue.shutdown_unsafe();
    producer.join().unwrap();
    assert!(queue.is_empty(), "the buffered item is cleared");
    assert_eq!(queue.take_all(), Vec::<i32>::new());
}

#[test]
fn shutdown_unsafe_resumes_batch_offers_with_their_unaccepted_suffix() {
    // Queue.test.ts :625 — a batch offer parked mid-batch resumes with the
    // suffix that never landed.
    let queue = Arc::new(Queue::new(1));
    let producer = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.offer_all([1, 2, 3]))
    };
    park_settle();
    assert_eq!(queue.take(), Ok(1));
    // Let the parked producer resume and land its second item before the
    // shutdown clears the buffer, matching the pin's deterministic order.
    park_settle();
    queue.shutdown_unsafe();
    assert_eq!(producer.join().unwrap(), vec![3]);
}

#[test]
fn shutdown_unsafe_returns_true_and_preserves_the_failure() {
    // Queue.test.ts :638 — shutdownUnsafe on a failed queue still reports
    // true, and the failure survives for the take.
    let queue: Queue<i32> = Queue::unbounded();
    assert!(queue.fail("boom"));
    assert!(queue.shutdown_unsafe());
    assert_eq!(queue.take(), Err(QueueError::Failed("boom".to_string())));
}

#[test]
fn shutdown_unsafe_returns_false_when_already_shut_down() {
    // Queue.test.ts :649 — the second shutdownUnsafe reports false, and the
    // await sees the interrupt.
    let queue: Queue<i32> = Queue::unbounded();
    assert!(queue.shutdown_unsafe());
    assert!(!queue.shutdown_unsafe());
    assert_eq!(queue.await_finished(), Err(QueueError::Interrupted));
}

#[test]
fn shutdown_returns_false_and_preserves_the_failure() {
    // Queue.test.ts :658 — a queue that already failed cannot shut down, and
    // its failure is preserved.
    let queue: Queue<i32> = Queue::unbounded();
    assert!(queue.fail("boom"));
    assert!(!queue.shutdown());
    assert_eq!(queue.take(), Err(QueueError::Failed("boom".to_string())));
}

#[test]
fn end_preserves_done_for_take_and_excludes_it_from_await() {
    // Queue.test.ts :699 — after end a parked take fails with Done while the
    // await completes normally: Done is the take's terminal error, not the
    // queue's failure.
    let queue = Arc::new(Queue::<i32>::unbounded());
    let taker = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.take())
    };
    park_settle();
    queue.end();
    assert_eq!(taker.join().unwrap(), Err(QueueError::Done));
    assert_eq!(queue.await_finished(), Ok(()));
}

#[test]
fn await_preserves_non_done_failures() {
    // Queue.test.ts :711 — a failed queue's await reports the failure.
    let queue: Queue<i32> = Queue::unbounded();
    assert!(queue.fail("boom"));
    assert_eq!(
        queue.await_finished(),
        Err(QueueError::Failed("boom".to_string()))
    );
}

#[test]
fn flush_unsafe_releases_a_waiting_taker_after_offer_unsafe() {
    // Queue.test.ts :721 — offerUnsafe parks the message without waking the
    // taker; flushUnsafe performs the release. The await stays pending: the
    // queue has not finished.
    let queue = Arc::new(Queue::<i32>::unbounded());
    let taker = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.take())
    };
    park_settle();
    queue.offer_unsafe(1);
    park_settle();
    queue.flush_unsafe();
    assert_eq!(taker.join().unwrap(), Ok(1));
    assert!(!queue.is_shutdown());
}

#[test]
fn flush_releases_a_waiting_taker_after_offer_unsafe() {
    // Queue.test.ts :738 — the effectful flush performs the same release.
    let queue = Arc::new(Queue::<i32>::unbounded());
    let taker = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.take())
    };
    park_settle();
    queue.offer_unsafe(1);
    park_settle();
    queue.flush();
    assert_eq!(taker.join().unwrap(), Ok(1));
}

#[test]
fn flush_is_a_no_op_after_shutdown() {
    // Queue.test.ts :788 — after shutdown the scheduled releases are
    // discarded and offers are refused.
    let queue: Queue<i32> = Queue::unbounded();
    queue.shutdown_unsafe();
    queue.flush_unsafe();
    queue.flush();
    assert!(!queue.try_offer(1));
}

#[test]
fn flush_keeps_a_buffered_message_when_no_taker_is_waiting() {
    // Queue.test.ts :799 — a flush with no waiter leaves the message
    // buffered for the next take.
    let queue: Queue<i32> = Queue::unbounded();
    queue.offer_unsafe(1);
    queue.flush();
    assert_eq!(queue.take(), Ok(1));
}

#[test]
fn flush_unsafe_preserves_later_offers_when_the_queue_is_empty() {
    // Queue.test.ts :809 — a flush with nothing scheduled does not disturb
    // the parked takers; the later offerAll still reaches both.
    let queue = Arc::new(Queue::<i32>::unbounded());
    let first = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.take())
    };
    let second = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.take())
    };
    park_settle();
    queue.flush_unsafe();
    queue.offer_all([1, 2]);
    let mut delivered = vec![
        first.join().unwrap().unwrap(),
        second.join().unwrap().unwrap(),
    ];
    delivered.sort();
    assert_eq!(delivered, vec![1, 2]);
}

#[test]
fn bounded_zero_capacity_rendezvouses() {
    // Queue.test.ts :824 — a zero-capacity queue hands an offer straight to
    // a taker, in both directions.
    let queue = Arc::new(Queue::new(0));
    let producer = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.offer(1))
    };
    park_settle();
    assert_eq!(queue.take(), Ok(1));
    producer.join().unwrap();

    let taker = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.take())
    };
    park_settle();
    queue.offer(2);
    assert_eq!(taker.join().unwrap(), Ok(2));
}

#[test]
fn take_between_drains_a_waiting_batch_on_failure() {
    // Queue.test.ts :330 — a taker parked below its minimum keeps its
    // already-buffered items when the queue fails: the batch drains, and
    // the next take reports the failure. (The row's state assertion is the
    // pin's internal tag; the port pins the same fact as the preserved
    // failure.)
    let queue = Arc::new(Queue::<i32>::unbounded());
    queue.offer_all([1, 2]);
    let taker = {
        let queue = Arc::clone(&queue);
        std::thread::spawn(move || queue.take_between(5, 8))
    };
    park_settle();
    assert!(queue.fail("boom"));
    assert_eq!(taker.join().unwrap(), Ok(vec![1, 2]));
    assert_eq!(
        queue.take_between(5, 8),
        Err(QueueError::Failed("boom".to_string()))
    );
}
