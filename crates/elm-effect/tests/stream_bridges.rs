//! Event primitives as stream sources.

use std::convert::Infallible;

use elm_effect::{Exit, PubSub, Stream, SubscriptionRef};

#[test]
fn from_change_stream_emits_the_snapshot_and_updates() {
    let cell = SubscriptionRef::new(1i32);
    let stream: Stream<i32, Infallible, ()> = Stream::from_change_stream(cell.subscribe());
    cell.set(2);
    cell.set(3);

    assert_eq!(stream.take(3.0).run_collect(), Exit::Success(vec![1, 2, 3]));
}

#[test]
fn from_pubsub_emits_published_values() {
    let hub = PubSub::unbounded();
    let subscription = hub.subscribe();
    assert!(hub.publish(7));
    assert!(hub.publish(8));

    let stream: Stream<i32, Infallible, ()> = Stream::from_pubsub(subscription);
    assert_eq!(stream.take(2.0).run_collect(), Exit::Success(vec![7, 8]));
}

#[test]
fn from_pubsub_ends_when_the_subscription_is_unsubscribed() {
    let hub = PubSub::bounded(4);
    let subscription = hub.subscribe();
    subscription.unsubscribe();

    let stream: Stream<i32, Infallible, ()> = Stream::from_pubsub(subscription);
    assert_eq!(stream.run_collect(), Exit::Success(Vec::new()));
}
