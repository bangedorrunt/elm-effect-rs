//! Supervision: an explicit child list plus a fiber dump for diagnostics.
//!
//! The runtime owns the live set; this type does not duplicate it and no
//! spawn path routes through it. It exists for operators: register the
//! fibers a scope owns, dump them when something stalls, and escalate a
//! bounded-shutdown straggler list into a message that names what is stuck.
//! Wiring shutdown to call [`Supervisor::escalate`] is a later bead; the hook
//! is ready here.

use std::sync::Mutex;

use crate::FiberId;

/// One watched child: identity plus the parent it was forked from, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildSnapshot {
    pub id: FiberId,
    pub parent: Option<FiberId>,
}

/// A straggler report: which timed-out fibers were watched (with parents)
/// and which were unknown to this supervisor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Escalation {
    pub known: Vec<ChildSnapshot>,
    pub unknown: Vec<FiberId>,
}

impl Escalation {
    /// One line per straggler: known children render with their parent, so
    /// the dump points at the subtree that stalled.
    pub fn message(&self) -> String {
        let mut lines = Vec::with_capacity(self.known.len() + self.unknown.len());
        for child in &self.known {
            match child.parent {
                Some(parent) => lines.push(format!(
                    "straggler fiber {} (parent {})",
                    child.id.0, parent.0
                )),
                None => lines.push(format!("straggler fiber {}", child.id.0)),
            }
        }
        for id in &self.unknown {
            lines.push(format!("straggler fiber {} (unwatched)", id.0));
        }
        lines.join("\n")
    }
}

/// An explicit child list for diagnostics. Cloneable and thread-safe;
/// every method takes `&self` so any holder can report.
#[derive(Default)]
pub struct Supervisor {
    children: Mutex<Vec<ChildSnapshot>>,
}

impl Supervisor {
    /// An empty child list.
    pub fn new() -> Self {
        Self {
            children: Mutex::new(Vec::new()),
        }
    }

    /// Register a child. Re-watching the same id refreshes its parent
    /// instead of duplicating the row.
    pub fn watch(&self, id: FiberId, parent: Option<FiberId>) {
        let mut children = self.children.lock().unwrap();
        if let Some(existing) = children.iter_mut().find(|c| c.id == id) {
            existing.parent = parent;
        } else {
            children.push(ChildSnapshot { id, parent });
        }
    }

    /// Forget a child. True when a row was removed.
    pub fn unwatch(&self, id: FiberId) -> bool {
        let mut children = self.children.lock().unwrap();
        let before = children.len();
        children.retain(|c| c.id != id);
        children.len() != before
    }

    /// The watched children, sorted by id for stable dumps.
    pub fn children(&self) -> Vec<ChildSnapshot> {
        let mut out = self.children.lock().unwrap().clone();
        out.sort_by_key(|c| c.id.0);
        out
    }

    /// A human-readable dump: one `fiber <id> parent=<id|->` line per child.
    pub fn dump(&self) -> String {
        self.children()
            .iter()
            .map(|c| match c.parent {
                Some(parent) => format!("fiber {} parent={}", c.id.0, parent.0),
                None => format!("fiber {} parent=-", c.id.0),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The escalation hook for bounded shutdown: partition a straggler list
    /// into watched children (with parents) and unknown ids. Pure, so the
    /// shutdown path can call it without touching spawn machinery.
    pub fn escalate(&self, stragglers: &[FiberId]) -> Escalation {
        let children = self.children.lock().unwrap();
        let mut known = Vec::new();
        let mut unknown = Vec::new();
        for id in stragglers {
            match children.iter().find(|c| &c.id == id) {
                Some(child) => known.push(child.clone()),
                None => unknown.push(*id),
            }
        }
        known.sort_by_key(|c| c.id.0);
        unknown.sort_by_key(|id| id.0);
        Escalation { known, unknown }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewatch_refreshes_parent_without_duplicating() {
        let sup = Supervisor::new();
        sup.watch(FiberId(3), None);
        sup.watch(FiberId(3), Some(FiberId(1)));
        assert_eq!(
            sup.children(),
            vec![ChildSnapshot {
                id: FiberId(3),
                parent: Some(FiberId(1))
            }]
        );
    }

    #[test]
    fn empty_escalation_is_empty() {
        let sup = Supervisor::new();
        let esc = sup.escalate(&[]);
        assert!(esc.known.is_empty() && esc.unknown.is_empty());
        assert_eq!(esc.message(), "");
    }
}
