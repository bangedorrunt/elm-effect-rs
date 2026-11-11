//! Executor-parameterized carrier laws.
//!
//! [`run`] drives the same public-API scenarios against any [`Runtime`]. The
//! deterministic executor and the Tokio pool must agree; a law that only
//! passes on one executor is an implementation accident, not a carrier
//! guarantee.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use elm_effect::{Defect, Effect, Exit, ForkOpts, Runtime, ScopeTag, use_service};

type LawFn = fn(&Runtime, &dyn Fn()) -> Result<(), String>;

/// One named law and its outcome.
#[derive(Debug)]
pub struct LawResult {
    pub name: &'static str,
    pub outcome: Result<(), String>,
}

/// Run every carrier law on `rt`.
///
/// `drive` advances an executor that needs an explicit turn (the
/// deterministic executor drains its queue; the Tokio executor is already
/// running and passes a no-op).
pub fn run(rt: &Runtime, drive: &dyn Fn()) -> Vec<LawResult> {
    let laws: &[(&str, LawFn)] = &[
        ("outcomes_stay_apart", outcomes_stay_apart),
        ("scope_releases_lifo", scope_releases_lifo),
        (
            "scope_releases_on_typed_failure",
            scope_releases_on_typed_failure,
        ),
        ("scope_releases_on_interrupt", scope_releases_on_interrupt),
        ("panic_is_a_defect", panic_is_a_defect),
        (
            "fiber_finalizer_runs_before_join",
            fiber_finalizer_runs_before_join,
        ),
    ];
    laws.iter()
        .map(|(name, law)| LawResult {
            name,
            outcome: law(rt, drive),
        })
        .collect()
}

fn outcomes_stay_apart(rt: &Runtime, drive: &dyn Fn()) -> Result<(), String> {
    let success = run_effect(rt, Effect::<u32, String>::succeed(7), drive);
    match success {
        Exit::Success(7) => {}
        other => return Err(format!("success became {other:?}")),
    }

    let failure = run_effect(rt, Effect::<u32, String>::fail("typed".to_string()), drive);
    match failure {
        Exit::Failure(cause)
            if cause.failure() == Some(&"typed".to_string())
                && cause.defects().is_empty()
                && !cause.is_interrupted() => {}
        other => return Err(format!("typed failure became {other:?}")),
    }

    let defect = run_effect(rt, Effect::<u32, String>::die(Defect::new("defect")), drive);
    match defect {
        Exit::Failure(cause)
            if cause.failure().is_none()
                && cause.defects().len() == 1
                && !cause.is_interrupted() => {}
        other => return Err(format!("defect became {other:?}")),
    }

    let interrupt = run_effect(rt, Effect::<u32, String>::interrupt(), drive);
    match interrupt {
        Exit::Failure(cause)
            if cause.is_interrupted()
                && cause.failure().is_none()
                && cause.defects().is_empty() => {}
        other => return Err(format!("interrupt became {other:?}")),
    }

    Ok(())
}

fn scope_releases_lifo(rt: &Runtime, drive: &dyn Fn()) -> Result<(), String> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let exit = run_effect(
        rt,
        scoped_with_finalizers(Arc::clone(&log), Body::Success),
        drive,
    );
    if !matches!(exit, Exit::Success(())) {
        return Err(format!("successful scope exited as {exit:?}"));
    }
    assert_releases(&log, "success")
}

fn scope_releases_on_typed_failure(rt: &Runtime, drive: &dyn Fn()) -> Result<(), String> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let exit = run_effect(
        rt,
        scoped_with_finalizers(Arc::clone(&log), Body::Fail),
        drive,
    );
    match exit {
        Exit::Failure(cause) if cause.failure() == Some(&"body failed".to_string()) => {}
        other => return Err(format!("typed failure exited as {other:?}")),
    }
    assert_releases(&log, "typed failure")
}

fn scope_releases_on_interrupt(rt: &Runtime, drive: &dyn Fn()) -> Result<(), String> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let exit = run_effect(
        rt,
        scoped_with_finalizers(Arc::clone(&log), Body::Interrupt),
        drive,
    );
    match exit {
        Exit::Failure(cause) if cause.is_interrupted() => {}
        other => return Err(format!("interrupt exited as {other:?}")),
    }
    assert_releases(&log, "interrupt")
}

fn panic_is_a_defect(rt: &Runtime, drive: &dyn Fn()) -> Result<(), String> {
    let handle = rt.fork_detach_with(
        |_ctx| -> Exit<(), String> { panic!("runtime law: child panic") },
        ForkOpts::immediate(),
    );
    drive();
    match handle.join() {
        Exit::Failure(cause)
            if cause.failure().is_none()
                && cause.defects().len() == 1
                && !cause.is_interrupted() =>
        {
            Ok(())
        }
        other => Err(format!("panicking child exited as {other:?}")),
    }
}

fn fiber_finalizer_runs_before_join(rt: &Runtime, drive: &dyn Fn()) -> Result<(), String> {
    let released = Arc::new(AtomicBool::new(false));
    let probe = Arc::clone(&released);
    let handle = rt.fork_detach_with(
        move |ctx| {
            ctx.add_finalizer(move || probe.store(true, Ordering::SeqCst));
            Exit::<(), String>::succeed(())
        },
        ForkOpts::immediate(),
    );
    drive();
    let _ = handle.join();
    if released.load(Ordering::SeqCst) {
        Ok(())
    } else {
        Err("fiber finalizer did not run before join returned".to_string())
    }
}

#[derive(Clone, Copy)]
enum Body {
    Success,
    Fail,
    Interrupt,
}

fn scoped_with_finalizers(log: Arc<Mutex<Vec<&'static str>>>, body: Body) -> Effect<(), String> {
    Effect::scoped(
        use_service::<ScopeTag>()
            .map(move |scope| {
                let first = Arc::clone(&log);
                scope.add_finalizer(move || {
                    first.lock().expect("release log lock").push("first");
                });
                let second = Arc::clone(&log);
                scope.add_finalizer(move || {
                    second.lock().expect("release log lock").push("second");
                });
            })
            .map_err(|never| match never {})
            .flat_map(move |()| match body {
                Body::Success => Effect::<(), String, ScopeTag>::succeed(()),
                Body::Fail => Effect::<(), String, ScopeTag>::fail("body failed".to_string()),
                Body::Interrupt => Effect::<(), String, ScopeTag>::interrupt(),
            }),
    )
}

fn run_effect<A: Send + 'static>(
    rt: &Runtime,
    effect: Effect<A, String>,
    drive: &dyn Fn(),
) -> Exit<A, String> {
    let handle = rt.spawn_effect_immediate(effect);
    drive();
    handle.join()
}

fn assert_releases(log: &Arc<Mutex<Vec<&'static str>>>, exit_name: &str) -> Result<(), String> {
    let seen = log.lock().expect("release log lock");
    if seen.as_slice() == ["second", "first"] {
        Ok(())
    } else {
        Err(format!(
            "{exit_name} released {seen:?}, expected [\"second\", \"first\"]"
        ))
    }
}
