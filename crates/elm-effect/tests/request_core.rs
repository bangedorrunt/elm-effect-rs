//! Request.test.ts rows that ride on direct execution: a resolver's typed
//! error, its defects, its interrupts, and one answer per submitted request.

use std::sync::Arc;

use elm_effect::{Cause, Defect, Effect, Exit, Runtime};
use elm_effect::request::{request, Entry, Request, Resolver, TaggedResolver};

/// The rows' `GetNameById` request class: a tag plus a payload.
#[derive(Clone, Debug, PartialEq, Eq)]
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

fn resolver<F>(handler: F) -> Arc<TaggedResolver<GetNameById>>
where
    F: Fn(&[GetNameById]) -> Result<Vec<String>, Cause<String>> + Send + Sync + 'static,
{
    Arc::new(TaggedResolver::<GetNameById>::new().on("GetNameById", handler))
}

/// Request.test.ts:204 `fromEffectTagged preserves typed errors`.
/// The handler's typed failure is the request's failure, unchanged.
#[test]
fn a_tagged_resolver_preserves_its_typed_error() {
    let effect = request(resolver(|_reqs| Err(Cause::fail("missing".to_owned()))), GetNameById { id: 1 })
        .flip();

    assert_eq!(Runtime::run(effect), Exit::Success("missing".to_owned()));
}

/// Request.test.ts:214 `fromEffectTagged preserves defects`.
/// A handler defect reaches the caller as one Die reason carrying the same
/// message, not as a typed failure.
#[test]
fn a_tagged_resolver_preserves_its_defect() {
    let effect = request(
        resolver(|_reqs| Err(Cause::die(Defect::new("boom")))),
        GetNameById { id: 1 },
    )
    .exit();

    let exit = match Runtime::run(effect) {
        Exit::Success(exit) => exit,
        Exit::Failure(cause) => panic!("exit must turn the request's outcome into a value: {cause:?}"),
    };
    let cause = match exit {
        Exit::Failure(cause) => cause,
        Exit::Success(value) => panic!("the request must fail, not answer {value:?}"),
    };
    assert_eq!(cause.reasons.len(), 1, "one reason: {cause:?}");
    assert_eq!(
        cause.defects().len(),
        1,
        "the reason is a Die: {cause:?}"
    );
    assert_eq!(cause.defects()[0].message, "boom");
}

/// Request.test.ts:233 `fromEffectTagged preserves interrupts`.
/// An interrupted handler leaves the request with an interrupt, never a typed
/// failure and never a value.
#[test]
fn a_tagged_resolver_preserves_its_interrupt() {
    let effect = request(
        resolver(|_reqs| Err(Cause::interrupt(None))),
        GetNameById { id: 1 },
    )
    .exit();

    let exit = match Runtime::run(effect) {
        Exit::Success(exit) => exit,
        Exit::Failure(cause) => panic!("exit must turn the request's outcome into a value: {cause:?}"),
    };
    match exit {
        Exit::Failure(cause) => {
            assert!(cause.has_interrupts(), "the interrupt is kept: {cause:?}");
            assert!(cause.failure().is_none(), "no typed failure: {cause:?}");
        }
        Exit::Success(value) => panic!("an interrupted request must not answer {value:?}"),
    }
}

/// Request.test.ts:243 `fromEffectTagged resolves requests from $name results`.
/// The handler answers one value per submitted request, and the three
/// submissions come back in submission order. The pin's array, iterator, and
/// generator forms all produce the same list, so one list form covers them.
#[test]
fn a_tagged_resolver_answers_every_submitted_request() {
    let resolver = resolver(|reqs| Ok(reqs.iter().map(|req| req.id.to_string()).collect()));

    // Effects are one-shot descriptions, so each request is built where it
    // runs; only the resolver is shared.
    let all = request(resolver.clone(), GetNameById { id: 1 }).flat_map(move |a| {
        let resolver = resolver.clone();
        request(resolver.clone(), GetNameById { id: 2 }).flat_map(move |b| {
            request(resolver.clone(), GetNameById { id: 3 })
                .map(move |c| vec![a.clone(), b.clone(), c])
        })
    });

    assert_eq!(
        Runtime::run(all),
        Exit::Success(vec!["1".to_owned(), "2".to_owned(), "3".to_owned()])
    );
}

/// The resolver sees the requests it was given, grouped by tag, in submission
/// order. A tag nobody claimed is a defect rather than a silent skip.
#[test]
fn a_tagged_resolver_refuses_a_tag_nobody_claimed() {
    #[derive(Clone)]
    struct Unclaimed;
    impl Request for Unclaimed {
        type Success = ();
        type Error = String;

        fn tag(&self) -> &'static str {
            "Unclaimed"
        }
    }

    let resolver = TaggedResolver::<Unclaimed>::new();
    match resolver.run(&[Entry::new(Unclaimed)]) {
        Ok(_) => panic!("an unclaimed tag must not be answered"),
        Err(cause) => assert!(
            cause.defects()[0].message.contains("Unclaimed"),
            "the defect names the tag: {cause:?}"
        ),
    }
}
