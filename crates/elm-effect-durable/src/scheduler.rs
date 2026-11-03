//! Pure wake selection. No I/O.

use crate::storage::ParkedRow;

/// One due wake the runtime should turn into a Msg.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[must_use]
pub struct WakeIntent {
    /// Run key.
    pub key: String,
    /// Park generation being woken.
    pub park_generation: u64,
    /// Absolute deadline that fired.
    pub until_ms: u64,
    /// `now_ms - until_ms` (zero when exactly on time).
    pub overdue_ms: u64,
}

/// Given parked rows and `now_ms`, which runs to wake.
///
/// A row is due when `until_ms <= now_ms`. Order is stable by
/// `(until_ms, key, park_generation)` so tests are deterministic.
#[must_use]
pub fn due_wakes(parked: &[ParkedRow], now_ms: u64) -> Vec<WakeIntent> {
    let mut due: Vec<&ParkedRow> = parked.iter().filter(|r| r.until_ms <= now_ms).collect();
    due.sort_by(|a, b| {
        (a.until_ms, a.key.as_str(), a.park_generation).cmp(&(
            b.until_ms,
            b.key.as_str(),
            b.park_generation,
        ))
    });
    due.into_iter()
        .map(|r| WakeIntent {
            key: r.key.clone(),
            park_generation: r.park_generation,
            until_ms: r.until_ms,
            overdue_ms: now_ms.saturating_sub(r.until_ms),
        })
        .collect()
}

/// Pure child fan-out join evaluation: one pass over child phases.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[must_use]
pub enum JoinOutcome {
    /// Every child settled (or no children at all).
    AllSettled,
    /// Some children still Running or Parked.
    Pending {
        /// Keys still outstanding, in slice order.
        remaining: Vec<String>,
    },
    /// At least one child Failed, Cancelled, or Unknown.
    Failed {
        /// Keys that settled Failed, in deterministic slice order.
        failed: Vec<String>,
        /// Keys that settled Cancelled, in deterministic slice order.
        cancelled: Vec<String>,
        /// Keys that are Unknown, in deterministic slice order.
        unknown: Vec<String>,
    },
}

/// Evaluate a fan-out join without I/O.
///
/// Terminal failure wins over pending: any Failed/Cancelled/Unknown child
/// short-circuits to [`JoinOutcome::Failed`] even when other children are
/// still Running or Parked. Otherwise any Running/Parked child yields
/// [`JoinOutcome::Pending`]; all Settled (or empty) is [`JoinOutcome::AllSettled`].
pub fn plan_join(children: &[(String, crate::engine::RunPhase)]) -> JoinOutcome {
    use crate::engine::RunPhase;
    let mut failed = Vec::new();
    let mut cancelled = Vec::new();
    let mut unknown = Vec::new();
    let mut remaining = Vec::new();
    for (key, phase) in children {
        match phase {
            RunPhase::Failed => failed.push(key.clone()),
            RunPhase::Cancelled => cancelled.push(key.clone()),
            RunPhase::Unknown => unknown.push(key.clone()),
            RunPhase::Running | RunPhase::Parked => remaining.push(key.clone()),
            RunPhase::Settled => {}
        }
    }
    if !failed.is_empty() || !cancelled.is_empty() || !unknown.is_empty() {
        JoinOutcome::Failed {
            failed,
            cancelled,
            unknown,
        }
    } else if remaining.is_empty() {
        JoinOutcome::AllSettled
    } else {
        JoinOutcome::Pending { remaining }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, until: u64, park_gen: u64) -> ParkedRow {
        ParkedRow {
            key: key.into(),
            until_ms: until,
            park_generation: park_gen,
        }
    }

    #[test]
    fn due_wakes_only_rows_at_or_before_now() {
        let parked = vec![row("a", 100, 1), row("b", 200, 1), row("c", 50, 2)];
        let got = due_wakes(&parked, 100);
        assert_eq!(
            got,
            vec![
                WakeIntent {
                    key: "c".into(),
                    park_generation: 2,
                    until_ms: 50,
                    overdue_ms: 50,
                },
                WakeIntent {
                    key: "a".into(),
                    park_generation: 1,
                    until_ms: 100,
                    overdue_ms: 0,
                },
            ]
        );
    }

    #[test]
    fn due_wakes_empty_when_none_ready() {
        let parked = vec![row("a", 100, 1)];
        assert!(due_wakes(&parked, 99).is_empty());
    }

    fn child(key: &str, phase: crate::engine::RunPhase) -> (String, crate::engine::RunPhase) {
        (key.into(), phase)
    }

    #[test]
    fn plan_join_all_settled_and_empty() {
        use crate::engine::RunPhase::Settled;
        assert_eq!(plan_join(&[]), JoinOutcome::AllSettled);
        assert_eq!(
            plan_join(&[child("a", Settled), child("b", Settled)]),
            JoinOutcome::AllSettled
        );
    }

    #[test]
    fn plan_join_pending_while_running_or_parked() {
        use crate::engine::RunPhase::{Parked, Running, Settled};
        assert_eq!(
            plan_join(&[child("a", Settled), child("b", Running)]),
            JoinOutcome::Pending {
                remaining: vec!["b".into()]
            }
        );
        assert_eq!(
            plan_join(&[child("a", Parked), child("b", Running)]),
            JoinOutcome::Pending {
                remaining: vec!["a".into(), "b".into()]
            }
        );
    }

    #[test]
    fn plan_join_failed_short_circuits_pending() {
        use crate::engine::RunPhase::{Cancelled, Failed, Running, Settled, Unknown};
        // One failure wins even with a pending child present.
        assert_eq!(
            plan_join(&[
                child("ok", Settled),
                child("slow", Running),
                child("bad", Failed),
            ]),
            JoinOutcome::Failed {
                failed: vec!["bad".into()],
                cancelled: vec![],
                unknown: vec![],
            }
        );
        // Each terminal kind lands in its own bucket, in slice order.
        assert_eq!(
            plan_join(&[
                child("u1", Unknown),
                child("c1", Cancelled),
                child("f1", Failed),
                child("f2", Failed),
            ]),
            JoinOutcome::Failed {
                failed: vec!["f1".into(), "f2".into()],
                cancelled: vec!["c1".into()],
                unknown: vec!["u1".into()],
            }
        );
    }
}
