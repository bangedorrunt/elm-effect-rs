//! hi5 tail slice: SubscriptionRef resubscribe cells.
//! Ports packages/effect/test/AtomRef.test.ts (listener resubscribe
//! notification), left uncovered by the area beads.

use elm_effect::SubscriptionRef;

// AtomRef.test.ts:5 — each subscribed listener is notified exactly once,
// including a listener that resubscribes while another stays live.
#[test]
fn resubscribe_notifies_each_listener_once() {
    let cell = SubscriptionRef::new(0u32);
    let first = cell.subscribe();
    assert_eq!(cell.poll(first), Some(0), "first listener opens on current");
    let second = cell.subscribe();
    assert_eq!(second, first + 1, "resubscribe registers a second listener");
    assert_eq!(
        cell.poll(second),
        Some(0),
        "resubscribe opens on current too"
    );
    cell.set(1);
    assert_eq!(cell.poll(first), Some(1), "first listener notified once");
    assert_eq!(
        cell.poll(second),
        Some(1),
        "resubscribed listener notified once"
    );
    assert_eq!(cell.poll(first), None, "no duplicate delivery");
    assert_eq!(cell.poll(second), None, "no duplicate delivery");
    assert_eq!(cell.poll(999), None, "unknown listener reads nothing");
}
