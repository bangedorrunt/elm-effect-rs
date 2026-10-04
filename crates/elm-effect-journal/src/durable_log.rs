//! Durable log (`DurableLog` trait + in-process kernel).
//!
//! The log IS the stream, not a client of any server: one ordered,
//! atomic-append record per live log. Offsets are opaque, minted, and
//! lexicographically ordered; the `-1` (start) and `now` (tail) sentinels
//! are never minted, and a hand-built offset is rejected everywhere a token
//! is taken. Producer identity is `(producer_id, epoch, seq)` all-or-none:
//! an empty producer id is a partial identity and is rejected; a `seq` that
//! is neither `last + 1` nor already accepted is a gap and is rejected,
//! while the same triple appends once (idempotent replay returns the first
//! offset). Every JSON message carries the schema version; an unknown
//! version is a typed error, never a silent accept.
//!
//! Residuals: the HTTP/pull transport (`durable-streams` feature),
//! multi-producer merge cursors, and compaction.

use std::collections::{HashMap, HashSet};

/// Schema version stamped on every JSON message.
pub const SCHEMA_VERSION: u32 = 1;

/// Start sentinel: reads from the beginning. Never minted.
pub const OFFSET_START: &str = "-1";
/// Tail sentinel: reads from the live end. Never minted.
pub const OFFSET_TAIL: &str = "now";

/// Opaque minted offset. Lexicographic order is append order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Offset(String);

impl Offset {
    /// Render for storage (snapshot rows store this token).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Build an unminted token. Test-only: the way to prove the log
    /// rejects tokens it never minted.
    pub fn from_unminted_for_test(token: &str) -> Self {
        Self(token.to_owned())
    }
}

/// Read bound: a sentinel or a minted offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadBound {
    /// From the beginning.
    Start,
    /// After a minted offset (exclusive).
    After(Offset),
    /// Nothing to read (already at the tail).
    Tail,
}

/// Producer identity: all three parts or nothing.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProducerId {
    /// Producer name. Empty is a partial identity and is rejected.
    pub producer_id: String,
    /// Epoch (forks restart at 0).
    pub epoch: u64,
    /// Sequence within the epoch.
    pub seq: u64,
}

/// One JSON message with its schema version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonMessage {
    /// Must equal [`SCHEMA_VERSION`] on append.
    pub schema_version: u32,
    /// JSON body (boundaries preserved verbatim).
    pub body: String,
}

/// Stored record: minted offset plus message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Minted at append.
    pub offset: Offset,
    /// Producer triple that appended it.
    pub producer: ProducerId,
    /// The message.
    pub message: JsonMessage,
}

/// Typed append/read failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogError {
    /// Offset token the log never minted (includes the sentinels where a
    /// real offset is required, and any hand-built string).
    HandBuiltOffset(String),
    /// `seq` is neither `last + 1` nor already accepted.
    SeqGap {
        /// Producer and epoch the gap belongs to.
        producer_id: String,
        /// Epoch of the gap.
        epoch: u64,
        /// Expected next sequence.
        expected: u64,
        /// Offered sequence.
        offered: u64,
    },
    /// Producer identity arrived partial (empty producer id).
    PartialIdentity,
    /// Message schema version is not [`SCHEMA_VERSION`].
    UnknownSchemaVersion(u32),
}

/// Durable log contract. The kernel below runs it in process; toron
/// implements this trait later over its own store.
pub trait DurableLog {
    /// Atomically append one message. Idempotent on the full triple:
    /// re-appending an accepted `(producer_id, epoch, seq)` returns the
    /// first offset without writing.
    fn append(&mut self, producer: ProducerId, message: JsonMessage) -> Result<Offset, LogError>;
    /// Read records after `bound` (up to `limit`). A non-minted offset is
    /// rejected; the sentinels are never mistaken for real offsets.
    fn read(&self, bound: &ReadBound, limit: usize) -> Result<Vec<Record>, LogError>;
    /// Latest minted offset, if any.
    fn latest(&self) -> Option<Offset>;
}

/// In-process kernel: the log is a `Vec` behind one lock-step mutex, so an
/// append is a single push (a crash leaves the previous offset) and a read
/// never sees a partial message.
#[derive(Debug, Default)]
pub struct InProcessLog {
    records: Vec<Record>,
    minted: HashSet<String>,
    next_seq: u64,
    accepted: HashMap<(String, u64), std::collections::BTreeSet<u64>>,
    offsets: HashMap<(String, u64, u64), Offset>,
}

impl InProcessLog {
    /// Empty log.
    pub fn new() -> Self {
        Self::default()
    }

    fn mint(&mut self) -> Offset {
        self.next_seq += 1;
        let offset = Offset(format!("{:020}", self.next_seq));
        self.minted.insert(offset.0.clone());
        offset
    }

    fn check_offset(&self, offset: &Offset) -> Result<(), LogError> {
        if offset.0 == OFFSET_START || offset.0 == OFFSET_TAIL || !self.minted.contains(&offset.0) {
            return Err(LogError::HandBuiltOffset(offset.0.clone()));
        }
        Ok(())
    }
}

impl DurableLog for InProcessLog {
    fn append(&mut self, producer: ProducerId, message: JsonMessage) -> Result<Offset, LogError> {
        if producer.producer_id.is_empty() {
            return Err(LogError::PartialIdentity);
        }
        if message.schema_version != SCHEMA_VERSION {
            return Err(LogError::UnknownSchemaVersion(message.schema_version));
        }
        let triple = (producer.producer_id.clone(), producer.epoch, producer.seq);
        if let Some(offset) = self.offsets.get(&triple) {
            return Ok(offset.clone());
        }
        let key = (producer.producer_id.clone(), producer.epoch);
        let expected = self
            .accepted
            .get(&key)
            .and_then(|s| s.last().copied())
            .unwrap_or(0)
            + 1;
        if producer.seq != expected {
            return Err(LogError::SeqGap {
                producer_id: producer.producer_id.clone(),
                epoch: producer.epoch,
                expected,
                offered: producer.seq,
            });
        }
        let offset = self.mint();
        self.accepted.entry(key).or_default().insert(producer.seq);
        self.offsets.insert(triple, offset.clone());
        self.records.push(Record {
            offset: offset.clone(),
            producer,
            message,
        });
        Ok(offset)
    }

    fn read(&self, bound: &ReadBound, limit: usize) -> Result<Vec<Record>, LogError> {
        let from = match bound {
            ReadBound::Start => None,
            ReadBound::Tail => return Ok(Vec::new()),
            ReadBound::After(offset) => {
                self.check_offset(offset)?;
                Some(offset.clone())
            }
        };
        let mut out = Vec::new();
        for record in &self.records {
            let after = match &from {
                None => true,
                Some(after) => record.offset > *after,
            };
            if after {
                out.push(record.clone());
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    fn latest(&self) -> Option<Offset> {
        self.records.last().map(|r| r.offset.clone())
    }
}
