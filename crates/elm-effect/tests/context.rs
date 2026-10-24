//! Service-map behavior: ordering, omission, and merge.

use elm_effect::{Context, ContextReference, Tag};

const A: Tag<i32> = Tag::new("ContextTest/A");
const B: Tag<i32> = Tag::new("ContextTest/B");
const C: Tag<i32> = Tag::new("ContextTest/C");

#[test]
fn context_add_keeps_source_immutable() {
    // Context.test.ts:13.
    let source = Context::make(A, 1);
    let result = source.clone().add(B, 2);
    assert_eq!(result.get(A), Some(&1));
    assert_eq!(result.get(B), Some(&2));
    assert_eq!(source.get(B), None);
    assert_eq!(source.keys(), vec!["ContextTest/A"]);
}

#[test]
fn context_add_or_omit_removes() {
    // Context.test.ts:21.
    let context = Context::make(A, 1).add_or_omit(A, None);
    assert_eq!(context.get(A), None);
    assert!(context.keys().is_empty());
    let kept = Context::make(A, 1).add_or_omit(A, Some(2));
    assert_eq!(kept.get(A), Some(&2));
}

#[test]
fn context_preserves_insertion_order() {
    // Context.test.ts:27 — replacement keeps position, appends go last.
    let context = Context::empty().add(A, 1).add(B, 3).add(A, 4).add(C, 5);
    assert_eq!(
        context.keys(),
        vec!["ContextTest/A", "ContextTest/B", "ContextTest/C"]
    );
    assert_eq!(context.get(A), Some(&4));
}

#[test]
fn context_merge_is_right_biased() {
    // Context.test.ts:207.
    let left = Context::make(A, 1).add(B, 2);
    let right = Context::make(A, 3).add(C, 4);
    let merged = left.clone().merge(right);
    assert_eq!(
        merged.keys(),
        vec!["ContextTest/A", "ContextTest/B", "ContextTest/C"]
    );
    assert_eq!(merged.get(A), Some(&3));
    assert_eq!(merged.get(B), Some(&2));
    assert_eq!(merged.get(C), Some(&4));
    assert_eq!(Context::empty().merge(left.clone()).keys(), left.keys());
    assert_eq!(left.clone().merge(Context::empty()).keys(), left.keys());
}

#[test]
fn context_pick_omit_retain_source_order() {
    // Context.test.ts: pick and omit retain source ordering.
    let source = Context::make(A, 1).add(B, 2).add(C, 3);
    assert_eq!(
        source
            .clone()
            .pick(&["ContextTest/C", "ContextTest/A"])
            .keys(),
        vec!["ContextTest/A", "ContextTest/C"]
    );
    assert_eq!(
        source.clone().omit(B).keys(),
        vec!["ContextTest/A", "ContextTest/C"]
    );
}

#[test]
fn context_reference_defaults_resolve_lazily_and_cache() {
    // Context.test.ts:224 — the default computes on first resolution
    // and the cached value is shared after.
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let tag = ContextReference::new("ContextTest/LazyRef", || {
        CALLS.fetch_add(1, Ordering::SeqCst);
        7i32
    });
    let mut context = Context::empty();
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
    let first = context.resolve(tag);
    assert_eq!(*first, 7);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    let second = context.resolve(tag);
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
}

const PRESENT: Tag<i32> = Tag::new("ContextTest/Present");
const ABSENT: Tag<i32> = Tag::new("ContextTest/Absent");

#[test]
fn get_distinguishes_present_from_absent() {
    // Context case "distinguishes an undefined service from an absent
    // service": a service that was ADDED is retrievable, one that was never
    // added is not. Effect 4 expresses this with getOption returning Some vs
    // None, which maps directly onto this crate's Option<&T>.
    let context = Context::empty().add(PRESENT, 1);
    assert!(
        context.get(PRESENT).is_some(),
        "an added service is present"
    );
    assert!(
        context.get(ABSENT).is_none(),
        "a service never added is absent, not defaulted"
    );
}

#[test]
fn get_on_an_empty_context_is_absent() {
    // Nothing has been added at all, so every tag reads absent.
    let context = Context::empty();
    assert!(context.get(PRESENT).is_none());
}
