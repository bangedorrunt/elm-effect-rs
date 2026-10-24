//! `Effect.test.ts:3127`, `:3142`, `:3157` and `:3176` — the tag-keyed error
//! handling combinators.
//!
//! `catchTag` recovers one kind, `catchTags` recovers several (Effect 4's array
//! and record forms are one call here), `orElseSucceed` recovers whatever is
//! left, and `tapErrorTag` observes a kind without recovering it. Effect 4 keeps
//! the failing error in a mutable variable and re-runs one description; these
//! tests do the same through `fail_sync`, which reads the variable when the
//! effect runs.

use std::sync::{Arc, Mutex};

use elm_effect::{Effect, Exit, Runtime, TaggedError};

#[derive(Clone, Debug, PartialEq)]
enum Error {
    A,
    B,
    C,
}

impl TaggedError for Error {
    fn tag(&self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
        }
    }
}

fn shared(error: Error) -> Arc<Mutex<Error>> {
    Arc::new(Mutex::new(error))
}

fn set(state: &Arc<Mutex<Error>>, error: Error) {
    *state.lock().unwrap() = error;
}

fn failing(state: &Arc<Mutex<Error>>) -> Effect<i32, Error> {
    let state = Arc::clone(state);
    Effect::fail_sync(move || state.lock().unwrap().clone())
}

/// `:3127` — each tag is caught by its own arm, and the error no arm names
/// reaches the final `orElseSucceed`.
#[test]
fn catch_tag_arms_compose_with_or_else_succeed() {
    let state = shared(Error::A);
    let effect = failing(&state)
        .catch_tag("A", |_| Effect::succeed(1))
        .catch_tag("B", |_| Effect::succeed(2))
        .or_else_succeed(|| 3);

    assert_eq!(Runtime::run(effect), Exit::Success(1));

    let state = shared(Error::B);
    let effect = failing(&state)
        .catch_tag("A", |_| Effect::succeed(1))
        .catch_tag("B", |_| Effect::succeed(2))
        .or_else_succeed(|| 3);
    assert_eq!(Runtime::run(effect), Exit::Success(2));

    let state = shared(Error::C);
    let effect = failing(&state)
        .catch_tag("A", |_| Effect::succeed(1))
        .catch_tag("B", |_| Effect::succeed(2))
        .or_else_succeed(|| 3);
    assert_eq!(Runtime::run(effect), Exit::Success(3));
}

/// `:3142` — Effect 4's array form: one handler listed under both tags, and
/// `orElse` for anything else.
#[test]
fn catch_tags_runs_one_handler_for_a_list_of_tags() {
    let build = |state: &Arc<Mutex<Error>>| {
        failing(state).catch_tags(
            vec![
                (
                    "A",
                    Box::new(|_| Effect::succeed(1))
                        as Box<dyn FnOnce(Error) -> Effect<i32, Error> + Send>,
                ),
                ("B", Box::new(|_| Effect::succeed(1))),
            ],
            |_| Effect::succeed(2),
        )
    };

    let state = shared(Error::A);
    assert_eq!(Runtime::run(build(&state)), Exit::Success(1));
    set(&state, Error::B);
    assert_eq!(Runtime::run(build(&state)), Exit::Success(1));
    set(&state, Error::C);
    assert_eq!(Runtime::run(build(&state)), Exit::Success(2));
}

/// `:3157` — Effect 4's record form: each tag has its own handler.
#[test]
fn catch_tags_runs_a_handler_per_tag() {
    let build = |state: &Arc<Mutex<Error>>| {
        failing(state).catch_tags(
            vec![
                (
                    "A",
                    Box::new(|_| Effect::succeed(1))
                        as Box<dyn FnOnce(Error) -> Effect<i32, Error> + Send>,
                ),
                ("B", Box::new(|_| Effect::succeed(2))),
            ],
            |_| Effect::succeed(3),
        )
    };

    let state = shared(Error::A);
    assert_eq!(Runtime::run(build(&state)), Exit::Success(1));
    set(&state, Error::B);
    assert_eq!(Runtime::run(build(&state)), Exit::Success(2));
    set(&state, Error::C);
    assert_eq!(Runtime::run(build(&state)), Exit::Success(3));
}

/// `:3176` — a tap sees its own tag and nothing else, and the failure still
/// comes out of `exit` unchanged.
#[test]
fn tap_error_tag_observes_without_recovering() {
    let tapped = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let state = shared(Error::A);

    // One description per run: a tap is a `FnOnce`, and the log is cleared
    // between runs so each answer stands on its own.
    let tap = |label: &'static str| {
        let tapped = Arc::clone(&tapped);
        move |_: &Error| {
            let tapped = Arc::clone(&tapped);
            Effect::<(), Error>::sync(move || tapped.lock().unwrap().push(label))
        }
    };

    let mut seen = Vec::new();
    for error in [Error::A, Error::B, Error::C] {
        set(&state, error.clone());
        tapped.lock().unwrap().clear();
        let effect = failing(&state)
            .tap_error_tag("A", tap("A"))
            .tap_error_tag("B", tap("B"))
            .exit();

        match Runtime::run(effect) {
            Exit::Success(Exit::Failure(cause)) => {
                assert_eq!(cause.failure(), Some(&error), "the failure is unchanged");
            }
            other => panic!("expected the original failure, got {other:?}"),
        }
        seen.push(tapped.lock().unwrap().clone());
    }

    assert_eq!(seen, vec![vec!["A"], vec!["B"], vec![]]);
}
