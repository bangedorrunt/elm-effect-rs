//! Dispatch table tests: every arm of `plan_attempt`, plus settle guards.

#[path = "../src/plan.rs"]
mod plan;

use elm_effect_journal::EffectState as S;
use plan::{
    DispatchPlan, EffectView, ProviderCapability, check_replay, may_settle, plan_attempt,
    terminal_success,
};

fn view(state: S, attempt: i64, lease_live: bool) -> EffectView {
    EffectView {
        state,
        attempt,
        lease_live,
    }
}

fn cap(idempotent: bool, query: bool) -> ProviderCapability {
    ProviderCapability {
        supports_idempotency_key: idempotent,
        supports_reconcile_query: query,
    }
}

#[test]
fn none_and_prepared_dispatch_new() {
    assert_eq!(
        plan_attempt(None, &cap(false, false)),
        DispatchPlan::DispatchNew
    );
    assert_eq!(
        plan_attempt(Some(&view(S::Prepared, 1, false)), &cap(false, false)),
        DispatchPlan::DispatchNew
    );
}

#[test]
fn reconcile_pending_refuses_blind_retry() {
    assert!(matches!(
        plan_attempt(Some(&view(S::ReconcilePending, 2, false)), &cap(true, true)),
        DispatchPlan::RefuseBlindRetry { .. }
    ));
}

#[test]
fn live_lease_refuses_regardless_of_contract() {
    assert!(matches!(
        plan_attempt(Some(&view(S::InFlight, 1, true)), &cap(true, true)),
        DispatchPlan::RefuseBlindRetry { .. }
    ));
}

#[test]
fn query_beats_idempotency() {
    assert!(matches!(
        plan_attempt(Some(&view(S::Accepted, 3, false)), &cap(true, true)),
        DispatchPlan::ReconcileFirst { .. }
    ));
}

#[test]
fn retry_same_key_bumps_attempt() {
    assert_eq!(
        plan_attempt(Some(&view(S::Unknown, 4, false)), &cap(true, false)),
        DispatchPlan::RetrySameKey { attempt: 5 }
    );
}

#[test]
fn no_contract_refuses() {
    assert!(matches!(
        plan_attempt(Some(&view(S::InFlight, 1, false)), &cap(false, false)),
        DispatchPlan::RefuseBlindRetry { .. }
    ));
}

#[test]
fn resume_parked_floors_at_one() {
    assert_eq!(
        plan_attempt(Some(&view(S::Parked, 0, false)), &cap(false, false)),
        DispatchPlan::ResumeParked { attempt: 1 }
    );
    assert_eq!(
        plan_attempt(Some(&view(S::Parked, 7, true)), &cap(true, true)),
        DispatchPlan::ResumeParked { attempt: 7 }
    );
}

#[test]
fn terminals_return_stored_outcome() {
    for state in [S::Settled, S::Failed, S::Cancelled] {
        assert_eq!(
            plan_attempt(Some(&view(state, 9, true)), &cap(true, true)),
            DispatchPlan::DuplicateTerminal
        );
    }
}

#[test]
fn empty_token_never_settles() {
    assert!(!may_settle(Some("tok"), ""));
    assert!(!may_settle(Some(""), "tok"));
    assert!(!may_settle(None, "tok"));
    assert!(!may_settle(None, ""));
    assert!(may_settle(Some("tok"), "tok"));
    assert!(!may_settle(Some("tok"), "other"));
}

#[test]
fn replay_digest_mismatch_mints_nothing() {
    assert!(check_replay("abc", "abc").is_ok());
    let err = check_replay("abc", "xyz").expect_err("mismatch is typed");
    assert_eq!(err.prepared, "abc");
    assert_eq!(err.redigest, "xyz");
}

#[test]
fn unknown_has_no_default_exit() {
    assert_eq!(terminal_success(S::Unknown), None);
    assert_eq!(terminal_success(S::Parked), None);
    assert_eq!(terminal_success(S::InFlight), None);
    assert_eq!(terminal_success(S::Settled), Some(true));
    assert_eq!(terminal_success(S::Failed), Some(false));
}
