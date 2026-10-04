//! Stream lifecycle (ADR-0002 Phase 9b, Law L16 Gone/close part).
//!
//! A stream is [`UpToDate`](Status::UpToDate) while caught up (more bytes
//! may arrive) and [`Closed`](Status::Closed) at EOF. Closure is monotonic:
//! appends after close fail, bytes before close stay readable. An offset
//! older than retention is [`Gone::Retention`] carrying the first
//! replayable offset; a soft-deleted stream is [`Gone::SoftDeleted`, a
//! DIFFERENT reason callers must match separately (a retention skip is
//! never a delete). A [`Snapshot`] is the `{model, offset}` pair stored
//! beside the stream. `collapsing_cursor` is deliberately absent: it is
//! the HTTP CDN fan-in token only and is never stored as a fold position;
//! the resume field is `offset`.

/// Liveness of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Caught up; more bytes may arrive.
    UpToDate,
    /// End of file; monotonic, never reopens.
    Closed,
}

/// Why a read is gone. Two reasons, never conflated.
#[derive(Clone, Debug, PartialEq)]
pub enum Gone {
    /// Offset fell below retention; resume here instead.
    Retention {
        /// First replayable offset.
        first_replayable: u64,
    },
    /// The stream was soft-deleted.
    SoftDeleted,
}

/// Read outcome.
#[derive(Clone, Debug, PartialEq)]
pub enum Read {
    /// Bytes at this offset.
    Entry {
        /// Offset read.
        offset: u64,
        /// Stored bytes.
        bytes: Vec<u8>,
    },
    /// Gone, with the reason.
    Gone(Gone),
}

/// Snapshot record stored beside the stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// Opaque model bytes.
    pub model: Vec<u8>,
    /// Log offset the snapshot includes.
    pub offset: u64,
}

/// Append/read failure.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamError {
    /// Appends after close fail.
    Closed,
    /// Appends to a soft-deleted stream fail.
    Deleted,
}

/// Byte stream with close, retention, soft delete, and snapshots.
#[derive(Clone, Debug, Default)]
pub struct Stream {
    entries: Vec<Vec<u8>>,
    closed: bool,
    deleted: bool,
    retention: usize,
    base: u64,
    snapshot: Option<Snapshot>,
}

impl Stream {
    /// Open stream retaining the last `retention` entries.
    pub fn new(retention: usize) -> Self {
        Self {
            entries: Vec::new(),
            closed: false,
            deleted: false,
            retention: retention.max(1),
            base: 0,
            snapshot: None,
        }
    }

    /// Liveness.
    pub fn status(&self) -> Status {
        if self.closed {
            Status::Closed
        } else {
            Status::UpToDate
        }
    }

    /// Append bytes; fails after close or delete.
    pub fn append(&mut self, bytes: Vec<u8>) -> Result<u64, StreamError> {
        if self.closed {
            return Err(StreamError::Closed);
        }
        if self.deleted {
            return Err(StreamError::Deleted);
        }
        let offset = self.base + self.entries.len() as u64;
        self.entries.push(bytes);
        while self.entries.len() > self.retention {
            self.entries.remove(0);
            self.base += 1;
        }
        Ok(offset)
    }

    /// Read one offset.
    pub fn read(&self, offset: u64) -> Read {
        if self.deleted {
            return Read::Gone(Gone::SoftDeleted);
        }
        if offset < self.base {
            return Read::Gone(Gone::Retention { first_replayable: self.base });
        }
        match self.entries.get((offset - self.base) as usize) {
            Some(bytes) => Read::Entry { offset, bytes: bytes.clone() },
            None => Read::Gone(Gone::Retention { first_replayable: self.base }),
        }
    }

    /// Close forever: monotonic, idempotent.
    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Soft-delete: every read reports [`Gone::SoftDeleted`].
    pub fn soft_delete(&mut self) {
        self.deleted = true;
    }

    /// Store a snapshot beside the stream.
    pub fn save_snapshot(&mut self, model: Vec<u8>, offset: u64) {
        self.snapshot = Some(Snapshot { model, offset });
    }

    /// Stored snapshot, if any.
    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }
}
