//! Service-map behavior: `Context.ts` ordering, omission, and merge rows.

use elm_effect::{Context, Tag};

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
    let context = Context::empty()
        .add(A, 1)
        .add(B, 3)
        .add(A, 4)
        .add(C, 5);
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
    assert_eq!(merged.keys(), vec!["ContextTest/A", "ContextTest/B", "ContextTest/C"]);
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
        source.clone().pick(&["ContextTest/C", "ContextTest/A"]).keys(),
        vec!["ContextTest/A", "ContextTest/C"]
    );
    assert_eq!(
        source.clone().omit(B).keys(),
        vec!["ContextTest/A", "ContextTest/C"]
    );
}
