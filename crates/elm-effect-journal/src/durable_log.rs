//! Law L16 (append part): a durable log is a trait with two kernels.
//! [`MemLog`] runs in-process. [`FileLog`] keeps the same records in a
//! JSON-lines file, so they survive a restart.
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
//!
//! Both kernels admit an envelope through the same [`Store::admit`]. A
//! [`FileLog`] reopened from disk replays its lines through that same path,
//! so fencing, sequence, and dedupe state come back exactly as they were.
//! A producer's epoch only moves forward: an append whose epoch is below the
//! producer's current epoch is fenced with [`AppendError::StaleEpoch`] and
//! writes nothing, even when it replays a triple the log accepted before the
//! epoch moved (fencing beats dedupe). A higher epoch starts its sequence
//! at 0. Epoch state never expires.
//!
//! Closure follows the Durable Streams Protocol: it is durable, monotonic,
//! and idempotent. An appends-after-close answer is [`AppendError::Closed`];
//! every byte written before the close stays readable. [`DurableLog::status`]
//! reports the same [`Status`] the stream model uses, so a reader can tell
//! "caught up, more may arrive" from "no more data ever". A closing append
//! is one operation: the bytes and the closed flag share a line, so a crash
//! leaves both or neither. Retrying the exact closing triple returns its
//! original offset without appending again; a replay of an earlier triple on
//! a closed stream is refused.
//!
//! [`DurableLog::read_page`] is the catch-up read an HTTP client needs: a
//! bounded slice of records plus a resume cursor that is valid even when the
//! page is empty. The kernel resolves `Offset::Now` to a concrete tail token,
//! so a live tail can start at "now" and still see the next append. A page
//! carries `up_to_date` (caught up, more may arrive) separately from `closed`
//! (EOF), the same split the stream header uses.

use std::collections::HashMap;

use crate::stream::Status;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Schema versions this log reads.
pub const SCHEMA_VERSION: u32 = 1;

/// An offset: opaque to callers, ordered lexicographically. `Start` reads
/// from the beginning, `Now` reads nothing (already at the tail).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Offset {
    /// Read from the beginning. Never minted.
    Start,
    /// Already at the tail: `read_from` returns nothing. `read_page` resolves
    /// it to the last existing record so a live reader catches the next
    /// append. Never minted.
    Now,
    /// A minted offset. Opaque: build only via append.
    At(String),
}

/// Typed append/read failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, serde::Serialize, serde::Deserialize)]
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
    /// The producer already moved to a newer epoch; this write is fenced.
    /// A stale incarnation (a zombie after a restart) writes nothing.
    #[error("stale epoch for producer {producer}: sent {sent}, current {current}")]
    StaleEpoch {
        /// Producer that sent it.
        producer: String,
        /// Epoch sent.
        sent: u64,
        /// The producer's current epoch.
        current: u64,
    },
    /// The same `(producer_id, epoch, seq)` was accepted before with a
    /// different payload. A retry must resend the same bytes.
    #[error("payload mismatch for {producer_id} epoch {epoch} seq {seq}")]
    PayloadMismatch {
        /// Producer that sent the write.
        producer_id: String,
        /// Epoch of the triple.
        epoch: u64,
        /// Seq of the triple.
        seq: u64,
    },
    /// The offset was not minted by this log.
    #[error("unknown offset")]
    UnknownOffset,
    /// The file behind a [`FileLog`] could not be read, written, or synced,
    /// or its contents are not a consistent log.
    #[error("storage failed: {0}")]
    Storage(String),
    /// The log could not be reached or did not answer (a store-backed log
    /// whose backing store failed, or whose bridge was called from the wrong
    /// thread). Unlike every other variant this is **not** a verdict on the
    /// envelope: the append's outcome is unknown, so a caller must reconcile
    /// (ask the log whether it holds the triple) before retrying. `MemLog`
    /// never returns it.
    #[error("log unavailable (outcome unknown, reconcile): {0}")]
    Unavailable(String),
    /// The stream is closed: no further appends are accepted. Bytes already
    /// written stay readable. This is a verdict on the log, not on the
    /// envelope, so only a retry of the exact closing triple can still be
    /// answered (with its original offset).
    #[error("stream is closed")]
    Closed,
}

/// One catch-up page: records, a resume cursor, and the two end signals.
///
/// `next` is the cursor to pass to [`DurableLog::read_page`] for the records
/// after this page: the offset of the last record returned, because a read is
/// strictly after its cursor. It is a concrete token even when the page is
/// empty (`Offset::Now` resolves to the last existing offset, or `Start` on an
/// empty log), so a live reader does not spin on a sentinel. `up_to_date`
/// means the page reached the tail at generation time; more records may still
/// arrive. `closed` is EOF: present only once the page has reached the tail of
/// a closed stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    /// Records in offset order, at most the requested limit.
    pub records: Vec<(Offset, String)>,
    /// Resume cursor for the next page.
    pub next: Offset,
    /// True when this page reached the log's current tail.
    pub up_to_date: bool,
    /// True when this page reached the tail of a closed stream (EOF).
    pub closed: bool,
}

/// A durable log: atomic appends, validated reads, explicit closure.
pub trait DurableLog: Send + Sync {
    /// Append one JSON envelope, returning its minted offset.
    fn append_json(&self, envelope: &str) -> Result<Offset, AppendError>;
    /// Read every message after `from`. Hand-built offsets are rejected.
    fn read_from(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError>;
    /// Read one bounded catch-up page after `from`. At least one record is
    /// admitted per call, even for `limit == 0`, so a page can never skip the
    /// log. The default implementation is correct but unbounded: it reads
    /// every record and truncates. [`MemLog`] and [`FileLog`] override it with
    /// a bounded read.
    fn read_page(&self, from: &Offset, limit: usize) -> Result<Page, AppendError> {
        let limit = limit.max(1);
        // `Now` has no record to name, so the default has to scan from the
        // start to find the last cursor. Kernels write this directly.
        let (mut records, cursor) = if matches!(from, Offset::Now) {
            let all = self.read_from(&Offset::Start)?;
            let cursor = all
                .last()
                .map_or(Offset::Start, |(offset, _)| offset.clone());
            (Vec::new(), cursor)
        } else {
            (self.read_from(from)?, from.clone())
        };
        let up_to_date = records.len() <= limit;
        records.truncate(limit);
        let next = records.last().map_or(cursor, |(offset, _)| offset.clone());
        Ok(Page {
            records,
            next,
            up_to_date,
            closed: up_to_date && self.is_closed(),
        })
    }
    /// Drop every record before `before` (a minted offset). Returns how many
    /// records were dropped. The caller must hold a snapshot that covers the
    /// dropped records, or recovery cannot rebuild the model.
    fn compact_before(&self, before: &Offset) -> Result<usize, AppendError>;
    /// Close the stream: durable, monotonic, idempotent. After this the
    /// status is [`Status::Closed`] and [`append_json`](Self::append_json)
    /// fails with [`AppendError::Closed`]; the records written before the
    /// close stay readable forever. Closing an already-closed log succeeds
    /// and changes nothing.
    fn close(&self) -> Result<(), AppendError>;
    /// Append the final message and close in one operation. The bytes and
    /// the closed flag commit together, so a crash leaves both or neither.
    /// Retrying the exact closing `(producer_id, epoch, seq)` triple returns
    /// the original offset without a duplicate append.
    fn append_json_and_close(&self, envelope: &str) -> Result<Offset, AppendError>;
    /// Liveness: [`Status::UpToDate`] while open, [`Status::Closed`] at EOF.
    /// The same enum the stream model uses, so a caller reads one vocabulary.
    fn status(&self) -> Status;
    /// True once closed. Bytes stay readable; only new appends are refused.
    fn is_closed(&self) -> bool {
        self.status() == Status::Closed
    }
}

/// In-process log: one lock, one record vector. Atomicity falls out of the
/// lock discipline (a crash between appends leaves the previous offset);
/// reads clone complete messages, never partial ones. Nothing survives the
/// process; use [`FileLog`] for that.
#[derive(Debug, Default)]
pub struct MemLog {
    store: Mutex<Store>,
}

impl MemLog {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }
}

/// File-backed log. Every append is written and synced before it becomes
/// visible, so a returned `Ok` means the record is on disk. A crash mid-write
/// leaves a torn tail, which [`FileLog::open`] discards, since that append
/// never returned `Ok`.
///
/// ponytail: one file and one lock; appends fsync each record. Batch the sync
/// if append throughput ever matters.
#[derive(Debug)]
pub struct FileLog {
    store: Mutex<Store>,
    path: PathBuf,
}

/// Admitted state: the records and the fencing tables derived from them.
#[derive(Debug, Default)]
struct Store {
    entries: Vec<Stored>,
    /// Last accepted seq per `(producer_id, epoch)`.
    last_seq: HashMap<(String, u64), u64>,
    /// Newest epoch accepted per producer. Lower epochs are fenced.
    max_epoch: HashMap<String, u64>,
    /// Offset and accepted payload per accepted triple. The payload lets a
    /// replay with the same triple be checked, not trusted blindly.
    offsets: HashMap<(String, u64, u64), (u64, String)>,
    next: u64,
    /// Once true, no append is admitted. Persisted by `FileLog`; `MemLog`
    /// keeps it in process.
    closed: bool,
    /// The triple of the append that closed the stream, when the close was
    /// an append-and-close rather than a bare close. Only this triple is
    /// still deduplicated on a closed stream.
    closed_by: Option<(String, u64, u64)>,
}

#[derive(Debug, Clone)]
struct Stored {
    offset: u64,
    message: String,
    envelope: String,
}

/// An envelope that passed every fencing rule, ready to commit.
struct Accepted {
    id: String,
    epoch: u64,
    seq: u64,
    offset: u64,
    message: String,
    envelope: String,
}

enum Admit {
    Fresh(Accepted),
    /// Exact triple seen before: the original offset, no duplicate.
    Duplicate(u64),
}

/// Mint an offset: zero-padded, lexicographically ordered.
fn mint(offset: u64) -> String {
    format!("{offset:020}")
}

fn storage(error: std::io::Error) -> AppendError {
    AppendError::Storage(error.to_string())
}

/// Parse an envelope into its producer identity and payload, checking the
/// schema version. Parsing does not check fencing.
fn parse(envelope: &str) -> Result<(String, u64, u64, String), AppendError> {
    let value: serde_json::Value =
        serde_json::from_str(envelope).map_err(|e| AppendError::NotAnEnvelope(e.to_string()))?;
    let version = value
        .get("v")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| AppendError::NotAnEnvelope("missing v".into()))?;
    if version != u64::from(SCHEMA_VERSION) {
        return Err(AppendError::UnknownVersion(version as u32));
    }
    let (id, epoch, seq) = match value.get("producer") {
        None => return Err(AppendError::PartialIdentity),
        Some(p) => {
            // Null producer parts are partial too: `as_str`/`as_u64` return
            // `None` for nulls.
            let id = p.get("producer_id").and_then(serde_json::Value::as_str);
            let epoch = p.get("epoch").and_then(serde_json::Value::as_u64);
            let seq = p.get("seq").and_then(serde_json::Value::as_u64);
            match (id, epoch, seq) {
                (Some(id), Some(epoch), Some(seq)) => (id.to_string(), epoch, seq),
                _ => return Err(AppendError::PartialIdentity),
            }
        }
    };
    let message = value
        .get("payload")
        .map(|v| v.to_string())
        .unwrap_or_default();
    Ok((id, epoch, seq, message))
}

/// One line of a [`FileLog`] file: the offset and the envelope exactly as
/// the caller sent it.
fn line(offset: u64, envelope: &str) -> String {
    serde_json::json!({ "offset": mint(offset), "envelope": envelope }).to_string()
}

/// A kept record rewritten by compaction. Replay restores it without the
/// fence checks: the checkpoint line already holds the fencing state.
fn replayed_line(offset: u64, envelope: &str) -> String {
    serde_json::json!({ "offset": mint(offset), "envelope": envelope, "replayed": true })
        .to_string()
}

impl Store {
    /// Check an envelope against the fencing rules without changing state.
    /// `offset` is the offset to give it if it is fresh: the next one for a
    /// live append, or the stored one when replaying a file.
    fn admit(&self, envelope: &str, offset: u64) -> Result<Admit, AppendError> {
        let (id, epoch, seq, message) = parse(envelope)?;
        // A producer that already reached a newer epoch cannot write under an
        // older one, even after a restart (L16: epoch state never expires).
        if let Some(&current) = self.max_epoch.get(&id)
            && epoch < current
        {
            return Err(AppendError::StaleEpoch {
                producer: id,
                sent: epoch,
                current,
            });
        }
        let triple = (id.clone(), epoch, seq);
        if let Some((recorded_offset, recorded)) = self.offsets.get(&triple) {
            if *recorded != message {
                return Err(AppendError::PayloadMismatch {
                    producer_id: id,
                    epoch,
                    seq,
                });
            }
            // A closed stream dedupes only the append that closed it. A
            // retry of an earlier triple is refused like any other append.
            if self.closed && self.closed_by.as_ref() != Some(&triple) {
                return Err(AppendError::Closed);
            }
            // Idempotent replay: same triple and same bytes, replies the offset.
            return Ok(Admit::Duplicate(*recorded_offset));
        }
        if self.closed {
            return Err(AppendError::Closed);
        }
        let last = self.last_seq.get(&(id.clone(), epoch)).copied();
        let expected = last.map_or(0, |l| l + 1);
        if seq != expected {
            return Err(AppendError::SeqGap { got: seq, last });
        }
        Ok(Admit::Fresh(Accepted {
            id,
            epoch,
            seq,
            offset,
            message,
            envelope: envelope.to_string(),
        }))
    }

    /// Commit an admitted envelope. Only call this after [`Store::admit`]
    /// accepted it and any durable write succeeded.
    fn apply(&mut self, accepted: Accepted) {
        let Accepted {
            id,
            epoch,
            seq,
            offset,
            message,
            envelope,
        } = accepted;
        self.next = self.next.max(offset + 1);
        self.last_seq.insert((id.clone(), epoch), seq);
        self.max_epoch
            .entry(id.clone())
            .and_modify(|current| *current = (*current).max(epoch))
            .or_insert(epoch);
        self.offsets
            .insert((id, epoch, seq), (offset, message.clone()));
        self.entries.push(Stored {
            offset,
            message,
            envelope,
        });
    }

    /// Restore a record a compaction kept. Its fencing effect is already in
    /// the checkpoint, so only the record and its dedupe entry are restored.
    fn restore_entry(&mut self, envelope: &str, offset: u64) -> Result<(), AppendError> {
        let (id, epoch, seq, message) = parse(envelope)?;
        self.next = self.next.max(offset + 1);
        self.offsets
            .insert((id, epoch, seq), (offset, message.clone()));
        self.entries.push(Stored {
            offset,
            message,
            envelope: envelope.to_string(),
        });
        Ok(())
    }

    /// Mark the stream closed without touching the records. Idempotent, and
    /// never clears who closed it.
    fn mark_closed(&mut self, by: Option<(String, u64, u64)>) {
        self.closed = true;
        if by.is_some() {
            self.closed_by = by;
        }
    }

    /// Append-and-close under an already-held lock. One admission, one
    /// commit: the caller wrote nothing until this returns, so a closed
    /// state can never exist without its final record (or vice versa).
    fn append_and_close(&mut self, envelope: &str) -> Result<u64, AppendError> {
        let (id, epoch, seq, _) = parse(envelope)?;
        let next = self.next;
        let offset = match self.admit(envelope, next)? {
            Admit::Duplicate(offset) => {
                // The final record was already accepted: close on it, and let
                // a retry of this same triple dedupe instead of hitting the
                // closed wall.
                self.mark_closed(Some((id, epoch, seq)));
                offset
            }
            Admit::Fresh(accepted) => {
                let offset = accepted.offset;
                let by = (accepted.id.clone(), accepted.epoch, accepted.seq);
                self.apply(accepted);
                self.mark_closed(Some(by));
                offset
            }
        };
        Ok(offset)
    }

    /// The fencing state as JSON: enough to keep sequence and epoch checks
    /// correct after the records behind them are compacted away. Closure is
    /// part of that state: a reopened log must not accept a late append.
    fn checkpoint(&self) -> serde_json::Value {
        let last_seq: Vec<_> = self
            .last_seq
            .iter()
            .map(|((id, epoch), seq)| serde_json::json!([id, epoch, seq]))
            .collect();
        let max_epoch: Vec<_> = self
            .max_epoch
            .iter()
            .map(|(id, epoch)| serde_json::json!([id, epoch]))
            .collect();
        let closed_by = self
            .closed_by
            .as_ref()
            .map(|(id, epoch, seq)| serde_json::json!([id, epoch, seq]));
        serde_json::json!({
            "next": self.next,
            "last_seq": last_seq,
            "max_epoch": max_epoch,
            "closed": self.closed,
            "closed_by": closed_by,
        })
    }

    fn restore_checkpoint(&mut self, value: &serde_json::Value) -> Option<()> {
        self.next = value["next"].as_u64()?;
        for item in value["last_seq"].as_array()? {
            let id = item[0].as_str()?.to_string();
            self.last_seq
                .insert((id, item[1].as_u64()?), item[2].as_u64()?);
        }
        for item in value["max_epoch"].as_array()? {
            self.max_epoch
                .insert(item[0].as_str()?.to_string(), item[1].as_u64()?);
        }
        self.closed = value["closed"].as_bool().unwrap_or(false);
        // A present-but-malformed `closed_by` fails the open rather than
        // degrading to "no closing triple": the checkpoint claims to know
        // which append sealed the stream, and silently forgetting it would
        // turn an idempotent retry into a Closed refusal.
        self.closed_by = if value["closed_by"].is_null() {
            None
        } else {
            let item = value["closed_by"].as_array()?;
            Some((
                item.first()?.as_str()?.to_string(),
                item.get(1)?.as_u64()?,
                item.get(2)?.as_u64()?,
            ))
        };
        Some(())
    }

    /// Validate a presented offset: `None` reads from the start, `Some(max)`
    /// reads nothing, and `Some(seq)` reads strictly after `seq`. Only a
    /// minted offset (or `Start`/`Now`) is accepted; the tail itself is not a
    /// cursor, because reading after it would skip the next append.
    fn check(&self, offset: &Offset) -> Result<Option<u64>, AppendError> {
        match offset {
            Offset::Start => Ok(None),
            Offset::Now => Ok(Some(u64::MAX)),
            Offset::At(raw) => {
                let seq: u64 = raw.parse().map_err(|_| AppendError::UnknownOffset)?;
                if seq < self.next {
                    Ok(Some(seq))
                } else {
                    Err(AppendError::UnknownOffset)
                }
            }
        }
    }

    /// The cursor that means "caught up": the last existing record, or
    /// `Start` when the log has no records. Reading after it is empty now and
    /// includes the next append, which is what a live tail needs.
    fn last_token(&self) -> Offset {
        if self.next == 0 {
            Offset::Start
        } else {
            Offset::At(mint(self.next - 1))
        }
    }

    fn read(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError> {
        let after = self.check(from)?;
        if after == Some(u64::MAX) {
            return Ok(Vec::new());
        }
        Ok(self
            .entries
            .iter()
            .filter(|e| after.is_none_or(|limit| e.offset > limit))
            .map(|e| (Offset::At(mint(e.offset)), e.message.clone()))
            .collect())
    }

    /// Bounded catch-up read with a resolved tail cursor. `Offset::Now` maps
    /// to the last existing offset (or `Start` on an empty log), so a live
    /// reader catches the next append instead of skipping it. The returned
    /// `next` is always a record offset or `Start`, never the unminted tail.
    fn page(&self, from: &Offset, limit: usize) -> Result<Page, AppendError> {
        let limit = limit.max(1);
        let cursor = match from {
            Offset::Start => Offset::Start,
            Offset::Now => self.last_token(),
            Offset::At(raw) => {
                let seq: u64 = raw.parse().map_err(|_| AppendError::UnknownOffset)?;
                if seq >= self.next {
                    return Err(AppendError::UnknownOffset);
                }
                Offset::At(mint(seq))
            }
        };
        let after = match &cursor {
            Offset::Start => None,
            Offset::At(raw) => Some(raw.parse::<u64>().map_err(|_| AppendError::UnknownOffset)?),
            Offset::Now => unreachable!("resolved above"),
        };
        let start = after.map_or(0, |after| {
            self.entries.partition_point(|entry| entry.offset <= after)
        });
        let available = &self.entries[start..];
        let take = available.len().min(limit);
        let records = available[..take]
            .iter()
            .map(|entry| (Offset::At(mint(entry.offset)), entry.message.clone()))
            .collect();
        let up_to_date = take == available.len();
        let next = if take == 0 {
            cursor
        } else {
            Offset::At(mint(available[take - 1].offset))
        };
        Ok(Page {
            records,
            next,
            up_to_date,
            closed: up_to_date && self.closed,
        })
    }

    /// The numeric offset a compaction keeps from; validates `before`.
    fn compact_limit(&self, before: &Offset) -> Result<u64, AppendError> {
        let Offset::At(raw) = before else {
            return Err(AppendError::UnknownOffset);
        };
        let limit: u64 = raw.parse().map_err(|_| AppendError::UnknownOffset)?;
        if limit >= self.next {
            return Err(AppendError::UnknownOffset);
        }
        Ok(limit)
    }

    /// Drop every record below `limit`. Returns how many were dropped. The
    /// fencing maps stay: they are what keeps later sequence checks correct.
    fn compact_to(&mut self, limit: u64) -> usize {
        let before_len = self.entries.len();
        self.entries.retain(|e| e.offset >= limit);
        // Dedupe entries for dropped records go too. A retry of one of them
        // now fails the seq check instead of replaying, which is the right
        // outcome: a producer never retries past its own snapshot.
        self.offsets.retain(|_, (offset, _)| *offset >= limit);
        before_len - self.entries.len()
    }
}

impl DurableLog for MemLog {
    fn append_json(&self, envelope: &str) -> Result<Offset, AppendError> {
        let mut store = self.store.lock().unwrap();
        let next = store.next;
        let offset = match store.admit(envelope, next)? {
            Admit::Duplicate(offset) => offset,
            Admit::Fresh(accepted) => {
                let offset = accepted.offset;
                store.apply(accepted);
                offset
            }
        };
        Ok(Offset::At(mint(offset)))
    }

    fn read_from(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError> {
        self.store.lock().unwrap().read(from)
    }

    fn read_page(&self, from: &Offset, limit: usize) -> Result<Page, AppendError> {
        self.store.lock().unwrap().page(from, limit)
    }

    fn compact_before(&self, before: &Offset) -> Result<usize, AppendError> {
        let mut store = self.store.lock().unwrap();
        let limit = store.compact_limit(before)?;
        Ok(store.compact_to(limit))
    }

    fn close(&self) -> Result<(), AppendError> {
        self.store.lock().unwrap().mark_closed(None);
        Ok(())
    }

    fn append_json_and_close(&self, envelope: &str) -> Result<Offset, AppendError> {
        let offset = self.store.lock().unwrap().append_and_close(envelope)?;
        Ok(Offset::At(mint(offset)))
    }

    fn status(&self) -> Status {
        status_of(&self.store.lock().unwrap())
    }
}

impl FileLog {
    /// Open the log at `path`, creating it if absent. Each stored record goes
    /// through the same admission as a live append, so the fencing state comes
    /// back exactly. A record that is inconsistent with what came before fails
    /// the open and is never skipped. A torn final line (no newline, so its
    /// append never returned `Ok`) is cut off.
    ///
    /// Closure survives too: a sealed line carries both the final record and
    /// the closed flag, and a bare close line marks EOF. A fresh record after
    /// either is corruption, not a late append to accept.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, AppendError> {
        let path = path.into();
        let mut store = Store::default();
        // A bare close line (no envelope) closes the file from that point on.
        // A checkpoint may also carry `closed`, from a compaction after a
        // close; `replayed` records after it are the kept history, not new
        // appends, so they restore rather than fail.
        let mut closed_marker = false;
        match fs::read_to_string(&path) {
            Ok(text) => {
                let complete = text.rfind('\n').map_or(0, |i| i + 1);
                for (index, line) in text[..complete].lines().enumerate() {
                    let at = |why: &str| {
                        AppendError::Storage(format!(
                            "{}: line {}: {why}",
                            path.display(),
                            index + 1
                        ))
                    };
                    let record: serde_json::Value =
                        serde_json::from_str(line).map_err(|e| at(&e.to_string()))?;
                    if !record["checkpoint"].is_null() {
                        store
                            .restore_checkpoint(&record["checkpoint"])
                            .ok_or_else(|| at("malformed checkpoint"))?;
                        continue;
                    }
                    let sealed = record["closed"].as_bool() == Some(true);
                    let Some(envelope) = record["envelope"].as_str() else {
                        if sealed {
                            // A close with no final append. Repeating it is
                            // harmless; a record after it is not.
                            store.mark_closed(None);
                            closed_marker = true;
                            continue;
                        }
                        return Err(at("missing envelope"));
                    };
                    let offset = record["offset"]
                        .as_str()
                        .and_then(|s| s.parse::<u64>().ok())
                        .ok_or_else(|| at("missing offset"))?;
                    if record["replayed"].as_bool() == Some(true) {
                        if closed_marker && !sealed {
                            return Err(at("record after close"));
                        }
                        store.restore_entry(envelope, offset)?;
                        if sealed {
                            store.mark_closed(parse(envelope).ok().map(|(id, e, s, _)| (id, e, s)));
                        }
                        continue;
                    }
                    if closed_marker {
                        return Err(at("record after close"));
                    }
                    if sealed {
                        let by = parse(envelope)
                            .map(|(id, epoch, seq, _)| (id, epoch, seq))
                            .map_err(|e| at(&format!("sealed record: {e}")))?;
                        match store.admit(envelope, offset)? {
                            Admit::Fresh(accepted) => store.apply(accepted),
                            Admit::Duplicate(_) => return Err(at("duplicate record")),
                        }
                        store.mark_closed(Some(by));
                        closed_marker = true;
                        continue;
                    }
                    match store.admit(envelope, offset)? {
                        Admit::Fresh(accepted) => store.apply(accepted),
                        Admit::Duplicate(_) => return Err(at("duplicate record")),
                    }
                }
                if complete < text.len() {
                    let file = OpenOptions::new()
                        .write(true)
                        .open(&path)
                        .map_err(storage)?;
                    file.set_len(complete as u64).map_err(storage)?;
                    file.sync_data().map_err(storage)?;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(storage(e)),
        }
        Ok(Self {
            store: Mutex::new(store),
            path,
        })
    }
}

impl DurableLog for FileLog {
    fn append_json(&self, envelope: &str) -> Result<Offset, AppendError> {
        let mut store = self.store.lock().unwrap();
        let next = store.next;
        let offset = match store.admit(envelope, next)? {
            Admit::Duplicate(offset) => offset,
            Admit::Fresh(accepted) => {
                // Durable before visible: the line reaches the disk before the
                // in-memory state moves. A failed write leaves no trace.
                let offset = accepted.offset;
                append_line(&self.path, &line(offset, envelope))?;
                store.apply(accepted);
                offset
            }
        };
        Ok(Offset::At(mint(offset)))
    }

    fn read_from(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError> {
        self.store.lock().unwrap().read(from)
    }

    fn read_page(&self, from: &Offset, limit: usize) -> Result<Page, AppendError> {
        self.store.lock().unwrap().page(from, limit)
    }

    fn compact_before(&self, before: &Offset) -> Result<usize, AppendError> {
        let mut store = self.store.lock().unwrap();
        let limit = store.compact_limit(before)?;
        // Rewrite the file before touching memory: if the rewrite fails, the
        // file and memory still agree.
        let kept: Vec<&Stored> = store.entries.iter().filter(|e| e.offset >= limit).collect();
        rewrite(&self.path, &store.checkpoint(), &kept)?;
        Ok(store.compact_to(limit))
    }

    fn close(&self) -> Result<(), AppendError> {
        let mut store = self.store.lock().unwrap();
        if store.closed {
            return Ok(());
        }
        // The marker is durable before the state moves: a returned Ok means a
        // reopen sees the close, and a failed write leaves the log open.
        append_line(
            &self.path,
            &serde_json::json!({ "closed": true }).to_string(),
        )?;
        store.mark_closed(None);
        Ok(())
    }

    fn append_json_and_close(&self, envelope: &str) -> Result<Offset, AppendError> {
        let mut store = self.store.lock().unwrap();
        let (id, epoch, seq, _) = parse(envelope)?;
        let next = store.next;
        let offset = match store.admit(envelope, next)? {
            // The final record already exists: close on it. A bare close
            // marker is the durable half; the triple stays the stream's
            // closing identity so a retry still dedupes.
            Admit::Duplicate(offset) => {
                if !store.closed {
                    append_line(
                        &self.path,
                        &serde_json::json!({ "closed": true }).to_string(),
                    )?;
                }
                offset
            }
            Admit::Fresh(accepted) => {
                let offset = accepted.offset;
                let by = (accepted.id.clone(), accepted.epoch, accepted.seq);
                // One line holds the final record and the closed flag, so the
                // pair is atomic on reopen.
                append_line(&self.path, &sealed_line(offset, envelope, &by))?;
                store.apply(accepted);
                offset
            }
        };
        store.mark_closed(Some((id, epoch, seq)));
        Ok(Offset::At(mint(offset)))
    }

    fn status(&self) -> Status {
        status_of(&self.store.lock().unwrap())
    }
}

fn status_of(store: &Store) -> Status {
    if store.closed {
        Status::Closed
    } else {
        Status::UpToDate
    }
}

/// One line for an append-and-close: the final record and the closed flag
/// commit together, so a crash cannot leave a closed stream without its
/// final bytes or vice versa.
fn sealed_line(offset: u64, envelope: &str, by: &(String, u64, u64)) -> String {
    let (producer, epoch, seq) = by;
    serde_json::json!({
        "offset": mint(offset),
        "envelope": envelope,
        "closed": true,
        "closed_by": [producer, epoch, seq],
    })
    .to_string()
}

fn append_line(path: &Path, line: &str) -> Result<(), AppendError> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(storage)?;
    writeln!(file, "{line}").map_err(storage)?;
    file.sync_data().map_err(storage)
}

/// Replace the file with a checkpoint line followed by the kept records.
/// Written to a sibling file, synced, then renamed over the original, so a
/// crash leaves either the old file or the new one, never a mix.
fn rewrite(
    path: &Path,
    checkpoint: &serde_json::Value,
    kept: &[&Stored],
) -> Result<(), AppendError> {
    let tmp = path.with_extension("compacting");
    {
        let mut file = File::create(&tmp).map_err(storage)?;
        writeln!(file, "{}", serde_json::json!({ "checkpoint": checkpoint })).map_err(storage)?;
        for entry in kept {
            writeln!(file, "{}", replayed_line(entry.offset, &entry.envelope)).map_err(storage)?;
        }
        file.sync_data().map_err(storage)?;
    }
    fs::rename(&tmp, path).map_err(storage)?;
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    File::open(dir).and_then(|d| d.sync_all()).map_err(storage)
}
