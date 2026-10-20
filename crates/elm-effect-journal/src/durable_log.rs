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

use std::collections::HashMap;
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
}

/// A durable log: atomic appends, validated reads.
pub trait DurableLog {
    /// Append one JSON envelope, returning its minted offset.
    fn append_json(&self, envelope: &str) -> Result<Offset, AppendError>;
    /// Read every message after `from`. Hand-built offsets are rejected.
    fn read_from(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError>;
    /// Drop every record before `before` (a minted offset). Returns how many
    /// records were dropped. The caller must hold a snapshot that covers the
    /// dropped records, or recovery cannot rebuild the model.
    fn compact_before(&self, before: &Offset) -> Result<usize, AppendError>;
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
        if let Some(&current) = self.max_epoch.get(&id) {
            if epoch < current {
                return Err(AppendError::StaleEpoch {
                    producer: id,
                    sent: epoch,
                    current,
                });
            }
        }
        if let Some((recorded_offset, recorded)) = self.offsets.get(&(id.clone(), epoch, seq)) {
            if *recorded != message {
                return Err(AppendError::PayloadMismatch {
                    producer_id: id,
                    epoch,
                    seq,
                });
            }
            // Idempotent replay: same triple and same bytes, replies the offset.
            return Ok(Admit::Duplicate(*recorded_offset));
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

    /// The fencing state as JSON: enough to keep sequence and epoch checks
    /// correct after the records behind them are compacted away.
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
        serde_json::json!({ "next": self.next, "last_seq": last_seq, "max_epoch": max_epoch })
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
        Some(())
    }

    /// Validate a presented offset: `None` reads from the start, `Some(max)`
    /// reads nothing, and `Some(seq)` reads strictly after `seq`.
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

    fn compact_before(&self, before: &Offset) -> Result<usize, AppendError> {
        let mut store = self.store.lock().unwrap();
        let limit = store.compact_limit(before)?;
        Ok(store.compact_to(limit))
    }
}

impl FileLog {
    /// Open the log at `path`, creating it if absent. Each stored record goes
    /// through the same admission as a live append, so the fencing state comes
    /// back exactly. A record that is inconsistent with what came before fails
    /// the open and is never skipped. A torn final line (no newline, so its
    /// append never returned `Ok`) is cut off.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, AppendError> {
        let path = path.into();
        let mut store = Store::default();
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
                    let offset = record["offset"]
                        .as_str()
                        .and_then(|s| s.parse::<u64>().ok())
                        .ok_or_else(|| at("missing offset"))?;
                    let envelope = record["envelope"]
                        .as_str()
                        .ok_or_else(|| at("missing envelope"))?;
                    if record["replayed"].as_bool() == Some(true) {
                        store.restore_entry(envelope, offset)?;
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

    fn compact_before(&self, before: &Offset) -> Result<usize, AppendError> {
        let mut store = self.store.lock().unwrap();
        let limit = store.compact_limit(before)?;
        // Rewrite the file before touching memory: if the rewrite fails, the
        // file and memory still agree.
        let kept: Vec<&Stored> = store.entries.iter().filter(|e| e.offset >= limit).collect();
        rewrite(&self.path, &store.checkpoint(), &kept)?;
        Ok(store.compact_to(limit))
    }
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
