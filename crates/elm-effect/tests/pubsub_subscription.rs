//! Wave-7 PubSub batch: bounded/dropping/sliding/unbounded hubs,
//! SubscriptionRef change streams, PartitionedSemaphore pools.
//! Modules load via `#[path]` includes: no facade touch. Threads stand
//! in for fibers; Effect-scope/interruption rows stay `unported` with
//! reasons in the close report.

#[allow(clippy::type_complexity)]
#[path = "../src/pubsub.rs"]
mod pubsub;
#[allow(dead_code, clippy::type_complexity)]
#[path = "../src/subscription_ref.rs"]
mod subscription_ref;

use elm_effect::PartitionedSemaphore;
use pubsub::{PubSub, Strategy};
use subscription_ref::SubscriptionRef;

// ----------------------------------------------------------------- PubSub ---

#[test]
fn pubsub_type_guard() {
    assert!(PubSub::<i32>::bounded(2).is_pubsub());
}

#[test]
fn pubsub_bounded_two_fans_out() {
    let hub = PubSub::bounded(2);
    let s1 = hub.subscribe();
    let s2 = hub.subscribe();
    assert!(hub.publish_all(vec![1, 2]));
    assert_eq!(s1.take_all(), vec![1, 2]);
    assert_eq!(s2.take_all(), vec![1, 2]);
}

#[test]
fn pubsub_bounded_four_keeps_order() {
    let hub = PubSub::bounded(4);
    let s = hub.subscribe();
    assert!(hub.publish_all(vec![1, 2, 3, 4]));
    assert_eq!(s.take_all(), vec![1, 2, 3, 4]);
}

#[test]
fn pubsub_arb_capacity_three() {
    let hub = PubSub::with_replay(3, Strategy::Bounded, 0);
    let s = hub.subscribe();
    assert!(hub.publish_all(vec![1, 2, 3]));
    assert_eq!(s.take_up_to(3.0), vec![1, 2, 3]);
}

#[test]
fn pubsub_sequential_one_publisher() {
    let hub = PubSub::bounded(16);
    let s = hub.subscribe();
    for i in 0..10 {
        assert!(hub.publish(i));
    }
    assert_eq!(s.take_all(), (0..10).collect::<Vec<_>>());
}

#[test]
fn pubsub_dropping_discards_newest() {
    let hub = PubSub::dropping(2);
    let s = hub.subscribe();
    assert!(hub.publish_all(vec![1, 2, 3, 4]));
    assert_eq!(s.take_all(), vec![1, 2]);
}

#[test]
fn pubsub_sliding_evicts_oldest() {
    let hub = PubSub::sliding(2);
    let s = hub.subscribe();
    assert!(hub.publish_all(vec![1, 2, 3]));
    assert_eq!(s.take_all(), vec![2, 3]);
}

#[test]
fn pubsub_unbounded_grows() {
    let hub = PubSub::unbounded();
    let s = hub.subscribe();
    assert!(hub.publish_all(0..100));
    assert_eq!(s.take_all().len(), 100);
}

#[test]
fn pubsub_infinite_capacity_is_unbounded() {
    let hub = PubSub::with_replay(usize::MAX, Strategy::Unbounded, 0);
    assert!(!hub.is_full());
    let s = hub.subscribe();
    assert!(hub.publish_all(0..50));
    assert_eq!(s.take_all().len(), 50);
}

#[test]
fn pubsub_null_values_flow() {
    let hub = PubSub::bounded(4);
    let s = hub.subscribe();
    assert!(hub.publish_all(vec![None, Some(1)]));
    assert_eq!(s.take_all(), vec![None, Some(1)]);
}

#[test]
fn pubsub_no_subscribers_drops() {
    let hub = PubSub::<i32>::dropping(2);
    assert!(hub.publish(1));
    assert!(hub.publish(2));
    assert_eq!(hub.size(), 0);
    assert!(hub.publish_all(vec![1, 2]));
    assert_eq!(hub.size(), 0);
}

#[test]
fn pubsub_poll_counts_floor_and_nan() {
    let hub = PubSub::bounded(4);
    let s = hub.subscribe();
    assert!(hub.publish_all(vec![1, 2, 3]));
    assert_eq!(s.poll_up_to(1.9), vec![1]);
    assert_eq!(s.poll_up_to(f64::NAN), Vec::<i32>::new());
    assert_eq!(s.poll_up_to(10.0), vec![2, 3]);
}

#[test]
fn pubsub_take_between_bounds() {
    let hub = PubSub::bounded(8);
    let s = hub.subscribe();
    assert!(hub.publish_all(vec![1, 2, 3, 4, 5]));
    assert_eq!(s.take_between(2, 3), vec![1, 2, 3]);
    assert_eq!(s.take_between(1, 10), vec![4, 5]);
}

#[test]
fn pubsub_shutdown_ends_takes() {
    let hub = PubSub::bounded(2);
    let s = hub.subscribe();
    assert!(!hub.is_shutdown());
    hub.shutdown();
    assert!(hub.is_shutdown());
    assert_eq!(s.take(), None);
    assert!(!hub.publish(1));
}

#[test]
fn pubsub_replay_window_prefills() {
    let hub = PubSub::with_replay(8, Strategy::Bounded, 2);
    assert!(hub.publish_all(vec![1, 2, 3]));
    let late = hub.subscribe_replay(2);
    assert_eq!(late.take_all(), vec![2, 3]);
    assert_eq!(hub.replay_window(), vec![2, 3]);
}

#[test]
fn pubsub_bounded_concurrent_all_delivered() {
    use std::sync::{Arc, Barrier};
    let hub = PubSub::bounded(4);
    let sub = hub.subscribe();
    let gate = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for base in [0, 50] {
        let (h, g) = (hub.clone(), gate.clone());
        handles.push(std::thread::spawn(move || {
            g.wait();
            for i in 0..50 {
                assert!(h.publish(base + i));
            }
        }));
    }
    gate.wait();
    let mut got = Vec::new();
    while got.len() < 100 {
        if let Some(v) = sub.take() {
            got.push(v);
        }
    }
    for h in handles {
        h.join().unwrap();
    }
    got.sort_unstable();
    let mut expected: Vec<i32> = (0..50).chain(50..100).collect();
    expected.sort_unstable();
    assert_eq!(got, expected);
}

#[test]
fn pubsub_dropping_sliding_concurrent_complete() {
    for hub in [PubSub::dropping(4), PubSub::sliding(4)] {
        let sub = hub.subscribe();
        let h = hub.clone();
        let t = std::thread::spawn(move || {
            for i in 0..50 {
                assert!(h.publish(i));
            }
        });
        t.join().unwrap();
        let got = sub.take_all();
        assert!(!got.is_empty());
        assert!(got.len() <= 50);
        let mut sorted = got.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(got.len(), sorted.len());
    }
}

#[test]
fn a_leaving_subscriber_releases_a_parked_bounded_publisher() {
    // Regression: `publish` parked on the hub's `space` latch while a full
    // subscription existed; unsubscribe woke only that subscription's own
    // condvar, so the publisher could wait forever.
    let hub = PubSub::bounded(1);
    let only = hub.subscribe();
    assert!(hub.publish(1), "fills the only queue");
    let publisher = {
        let hub = hub.clone();
        std::thread::spawn(move || hub.publish(2))
    };
    // Give the publisher time to reach the parked branch.
    std::thread::sleep(std::time::Duration::from_millis(50));
    only.unsubscribe();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !publisher.is_finished() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        publisher.is_finished(),
        "the publisher stayed parked after the full subscriber left"
    );
    assert!(publisher.join().unwrap(), "publish still reports its value");
    assert_eq!(hub.size(), 0, "the value went to no live subscriber");
}

#[test]
fn pubsub_unsubscribe_then_publish() {
    let hub = PubSub::bounded(4);
    let gone = hub.subscribe();
    let stays = hub.subscribe();
    gone.unsubscribe();
    assert!(hub.publish(7));
    assert_eq!(stays.take_all(), vec![7]);
    assert_eq!(hub.size(), 0);
}

#[test]
fn pubsub_replay_count_rounding() {
    // Fractional replay rounds up; non-positive disables.
    let hub = PubSub::with_replay(8, Strategy::Unbounded, 8);
    assert!(hub.publish_all(vec![1, 2, 3]));
    assert_eq!(hub.subscribe_replay_count(1.9).take_all(), vec![2, 3]);
    assert_eq!(
        hub.subscribe_replay_count(0.0).take_all(),
        Vec::<i32>::new()
    );
    assert_eq!(
        hub.subscribe_replay_count(f64::NAN).take_all(),
        Vec::<i32>::new()
    );
}

#[test]
fn pubsub_take_up_to_unbounded() {
    let hub = PubSub::unbounded();
    let s = hub.subscribe();
    assert!(hub.publish_all(vec![1, 2, 3, 4, 5]));
    assert_eq!(s.take_up_to(3.0), vec![1, 2, 3]);
    assert_eq!(s.take_up_to(10.0), vec![4, 5]);
}

#[test]
fn pubsub_concurrent_publishers_subscribers() {
    use std::sync::{Arc, Barrier};
    let hub = PubSub::unbounded();
    let gate = Arc::new(Barrier::new(5));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let (h, g) = (hub.clone(), gate.clone());
        handles.push(std::thread::spawn(move || {
            g.wait();
            for i in 0..25 {
                assert!(h.publish(i));
            }
        }));
    }
    let sub = hub.subscribe();
    gate.wait();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(sub.take_all().len(), 100);
}

#[test]
fn pubsub_size_tracks_queues() {
    let hub = PubSub::bounded(4);
    assert!(hub.is_empty());
    let s = hub.subscribe();
    assert!(hub.publish_all(vec![1, 2]));
    assert_eq!(hub.size(), 2);
    assert!(!hub.is_empty());
    s.take_all();
    assert!(hub.is_empty());
}

// -------------------------------------------------------- SubscriptionRef ---

#[test]
fn subscription_ref_guard() {
    assert!(SubscriptionRef::new(1).is_subscription_ref());
}

#[test]
fn subscription_ref_get_update() {
    let cell = SubscriptionRef::new(1);
    assert_eq!(cell.get(), 1);
    assert_eq!(cell.update(|v| v + 1), 2);
    assert_eq!(cell.get_and_update(|v| v * 10), 2);
    assert_eq!(cell.get(), 20);
}

#[test]
fn subscription_ref_update_some() {
    let cell = SubscriptionRef::new(5);
    assert_eq!(cell.update_some(|_| None), 5);
    assert_eq!(cell.update_some(|v| Some(v + 1)), 6);
    assert_eq!(cell.get(), 6);
}

#[test]
fn subscription_ref_streams_changes() {
    let cell = SubscriptionRef::new(0);
    let s1 = cell.subscribe();
    let s2 = cell.subscribe();
    assert_eq!(s1.next(), Some(0));
    cell.set(1);
    cell.set(2);
    assert_eq!(s1.next(), Some(1));
    assert_eq!(s2.next(), Some(0));
    assert_eq!(s2.next(), Some(1));
    assert_eq!(s2.next(), Some(2));
}

#[test]
fn a_panicking_update_does_not_brick_the_cell() {
    // The update closure runs under the hub lock. A panic leaves the current
    // value untouched, so a poisoned lock must not brick every later call.
    let cell = SubscriptionRef::new(0u32);
    let stream = cell.subscribe();
    assert_eq!(stream.next(), Some(0));
    let boom = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cell.update(|_| panic!("boom"));
    }));
    assert!(boom.is_err(), "the update panic must propagate");
    assert_eq!(cell.get(), 0, "the value was not half-written");
    cell.set(2);
    assert_eq!(stream.next(), Some(2), "the stream still delivers");
}

#[test]
fn subscription_ref_unsubscribe_stops() {
    let cell = SubscriptionRef::new(0);
    let s = cell.subscribe();
    assert_eq!(s.next(), Some(0));
    s.unsubscribe();
    cell.set(1);
    assert_eq!(s.next(), None);
    assert_eq!(s.poll(), None);
}

#[test]
fn subscription_ref_concurrent_use() {
    use std::sync::{Arc, Barrier};
    let cell = SubscriptionRef::new(0);
    let gate = Arc::new(Barrier::new(4));
    let mut handles = Vec::new();
    for i in 1..=3 {
        let (c, g) = (cell.clone(), gate.clone());
        handles.push(std::thread::spawn(move || {
            g.wait();
            for _ in 0..50 {
                c.update(|v| v + i);
            }
        }));
    }
    let s = cell.subscribe();
    gate.wait();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(cell.get(), 50 * (1 + 2 + 3));
    s.unsubscribe();
}

/// Regression: `subscribe` used to read the current value and register the
/// stream under two separate acquisitions, so a `set` in between was neither
/// in the snapshot nor queued: the new stream skipped it.
#[test]
fn subscription_ref_subscribe_never_skips_a_concurrent_set() {
    const LAST: u64 = 200;
    for _ in 0..50 {
        let cell = SubscriptionRef::new(0u64);
        let writer = cell.clone();
        let t = std::thread::spawn(move || {
            for value in 1..=LAST {
                writer.set(value);
            }
        });
        let stream = cell.subscribe();
        let mut seen = stream.next().expect("a stream opens on the current value");
        while seen < LAST {
            let next = stream.next().expect("the stream stays live");
            assert_eq!(next, seen + 1, "an update was skipped");
            seen = next;
        }
        t.join().expect("writer");
    }
}

// ---------------------------------------------------- PartitionedSemaphore ---

#[test]
fn semaphore_capacity_and_available_agree() {
    let sem = PartitionedSemaphore::<String>::new(2);
    assert_eq!(sem.capacity(), 2);
    assert_eq!(sem.available(), 2);
    sem.take("k".to_string(), 1);
    assert_eq!(sem.available(), 1);
    sem.release(1);
    assert_eq!(sem.available(), 2);
}

#[test]
fn semaphore_zero_permits_run_immediately() {
    let sem = PartitionedSemaphore::<String>::new(1);
    let ran = sem.with_permits("k".to_string(), 0, || 42);
    assert_eq!(ran, 42);
    assert_eq!(sem.available(), 1);
}

#[test]
fn semaphore_try_acquire_reports() {
    let sem = PartitionedSemaphore::<String>::new(1);
    sem.take("k".to_string(), 1);
    let mut ran = false;
    assert_eq!(
        sem.with_permits_if_available(1, || ran = true),
        None,
        "the pool is empty while the take holds it"
    );
    assert!(!ran, "an unavailable take never runs the body");
    sem.release(1);
    assert_eq!(sem.with_permits_if_available(1, || ()), Some(()));
}

/// The pin's defining rule: keys share ONE pool, so a busy key can leave
/// another key waiting. (This is the row the port used to get wrong with a
/// pool per key.)
#[test]
fn semaphore_keys_share_one_pool() {
    let sem = PartitionedSemaphore::<String>::new(1);
    sem.take("a".to_string(), 1);
    assert!(
        !sem.try_take(1),
        "key b cannot borrow a permit that key a holds"
    );
    sem.release(1);
    assert_eq!(sem.with_permits_if_available(1, || ()), Some(()));
}

#[test]
fn semaphore_with_permits_releases() {
    let sem = PartitionedSemaphore::<String>::new(2);
    sem.with_permits("k".to_string(), 2, || {
        assert_eq!(sem.available(), 0);
    });
    assert_eq!(sem.available(), 2);
}

/// Row 122: competing takes cannot acquire the same permit. Two takers of one
/// permit on one key run one after the other, and the permit comes back.
#[test]
fn semaphore_competing_takes_serialize() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    let sem = PartitionedSemaphore::<String>::new(1);
    let gate = Arc::new(Barrier::new(3));
    let order = Arc::new(Mutex::new(Vec::new()));
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for i in 0..2 {
        let (s, g, o, r, p) = (
            sem.clone(),
            gate.clone(),
            order.clone(),
            running.clone(),
            peak.clone(),
        );
        handles.push(std::thread::spawn(move || {
            g.wait();
            s.with_permits("k".to_string(), 1, || {
                p.fetch_max(r.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                o.lock().unwrap().push(i);
                std::thread::sleep(std::time::Duration::from_millis(20));
                r.fetch_sub(1, Ordering::SeqCst);
            });
        }));
    }
    gate.wait();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(peak.load(Ordering::SeqCst), 1, "never two holders at once");
    assert_eq!(order.lock().unwrap().len(), 2);
    assert_eq!(sem.available(), 1);
}

#[test]
fn replay_window_does_not_retain_values_that_slid_out() {
    // Row: "does not retain values published outside an undrained replay
    // window". A replay-1 hub: publish one value, snapshot the window, then
    // publish two more. The first value must have slid out of the LIVE
    // window, while the snapshot still holds it.
    let hub = PubSub::with_replay(usize::MAX, Strategy::Unbounded, 1);
    assert!(hub.publish(1));
    let snapshot = hub.replay_window();
    assert_eq!(snapshot, vec![1]);

    // Two more publishes push the first value out of the live window.
    assert!(hub.publish(2));
    assert!(hub.publish(3));
    assert_eq!(hub.replay_window(), vec![3], "only the last value survives");

    // The snapshot is independent: it retains what it copied, and the value
    // that slid out is NOT in the live window.
    assert!(!hub.replay_window().contains(&1));
    assert_eq!(snapshot, vec![1]);
}
