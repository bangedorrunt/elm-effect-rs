//! Exit-aware scope finalizers: a release can see whether the body
//! succeeded, failed, was interrupted, or left no exit at all.

#![cfg(feature = "tokio")]

use std::sync::{Arc, Mutex};

use elm_effect::{
    Cause, Defect, Effect, Exit, ForkOpts, Runtime, Scope, ScopeExit, ScopeOutcome, ScopeTag,
    use_service,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Body {
    Succeed,
    Fail,
    Die,
    Interrupt,
}

#[derive(Clone, Debug, PartialEq)]
struct Shape {
    outcome: ScopeOutcome,
    is_success: bool,
    is_failure: bool,
    interrupted: bool,
    failures: usize,
    defects: usize,
}

impl Shape {
    fn of(exit: &ScopeExit) -> Self {
        Self {
            outcome: exit.outcome(),
            is_success: exit.is_success(),
            is_failure: exit.is_failure(),
            interrupted: exit.is_interrupted(),
            failures: exit.failures(),
            defects: exit.defects().len(),
        }
    }
}

fn scoped_with_exit(body: Body, seen: Arc<Mutex<Vec<Shape>>>) -> Effect<(), String> {
    Effect::scoped(
        use_service::<ScopeTag>()
            .map(move |scope| {
                let seen = Arc::clone(&seen);
                scope.add_finalizer_with_exit(move |exit| {
                    seen.lock().unwrap().push(Shape::of(exit));
                });
            })
            // Reading a service cannot fail; widen once to the body channel.
            .map_err(|never| match never {})
            .flat_map(move |()| match body {
                Body::Succeed => Effect::<(), String, ScopeTag>::succeed(()),
                Body::Fail => Effect::<(), String, ScopeTag>::fail("body failed".to_string()),
                Body::Die => Effect::<(), String, ScopeTag>::die(Defect::new("kaput")),
                Body::Interrupt => Effect::<(), String, ScopeTag>::interrupt(),
            }),
    )
}

#[test]
fn scoped_finalizer_sees_the_body_exit() {
    let cases = [
        (
            Body::Succeed,
            Shape {
                outcome: ScopeOutcome::Success,
                is_success: true,
                is_failure: false,
                interrupted: false,
                failures: 0,
                defects: 0,
            },
        ),
        (
            Body::Fail,
            Shape {
                outcome: ScopeOutcome::Failure,
                is_success: false,
                is_failure: true,
                interrupted: false,
                failures: 1,
                defects: 0,
            },
        ),
        (
            Body::Die,
            Shape {
                outcome: ScopeOutcome::Failure,
                is_success: false,
                is_failure: true,
                interrupted: false,
                failures: 0,
                defects: 1,
            },
        ),
        (
            Body::Interrupt,
            Shape {
                outcome: ScopeOutcome::Failure,
                is_success: false,
                is_failure: true,
                interrupted: true,
                failures: 0,
                defects: 0,
            },
        ),
    ];
    for (body, expected) in cases {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let out = Runtime::run(scoped_with_exit(body, Arc::clone(&seen)));
        match body {
            Body::Succeed => assert!(matches!(out, Exit::Success(()))),
            _ => assert!(matches!(out, Exit::Failure(_))),
        }
        assert_eq!(
            *seen.lock().unwrap(),
            vec![expected],
            "finalizer saw the wrong exit for {body:?}"
        );
    }
}

#[test]
fn an_effect_finalizer_sees_the_body_exit() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&seen);
    let effect = Effect::scoped(
        use_service::<ScopeTag>()
            .map(move |scope| {
                let probe = Arc::clone(&probe);
                scope.add_finalizer_effect_with_exit(move |exit| {
                    probe.lock().unwrap().push(Shape::of(exit));
                    Effect::<(), String>::succeed(())
                });
            })
            .map_err(|never| match never {})
            .flat_map(|()| Effect::<(), String, ScopeTag>::fail("body failed".to_string())),
    );
    assert!(matches!(Runtime::run(effect), Exit::Failure(_)));
    assert_eq!(
        *seen.lock().unwrap(),
        vec![Shape {
            outcome: ScopeOutcome::Failure,
            is_success: false,
            is_failure: true,
            interrupted: false,
            failures: 1,
            defects: 0,
        }],
        "the effect release must see the typed failure it is settling"
    );
}

#[test]
fn a_late_exit_aware_effect_finalizer_sees_the_closing_exit() {
    let scope = Scope::new(4);
    let seen = Arc::new(Mutex::new(Vec::new()));
    assert!(scope.close_with(&ScopeExit::success()).is_empty());

    let probe = Arc::clone(&seen);
    let defect = scope.add_finalizer_effect_with_exit(move |exit| {
        probe.lock().unwrap().push(Shape::of(exit));
        Effect::fail("late release".to_string())
    });

    assert_eq!(
        seen.lock().unwrap()[0].outcome,
        ScopeOutcome::Success,
        "a late release sees the exit the scope closed with"
    );
    let defect = defect.expect("a late effect-finalizer failure must surface, never drop");
    assert!(defect.message.contains("late release"));
}

#[test]
fn a_fiber_finalizer_sees_the_fiber_exit() {
    let rt = Runtime::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&seen);
    let handle = rt.fork_detach_with(
        move |ctx| {
            let probe = Arc::clone(&probe);
            ctx.add_finalizer_with_exit(move |exit| {
                probe.lock().unwrap().push(Shape::of(exit));
            });
            Exit::<(), String>::fail("body failed".to_string())
        },
        ForkOpts::immediate(),
    );
    let _ = handle.join();
    assert_eq!(
        *seen.lock().unwrap(),
        vec![Shape {
            outcome: ScopeOutcome::Failure,
            is_success: false,
            is_failure: true,
            interrupted: false,
            failures: 1,
            defects: 0,
        }]
    );
    assert!(rt.shutdown().is_empty());
}

#[test]
fn an_interrupted_success_is_seen_as_an_interrupt() {
    // The body returns success after requesting its own interrupt. The
    // supervisor settles the exit to interrupted before the scope closes, so
    // the finalizer must not be told the body succeeded.
    let rt = Runtime::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&seen);
    let handle = rt.fork_detach_with(
        move |ctx| {
            let probe = Arc::clone(&probe);
            ctx.add_finalizer_with_exit(move |exit| {
                probe.lock().unwrap().push(Shape::of(exit));
            });
            ctx.fiber().interrupt();
            Exit::<(), String>::succeed(())
        },
        ForkOpts::immediate(),
    );
    assert!(matches!(handle.join(), Exit::Failure(_)));
    let shapes = seen.lock().unwrap();
    assert_eq!(shapes.len(), 1);
    assert!(
        shapes[0].interrupted && shapes[0].is_failure,
        "the finalizer saw {:?}",
        shapes[0]
    );
    assert!(rt.shutdown().is_empty());
}

#[test]
fn a_direct_close_has_no_exit() {
    let scope = Scope::new(1);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&seen);
    scope.add_finalizer_with_exit(move |exit| {
        probe.lock().unwrap().push(Shape::of(exit));
    });
    assert!(scope.close().is_empty());
    assert_eq!(
        *seen.lock().unwrap(),
        vec![Shape {
            outcome: ScopeOutcome::Unknown,
            is_success: false,
            is_failure: false,
            interrupted: false,
            failures: 0,
            defects: 0,
        }]
    );
}

#[test]
fn a_late_exit_aware_finalizer_sees_the_closing_exit() {
    let scope = Scope::new(3);
    let seen = Arc::new(Mutex::new(Vec::new()));
    assert!(scope.close_with(&ScopeExit::success()).is_empty());
    let probe = Arc::clone(&seen);
    scope.add_finalizer_with_exit(move |exit| {
        probe.lock().unwrap().push(Shape::of(exit));
    });
    assert_eq!(
        *seen.lock().unwrap(),
        vec![Shape {
            outcome: ScopeOutcome::Success,
            is_success: true,
            is_failure: false,
            interrupted: false,
            failures: 0,
            defects: 0,
        }]
    );
}

#[test]
fn exit_aware_and_plain_finalizers_share_lifo() {
    let scope = Scope::new(2);
    let order = Arc::new(Mutex::new(Vec::new()));
    let first = Arc::clone(&order);
    scope.add_finalizer(move || first.lock().unwrap().push("plain"));
    let second = Arc::clone(&order);
    scope.add_finalizer_with_exit(move |_| second.lock().unwrap().push("exit-aware"));
    let exit = ScopeExit::from_exit(&Exit::<(), String>::fail("e".to_string()));
    assert!(scope.close_with(&exit).is_empty());
    assert_eq!(*order.lock().unwrap(), vec!["exit-aware", "plain"]);
}

#[test]
fn scoped_typed_failure_and_defect_are_counted_separately() {
    // A mixed cause keeps both: one typed failure and one defect.
    let cause = Cause::<String> {
        reasons: vec![
            elm_effect::Reason::Fail("f".to_string()),
            elm_effect::Reason::Die(Defect::new("d")),
        ],
        ..Cause::empty()
    };
    let exit = ScopeExit::from_exit(&Exit::<(), String>::Failure(cause));
    assert!(exit.is_failure());
    assert_eq!(exit.failures(), 1);
    assert_eq!(exit.defects().len(), 1);
    assert!(!exit.is_interrupted());
}
