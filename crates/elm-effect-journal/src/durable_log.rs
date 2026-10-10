//! Law L16 (append part): a durable log is a trait; the kernel here runs
//! in-process (the log IS the stream, not a client of any server).
//!
//! Offsets are opaque, lexicographically ordered, and minted by the log:
//! [`Offset::Start`] (`-1`) and [`Offset::Now`] (tail) are sentinels and are
//! never minted. A hand-built offset fails validation on read. Appends are
//! atomic under one lock: a crash leaves the previous offset, reads never
//! see partial messages, and JSON mode keeps message boundaries. Producer
//! identity travels all-or-none as `(producer_id, epoch, seq)`; a `seq` that
//! is neither `last + 1` nor already accepted is rejected, while a replayed
//! accepted `seq` returns its original offset without duplicating. Every
//! JSON message carries the schema version; unknown versions are typed
//! errors.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// Schema versions this log reads.
pub const SCHEMA_VERSION: u32 = 1;

/// An offset: opaque to callers, ordered lexicographically. `Start` reads
/// from the beginning, `Now` reads nothing (already at the tail).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Offset {
    /// Read from the beginning. Never minted.
    Start,
    /// Already at the tail: reads return nothing. Never minted.
    Now,
    /// A minted offset. Opaque: build only via append.
    At(String),
}

/// Producer identity: all three parts travel together or not at all.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProducerId {
    /// Who is producing.
    pub producer_id: String,
    /// The producer's epoch (restarts begin a new epoch).
    pub epoch: u64,
    /// The message sequence within the epoch.
    pub seq: u64,
}

/// Typed append/read failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppendError {
    /// The envelope is not valid JSON or not an envelope.
    #[error("not a log envelope: {0}")]
    NotAnEnvelope(String),
    /// Identity arrived partial: all of `(producer_id, epoch, seq)` or none.
    #[error("partial producer identity")]
    PartialIdentity,
    /// Unknown schema version (this log reads v1).
    #[error("unknown schema version: {0}")]
    UnknownVersion(u32),
    /// `seq` is neither `last + 1` nor already accepted.
    #[error("seq gap: got {got}, last accepted {last:?}")]
    SeqGap { got: u64, last: Option<u64> },
    /// The offset was not minted by this log.
    #[error("unknown offset")]
    UnknownOffset,
}

/// A durable log: atomic appends, validated reads.
pub trait DurableLog {
    /// Append one JSON envelope, returning its minted offset.
    fn append_json(&self, envelope: &str) -> Result<Offset, AppendError>;
    /// Read every message after `from`. Hand-built offsets are rejected.
    fn read_from(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError>;
}

/// In-process log: one lock, one message vector. Atomicity falls out of
/// the lock discipline (a crash between appends leaves the previous
/// offset); reads clone complete messages, never partial ones.
#[derive(Debug, Default)]
pub struct MemLog {
    inner: Mutex<MemInner>,
}

#[derive(Debug, Default)]
struct MemInner {
    entries: Vec<MemEntry>,
    /// Last accepted seq per `(producer_id, epoch)`.
    last_seq: HashMap<(String, u64), u64>,
    /// Accepted seqs per producer, for idempotent replays.
    accepted: HashMap<(String, u64), HashSet<u64>>,
    /// Minted offset per accepted seq, for replay replies.
    offsets: HashMap<(String, u64, u64), String>,
    next: u64,
}

#[derive(Debug, Clone)]
struct MemEntry {
    offset: String,
    message: String,
}

impl MemLog {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint the next offset: zero-padded, lexicographically ordered.
    fn mint(next: u64) -> String {
        format!("{next:020}")
    }

    /// Validate a presented offset against minted history.
    fn check(&self, offset: &Offset) -> Result<Option<u64>, AppendError> {
        match offset {
            Offset::Start => Ok(None),
            Offset::Now => Ok(Some(u64::MAX)),
            Offset::At(raw) => {
                let seq: u64 = raw.parse().map_err(|_| AppendError::UnknownOffset)?;
                let inner = self.inner.lock().unwrap();
                if seq < inner.next {
                    Ok(Some(seq))
                } else {
                    Err(AppendError::UnknownOffset)
                }
            }
        }
    }
}

impl DurableLog for MemLog {
    fn append_json(&self, envelope: &str) -> Result<Offset, AppendError> {
        let value: serde_json::Value = serde_json::from_str(envelope)
            .map_err(|e| AppendError::NotAnEnvelope(e.to_string()))?;
        let version = value
            .get("v")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| AppendError::NotAnEnvelope("missing v".into()))?;
        if version != u64::from(SCHEMA_VERSION) {
            return Err(AppendError::UnknownVersion(version as u32));
        }
        let producer = value.get("producer");
        let (id, epoch, seq) = match producer {
            None => return Err(AppendError::PartialIdentity),
            Some(p) => {
                let id = p.get("producer_id").and_then(serde_json::Value::as_str);
                let epoch = p.get("epoch").and_then(serde_json::Value::as_u64);
                let seq = p.get("seq").and_then(serde_json::Value::as_u64);
                match (id, epoch, seq) {
                    (Some(id), Some(epoch), Some(seq)) => (id.to_string(), epoch, seq),
                    _ => return Err(AppendError::PartialIdentity),
                }
            }
        };
        // Null producer part is also partial: explicit nulls fail as above
        // (`as_str`/`as_u64` return `None` for nulls).
        let message = value
            .get("payload")
            .map(|v| v.to_string())
            .unwrap_or_default();
        let mut inner = self.inner.lock().unwrap();
        let key = (id.clone(), epoch);
        let last = inner.last_seq.get(&key).copied();
        if let Some(offset) = inner
            .accepted
            .get(&key)
            .filter(|set| set.contains(&seq))
            .and_then(|_| inner.offsets.get(&(id.clone(), epoch, seq)).cloned())
        {
            // Idempotent replay: same triple appends once, replies the offset.
            return Ok(Offset::At(offset));
        }
        let expected = last.map(|l| l + 1).unwrap_or(0);
        if seq != expected {
            return Err(AppendError::SeqGap { got: seq, last });
        }
        let offset = Self::mint(inner.next);
        inner.next += 1;
        inner.last_seq.insert(key.clone(), seq);
        inner.accepted.entry(key.clone()).or_default().insert(seq);
        inner
            .offsets
            .insert((id.clone(), epoch, seq), offset.clone());
        inner.entries.push(MemEntry { offset: offset.clone(), message });
        Ok(Offset::At(offset))
    }

    fn read_from(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError> {
        let after = self.check(from)?;
        if after == Some(u64::MAX) {
            return Ok(Vec::new());
        }
        let inner = self.inner.lock().unwrap();
        let mut out = Vec::new();
        for entry in &inner.entries {
            let seq: u64 = entry.offset.parse().expect("minted");
            let past = after.map_or(true, |limit| seq > limit);
            if past {
                out.push((Offset::At(entry.offset.clone()), entry.message.clone()));
            }
        }
        Ok(out)
    }
}
