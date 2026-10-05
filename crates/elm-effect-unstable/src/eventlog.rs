//! Event log (`eventlog/Event`, `EventGroup`, `EventJournal`, `EventLog`,
//! `EventLogMessage` at the pin).
//!
//! Append-only data contract, not a cache: [`EventJournal`] records entries
//! and publishes local changes through scoped subscriptions;
//! [`EventLog`] writes typed events, commits entries, runs handlers, tracks
//! remote sequences, and reports the first newer conflicting entry;
//! [`split_data`]/[`join_chunks`] move bytes with duplicate-tolerant joins.
//!
//! Residuals (transport/security surface, out of timebox): remotes over the
//! wire, session auth, entry encryption (distinct IV per entry), SQL
//! storage backends, and the server variants.

use std::collections::{HashMap, HashSet};

/// One typed event: a name plus opaque bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// Event type name.
    pub event_type: String,
    /// Payload bytes.
    pub payload: Vec<u8>,
}

impl Event {
    /// Build from a type name and payload bytes.
    pub fn new(event_type: &str, payload: Vec<u8>) -> Self {
        Self {
            event_type: event_type.to_owned(),
            payload,
        }
    }
}

/// Handler for one event type.
type Handler = Box<dyn Fn(&Event) + Send + Sync>;

/// Registry of known event types with their handlers.
#[derive(Default)]
pub struct EventGroup {
    handlers: HashMap<String, Handler>,
}

impl EventGroup {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler for one event type.
    pub fn on(&mut self, event_type: &str, handler: impl Fn(&Event) + Send + Sync + 'static) {
        self.handlers
            .insert(event_type.to_owned(), Box::new(handler));
    }

    /// Known event types.
    pub fn event_types(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }

    /// Run the handler for `event`, if registered. `true` when handled.
    pub fn handle(&self, event: &Event) -> bool {
        match self.handlers.get(&event.event_type) {
            Some(handler) => {
                handler(event);
                true
            }
            None => false,
        }
    }
}

/// One committed journal entry: sequence plus event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Monotonic sequence within the journal.
    pub seq: u64,
    /// The event.
    pub event: Event,
}

/// Subscription id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SubscriptionId(u64);

/// Local-change subscriber.
type Subscriber = Box<dyn Fn(&Entry) + Send + Sync>;

/// In-memory journal: records entries, publishes local changes through
/// scoped subscriptions (dropping the subscription stops delivery).
#[derive(Default)]
pub struct EventJournal {
    entries: Vec<Entry>,
    next_sub: u64,
    subscribers: HashMap<SubscriptionId, Subscriber>,
}

impl EventJournal {
    /// Empty journal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one event, publishing to live subscribers. Returns the entry.
    pub fn record(&mut self, event: Event) -> Entry {
        let entry = Entry {
            seq: self.entries.len() as u64 + 1,
            event,
        };
        self.entries.push(entry.clone());
        for subscriber in self.subscribers.values() {
            subscriber(&entry);
        }
        entry
    }

    /// All entries in order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Subscribe to local changes; the returned id unsubscribes.
    pub fn subscribe(
        &mut self,
        on_entry: impl Fn(&Entry) + Send + Sync + 'static,
    ) -> SubscriptionId {
        self.next_sub += 1;
        let id = SubscriptionId(self.next_sub);
        self.subscribers.insert(id, Box::new(on_entry));
        id
    }

    /// Drop a subscription; `false` when unknown.
    pub fn unsubscribe(&mut self, id: &SubscriptionId) -> bool {
        self.subscribers.remove(id).is_some()
    }
}

/// Typed event log over a journal: writes typed events, commits entries,
/// runs group handlers, and tracks remote sequences with conflict reports.
pub struct EventLog {
    journal: EventJournal,
    group: EventGroup,
    remote_seqs: HashMap<String, u64>,
}

impl EventLog {
    /// Empty log with a handler group.
    pub fn new(group: EventGroup) -> Self {
        Self {
            journal: EventJournal::new(),
            group,
            remote_seqs: HashMap::new(),
        }
    }

    /// Write a typed event: commit the entry and run its handler.
    /// Returns `true` when a handler ran.
    pub fn write(&mut self, event: Event) -> bool {
        let entry = self.journal.record(event.clone());
        self.group.handle(&entry.event)
    }

    /// Committed entries in order.
    pub fn entries(&self) -> &[Entry] {
        self.journal.entries()
    }

    /// Next unused remote sequence for `remote` (starts at 1).
    pub fn next_remote_seq(&mut self, remote: &str) -> u64 {
        let next = self.remote_seqs.get(remote).copied().unwrap_or(0) + 1;
        self.remote_seqs.insert(remote.to_owned(), next);
        next
    }

    /// Relay an imported entry to another remote: records it locally and
    /// advances that remote's sequence past it.
    pub fn relay(&mut self, remote: &str, event: Event, seq: u64) -> Entry {
        let current = self.remote_seqs.get(remote).copied().unwrap_or(0);
        if seq > current {
            self.remote_seqs.insert(remote.to_owned(), seq);
        }
        self.journal.record(event)
    }

    /// First entry newer than `seq` from a different producer shape: the
    /// conflict report is the earliest entry the caller has not seen.
    /// Here: earliest entry with `seq` greater than the given one, if any.
    pub fn first_conflict_after(&self, seq: u64) -> Option<&Entry> {
        self.journal.entries().iter().find(|e| e.seq > seq)
    }

    /// Retry a failed remote write up to `max_attempts` times: re-run
    /// `attempt` until it succeeds, flushing changes made meanwhile
    /// (entries recorded during the retry stay committed in order).
    pub fn retry_remote_write(
        &mut self,
        max_attempts: usize,
        mut attempt: impl FnMut() -> Result<Event, String>,
    ) -> Result<Entry, String> {
        let mut last_err = "attempts exhausted".to_owned();
        for _ in 0..max_attempts.max(1) {
            match attempt() {
                Ok(event) => return Ok(self.journal.record(event)),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }
}

/// Split bytes into `chunk_size` pieces; empty data splits into one chunk.
pub fn split_data(data: &[u8], chunk_size: usize) -> Vec<Vec<u8>> {
    let chunk_size = chunk_size.max(1);
    if data.is_empty() {
        return vec![Vec::new()];
    }
    data.chunks(chunk_size).map(|c| c.to_vec()).collect()
}

/// Join chunks, ignoring exact duplicates (idempotent re-delivery).
pub fn join_chunks(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for chunk in chunks {
        if seen.insert(chunk.clone()) {
            out.extend_from_slice(chunk);
        }
    }
    out
}
