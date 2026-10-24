//! Elm time travel (ADR-0002 Phase 8 Redux rules, Law L12).
//!
//! A [`Player`] folds messages purely while recording them in a
//! [`MessageLog`]. [`Snapshot`] stores the model plus the log offset it
//! includes; snapshots land every [`Player::snapshot_interval_messages`]
//! messages (100 by default, configurable per program). [`replay`] re-folds
//! from the latest snapshot without re-executing providers (their results
//! are already messages) and compares the derived command lists against
//! the recorded ones: a mismatch is a divergence, not a silent replay.
//! [`Middleware`] is `fn(&Msg) -> Msg` applied before `update`; the
//! signature leaves no room for `Cmd`, effects, sagas, thunks, or
//! listeners. [`Snapshots`] streams recorded snapshots; the handle yields
//! owned data and cannot send into the model.

use serde::{Deserialize, Serialize};

use super::Program;

/// Message log trait. Wave 12 backs this durably; until then the
/// in-crate [`InMemoryLog`] is the one implementation (do not fork it).
pub trait MessageLog<Msg> {
    /// Append one message.
    fn push(&mut self, msg: Msg);
    /// Messages from `offset` (exclusive of earlier history).
    fn messages_from(&self, offset: usize) -> Vec<Msg>
    where
        Msg: Clone;
    /// Total messages recorded.
    fn len(&self) -> usize;

    /// True when no message has been recorded.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Minimal in-crate log: a vector behind the [`MessageLog`] trait.
#[derive(Clone, Debug)]
pub struct InMemoryLog<Msg> {
    entries: Vec<Msg>,
}

impl<Msg> Default for InMemoryLog<Msg> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<Msg: Clone> MessageLog<Msg> for InMemoryLog<Msg> {
    fn push(&mut self, msg: Msg) {
        self.entries.push(msg);
    }

    fn messages_from(&self, offset: usize) -> Vec<Msg> {
        self.entries.iter().skip(offset).cloned().collect()
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Snapshot: the model plus the log offset it includes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot<Model> {
    /// Model at the snapshot point.
    pub model: Model,
    /// Log offset included (messages before this are folded in).
    pub offset: usize,
}

/// Message middleware: maps messages before `update`, no command access.
pub type Middleware<Msg> = Box<dyn Fn(&Msg) -> Msg + Send + Sync>;

/// What replay found.
#[derive(Clone, Debug, PartialEq)]
pub enum Replay<Model> {
    /// Re-fold matches the live model and every command list.
    Consistent {
        /// Re-folded model.
        model: Model,
    },
    /// Command lists diverge at this log index.
    Divergent {
        /// Log index of the first divergence.
        index: usize,
    },
}

/// Pure player: fold, log, snapshot, replay. No effects run here.
pub struct Player<P: Program> {
    genesis: P::Model,
    model: P::Model,
    log: InMemoryLog<P::Msg>,
    commands: Vec<Vec<serde_json::Value>>,
    snapshots: Vec<Snapshot<P::Model>>,
    interval: usize,
    middleware: Option<Middleware<P::Msg>>,
}

impl<P: Program> Player<P>
where
    P::Msg: Clone + Send + 'static,
    P::Model: Clone + PartialEq,
    P::Cmd: Clone,
{
    /// Boot from the init model with the default interval (100).
    pub fn boot(model: P::Model) -> Self {
        Self::boot_with_interval(model, 100)
    }

    /// Boot with a per-program snapshot interval.
    pub fn boot_with_interval(model: P::Model, interval: usize) -> Self {
        Self {
            genesis: model.clone(),
            model,
            log: InMemoryLog::default(),
            commands: Vec::new(),
            snapshots: Vec::new(),
            interval: interval.max(1),
            middleware: None,
        }
    }

    /// Snapshot cadence in messages.
    pub fn snapshot_interval_messages(&self) -> usize {
        self.interval
    }

    /// Install message middleware.
    pub fn middleware(&mut self, f: impl Fn(&P::Msg) -> P::Msg + Send + Sync + 'static) {
        self.middleware = Some(Box::new(f));
    }

    /// Fold one message: middleware, log, update, maybe snapshot.
    pub fn send(&mut self, msg: P::Msg) {
        let msg = match &self.middleware {
            Some(f) => f(&msg),
            None => msg,
        };
        self.log.push(msg.clone());
        let update = P::update(&self.model, &msg);
        self.model = update.model;
        self.commands.push(
            update
                .commands
                .iter()
                .map(|c| serde_json::to_value(c).unwrap_or_default())
                .collect(),
        );
        if self.log.len() % self.interval == 0 {
            self.snapshots.push(Snapshot {
                model: self.model.clone(),
                offset: self.log.len(),
            });
        }
    }

    /// Current model.
    pub fn model(&self) -> &P::Model {
        &self.model
    }

    /// Latest snapshot, if any.
    pub fn snapshot(&self) -> Option<&Snapshot<P::Model>> {
        self.snapshots.last()
    }

    /// Replay from the latest snapshot (or genesis): pure re-fold plus
    /// per-step command-list comparison. Providers never re-execute.
    pub fn replay(&self) -> Replay<P::Model> {
        let (mut model, offset) = match self.snapshots.last() {
            Some(snap) => (snap.model.clone(), snap.offset),
            None => (self.genesis.clone(), 0),
        };
        for (step, msg) in self.log.messages_from(offset).into_iter().enumerate() {
            let update = P::update(&model, &msg);
            model = update.model;
            let derived: Vec<serde_json::Value> = update
                .commands
                .iter()
                .map(|c| serde_json::to_value(c).unwrap_or_default())
                .collect();
            if derived != self.commands[offset + step] {
                return Replay::Divergent {
                    index: offset + step,
                };
            }
        }
        Replay::Consistent { model }
    }

    /// Snapshot stream: every recorded snapshot, oldest first. Owned
    /// data out, no send path in.
    pub fn snapshots(&self) -> Snapshots<P::Model> {
        Snapshots {
            rest: self.snapshots.clone().into_iter(),
        }
    }
}

/// Read-only snapshot stream handle.
pub struct Snapshots<Model> {
    rest: std::vec::IntoIter<Snapshot<Model>>,
}

impl<Model: Clone> Iterator for Snapshots<Model> {
    type Item = Snapshot<Model>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rest.next()
    }
}
