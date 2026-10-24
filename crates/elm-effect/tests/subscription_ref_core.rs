//! SubscriptionRef resubscribe behavior: each subscribed stream opens on the
//! current value and is notified once per later update, including a stream
//! opened while another stays live.

use elm_effect::SubscriptionRef;

#[test]
fn resubscribe_notifies_each_listener_once() {
    let cell = SubscriptionRef::new(0u32);
    let first = cell.subscribe();
    assert_eq!(first.poll(), Some(0), "first listener opens on current");
    let second = cell.subscribe();
    assert_eq!(second.poll(), Some(0), "resubscribe opens on current too");
    cell.set(1);
    assert_eq!(first.next(), Some(1), "first listener notified once");
    assert_eq!(
        second.next(),
        Some(1),
        "resubscribed listener notified once"
    );
    assert_eq!(first.poll(), None, "no duplicate delivery");
    assert_eq!(second.poll(), None, "no duplicate delivery");
}
