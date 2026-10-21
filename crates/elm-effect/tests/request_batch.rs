//! Request.test.ts rows that need the batching executor: how many resolver
//! calls a fan-out makes, how a cancelled caller withdraws only its own entry,
//! what the cache coalesces and replays, and which services a batch sees.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use elm_effect::request::{Request, Resolver, TaggedResolver, request};
use elm_effect::{
    Cause, Deferred, Effect, Exit, FiberHandle, ForkOpts, Reference, Runtime, read_reference,
};

/// The rows' `GetNameById` request class: a tag plus an id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GetNameById {
    id: u32,
}

impl Request for GetNameById {
    type Success = String;
    type Error = String;

    fn tag(&self) -> &'static str {
        "GetNameById"
    }
}

/// The pin's `userIds`: 1..=26.
fn user_ids() -> Vec<u32> {
    (1..=26).collect()
}

fn user_name(id: u32) -> String {
    format!("name-{id}")
}

/// The pin's `Effect.yieldNow` for a batch: let every submission register
/// before the batch closes.
fn settle() {
    std::thread::sleep(Duration::from_millis(50));
}

/// A resolver that records its invocations and the entries it was handed.
#[derive(Clone, Default)]
struct Tally {
    calls: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    batches: Arc<Mutex<Vec<Vec<u32>>>>,
}

impl Tally {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    fn batches(&self) -> Vec<Vec<u32>> {
        self.batches.lock().unwrap().clone()
    }
}

fn tallying_resolver<R, F>(tally: Tally, answer: F) -> Resolver<R>
where
    R: Request + 'static,
    F: Fn(&R) -> Exit<R::Success, R::Error> + Send + Sync + 'static,
{
    Resolver::<R>::make_sync(move |entries| {
        tally.calls.fetch_add(1, Ordering::SeqCst);
        tally.requests.fetch_add(entries.len(), Ordering::SeqCst);
        for entry in entries {
            entry.complete(answer(&entry.request));
        }
    })
}

fn names_resolver(tally: Tally) -> Resolver<GetNameById> {
    Resolver::<GetNameById>::make_sync(move |entries| {
        tally.calls.fetch_add(1, Ordering::SeqCst);
        tally.requests.fetch_add(entries.len(), Ordering::SeqCst);
        tally
            .batches
            .lock()
            .unwrap()
            .push(entries.iter().map(|entry| entry.request.id).collect());
        for entry in entries {
            entry.complete(Exit::Success(user_name(entry.request.id)));
        }
    })
}

/// Run a request effect on a fiber of its own, so the test can interrupt it.
fn fork_request(
    rt: &Runtime,
    resolver: &Resolver<GetNameById>,
    id: u32,
) -> FiberHandle<String, String> {
    let resolver = resolver.clone();
    rt.fork_detach_with(
        move |ctx| {
            ctx.spawn_effect(request(&resolver, GetNameById { id }))
                .join()
        },
        ForkOpts::immediate(),
    )
}

/// Submit one request per id on its own fiber and keep the handles.
fn fan_out(
    rt: &Runtime,
    resolver: &Resolver<GetNameById>,
    ids: impl IntoIterator<Item = u32>,
) -> Vec<FiberHandle<String, String>> {
    ids.into_iter()
        .map(|id| fork_request(rt, resolver, id))
        .collect()
}

fn join_all(handles: Vec<FiberHandle<String, String>>) -> Vec<Exit<String, String>> {
    handles.into_iter().map(|handle| handle.join()).collect()
}

/// A latch a batch's delay waits on, so a test can hold the batch open.
#[derive(Default)]
struct Gate {
    open: AtomicBool,
}

impl Gate {
    fn open(&self) {
        self.open.store(true, Ordering::SeqCst);
    }

    fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }
}

fn hold_until_open(gate: Arc<Gate>) -> Effect<(), std::convert::Infallible> {
    Effect::sync(move || {
        while !gate.is_open() {
            std::thread::sleep(Duration::from_millis(1));
        }
    })
}

/// Request.test.ts:167/180 `requests are executed correctly` (and the dual
/// syntax form, which is the same data-last call in this port): 27 requests
/// through a `batchN(15)` resolver make 3 downstream calls, and every name
/// comes back.
#[test]
fn a_fan_out_runs_three_batches_and_answers_every_request() {
    let tally = Tally::default();
    let gate = Arc::new(Gate::default());
    gate.open();
    let gate_for_resolver = Arc::clone(&gate);
    let resolver = names_resolver(tally.clone())
        .batch_n(15.0)
        .set_delay_effect(move || hold_until_open(Arc::clone(&gate_for_resolver)));
    let rt = Runtime::new();

    let all = Runtime::run(request(&resolver, GetNameById { id: 0 }));
    assert_eq!(
        all,
        Exit::Success(user_name(0)),
        "the ids batch answers once"
    );

    gate.close();
    let handles = fan_out(&rt, &resolver, user_ids());
    settle();
    gate.open();
    let exits = join_all(handles);
    for (index, exit) in exits.iter().enumerate() {
        assert_eq!(exit, &Exit::Success(user_name(index as u32 + 1)));
    }

    assert_eq!(tally.calls(), 3, "one ids batch plus two name batches");
    assert_eq!(
        tally.requests(),
        27,
        "every submitted request reached a batch"
    );
    rt.shutdown();
}

/// Request.test.ts:192 `requests are executed correctly with fromEffectTagged`:
/// the same fan-out through a tagged resolver keeps the batch counts.
#[test]
fn a_tagged_fan_out_runs_three_batches() {
    let calls = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let seen = Arc::clone(&requests);
    let gate = Arc::new(Gate::default());
    let gate_for_resolver = Arc::clone(&gate);
    let resolver = TaggedResolver::<GetNameById>::new()
        .on("GetNameById", move |batch| {
            counted.fetch_add(1, Ordering::SeqCst);
            seen.fetch_add(batch.len(), Ordering::SeqCst);
            Ok(batch.iter().map(|request| user_name(request.id)).collect())
        })
        .into_resolver()
        .batch_n(15.0)
        .set_delay_effect(move || hold_until_open(Arc::clone(&gate_for_resolver)));
    let rt = Runtime::new();

    gate.open();
    assert_eq!(
        Runtime::run(request(&resolver, GetNameById { id: 0 })),
        Exit::Success(user_name(0))
    );
    gate.close();
    let handles = fan_out(&rt, &resolver, user_ids());
    settle();
    gate.open();
    let exits = join_all(handles);
    assert_eq!(exits.len(), 26);
    assert!(exits.iter().all(|exit| exit.is_success()), "{exits:?}");

    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(requests.load(Ordering::SeqCst), 27);
    rt.shutdown();
}

/// Request.test.ts:654 `batching preserves individual & identical requests`:
/// two identical requests still count as two entries in one batch, and the
/// resolver runs once. No cache is involved.
#[test]
fn identical_requests_share_one_invocation() {
    let tally = Tally::default();
    let gate = Arc::new(Gate::default());
    let gate_for_resolver = Arc::clone(&gate);
    let resolver = names_resolver(tally.clone())
        .set_delay_effect(move || hold_until_open(Arc::clone(&gate_for_resolver)));
    let rt = Runtime::new();

    let handles = fan_out(&rt, &resolver, [1, 1]);
    settle();
    gate.open();
    let exits = join_all(handles);

    assert_eq!(
        exits,
        vec![Exit::Success(user_name(1)), Exit::Success(user_name(1))]
    );
    assert_eq!(tally.requests(), 2, "both submissions reached the batch");
    assert_eq!(tally.calls(), 1, "one invocation for the batch");
    rt.shutdown();
}

/// Request.test.ts:669 `grouped requests + batchN`: 26 requests grouped by
/// `id % 2` and capped at 5 per batch make 6 calls over 26 entries.
#[test]
fn grouped_batches_cap_each_key() {
    let calls = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let seen = Arc::clone(&requests);
    let gate = Arc::new(Gate::default());
    let gate_for_resolver = Arc::clone(&gate);
    let resolver = Resolver::<GetNameById>::make_sync(move |entries| {
        counted.fetch_add(1, Ordering::SeqCst);
        seen.fetch_add(entries.len(), Ordering::SeqCst);
        for entry in entries {
            entry.complete(Exit::Success(user_name(entry.request.id)));
        }
    })
    .batch_n(5.0)
    .grouped(|request: &GetNameById| request.id % 2)
    .set_delay_effect(move || hold_until_open(Arc::clone(&gate_for_resolver)));
    let rt = Runtime::new();

    let handles = fan_out(&rt, &resolver, user_ids());
    settle();
    gate.open();
    let exits = join_all(handles);
    assert!(exits.iter().all(|exit| exit.is_success()), "{exits:?}");

    assert_eq!(calls.load(Ordering::SeqCst), 6);
    assert_eq!(requests.load(Ordering::SeqCst), 26);
    rt.shutdown();
}

/// Request.test.ts:697 `batchN normalizes the maximum batch size`: NaN and
/// values below one behave as one, fractions floor, and 2.9 runs three
/// requests as 2 + 1.
#[test]
fn batch_n_normalizes_its_size() {
    let sizes = [
        (f64::NAN, vec![1usize, 1, 1]),
        (-1.0, vec![1, 1, 1]),
        (0.0, vec![1, 1, 1]),
        (0.5, vec![1, 1, 1]),
        (1.9, vec![1, 1, 1]),
        (2.9, vec![2, 1]),
    ];
    for (n, expected) in sizes {
        let tally = Tally::default();
        let gate = Arc::new(Gate::default());
        let gate_for_resolver = Arc::clone(&gate);
        let resolver = names_resolver(tally.clone())
            .batch_n(n)
            .set_delay_effect(move || hold_until_open(Arc::clone(&gate_for_resolver)));
        let rt = Runtime::new();
        let handles = fan_out(&rt, &resolver, [1, 2, 3]);
        settle();
        gate.open();
        let exits = join_all(handles);
        assert!(
            exits.iter().all(|exit| exit.is_success()),
            "n={n}: {exits:?}"
        );

        // Batches run concurrently here, so the order they report in is
        // scheduling; the batch sizes, which is what `batchN` decides, are not.
        let mut seen: Vec<usize> = tally.batches().iter().map(|batch| batch.len()).collect();
        seen.sort_unstable();
        let mut expected = expected;
        expected.sort_unstable();
        assert_eq!(seen, expected, "batchN({n})");
        rt.shutdown();
    }
}

// Services

static REQUEST_SERVICE: Reference<String> = Reference {
    id: "test/request-service",
    default: String::new(),
};

/// Request.test.ts:727 `batch fibers use request services for runAll`: the
/// resolver body reads the value provided on the request effect. This port's
/// per-effect service channel is a [`Reference`], read inside the batch.
#[test]
fn a_batch_sees_the_services_provided_to_the_request() {
    let resolver = Resolver::<GetNameById>::make(move |entries| {
        read_reference(&REQUEST_SERVICE)
            .flat_map(move |value| {
                let entries = entries.clone();
                Effect::sync(move || {
                    for entry in &entries {
                        entry.complete(Exit::Success(value.clone()));
                    }
                })
            })
            .map_err(|never: std::convert::Infallible| -> String { match never {} })
    })
    .batch_n(1.0);

    let effect = request(&resolver, GetNameById { id: 1 })
        .provide_reference(&REQUEST_SERVICE, "provided".to_owned());
    assert_eq!(Runtime::run(effect), Exit::Success("provided".to_owned()));
}

/// Request.test.ts:745 `batch fibers use request services for delay effects`:
/// the delay runs with the request's services too, not the resolver's
/// defaults.
#[test]
fn a_delay_sees_the_services_provided_to_the_request() {
    let seen = Arc::new(Mutex::new(String::new()));
    let recorded = Arc::clone(&seen);
    let resolver = Resolver::make_sync(move |entries| {
        for entry in entries {
            entry.complete(Exit::Success("ok".to_owned()));
        }
    })
    .set_delay_effect(move || {
        let recorded = Arc::clone(&recorded);
        read_reference(&REQUEST_SERVICE).flat_map(move |value| {
            *recorded.lock().unwrap() = value;
            Effect::sync(|| ())
        })
    });

    let effect = request(&resolver, GetNameById { id: 1 })
        .provide_reference(&REQUEST_SERVICE, "provided".to_owned());
    assert_eq!(Runtime::run(effect), Exit::Success("ok".to_owned()));
    assert_eq!(*seen.lock().unwrap(), "provided");
}

// Cache

/// Request.test.ts:427 `withCache still caches completed failures`: two
/// requests for the same id, one resolver call, both callers see the failure.
#[test]
fn cache_replays_completed_failures() {
    let tally = Tally::default();
    let resolver = tallying_resolver(tally.clone(), |_request: &GetNameById| {
        Exit::<String, String>::Failure(Cause::fail("Not Found".to_owned()))
    })
    .with_cache(10);

    let failure = Exit::<String, String>::Failure(Cause::fail("Not Found".to_owned()));
    assert_eq!(
        Runtime::run(request(&resolver, GetNameById { id: 1 }).exit()),
        Exit::Success(failure.clone())
    );
    assert_eq!(
        Runtime::run(request(&resolver, GetNameById { id: 1 }).exit()),
        Exit::Success(failure)
    );
    assert_eq!(tally.calls(), 1);
}

/// Request.test.ts:446 `withCache does not cache interrupted resolver
/// results`: the interrupted answer is dropped, so the retry runs the
/// resolver again and succeeds.
#[test]
fn cache_drops_interrupted_results() {
    let tally = Tally::default();
    let calls = Arc::clone(&tally.calls);
    let resolver = Resolver::make_sync(move |entries| {
        let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
        for entry in entries {
            if call == 1 {
                entry.complete(Exit::Failure(Cause::interrupt(None)));
            } else {
                entry.complete(Exit::Success("Alice".to_owned()));
            }
        }
    })
    .with_cache(10);

    let first = Runtime::run(request(&resolver, GetNameById { id: 1 }).exit());
    match first {
        Exit::Success(Exit::Failure(cause)) => assert!(cause.has_interrupts()),
        other => panic!("the first call is interrupted: {other:?}"),
    }
    assert_eq!(
        Runtime::run(request(&resolver, GetNameById { id: 1 })),
        Exit::Success("Alice".to_owned())
    );
    assert_eq!(tally.calls(), 2);
}

/// Request.test.ts:338 `repro: cancelling a pending batch does not poison
/// ${cached ? "cached" : "uncached"} requests`: a caller cancelled while the
/// batch's delay is pending does not keep the retry's batch from running.
#[test]
fn cancelling_a_pending_batch_does_not_poison_the_retry() {
    for cached in [false, true] {
        let tally = Tally::default();
        let gate = Arc::new(Gate::default());
        let gate_for_resolver = Arc::clone(&gate);
        let base = names_resolver(tally.clone())
            .set_delay_effect(move || hold_until_open(Arc::clone(&gate_for_resolver)));
        let resolver = if cached { base.with_cache(10) } else { base };
        let rt = Runtime::new();

        let first = fork_request(&rt, &resolver, 1);
        settle();
        first.interrupt();
        assert_eq!(tally.calls(), 0, "cached={cached}: nothing ran while held");

        let next = fork_request(&rt, &resolver, 1);
        settle();
        gate.open();
        let exit = next.join();
        assert_eq!(exit, Exit::Success(user_name(1)), "cached={cached}");
        first.join();
        rt.shutdown();
    }
}

/// Request.test.ts:363 `withCache removes cancelled entries from a surviving
/// batch`: the withdrawn entry is not handed to the resolver, and a later
/// request for it runs its own batch.
#[test]
fn cache_removes_a_cancelled_entry_from_the_batch() {
    let tally = Tally::default();
    let gate = Arc::new(Gate::default());
    let gate_for_resolver = Arc::clone(&gate);
    let resolver = names_resolver(tally.clone())
        .set_delay_effect(move || hold_until_open(Arc::clone(&gate_for_resolver)))
        .with_cache(10);
    let rt = Runtime::new();

    let first = fork_request(&rt, &resolver, 1);
    let other = fork_request(&rt, &resolver, 2);
    settle();
    first.interrupt();
    gate.open();
    assert_eq!(other.join(), Exit::Success(user_name(2)));
    assert_eq!(
        tally.batches(),
        vec![vec![2]],
        "only the surviving entry ran"
    );

    let retry = fork_request(&rt, &resolver, 1);
    assert_eq!(retry.join(), Exit::Success(user_name(1)));
    assert_eq!(tally.batches(), vec![vec![2], vec![1]]);
    first.join();
    rt.shutdown();
}

/// Request.test.ts:397 `withCache preserves coalesced requests when the
/// ${cancelFirst ? "first" : "second"} caller cancels`: both callers share one
/// entry, cancelling either leaves the other's answer in place, and the
/// resolver sees one entry.
#[test]
fn cache_keeps_coalesced_requesters_alive() {
    for cancel_first in [false, true] {
        let tally = Tally::default();
        let gate = Arc::new(Gate::default());
        let gate_for_resolver = Arc::clone(&gate);
        let resolver = names_resolver(tally.clone())
            .set_delay_effect(move || hold_until_open(Arc::clone(&gate_for_resolver)))
            .with_cache(10);
        let rt = Runtime::new();

        let first = fork_request(&rt, &resolver, 1);
        let second = fork_request(&rt, &resolver, 1);
        settle();
        let cancelled = if cancel_first { &first } else { &second };
        cancelled.interrupt();
        assert!(tally.batches().is_empty(), "nothing ran before the gate");
        gate.open();

        let remaining = if cancel_first {
            second.join()
        } else {
            first.join()
        };
        assert_eq!(
            remaining,
            Exit::Success(user_name(1)),
            "cancel_first={cancel_first}"
        );
        assert_eq!(tally.batches(), vec![vec![1]], "one shared entry ran");
        assert_eq!(
            Runtime::run(request(&resolver, GetNameById { id: 1 })),
            Exit::Success(user_name(1))
        );
        assert_eq!(tally.batches(), vec![vec![1]], "the answer is cached");
        rt.shutdown();
    }
}

/// Request.test.ts:463 `withCache cancellation does not evict a replacement
/// entry`: filling a capacity-1 cache evicts the pending row; a replacement
/// request opens a new row; cancelling the old entry leaves the replacement
/// alone.
#[test]
fn cache_cancellation_does_not_evict_a_replacement() {
    let tally = Tally::default();
    let gate = Arc::new(Gate::default());
    let delays = Arc::new(AtomicUsize::new(0));
    let gate_for_resolver = Arc::clone(&gate);
    let counter = Arc::clone(&delays);
    let resolver = names_resolver(tally.clone())
        .grouped(|request: &GetNameById| request.id)
        .set_delay_effect(move || {
            let first = counter.fetch_add(1, Ordering::SeqCst) == 0;
            let gate = Arc::clone(&gate_for_resolver);
            Effect::sync(move || {
                if first {
                    while !gate.is_open() {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            })
        })
        .with_cache(1);
    let rt = Runtime::new();

    let first = fork_request(&rt, &resolver, 1);
    settle();
    // Completing id 2 evicts id 1's pending row, so id 1 can be requested
    // again while the first batch is still held.
    assert_eq!(
        Runtime::run(request(&resolver, GetNameById { id: 2 })),
        Exit::Success(user_name(2))
    );
    let replacement = fork_request(&rt, &resolver, 1);
    settle();
    first.interrupt();
    let coalesced = fork_request(&rt, &resolver, 1);
    settle();
    gate.open();

    assert_eq!(replacement.join(), Exit::Success(user_name(1)));
    assert_eq!(coalesced.join(), Exit::Success(user_name(1)));
    assert_eq!(tally.batches(), vec![vec![2], vec![1]]);
    first.join();
    rt.shutdown();
}

// Racing resolvers

/// Request.test.ts:548 `withCache preserves the race winner when the losing
/// resolver is interrupted`: the fast resolver wins, the slow one is
/// interrupted with its handler run, and the cached winner serves the retry
/// without calling the winner again.
#[test]
fn race_keeps_the_winner_and_interrupts_the_loser() {
    let finished = Arc::new(Deferred::<(), ()>::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let interruptions = Arc::new(AtomicUsize::new(0));
    let slow_running = Arc::new(Deferred::<(), ()>::new());

    let fast_calls = Arc::clone(&calls);
    let fast_wait = Arc::clone(&slow_running);
    let fast = Resolver::<GetNameById>::from_effect(move |_entry| {
        let calls = Arc::clone(&fast_calls);
        let wait = Arc::clone(&fast_wait);
        Effect::sync(move || {
            // The pin's `yieldNow` here lets the losing leg install its
            // interrupt handler. A fiber start has no observable edge in this
            // port, so the winner waits a fixed window instead.
            wait.wait();
            std::thread::sleep(Duration::from_millis(50));
            calls.fetch_add(1, Ordering::SeqCst);
            user_name(1)
        })
    });

    let slow_interruptions = Arc::clone(&interruptions);
    let slow_signal = Arc::clone(&slow_running);
    let slow = Resolver::<GetNameById>::from_effect(move |_entry| {
        let signal = Arc::clone(&slow_signal);
        let interruptions = Arc::clone(&slow_interruptions);
        Effect::sync(move || {
            signal.complete(());
        })
        .flat_map(move |()| {
            let interruptions = Arc::clone(&interruptions);
            Effect::<String, String, ()>::never().on_interrupt(move || {
                interruptions.fetch_add(1, Ordering::SeqCst);
            })
        })
    });

    let after = Arc::clone(&finished);
    let resolver = fast
        .race(slow)
        .around(
            |_entries: &[elm_effect::request::Entry<GetNameById>]| Effect::sync(|| ()),
            move |_entries: &[elm_effect::request::Entry<GetNameById>]| {
                after.complete(());
                Effect::sync(|| ())
            },
        )
        .with_cache(10);

    assert_eq!(
        Runtime::run(request(&resolver, GetNameById { id: 1 })),
        Exit::Success(user_name(1))
    );
    finished.wait();
    assert_eq!(
        interruptions.load(Ordering::SeqCst),
        1,
        "the losing resolver's interrupt handler ran"
    );
    assert_eq!(
        Runtime::run(request(&resolver, GetNameById { id: 1 })),
        Exit::Success(user_name(1)),
        "the winner was cached"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the winner ran once");
}

// Interruption

/// Request.test.ts:267 `requests don't break interruption`: interrupting the
/// caller while its batch is still held ends it with an interrupt and never
/// runs the resolver.
#[test]
fn interrupting_a_held_batch_never_runs_the_resolver() {
    let tally = Tally::default();
    let gate = Arc::new(Gate::default());
    let started = Arc::new(Deferred::<(), ()>::new());
    let gate_for_resolver = Arc::clone(&gate);
    let started_for_resolver = Arc::clone(&started);
    let resolver = names_resolver(tally.clone()).set_delay_effect(move || {
        started_for_resolver.complete(());
        hold_until_open(Arc::clone(&gate_for_resolver))
    });
    let rt = Runtime::new();

    let caller = fork_request(&rt, &resolver, 1);
    started.wait();
    caller.interrupt();
    let exit = caller.join();
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.has_interrupts()),
        "the caller ends interrupted: {exit:?}"
    );
    assert_eq!(tally.calls(), 0, "the resolver never ran");
    gate.open();
    rt.shutdown();
}

/// Request.test.ts:288 `requests work with uninterruptible`: a masked caller
/// keeps waiting for each batch, so all three requests are answered even
/// though interruption was requested, and the fiber still ends interrupted.
#[test]
fn uninterruptible_requests_wait_for_their_batch() {
    let tally = Tally::default();
    let gate = Arc::new(Gate::default());
    let started = Arc::new(Deferred::<(), ()>::new());
    let gate_for_resolver = Arc::clone(&gate);
    let started_for_resolver = Arc::clone(&started);
    let resolver = names_resolver(tally.clone())
        .batch_n(1.0)
        .set_delay_effect(move || {
            started_for_resolver.complete(());
            hold_until_open(Arc::clone(&gate_for_resolver))
        });
    let rt = Runtime::new();

    let caller = rt.fork_detach_with(
        {
            let resolver = resolver.clone();
            move |ctx| {
                let one = resolver.clone();
                let two = resolver.clone();
                let three = resolver.clone();
                let body = request(&one, GetNameById { id: 1 }).flat_map(move |a| {
                    let two = two.clone();
                    request(&two, GetNameById { id: 2 }).flat_map(move |b| {
                        request(&three, GetNameById { id: 3 })
                            .map(move |c| (a.clone(), b.clone(), c))
                    })
                });
                ctx.spawn_effect(body.uninterruptible()).join()
            }
        },
        ForkOpts::immediate(),
    );
    started.wait();
    caller.interrupt_unsafe();
    gate.open();
    let exit = caller.join();
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.has_interrupts()),
        "the fiber ends interrupted after the masked region: {exit:?}"
    );
    assert_eq!(tally.calls(), 3, "every batch ran despite the interrupt");
    rt.shutdown();
}

/// Request.test.ts:309 `grouped requests can be interrupted before
/// execution`: the delay never completes, and the interrupted caller leaves
/// the resolver unexecuted.
#[test]
fn a_grouped_batch_held_forever_never_executes() {
    let executed = Arc::new(AtomicBool::new(false));
    let ran = Arc::clone(&executed);
    let resolver = Resolver::<GetNameById>::make(move |entries| {
        ran.store(true, Ordering::SeqCst);
        Effect::sync(move || {
            for entry in &entries {
                entry.complete(Exit::Success(user_name(entry.request.id)));
            }
        })
    })
    .grouped(|request: &GetNameById| request.id)
    .set_delay_effect(Effect::never);
    let rt = Runtime::new();

    let caller = fork_request(&rt, &resolver, 1);
    settle();
    caller.interrupt();
    let exit = caller.join();
    assert!(
        matches!(&exit, Exit::Failure(cause) if cause.has_interrupts()),
        "the caller ends interrupted: {exit:?}"
    );
    assert!(!executed.load(Ordering::SeqCst), "the resolver never ran");
    rt.shutdown();
}
