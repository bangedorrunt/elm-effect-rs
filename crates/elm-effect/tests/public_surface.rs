//! The facade must expose the primitives the audit says are kept.

use elm_effect::prelude::*;
use elm_effect::pull::Pull;
use elm_effect::take::Take;
use elm_effect::{
    Crypto, FiberMap, FiberSet, IdlePolicy, LookupFailure, PubSub, RcMap, Request, RequestResolver,
    Resolver, Sink, TestCrypto, request,
};

#[derive(Clone)]
struct Echo(u32);

impl Request for Echo {
    type Success = u32;
    type Error = String;

    fn tag(&self) -> &'static str {
        "Echo"
    }
}

#[test]
fn kept_primitives_are_public() {
    let mut set = FiberSet::<(), String>::new(Runtime::new());
    set.run(|_| Exit::Success(()));
    assert!(matches!(set.join(), Exit::Success(())));

    let mut map = FiberMap::<u8, (), String>::new(Runtime::new());
    map.set(1, |_| Exit::Success(()));
    assert!(matches!(map.join(), Exit::Success(())));

    let hub = PubSub::<i32>::bounded(1);
    let subscription = hub.subscribe();
    assert!(hub.publish(7));
    assert_eq!(subscription.take(), Some(7));

    let cache = Cache::new(4, |key: &u32| Ok::<u32, String>(*key));
    assert_eq!(cache.get(&7), Ok(7));

    let pool = Pool::new(1, IdlePolicy::default(), || 3u32);
    assert_eq!(*pool.acquire(), 3);

    let latch = Latch::new_open();
    assert_eq!(latch.await_open(&|| false), Ok(()));

    let _: Option<RequestResolver<Echo>> = None;
    let resolver = Resolver::<Echo>::make_sync(|entries| {
        for entry in entries {
            entry.complete(Exit::Success(entry.request.0));
        }
    });
    assert_eq!(Runtime::run(request(&resolver, Echo(7))), Exit::Success(7));

    let resources =
        RcMap::<u32, u32, String>::make(|key: &u32| Ok::<u32, LookupFailure<String>>(*key), |_| {});
    let lease = resources.get(&3).unwrap();
    assert_eq!(lease.value(), Some(&3));
    drop(lease);

    let crypto = TestCrypto::new();
    assert_eq!(crypto.random_bytes(2).unwrap().len(), 2);

    let sink = Sink::<i32, String, usize>::from_fn(|mut source| {
        let mut count = 0;
        loop {
            match source.pull() {
                Take::Items(items) => count += items.len(),
                Take::End => return Exit::Success(count),
                Take::Fail(cause) => return Exit::Failure(cause),
            }
        }
    });
    assert!(matches!(sink.run(Pull::empty()), Exit::Success(0)));
}

#[test]
fn prelude_exposes_the_core_and_the_macro() {
    let effect: Effect<i32, std::convert::Infallible> = effect! {
        let a = Effect::<i32, std::convert::Infallible>::succeed(20);
        let b = Effect::<i32, std::convert::Infallible>::succeed(22);
        Effect::<i32, std::convert::Infallible>::succeed(a + b)
    };
    assert_eq!(Runtime::run(effect), Exit::Success(42));

    // The module path is part of the prelude too, not only its root types.
    let _: Option<policy::RemotePolicy<i32, String>> = None;
}
