use crate::configuration::Configuration;
use crate::types::StepOutcome;
use elm_effect_journal::durable_log::{AppendError, DurableLog, Offset, SCHEMA_VERSION};

/// Why a record chain could not be replayed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplayError {
    /// A record starts from a configuration the chain was not in.
    #[error("record {index} starts from {found:?}, expected {expected:?}")]
    Discontinuous {
        /// Position of the offending record.
        index: usize,
        /// The configuration the previous record ended in.
        expected: Vec<String>,
        /// The configuration this record claims to start from.
        found: Vec<String>,
    },
}

/// The producer triple an append is written under.
///
/// The statechart does not mint identity. A restarted host reuses its own
/// epoch and the next sequence number, so a retry stays gap-free and a zombie
/// producer is fenced (ADR-0007 law L5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionStamp<'a> {
    /// Stable writer id.
    pub producer_id: &'a str,
    /// The producer's epoch (a restart begins a new one).
    pub epoch: u64,
    /// The next sequence number to append.
    pub seq: u64,
}

/// One committed statechart transition, ready for a durable log.
///
/// The record names the configurations rather than holding them, because the
/// log stores JSON and a state key is not required to serialize. Record it
/// before the entry actions run (ADR-0007 law L5), so a crash replays the
/// transition rather than only part of its effects.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TransitionRecord {
    /// Active leaves before the step.
    pub from: Vec<String>,
    /// Active leaves after the step.
    pub to: Vec<String>,
    /// The event or timer that caused it.
    pub event: String,
    /// The producer generation that committed it.
    pub generation: u64,
    /// Serialized `MachineState` after this step, when the host can snapshot it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
}

impl TransitionRecord {
    /// Build a record from the configuration before and after a step.
    pub fn new<S: Ord + Clone + ToString>(
        from: &Configuration<S>,
        to: &Configuration<S>,
        event: impl Into<String>,
        generation: u64,
    ) -> Self {
        Self {
            from: from.leaves().map(ToString::to_string).collect(),
            to: to.leaves().map(ToString::to_string).collect(),
            event: event.into(),
            generation,
            snapshot: None,
        }
    }

    /// Attach a serialized run-state snapshot to this record.
    ///
    /// A host that can serialize its `MachineState` writes it here, so a
    /// restart resumes the exact configuration, history, and context instead
    /// of only the leaf names.
    #[must_use]
    pub fn with_snapshot(mut self, snapshot: impl Into<String>) -> Self {
        self.snapshot = Some(snapshot.into());
        self
    }

    /// The latest snapshot in a chain, if any record carries one.
    #[must_use]
    pub fn latest_snapshot(records: &[Self]) -> Option<&str> {
        records
            .iter()
            .rev()
            .find_map(|record| record.snapshot.as_deref())
    }

    /// Build a record from the configuration a step started in and its outcome.
    pub fn from_outcome<S: Ord + Clone + ToString, C, CMD, E>(
        before: &Configuration<S>,
        outcome: &StepOutcome<S, C, CMD, E>,
        event: impl Into<String>,
        generation: u64,
    ) -> Self {
        Self::new(before, &outcome.configuration, event, generation)
    }

    /// Replay a continuous record chain, returning the configuration after the
    /// last record.
    ///
    /// Replay applies records only: it never runs entry or exit actions, so
    /// side effects are not repeated. An empty chain returns the empty
    /// configuration; the caller starts from the machine's initial one.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayError::Discontinuous`] when a record starts from a
    /// configuration the previous record did not end in.
    pub fn replay(records: &[Self]) -> Result<Configuration<String>, ReplayError> {
        let mut current = Configuration::new();
        for (index, record) in records.iter().enumerate() {
            let from = Configuration::from_leaves(record.from.iter().cloned());
            if index > 0 && from != current {
                return Err(ReplayError::Discontinuous {
                    index,
                    expected: current.leaves().cloned().collect(),
                    found: record.from.clone(),
                });
            }
            current = Configuration::from_leaves(record.to.iter().cloned());
        }
        Ok(current)
    }

    /// Append the record to `log` as the payload of one journal envelope.
    ///
    /// # Errors
    ///
    /// Returns the log's [`AppendError`] when the record cannot be encoded or
    /// the log refuses the append.
    pub fn append(
        &self,
        log: &dyn DurableLog,
        stamp: TransitionStamp<'_>,
    ) -> Result<Offset, AppendError> {
        let payload = serde_json::to_value(self)
            .map_err(|error| AppendError::NotAnEnvelope(error.to_string()))?;
        let envelope = serde_json::json!({
            "v": SCHEMA_VERSION,
            "producer": {
                "producer_id": stamp.producer_id,
                "epoch": stamp.epoch,
                "seq": stamp.seq,
            },
            "payload": payload,
        });
        log.append_json(&envelope.to_string())
    }
}
