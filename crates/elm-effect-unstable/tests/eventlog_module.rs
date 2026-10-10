//! Eventlog module fence: journal records, typed writes with handlers,
//! remote sequences, conflict reports, retry flush, chunk split/join.
//! Requires feature `eventlog` (off by default).

#![cfg(feature = "eventlog")]

use elm_effect_unstable::eventlog::{
    Entry, Event, EventGroup, EventJournal, EventLog, RemoteEntry, join_chunks, split_data,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn counter() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}

#[test]
fn journal_records_and_publishes() {
    let mut journal = EventJournal::new();
    let seen = counter();
    let seen_body = seen.clone();
    let sub = journal.subscribe(move |_| {
        seen_body.fetch_add(1, Ordering::SeqCst);
    });
    let e1 = journal.record(Event::new("a", vec![1]));
    let e2 = journal.record(Event::new("b", vec![2]));
    assert_eq!(e1.seq, 1);
    assert_eq!(e2.seq, 2);
    assert_eq!(journal.entries().len(), 2);
    assert_eq!(seen.load(Ordering::SeqCst), 2);
    assert!(journal.unsubscribe(&sub));
    journal.record(Event::new("c", vec![3]));
    assert_eq!(seen.load(Ordering::SeqCst), 2);
    assert!(!journal.unsubscribe(&sub));
}

#[test]
fn write_from_remote_reports_duplicates_and_publishes() {
    // EventLog.test.ts:159 — each committed import publishes to subscribers,
    // and re-importing the same entry id, even under a fresh remote sequence,
    // is reported as a duplicate instead of being written twice.
    let mut log = EventLog::new(EventGroup::new());
    let seen = counter();
    let seen_body = seen.clone();
    let sub = log.subscribe(move |_| {
        seen_body.fetch_add(1, Ordering::SeqCst);
    });

    let remote_entry = |seq: u64| RemoteEntry {
        entry_id: "e1".to_string(),
        remote_sequence: seq,
        event: Event::new("UserCreated", vec![1]),
    };

    let first = log.write_from_remote("remote-1", vec![remote_entry(1)]);
    assert!(first.duplicates.is_empty());
    assert_eq!(first.written.len(), 1);
    assert_eq!(seen.load(Ordering::SeqCst), 1);

    // Same entry id, new sequence: a duplicate, and nothing new published.
    let second = log.write_from_remote("remote-1", vec![remote_entry(2)]);
    assert_eq!(second.duplicates.len(), 1);
    assert_eq!(second.duplicates[0].entry_id, "e1");
    assert!(second.written.is_empty());
    assert_eq!(seen.load(Ordering::SeqCst), 1);

    // Identity is per remote: the same id from another remote is its own entry.
    let other = log.write_from_remote("remote-2", vec![remote_entry(1)]);
    assert_eq!(other.written.len(), 1);
    assert!(other.duplicates.is_empty());
    assert_eq!(seen.load(Ordering::SeqCst), 2);

    // Dropping the subscription stops delivery of later local changes.
    assert!(log.unsubscribe(&sub));
    let third = log.write_from_remote(
        "remote-2",
        vec![RemoteEntry {
            entry_id: "e2".to_string(),
            remote_sequence: 2,
            event: Event::new("UserCreated", vec![2]),
        }],
    );
    assert_eq!(third.written.len(), 1);
    assert_eq!(seen.load(Ordering::SeqCst), 2);
    assert_eq!(log.entries().len(), 3);
}

#[test]
fn log_writes_typed_events_and_runs_handlers() {
    let mut group = EventGroup::new();
    let handled = counter();
    let handled_body = handled.clone();
    group.on("user.created", move |_| {
        handled_body.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(group.event_types(), ["user.created".to_owned()]);
    let mut log = EventLog::new(group);
    assert!(log.write(Event::new("user.created", vec![9])));
    assert!(!log.write(Event::new("unknown.kind", vec![0])));
    assert_eq!(handled.load(Ordering::SeqCst), 1);
    assert_eq!(log.entries().len(), 2);
}

#[test]
fn remote_seq_conflict_and_relay() {
    let mut log = EventLog::new(EventGroup::new());
    assert_eq!(log.next_remote_seq("r1"), 1);
    assert_eq!(log.next_remote_seq("r1"), 2);
    let entry = log.relay("r2", Event::new("e", vec![1]), 5);
    assert_eq!(entry.seq, 1);
    assert_eq!(log.next_remote_seq("r2"), 6);
    log.write(Event::new("e", vec![2]));
    let conflict: Vec<Entry> = log.first_conflict_after(1).into_iter().cloned().collect();
    assert_eq!(conflict.len(), 1);
    assert_eq!(conflict[0].seq, 2);
    assert!(log.first_conflict_after(99).is_none());
}

#[test]
fn retry_flushes_changes_made_during_retry() {
    let mut log = EventLog::new(EventGroup::new());
    let tries = counter();
    let tries_body = tries.clone();
    let entry = log
        .retry_remote_write(3, move || {
            let n = tries_body.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Err("timeout".to_owned())
            } else {
                Ok(Event::new("e", vec![n as u8]))
            }
        })
        .expect("succeeds on retry");
    assert_eq!(entry.seq, 1);
    assert_eq!(tries.load(Ordering::SeqCst), 2);
    assert_eq!(log.entries().len(), 1);
    assert!(
        log.retry_remote_write(2, || Err::<Event, String>("down".to_owned()))
            .is_err()
    );
}

#[test]
fn chunks_split_and_join() {
    assert_eq!(split_data(&[], 4), [Vec::<u8>::new()]);
    assert_eq!(
        split_data(&[1, 2, 3, 4, 5], 2),
        [vec![1, 2], vec![3, 4], vec![5]]
    );
    assert_eq!(join_chunks(&[vec![1, 2], vec![1, 2], vec![3]]), [1, 2, 3]);
    assert_eq!(join_chunks(&[Vec::new()]), Vec::<u8>::new());
}
