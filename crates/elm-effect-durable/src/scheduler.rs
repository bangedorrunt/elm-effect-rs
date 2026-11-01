//! Pure wake selection. No I/O.

use crate::storage::ParkedRow;

/// One due wake the runtime should turn into a Msg.
#[derive(Clone, Debug, PartialEq, Eq)]
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
}
