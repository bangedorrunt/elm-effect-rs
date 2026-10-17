//! Epoch fencing (ADR-0002 Phase 9b, Law L16 epoch part).
//!
//! Every append carries `(producer_id, epoch, seq)`. The same triple
//! appends once: repeats return the original offset without duplicating.
//! A lower epoch than the producer's current is fenced (rejected). A new,
//! higher epoch resets the sequence baseline: the epoch starts at
//! sequence 0. Epoch state does NOT expire: there is deliberately no TTL
//! (stricter than the draft, which proposed one). A [`fork`] inherits
//! bytes up to an offset but never the producer epochs: the fork's first
//! epoch is 0.

use std::collections::HashMap;

/// Rejection reason.
#[derive(Clone, Debug, PartialEq)]
pub enum Fence {
    /// Epoch below the producer's current epoch.
    StaleEpoch {
        /// Producer that sent it.
        producer: String,
        /// Epoch sent.
        sent: u64,
        /// Current epoch.
        current: u64,
    },
    /// Same epoch, sequence at or below the recorded one (and not the
    /// exact recorded triple, which dedupes instead).
    StaleSeq {
        /// Producer that sent it.
        producer: String,
        /// Sequence sent.
        sent: u64,
        /// Last recorded sequence.
        current: u64,
    },
}

/// Append outcome.
#[derive(Clone, Debug, PartialEq)]
pub enum Appended {
    /// New entry at this offset.
    Fresh {
        /// Assigned offset.
        offset: u64,
    },
    /// Exact triple seen before: original offset, no duplicate.
    Duplicate {
        /// Original offset.
        offset: u64,
    },
}

/// One log entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Assigned offset.
    pub offset: u64,
    /// Producer id.
    pub producer: String,
    /// Producer epoch.
    pub epoch: u64,
    /// Sequence within the epoch.
    pub seq: u64,
}

/// Epoch-fenced log. Clone shares nothing; [`fork`](EpochLog::fork)
/// shares bytes only.
#[derive(Clone, Debug, Default)]
pub struct EpochLog {
    entries: Vec<Entry>,
    /// Producer -> (current epoch, last seq, last offset).
    producers: HashMap<String, (u64, u64, u64)>,
}

impl EpochLog {
    /// Empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one entry under the fencing rules.
    pub fn append(&mut self, producer: &str, epoch: u64, seq: u64) -> Result<Appended, Fence> {
        if let Some((cur_epoch, last_seq, _)) = self.producers.get(producer).copied() {
            if epoch < cur_epoch {
                return Err(Fence::StaleEpoch {
                    producer: producer.to_string(),
                    sent: epoch,
                    current: cur_epoch,
                });
            }
            if epoch == cur_epoch {
                // Exact triple seen before: original offset, no duplicate.
                if let Some(entry) = self
                    .entries
                    .iter()
                    .rev()
                    .find(|e| e.producer == producer && e.epoch == epoch && e.seq == seq)
                {
                    return Ok(Appended::Duplicate {
                        offset: entry.offset,
                    });
                }
                if seq <= last_seq {
                    return Err(Fence::StaleSeq {
                        producer: producer.to_string(),
                        sent: seq,
                        current: last_seq,
                    });
                }
            }
            // Higher epoch: baseline resets; any sequence starts fresh.
        }
        let offset = self.entries.len() as u64;
        self.entries.push(Entry {
            offset,
            producer: producer.to_string(),
            epoch,
            seq,
        });
        self.producers
            .insert(producer.to_string(), (epoch, seq, offset));
        Ok(Appended::Fresh { offset })
    }

    /// Entries in offset order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Fork: bytes up to `offset` (exclusive end by count), producer
    /// epochs dropped. The fork's first epoch is 0.
    pub fn fork(&self, up_to_offset: u64) -> Self {
        let mut forked = Self::new();
        for entry in self.entries.iter().filter(|e| e.offset < up_to_offset) {
            forked.entries.push(entry.clone());
        }
        forked
    }

    /// Current epoch for a producer, if any.
    pub fn epoch_of(&self, producer: &str) -> Option<u64> {
        self.producers.get(producer).map(|(e, _, _)| *e)
    }
}
