//! `Atom.swr`: serve the base's answer, refetch when it is stale.

use elm_effect_unstable::reactivity::{async_atom, swr, AsyncResult, Registry, SwrOptions};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn value<T: Clone, E>(result: &AsyncResult<T, E>) -> Option<T> {
    match result {
        AsyncResult::Success { value, .. } => Some(value.clone()),
        _ => None,
    }
}

#[test]
fn a_fresh_answer_is_not_refetched() {
    let calls = Arc::new(AtomicUsize::new(0));
    let fed = Arc::clone(&calls);
    let base = async_atom(move || {
        let call = fed.fetch_add(1, Ordering::SeqCst) as i64;
        Ok::<_, String>(call + 1)
    });
    let cached = swr(&base, SwrOptions::new(std::time::Duration::from_secs(60)));
    let registry = Registry::make();
    assert_eq!(value(&registry.get(&cached)), Some(1));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "a fresh answer stays cached");
}

#[test]
fn a_stale_answer_revalidates_on_each_build() {
    let calls = Arc::new(AtomicUsize::new(0));
    let fed = Arc::clone(&calls);
    let base = async_atom(move || {
        let call = fed.fetch_add(1, Ordering::SeqCst) as i64;
        Ok::<_, String>(call + 1)
    });
    // staleTime 0: every answer is stale as soon as it exists, so the mount
    // revalidates once and a later rebuild revalidates again.
    let cached = swr(&base, SwrOptions::new(std::time::Duration::ZERO));
    let registry = Registry::make();
    assert_eq!(value(&registry.get(&cached)), Some(2));
    assert_eq!(calls.load(Ordering::SeqCst), 2, "mount: fetch plus revalidate");
    registry.refresh(&cached);
    assert_eq!(value(&registry.get(&cached)), Some(3));
    assert_eq!(calls.load(Ordering::SeqCst), 3, "a rebuild revalidates a stale answer");
}

#[test]
fn revalidate_on_mount_can_be_turned_off() {
    let calls = Arc::new(AtomicUsize::new(0));
    let fed = Arc::clone(&calls);
    let base = async_atom(move || {
        let call = fed.fetch_add(1, Ordering::SeqCst) as i64;
        Ok::<_, String>(call + 1)
    });
    let cached = swr(
        &base,
        SwrOptions::new(std::time::Duration::from_secs(60)).without_revalidate_on_mount(),
    );
    let registry = Registry::make();
    assert_eq!(value(&registry.get(&cached)), Some(1));
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the first read serves what is there");
}

#[test]
fn base_changes_reach_the_swr_node() {
    let calls = Arc::new(AtomicUsize::new(0));
    let fed = Arc::clone(&calls);
    let base = async_atom(move || {
        let call = fed.fetch_add(1, Ordering::SeqCst) as i64;
        Ok::<_, String>(call + 1)
    });
    let cached = swr(&base, SwrOptions::new(std::time::Duration::from_secs(60)));
    let registry = Registry::make();
    assert_eq!(value(&registry.get(&cached)), Some(1));
    registry.refresh(&base);
    assert_eq!(value(&registry.get(&cached)), Some(2));
}
