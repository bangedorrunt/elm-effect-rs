//! Journal fence: the golden transition table plus Parked semantics.

use elm_effect_journal::{EffectState, can_transition, settle_from_exit, transition};
use elm_effect_journal::EffectState as S;

#[test]
fn golden_transitions() {
    // Every golden edge accepts.
    let legal = [
        (S::Prepared, S::InFlight),
        (S::Prepared, S::Failed),
        (S::Prepared, S::Cancelled),
        (S::InFlight, S::Accepted),
        (S::InFlight, S::Settled),
        (S::InFlight, S::Failed),
        (S::InFlight, S::Unknown),
        (S::InFlight, S::Cancelled),
        (S::Accepted, S::Settled),
        (S::Accepted, S::Failed),
        (S::Accepted, S::Unknown),
        (S::Accepted, S::Cancelled),
        (S::Unknown, S::ReconcilePending),
        (S::Unknown, S::Cancelled),
        (S::ReconcilePending, S::Settled),
        (S::ReconcilePending, S::Failed),
        (S::ReconcilePending, S::Unknown),
        (S::ReconcilePending, S::Cancelled),
    ];
    assert_eq!(legal.len(), 18);
    for (from, to) in legal {
        assert!(can_transition(from, to), "{from:?} -> {to:?}");
        assert_eq!(transition(from, to), Ok(to));
    }
    // Rejections: terminal states go nowhere, Prepared skips nothing
    // forward, failures never settle, Parked has no edge at all.
    for (from, to) in [
        (S::Settled, S::Unknown),
        (S::Failed, S::Settled),
        (S::Prepared, S::Settled),
        (S::Settled, S::Settled),
        (S::Failed, S::Failed),
        (S::Cancelled, S::Prepared),
        (S::Parked, S::InFlight),
        (S::InFlight, S::Parked),
        (S::Parked, S::Parked),
        (S::Prepared, S::Parked),
    ] {
        assert!(!can_transition(from, to), "{from:?} -/-> {to:?}");
        assert!(transition(from, to).is_err());
    }
}

#[test]
fn parked_is_not_terminal_not_failed_not_settled() {
    assert!(!EffectState::Parked.is_terminal());
    assert!(!EffectState::Parked.is_failed());
    assert!(!EffectState::Parked.is_settled());
    assert!(EffectState::Parked.is_resumable());
    assert!(EffectState::Settled.is_terminal());
    assert!(EffectState::Failed.is_terminal());
    assert!(EffectState::Failed.is_failed());
    assert!(EffectState::Settled.is_settled());
}

#[test]
fn settle_from_exit_maps_outcomes() {
    assert_eq!(
        settle_from_exit(&elm_effect::Exit::<i32, String>::Success(1)),
        EffectState::Settled
    );
    let failed: elm_effect::Exit<i32, String> =
        elm_effect::Exit::Failure(elm_effect::Cause::fail("boom".to_string()));
    assert_eq!(settle_from_exit(&failed), EffectState::Failed);
}
