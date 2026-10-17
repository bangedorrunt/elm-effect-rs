//! Eventlog module fence: journal records, typed writes with handlers,
//! remote sequences, conflict reports, retry flush, chunk split/join.
//! Requires feature `eventlog` (off by default).

#![cfg(feature = "eventlog")]

use elm_effect_unstable::eventlog::{
    to_conflicts, unencrypted_server_authenticate, ConflictEntry, Entry, Event, EventGroup, EventJournal,
    EventLog, RemoteEntry, UnencryptedServerAuth, join_chunks, split_data,
};
use std::cell::{Cell, RefCell};
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

fn conflict_entry(id: &str, created_at_millis: u64, primary_key: &str) -> ConflictEntry {
    ConflictEntry {
        id: id.to_owned(),
        created_at_millis,
        event: "ReproEvent".to_owned(),
        primary_key: primary_key.to_owned(),
    }
}

/// `EventLogServerUnencrypted.test.ts:43` — conflicts are indexed from the
/// sliced history: an entry with an unseen key reports no conflicts, while a
/// later entry with a seen key reports the newer same-key entry.
#[test]
fn indexes_conflicts_from_the_sliced_history() {
    let history = vec![
        conflict_entry("old-same-key", 2_000, "key"),
        conflict_entry("newer-other-key", 4_000, "other"),
        conflict_entry("newer-same-key", 5_000, "key"),
    ];
    let origin_a = conflict_entry("origin-a", 1_000, "other-origin");
    let origin_b = conflict_entry("origin-b", 3_000, "key");

    let (duplicate_a, conflicts_a, sliced) = to_conflicts(&history, &origin_a);
    assert!(!duplicate_a);
    assert!(conflicts_a.is_empty());

    let (duplicate_b, conflicts_b, _) = to_conflicts(&sliced, &origin_b);
    assert!(!duplicate_b);
    assert_eq!(
        conflicts_b.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ["newer-same-key"]
    );
}

/// `EventLogServerUnencrypted.test.ts:147` — a denied first-use identity fails
/// closed and never creates a session-auth binding.
#[test]
fn authenticate_denies_first_use_without_creating_binding() {
    let mut server = UnencryptedServerAuth::new();
    let policy_calls = RefCell::new(Vec::new());
    let result = unencrypted_server_authenticate(&mut server, "identity-1", &[7u8; 32], |public_key| {
        policy_calls.borrow_mut().push(public_key.to_owned());
        Err("Forbidden".to_owned())
    });
    assert_eq!(result, Err("Forbidden".to_owned()));
    assert_eq!(policy_calls.borrow().as_slice(), ["identity-1"]);
    assert_eq!(server.binding_lookups(), 0);
}

/// `EventLogServerUnencrypted.test.ts:185` — identity authorization runs on
/// every authenticate, even when the session-auth binding is cached: revoking
/// after a first success fails the retry without a second binding lookup.
#[test]
fn authenticate_rechecks_identity_authorization_on_cached_binding() {
    let mut server = UnencryptedServerAuth::new();
    let allowed = Cell::new(true);
    let policy_calls = RefCell::new(Vec::new());
    let authorize = |public_key: &str| {
        policy_calls.borrow_mut().push(public_key.to_owned());
        if allowed.get() {
            Ok(())
        } else {
            Err("Forbidden".to_owned())
        }
    };

    assert_eq!(
        unencrypted_server_authenticate(&mut server, "identity-1", &[7u8; 32], &authorize),
        Ok(())
    );
    assert_eq!(server.binding_lookups(), 1);

    allowed.set(false);
    assert_eq!(
        unencrypted_server_authenticate(&mut server, "identity-1", &[7u8; 32], &authorize),
        Err("Forbidden".to_owned())
    );
    assert_eq!(policy_calls.borrow().as_slice(), ["identity-1", "identity-1"]);
    assert_eq!(server.binding_lookups(), 1);
}
