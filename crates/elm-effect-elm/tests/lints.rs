//! Elm fold lint tests (harness in `src/lints.rs`): fixtures fire, clean passes.

#[path = "../src/lints.rs"]
mod lints;

use lints::{fold_model_no_hashmap, fold_no_ambient_world};

const DIRTY: &str = include_str!("fixtures/dirty.rs");
const CLEAN: &str = include_str!("fixtures/clean.rs");

#[test]
fn ambient_world_fires_on_dirty_fixture() {
    let hits = fold_no_ambient_world(DIRTY);
    assert_eq!(
        hits.len(),
        3,
        "dirty fixture must trip all three ambient tokens: {hits:?}"
    );
    assert!(hits.iter().any(|h| h.contains("SystemTime")));
    assert!(hits.iter().any(|h| h.contains("std::env")));
    assert!(hits.iter().any(|h| h.contains("rand::thread_rng")));
    assert_eq!(fold_no_ambient_world(CLEAN), Vec::<String>::new());
}

#[test]
fn model_hashmap_fires_on_dirty_fixture() {
    let hits = fold_model_no_hashmap(DIRTY);
    assert_eq!(hits, vec!["AppModel stores a HashMap; use BTreeMap"]);
    assert_eq!(fold_model_no_hashmap(CLEAN), Vec::<String>::new());
}

#[test]
fn real_crate_passes_both_lints() {
    let src = include_str!("../src/lib.rs");
    assert_eq!(fold_no_ambient_world(src), Vec::<String>::new());
    assert_eq!(fold_model_no_hashmap(src), Vec::<String>::new());
}
