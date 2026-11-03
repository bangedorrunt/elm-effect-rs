//! JournalStore fence: claim/settle generation fencing, atomic
//! offset+state commits, retention Gone, dead letters.

use elm_effect_journal::EffectState;
use elm_effect_journal::store::{Disposition, InMemoryStore, JournalStore, Read};

fn store() -> InMemoryStore {
    InMemoryStore::new(8)
}

#[test]
fn intent_before_provider_same_key_on_retry() {
    let db = store();
    let first = db.record_intent("job-1", 3).expect("record intent");
    assert_eq!(first.state, EffectState::Prepared);
    assert_eq!(first.generation, 0);
    let second = db.record_intent("job-1", 3).expect("record intent");
    assert_eq!(first, second);
}

#[test]
fn claim_sets_owner_lease_generation_atomically() {
    let db = store();
    db.record_intent("job-1", 3).expect("record intent");
    let row = db
        .claim("job-1", "worker-a", 9999, 0, 0)
        .expect("claim")
        .expect("row");
    assert_eq!(row.owner.as_deref(), Some("worker-a"));
    assert_eq!(row.lease_deadline, Some(9999));
    assert_eq!(row.generation, 1);
    // Same owner renews its own lease with a fresh generation.
    let row = db
        .claim("job-1", "worker-a", 10000, 0, 1)
        .expect("claim")
        .expect("row");
    assert_eq!(row.generation, 2);
    assert_eq!(row.owner.as_deref(), Some("worker-a"));
}

#[test]
fn live_lease_steal_refused() {
    let db = store();
    db.record_intent("job-1", 3).expect("record intent");
    db.claim("job-1", "worker-a", 9999, 0, 0)
        .expect("claim")
        .expect("row");
    // Lease live at now_ms=0 under another owner: steal fails, row untouched.
    assert!(
        db.claim("job-1", "worker-b", 10000, 0, 1)
            .expect("claim")
            .is_none()
    );
    let row = db.get("job-1").expect("get").expect("row");
    assert_eq!(row.owner.as_deref(), Some("worker-a"));
    assert_eq!(row.generation, 1);
}

#[test]
fn stale_generation_claim_refused() {
    let db = store();
    db.record_intent("job-1", 3).expect("record intent");
    db.claim("job-1", "worker-a", 9999, 0, 0)
        .expect("claim")
        .expect("row");
    // Generation moved to 1: presenting 0 claims nothing, even after expiry.
    assert!(
        db.claim("job-1", "worker-b", 20000, 9999, 0)
            .expect("claim")
            .is_none()
    );
    assert_eq!(db.get("job-1").expect("get").expect("row").generation, 1);
}

#[test]
fn expired_lease_reclaim_with_fresh_generation_succeeds() {
    let db = store();
    db.record_intent("job-1", 3).expect("record intent");
    db.claim("job-1", "worker-a", 9999, 0, 0)
        .expect("claim")
        .expect("row");
    // Past the deadline the lease is stale: fresh generation reclaims.
    let row = db
        .claim("job-1", "worker-b", 20000, 9999, 1)
        .expect("claim")
        .expect("row");
    assert_eq!(row.owner.as_deref(), Some("worker-b"));
    assert_eq!(row.lease_deadline, Some(20000));
    assert_eq!(row.generation, 2);
    // The old generation no longer settles: the steal invalidated it.
    assert!(
        db.settle("job-1", 1, EffectState::InFlight)
            .expect("settle")
            .is_none()
    );
}

#[test]
fn terminal_reclaim_refused() {
    let db = store();
    db.record_intent("job-1", 3).expect("record intent");
    db.claim("job-1", "worker-a", 9999, 0, 0)
        .expect("claim")
        .expect("row");
    let generation = db.get("job-1").expect("get").expect("row").generation;
    db.settle("job-1", generation, EffectState::InFlight)
        .expect("settle")
        .expect("row");
    let generation = db.get("job-1").expect("get").expect("row").generation;
    db.settle("job-1", generation, EffectState::Settled)
        .expect("settle")
        .expect("row");
    // Settled rows never move, even with a fresh generation past expiry.
    assert!(
        db.claim("job-1", "worker-b", 20000, 9999, 1)
            .expect("claim")
            .is_none()
    );
    assert_eq!(
        db.get("job-1").expect("get").expect("row").state,
        EffectState::Settled
    );
}

#[test]
fn first_claim_then_settle_stays_green() {
    let db = store();
    db.record_intent("job-1", 3).expect("record intent");
    db.claim("job-1", "worker-a", 9999, 0, 0).expect("claim");
    assert!(
        db.settle("job-1", 0, EffectState::InFlight)
            .expect("settle")
            .is_none()
    );
    let row = db
        .settle("job-1", 1, EffectState::InFlight)
        .expect("settle")
        .expect("row");
    assert_eq!(row.state, EffectState::InFlight);
}

#[test]
fn terminal_states_never_move() {
    let db = store();
    db.record_intent("job-1", 3).expect("record intent");
    db.claim("job-1", "w", 1, 0, 0).expect("claim");
    let generation = db.get("job-1").expect("get").expect("row").generation;
    db.settle("job-1", generation, EffectState::InFlight)
        .expect("settle")
        .expect("row");
    let generation = db.get("job-1").expect("get").expect("row").generation;
    db.settle("job-1", generation, EffectState::Settled)
        .expect("settle")
        .expect("row");
    let generation = db.get("job-1").expect("get").expect("row").generation;
    assert!(
        db.settle("job-1", generation, EffectState::Failed)
            .expect("settle")
            .is_none()
    );
    assert!(
        db.settle("job-1", generation, EffectState::Cancelled)
            .expect("settle")
            .is_none()
    );
    assert_eq!(
        db.get("job-1").expect("get").expect("row").state,
        EffectState::Settled
    );
}

#[test]
fn unknown_has_no_default_exit() {
    let db = store();
    db.record_intent("job-1", 3).expect("record intent");
    db.claim("job-1", "w", 1, 0, 0).expect("claim");
    let generation = db.get("job-1").expect("get").expect("row").generation;
    db.settle("job-1", generation, EffectState::InFlight)
        .expect("settle")
        .expect("row");
    let generation = db.get("job-1").expect("get").expect("row").generation;
    db.settle("job-1", generation, EffectState::Unknown)
        .expect("settle")
        .expect("row");
    let generation = db.get("job-1").expect("get").expect("row").generation;
    // Only the golden Unknown edges admit; everything else settles nothing.
    assert!(
        db.settle("job-1", generation, EffectState::Accepted)
            .expect("settle")
            .is_none()
    );
    assert!(
        db.settle("job-1", generation, EffectState::Prepared)
            .expect("settle")
            .is_none()
    );
    let row = db
        .settle("job-1", generation, EffectState::ReconcilePending)
        .expect("settle")
        .expect("row");
    assert_eq!(row.state, EffectState::ReconcilePending);
}

#[test]
fn accepted_is_its_own_state() {
    let db = store();
    db.record_intent("job-1", 3).expect("record intent");
    db.claim("job-1", "w", 1, 0, 0).expect("claim");
    let generation = db.get("job-1").expect("get").expect("row").generation;
    db.settle("job-1", generation, EffectState::InFlight)
        .expect("settle")
        .expect("row");
    let generation = db.get("job-1").expect("get").expect("row").generation;
    let row = db
        .settle("job-1", generation, EffectState::Accepted)
        .expect("settle")
        .expect("row");
    assert_eq!(row.state, EffectState::Accepted);
    // Accepted advances only through explicit legal edges.
    let generation = db.get("job-1").expect("get").expect("row").generation;
    assert!(
        db.settle("job-1", generation, EffectState::Prepared)
            .expect("settle")
            .is_none()
    );
}

#[test]
fn exhausted_attempts_dead_letter() {
    let db = store();
    db.record_intent("job-1", 1).expect("record intent");
    db.attempt("job-1", 0).expect("attempt");
    let row = db.attempt("job-1", 0).expect("attempt").expect("row");
    assert_eq!(row.state, EffectState::Failed);
    assert_eq!(row.disposition, Some(Disposition::TerminalFailed));
    assert_eq!(Disposition::TerminalFailed.as_str(), "terminal_failed");
}

#[test]
fn an_attempt_from_a_stale_generation_notes_nothing() {
    let db = store();
    db.record_intent("job-1", 0).expect("record intent");
    // The lease expires and a second worker takes over. The first is a
    // zombie: it still believes it owns the row.
    let stale = db
        .claim("job-1", "old", 10, 0, 0)
        .expect("claim")
        .expect("row");
    let live = db
        .claim("job-1", "new", 1_000, 100, stale.generation)
        .expect("claim")
        .expect("row");
    assert!(live.generation > stale.generation);

    assert!(
        db.attempt("job-1", stale.generation)
            .expect("attempt")
            .is_none(),
        "a stale holder must not spend the live holder's attempt budget"
    );
    let row = db.get("job-1").expect("get").expect("row");
    assert_eq!(row.attempts, 0, "the live row was left untouched");
    assert_eq!(row.state, EffectState::Prepared);
    assert_eq!(row.owner.as_deref(), Some("new"));
}

#[test]
fn a_terminal_row_notes_no_attempt() {
    let db = store();
    db.record_intent("job-1", 3).expect("record intent");
    let claimed = db
        .claim("job-1", "w", 1_000, 0, 0)
        .expect("claim")
        .expect("row");
    db.settle("job-1", claimed.generation, EffectState::Cancelled)
        .expect("settle")
        .expect("row");
    assert!(
        db.attempt("job-1", claimed.generation)
            .expect("attempt")
            .is_none()
    );
    assert_eq!(
        db.get("job-1").expect("get").expect("row").state,
        EffectState::Cancelled
    );
}

#[test]
fn read_returns_the_row_as_it_was_at_that_offset() {
    let db = store();
    db.record_intent("job-1", 3).expect("record intent"); // offset 0: Prepared, generation 0
    let claimed = db
        .claim("job-1", "w", 1_000, 0, 0)
        .expect("claim")
        .expect("row");
    db.settle("job-1", claimed.generation, EffectState::InFlight)
        .expect("settle")
        .expect("row");

    match db.read(0).expect("read").expect("row") {
        Read::Entry { offset, row } => {
            assert_eq!(offset, 0);
            assert_eq!(row.offset, 0, "the row carries its own offset");
            assert_eq!(row.state, EffectState::Prepared, "not the later state");
            assert_eq!(row.generation, 0, "not the later generation");
        }
        Read::Gone { .. } => panic!("expected a retained entry"),
    }

    // The latest read still answers the current row.
    match db.read(-1).expect("read").expect("row") {
        Read::Entry { row, .. } => {
            assert_eq!(row.state, EffectState::InFlight);
            assert_eq!(row.generation, 1);
        }
        Read::Gone { .. } => panic!("expected the latest entry"),
    }
}

#[test]
fn offset_and_state_commit_together() {
    let db = store();
    db.record_intent("a", 3).expect("record intent");
    db.record_intent("b", 3).expect("record intent");
    let ra = db.get("a").expect("get").expect("row");
    let rb = db.get("b").expect("get").expect("row");
    assert!(rb.offset > ra.offset);
    match db.read(rb.offset).expect("read").expect("row") {
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
    db.record_intent("a", 3).expect("record intent");
    let first = db.get("a").expect("get").expect("row").offset;
    db.claim("a", "w", 1, 0, 0).expect("claim");
    let second = db.get("a").expect("get").expect("row").offset;
    assert!(second > first);
}

#[test]
fn old_offset_returns_gone_with_first_replayable() {
    let db = InMemoryStore::new(2);
    db.record_intent("a", 3).expect("record intent");
    db.record_intent("b", 3).expect("record intent");
    db.record_intent("c", 3).expect("record intent");
    // Retention keeps 2; offset 0 is gone.
    match db.read(0).expect("read").expect("row") {
        Read::Gone { first_replayable } => assert!(first_replayable > 0),
        Read::Entry { .. } => panic!("expected gone"),
    }
    // Latest via -1 still works.
    assert!(matches!(
        db.read(-1).expect("read").expect("row"),
        Read::Entry { .. }
    ));
}

#[test]
fn no_issue_events_become_gate_rows() {
    // The store surface never mentions gate rows: grep the source.
    let src = include_str!("../src/store.rs");
    assert!(!src.contains("gate"), "store must not know gates");
}

#[test]
fn store_laws_hold_for_in_memory() {
    let db = store();
    let results = elm_effect_journal::laws::journal_store_all(&db);
    assert_eq!(results.len(), 4);
    for (name, result) in &results {
        assert!(
            result.is_ok(),
            "law {name} failed: {:?}",
            result.as_ref().err()
        );
    }
}

#[test]
fn disposition_round_trips() {
    use std::str::FromStr;
    for d in [
        Disposition::Continue,
        Disposition::Retry,
        Disposition::Reroute,
        Disposition::Blocked,
        Disposition::TerminalFailed,
        Disposition::Cancelled,
        Disposition::Reconcile,
    ] {
        assert_eq!(Disposition::from_str(d.as_str()).expect("parse"), d);
        assert_eq!(Disposition::from_str(&d.to_string()).expect("parse"), d);
        let json = serde_json::to_string(&d).expect("serialize");
        assert_eq!(
            serde_json::from_str::<Disposition>(&json).expect("deserialize"),
            d
        );
    }
    assert!(Disposition::from_str("nope").is_err());
}

#[test]
fn row_serde_roundtrip() {
    let db = store();
    db.record_intent("serde-1", 3).expect("record intent");
    let row = db.get("serde-1").expect("get").expect("row");
    let json = serde_json::to_string(&row).expect("serialize row");
    assert_eq!(
        serde_json::from_str::<elm_effect_journal::store::Row>(&json).expect("deserialize row"),
        row
    );
}

#[test]
fn row_hashes() {
    use std::collections::HashSet;
    let db = store();
    db.record_intent("hash-1", 3).expect("record intent");
    let row = db.get("hash-1").expect("get").expect("row");
    let mut set = HashSet::new();
    set.insert(row);
    assert_eq!(set.len(), 1);
}

#[test]
fn read_serde_roundtrip() {
    let db = store();
    db.record_intent("serde-read", 3).expect("record intent");
    let row = db.get("serde-read").expect("get").expect("row");
    let entry = Read::Entry {
        offset: row.offset,
        row: row.clone(),
    };
    let json = serde_json::to_string(&entry).expect("serialize read");
    assert_eq!(
        serde_json::from_str::<Read>(&json).expect("deserialize read"),
        entry
    );
    let gone = Read::Gone {
        first_replayable: 7,
    };
    let json = serde_json::to_string(&gone).expect("serialize gone");
    assert_eq!(
        serde_json::from_str::<Read>(&json).expect("deserialize gone"),
        gone
    );
}
