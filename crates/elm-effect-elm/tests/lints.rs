//! Elm fold lint tests (rules live in `elm-effect-lints`): fixtures fire, clean
//! passes. The fixture path is synthetic because the lint crate skips
//! `fixtures/` paths by design; the fixtures are read as text targets here.

use elm_effect_lints::{Rule, ambient_world_in_a_fold, hashmap_in_a_model};

const DIRTY: &str = include_str!("fixtures/dirty.rs");
const CLEAN: &str = include_str!("fixtures/clean.rs");
const TARGET: &str = "src/app.rs";

#[test]
fn ambient_world_fires_on_dirty_fixture() {
    let hits = ambient_world_in_a_fold(TARGET, DIRTY);
    assert_eq!(
        hits.len(),
        3,
        "dirty fixture must trip all three ambient tokens: {hits:?}"
    );
    assert!(hits.iter().any(|h| h.text.contains("SystemTime")));
    assert!(hits.iter().any(|h| h.text.contains("std::env")));
    assert!(hits
        .iter()
        .any(|h| h.text.contains("rand::thread_rng")));
    assert!(hits.iter().all(|h| h.rule == Rule::AmbientWorldInAFold));
    assert_eq!(ambient_world_in_a_fold(TARGET, CLEAN), Vec::new());
}

#[test]
fn model_hashmap_fires_on_dirty_fixture() {
    let hits = hashmap_in_a_model(TARGET, DIRTY);
    assert_eq!(
        hits.len(),
        1,
        "dirty fixture stores one HashMap in AppModel: {hits:?}"
    );
    assert_eq!(hits[0].rule, Rule::HashMapInAModel);
    assert!(hits[0].text.contains("HashMap"));
    assert_eq!(hashmap_in_a_model(TARGET, CLEAN), Vec::new());
}

#[test]
fn real_crate_passes_both_lints() {
    let src = include_str!("../src/lib.rs");
    assert_eq!(ambient_world_in_a_fold("src/lib.rs", src), Vec::new());
    assert_eq!(hashmap_in_a_model("src/lib.rs", src), Vec::new());
}
