//! Epoch fence: same-seq dedupe, stale fencing, zero-start epochs,
//! epoch-less forks. The module is public on the crate (lib.rs
//! wires it); the old `#[path]` include is gone.

use elm_effect_journal::epoch;

use epoch::{Appended, EpochLog, Fence};

#[test]
fn same_epoch_same_seq_appends_once() {
    let mut log = EpochLog::new();
    let first = log.append("p", 0, 0).unwrap();
    let dup = log.append("p", 0, 0).unwrap();
    assert!(matches!(first, Appended::Fresh { offset: 0 }));
    assert!(matches!(dup, Appended::Duplicate { offset: 0 }));
    assert_eq!(log.entries().len(), 1);
}

#[test]
fn stale_epoch_is_fenced() {
    let mut log = EpochLog::new();
    log.append("p", 1, 0).unwrap();
    let err = log.append("p", 0, 5).unwrap_err();
    assert_eq!(
        err,
        Fence::StaleEpoch {
            producer: "p".to_string(),
            sent: 0,
            current: 1
        }
    );
    assert_eq!(log.entries().len(), 1);
}

#[test]
fn new_epoch_starts_at_zero() {
    let mut log = EpochLog::new();
    log.append("p", 0, 7).unwrap();
    let next = log.append("p", 1, 0).unwrap();
    assert!(matches!(next, Appended::Fresh { .. }));
    assert_eq!(log.epoch_of("p"), Some(1));
    // Old-epoch traffic stays fenced after the move.
    assert!(log.append("p", 0, 8).is_err());
}

#[test]
fn fork_drops_producer_epoch() {
    let mut log = EpochLog::new();
    log.append("p", 3, 9).unwrap();
    log.append("q", 0, 0).unwrap();
    let fork = log.fork(1);
    assert_eq!(fork.entries().len(), 1);
    assert_eq!(fork.epoch_of("p"), None);
    // The fork's first epoch is 0, even though the parent was at 3.
    let mut fork = fork;
    assert!(matches!(
        fork.append("p", 0, 0).unwrap(),
        Appended::Fresh { .. }
    ));
}
