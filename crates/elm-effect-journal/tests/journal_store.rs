//! JournalStore fence: claim/settle generation fencing, atomic
//! offset+state commits, retention Gone, dead letters.

use elm_effect_journal::store::{Disposition, InMemoryStore, JournalStore, Read};
use elm_effect_journal::EffectState;

fn store() -> InMemoryStore {
    InMemoryStore::new(8)
}

#[test]
fn intent_before_provider_same_key_on_retry() {
    let db = store();
    let first = db.record_intent("job-1", 3);
    assert_eq!(first.state, EffectState::Prepared);
    assert_eq!(first.generation, 0);
    let second = db.record_intent("job-1", 3);
    assert_eq!(first, second);
}

#[test]
fn claim_sets_owner_lease_generation_atomically() {
    let db = store();
    db.record_intent("job-1", 3);
    let row = db.claim("job-1", "worker-a", 9999).unwrap();
    assert_eq!(row.owner.as_deref(), Some("worker-a"));
    assert_eq!(row.lease_deadline, Some(9999));
    assert_eq!(row.generation, 1);
    let row = db.claim("job-1", "worker-b", 10000).unwrap();
    assert_eq!(row.generation, 2);
    assert_eq!(row.owner.as_deref(), Some("worker-b"));
}

#[test]
fn settle_carries_generation_stale_settles_nothing() {
    let db = store();
    db.record_intent("job-1", 3);
    db.claim("job-1", "worker-a", 9999);
    assert!(db.settle("job-1", 0, EffectState::InFlight).is_none());
    let row = db.settle("job-1", 1, EffectState::InFlight).unwrap();
    assert_eq!(row.state, EffectState::InFlight);
}

#[test]
fn terminal_states_never_move() {
    let db = store();
    db.record_intent("job-1", 3);
    db.claim("job-1", "w", 1);
    let generation = db.get("job-1").unwrap().generation;
    db.settle("job-1", generation, EffectState::InFlight).unwrap();
    let generation = db.get("job-1").unwrap().generation;
    db.settle("job-1", generation, EffectState::Settled).unwrap();
    let generation = db.get("job-1").unwrap().generation;
    assert!(db.settle("job-1", generation, EffectState::Failed).is_none());
    assert!(db.settle("job-1", generation, EffectState::Cancelled).is_none());
    assert_eq!(db.get("job-1").unwrap().state, EffectState::Settled);
}

#[test]
fn unknown_has_no_default_exit() {
    let db = store();
    db.record_intent("job-1", 3);
    db.claim("job-1", "w", 1);
    let generation = db.get("job-1").unwrap().generation;
    db.settle("job-1", generation, EffectState::InFlight).unwrap();
    let generation = db.get("job-1").unwrap().generation;
    db.settle("job-1", generation, EffectState::Unknown).unwrap();
    let generation = db.get("job-1").unwrap().generation;
    // Only the golden Unknown edges admit; everything else settles nothing.
    assert!(db.settle("job-1", generation, EffectState::Accepted).is_none());
    assert!(db.settle("job-1", generation, EffectState::Prepared).is_none());
    let row = db.settle("job-1", generation, EffectState::ReconcilePending).unwrap();
    assert_eq!(row.state, EffectState::ReconcilePending);
}

#[test]
fn accepted_is_its_own_state() {
    let db = store();
    db.record_intent("job-1", 3);
    db.claim("job-1", "w", 1);
    let generation = db.get("job-1").unwrap().generation;
    db.settle("job-1", generation, EffectState::InFlight).unwrap();
    let generation = db.get("job-1").unwrap().generation;
    let row = db.settle("job-1", generation, EffectState::Accepted).unwrap();
    assert_eq!(row.state, EffectState::Accepted);
    // Accepted advances only through explicit legal edges.
    let generation = db.get("job-1").unwrap().generation;
    assert!(db.settle("job-1", generation, EffectState::Prepared).is_none());
}

#[test]
fn exhausted_attempts_dead_letter() {
    let db = store();
    db.record_intent("job-1", 1);
    db.attempt("job-1");
    let row = db.attempt("job-1").unwrap();
    assert_eq!(row.state, EffectState::Failed);
    assert_eq!(row.disposition, Some(Disposition::TerminalFailed));
    assert_eq!(Disposition::TerminalFailed.as_str(), "terminal_failed");
}

#[test]
fn offset_and_state_commit_together() {
    let db = store();
    db.record_intent("a", 3);
    db.record_intent("b", 3);
    let ra = db.get("a").unwrap();
    let rb = db.get("b").unwrap();
    assert!(rb.offset > ra.offset);
    match db.read(rb.offset).unwrap() {
        Read::Entry { offset, row } => {
            assert_eq!(offset, rb.offset);
            assert_eq!(row.key, "b");
        }
        Read::Gone { .. } => panic!("expected entry"),
    }
}

#[test]
fn offsets_move_only_forward() {
    let db = store();
    db.record_intent("a", 3);
    let first = db.get("a").unwrap().offset;
    db.claim("a", "w", 1);
    let second = db.get("a").unwrap().offset;
    assert!(second > first);
}

#[test]
fn old_offset_returns_gone_with_first_replayable() {
    let db = InMemoryStore::new(2);
    db.record_intent("a", 3);
    db.record_intent("b", 3);
    db.record_intent("c", 3);
    // Retention keeps 2; offset 0 is gone.
    match db.read(0).unwrap() {
        Read::Gone { first_replayable } => assert!(first_replayable > 0),
        Read::Entry { .. } => panic!("expected gone"),
    }
    // Latest via -1 still works.
    assert!(matches!(db.read(-1).unwrap(), Read::Entry { .. }));
}

#[test]
fn no_issue_events_become_gate_rows() {
    // The store surface never mentions gate rows: grep the source.
    let src = include_str!("../src/store.rs");
    assert!(!src.contains("gate"), "store must not know gates");
}
