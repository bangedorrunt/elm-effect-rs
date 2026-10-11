//! `plan_park` fence: refusal above the ceiling, no wrapping, due-now
//! resumes, idempotent pending upserts, and a resumable non-terminal Parked.
//! The plan module is public on the crate (lib.rs wires it).

use elm_effect_journal::plan;

use elm_effect_journal::EffectState;
use plan::{ParkDecision, PendingKey, PendingRow, PendingStore, park_intent, plan_park};
use std::str::FromStr;

#[test]
fn plan_park_refusal_no_wrap_due_now() {
    // A wait above the 86400 s ceiling is a typed refusal, not a sleep.
    assert_eq!(
        plan_park(10_000, 90_000, 0, None),
        ParkDecision::RefusedOverCeiling {
            asked_secs: 90_000,
            ceiling_secs: 86_400
        }
    );
    // Custom ceilings apply the same rule.
    assert_eq!(
        plan_park(10_000, 61, 0, Some(60)),
        ParkDecision::RefusedOverCeiling {
            asked_secs: 61,
            ceiling_secs: 60
        }
    );
    // A past deadline resumes now with the overdue gap, never wrapping.
    assert_eq!(
        plan_park(100, 60, 250, None),
        ParkDecision::ResumeNow { overdue_ms: 150 }
    );
    // Due now is overdue zero.
    assert_eq!(
        plan_park(250, 60, 250, None),
        ParkDecision::ResumeNow { overdue_ms: 0 }
    );
    // Far-past deadlines saturate instead of wrapping.
    assert_eq!(
        plan_park(0, 60, u64::MAX, None),
        ParkDecision::ResumeNow {
            overdue_ms: u64::MAX
        }
    );
    // A future deadline waits, capped by max_wait_secs.
    match plan_park(10_000, 60, 0, None) {
        ParkDecision::Wait { wait_ms, intent } => {
            assert_eq!(wait_ms, 10_000);
            assert_eq!(intent, park_intent(10_000, 10_000));
        }
        other => panic!("expected Wait, got {other:?}"),
    }
    match plan_park(1_000_000, 60, 0, None) {
        ParkDecision::Wait { wait_ms, .. } => assert_eq!(wait_ms, 60_000),
        other => panic!("expected capped Wait, got {other:?}"),
    }
}

#[test]
fn second_pending_upsert_writes_nothing() {
    let mut store = PendingStore::new();
    assert!(store.is_empty());
    let key = PendingKey {
        run_id: "run-1".to_owned(),
        promise_id: "promise-7".to_owned(),
        park_generation: 0,
    };
    let first = PendingRow {
        key: key.clone(),
        intent: park_intent(10_000, 10_000),
    };
    assert!(store.upsert_pending(first));
    assert_eq!(store.len(), 1);
    // Same key, different intent: writes nothing, keeps the original.
    let second = PendingRow {
        key: key.clone(),
        intent: park_intent(99_000, 99_000),
    };
    assert!(!store.upsert_pending(second));
    assert_eq!(store.len(), 1);
    assert_eq!(
        store.get(&key).expect("row").intent,
        park_intent(10_000, 10_000)
    );
    // A bumped generation is a new row.
    let next = PendingRow {
        key: PendingKey {
            park_generation: 1,
            ..key.clone()
        },
        intent: park_intent(20_000, 20_000),
    };
    assert!(store.upsert_pending(next));
    assert_eq!(store.len(), 2);
}

#[test]
fn parked_row_is_resumable_not_terminal() {
    let parked = EffectState::from_str("parked").expect("parked parses");
    assert_eq!(parked, EffectState::Parked);
    assert!(!parked.is_terminal());
    assert!(!parked.is_failed());
}
