//! Saga compensation planner + typed fence diagnostics (bead f9tg).
//!
//! Pure LIFO rollback: forward steps in, reverse-order compensations out.
//! Already-compensated steps are skipped so a crashed rollback resumes
//! without double-compensating.

use elm_effect_journal::EffectState;
use elm_effect_journal::plan::{ClaimRejection, SettleRejection, check_claim, check_settle};
use elm_effect_journal::saga::{
    SagaPlan, SagaStep, SagaStepStatus, compensate_key, plan_saga_rollback,
};

fn step(key: &str, status: SagaStepStatus) -> SagaStep {
    SagaStep {
        step_key: key.to_string(),
        compensate_key: compensate_key("run-1", key),
        status,
    }
}

#[test]
fn compensate_key_formatting() {
    assert_eq!(compensate_key("run-1", "book"), "run-1:compensate:book");
    assert_eq!(compensate_key("", ""), ":compensate:");
}

#[test]
fn rollback_is_strict_lifo_over_completed_steps() {
    let steps = vec![
        step("a", SagaStepStatus::Completed),
        step("b", SagaStepStatus::Completed),
        step("c", SagaStepStatus::Completed),
    ];
    let SagaPlan {
        compensations,
        all_compensated,
    } = plan_saga_rollback(&steps);
    assert_eq!(
        compensations,
        vec![
            "run-1:compensate:c".to_string(),
            "run-1:compensate:b".to_string(),
            "run-1:compensate:a".to_string(),
        ]
    );
    assert!(!all_compensated);
}

#[test]
fn rollback_skips_failed_and_skipped_steps() {
    let steps = vec![
        step("a", SagaStepStatus::Completed),
        step("b", SagaStepStatus::Failed),
        step("c", SagaStepStatus::Skipped),
        step("d", SagaStepStatus::Completed),
    ];
    let plan = plan_saga_rollback(&steps);
    assert_eq!(
        plan.compensations,
        vec![
            "run-1:compensate:d".to_string(),
            "run-1:compensate:a".to_string(),
        ]
    );
    assert!(!plan.all_compensated);
}

#[test]
fn rollback_resume_skips_already_compensated_steps() {
    // Crash mid-rollback: c already compensated, b still completed.
    let steps = vec![
        step("a", SagaStepStatus::Compensated),
        step("b", SagaStepStatus::Completed),
        step("c", SagaStepStatus::Compensated),
    ];
    let plan = plan_saga_rollback(&steps);
    assert_eq!(plan.compensations, vec!["run-1:compensate:b".to_string()]);
    assert!(!plan.all_compensated);
}

#[test]
fn rollback_empty_when_nothing_completed() {
    let steps = vec![
        step("a", SagaStepStatus::Compensated),
        step("b", SagaStepStatus::Failed),
        step("c", SagaStepStatus::Skipped),
    ];
    let plan = plan_saga_rollback(&steps);
    assert!(plan.compensations.is_empty());
    assert!(plan.all_compensated);
}

#[test]
fn rollback_empty_input_is_all_compensated() {
    let plan = plan_saga_rollback(&[]);
    assert!(plan.compensations.is_empty());
    assert!(plan.all_compensated);
}

#[test]
fn check_claim_accepts_fresh_and_renewal() {
    // Unclaimed row.
    assert_eq!(
        check_claim(EffectState::Prepared, None, None, "w1", 100, 3, 3),
        Ok(())
    );
    // Same-owner renewal under a live lease.
    assert_eq!(
        check_claim(
            EffectState::InFlight,
            Some("w1"),
            Some(200),
            "w1",
            100,
            3,
            3
        ),
        Ok(())
    );
    // Expired lease under another owner is reclaimable.
    assert_eq!(
        check_claim(EffectState::Accepted, Some("w1"), Some(50), "w2", 100, 3, 3),
        Ok(())
    );
}

#[test]
fn check_claim_rejects_terminal_stale_and_live_lease() {
    assert_eq!(
        check_claim(EffectState::Settled, None, None, "w1", 100, 0, 0),
        Err(ClaimRejection::Terminal {
            state: EffectState::Settled,
        })
    );
    assert_eq!(
        check_claim(EffectState::Prepared, None, None, "w1", 100, 2, 3),
        Err(ClaimRejection::StaleGeneration {
            expected: 2,
            actual: 3,
        })
    );
    assert_eq!(
        check_claim(
            EffectState::InFlight,
            Some("w1"),
            Some(200),
            "w2",
            100,
            3,
            3
        ),
        Err(ClaimRejection::LeaseLive {
            owner: "w1".to_string(),
            lease_deadline: 200,
        })
    );
    // Terminal beats stale generation: a finished row rejects everything.
    assert_eq!(
        check_claim(EffectState::Failed, None, None, "w1", 100, 2, 3),
        Err(ClaimRejection::Terminal {
            state: EffectState::Failed,
        })
    );
}

#[test]
fn check_settle_accepts_legal_edge() {
    assert_eq!(
        check_settle(Some((EffectState::InFlight, 4)), 4, EffectState::Settled),
        Ok(())
    );
}

#[test]
fn check_settle_rejects_all_four_ways() {
    assert_eq!(
        check_settle(None, 0, EffectState::Settled),
        Err(SettleRejection::NotFound)
    );
    assert_eq!(
        check_settle(Some((EffectState::Settled, 4)), 4, EffectState::Failed),
        Err(SettleRejection::Terminal {
            state: EffectState::Settled,
        })
    );
    assert_eq!(
        check_settle(Some((EffectState::InFlight, 4)), 3, EffectState::Settled),
        Err(SettleRejection::StaleGeneration {
            expected: 3,
            actual: 4,
        })
    );
    assert_eq!(
        check_settle(Some((EffectState::Prepared, 4)), 4, EffectState::Settled),
        Err(SettleRejection::IllegalTransition {
            from: EffectState::Prepared,
            to: EffectState::Settled,
        })
    );
}
