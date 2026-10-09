//! Law L2: causes classify every failure as [`Reason::Fail`], [`Reason::Die`],
//! or [`Reason::Interrupt`]. A cause is a vector of reasons: no `Empty`,
//! `Sequential`, or `Parallel` variants, and [`Cause::empty`] is an empty
//! vector, not a special case.

use std::backtrace::Backtrace;

/// Identity of the fiber that was interrupted, when known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FiberId(pub u64);

/// An unexpected, untyped failure: a panic, a broken invariant, an I/O error
/// nobody modeled. Reportable via [`std::error::Error`], but never a typed
/// error value `E`: the types cannot collapse.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct Defect {
    /// What went wrong, in one line.
    pub message: String,
    /// Captured [`Backtrace`] rendered to text at construction. Stored as a
    /// string (not `Backtrace`) so causes keep structural equality.
    pub backtrace: String,
}

impl Defect {
    /// Capture the current backtrace alongside `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            backtrace: Backtrace::capture().to_string(),
        }
    }
}

/// One failure reason. A typed error, a defect, or an interruption: the three
/// never share a constructor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason<E> {
    /// An expected, typed error value.
    Fail(E),
    /// An unexpected defect. Never an `E`.
    Die(Defect),
    /// The fiber was interrupted. Carries the interrupting fiber when known.
    Interrupt { fiber_id: Option<FiberId> },
}

impl<E> Reason<E> {
    /// Type identity string, mirroring `Cause.ReasonTypeId`.
    pub const REASON_TYPE_ID: &'static str = "~effect/Cause/Reason";
}

/// True for [`Reason::Fail`] reasons, mirroring `Cause.isFailReason`.
pub fn is_fail_reason<E>(reason: &Reason<E>) -> bool {
    matches!(reason, Reason::Fail(_))
}

/// True for [`Reason::Die`] reasons, mirroring `Cause.isDieReason`.
pub fn is_die_reason<E>(reason: &Reason<E>) -> bool {
    matches!(reason, Reason::Die(_))
}

/// True for [`Reason::Interrupt`] reasons, mirroring `Cause.isInterruptReason`.
pub fn is_interrupt_reason<E>(reason: &Reason<E>) -> bool {
    matches!(reason, Reason::Interrupt { .. })
}

/// Tag-compatible rendering (`Fail("boom")`, `Die("kaboom"),
/// `Interrupt(7)`). Values render with `Debug`, which matches the tag's
/// inspect for strings (quoted) and numbers (bare). An unknown interrupter
/// renders as `undefined`: the tag's word for an absent value.
impl<E: std::fmt::Debug> std::fmt::Display for Reason<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::Fail(error) => write!(f, "Fail({error:?})"),
            Reason::Die(defect) => write!(f, "Die({:?})", defect.message),
            Reason::Interrupt { fiber_id } => match fiber_id {
                Some(id) => write!(f, "Interrupt({})", id.0),
                None => write!(f, "Interrupt(undefined)"),
            },
        }
    }
}

/// Why an effect failed: a vector of [`Reason`]. Ordinary data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cause<E> {
    /// Every reason, in the order recorded. Empty means no failure recorded.
    pub reasons: Vec<Reason<E>>,
    /// Context annotations (`Cause.annotate`): string-keyed pairs.
    /// The tag keys these by service type; `std` has no such keys, so
    /// the key is the service name (last segment preserved verbatim).
    pub annotations: Vec<(String, String)>,
}

/// Tag-compatible rendering (`Cause([Fail("boom")])`); reasons join with
/// `", "` like the tag's array rendering. Empty renders `Cause([])`.
impl<E: std::fmt::Debug> std::fmt::Display for Cause<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cause([")?;
        for (i, reason) in self.reasons.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{reason}")?;
        }
        write!(f, "])")
    }
}

impl<E> Cause<E> {
    /// Type identity string, mirroring `Cause.TypeId`.
    pub const TYPE_ID: &'static str = "~effect/Cause";

    /// A cause with no reasons. There is no `Empty` variant to confuse with
    /// "failed with nothing".
    pub fn empty() -> Self {
        Self {
            reasons: Vec::new(),
            annotations: Vec::new(),
        }
    }

    /// A single typed failure.
    pub fn fail(error: E) -> Self {
        Self {
            reasons: vec![Reason::Fail(error)],
            annotations: Vec::new(),
        }
    }

    /// A single defect. This never counts as a typed error value.
    pub fn die(defect: Defect) -> Self {
        Self {
            reasons: vec![Reason::Die(defect)],
            annotations: Vec::new(),
        }
    }

    /// A single interruption, naming the interrupter when known.
    pub fn interrupt(fiber_id: Option<FiberId>) -> Self {
        Self {
            reasons: vec![Reason::Interrupt { fiber_id }],
            annotations: Vec::new(),
        }
    }

    /// Build a failure cause purely from finalizer/recorded defects (L6:
    /// a failed release on a successful body is still a failure).
    pub fn from_defects(defects: Vec<Defect>) -> Self {
        Self {
            reasons: defects.into_iter().map(Reason::Die).collect(),
            annotations: Vec::new(),
        }
    }

    /// Append finalizer failures without erasing the original reasons (L6).
    pub fn append_defects(&mut self, defects: Vec<Defect>) {
        self.reasons.extend(defects.into_iter().map(Reason::Die));
    }

    /// The first typed error, if any reason is a `Fail`.
    pub fn failure(&self) -> Option<&E> {
        self.reasons.iter().find_map(|r| match r {
            Reason::Fail(e) => Some(e),
            _ => None,
        })
    }

    /// Every defect carried by this cause. Empty for pure typed failures.
    pub fn defects(&self) -> Vec<&Defect> {
        self.reasons
            .iter()
            .filter_map(|r| match r {
                Reason::Die(d) => Some(d),
                _ => None,
            })
            .collect()
    }

    /// True if any reason is an interruption (not a failure).
    pub fn is_interrupted(&self) -> bool {
        self.reasons
            .iter()
            .any(|r| matches!(r, Reason::Interrupt { .. }))
    }

    /// Expected error values, in order ().
    pub fn failures(&self) -> Vec<&E> {
        self.reasons
            .iter()
            .filter_map(|r| match r {
                Reason::Fail(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    /// Interrupting fibers, in order ().
    pub fn interruptors(&self) -> Vec<Option<FiberId>> {
        self.reasons
            .iter()
            .filter_map(|r| match r {
                Reason::Interrupt { fiber_id } => Some(*fiber_id),
                _ => None,
            })
            .collect()
    }

    /// Keep only  reasons (narrow to failures, rejecting dies and
    /// interrupts).
    pub fn strip_dies_and_interrupts(&self) -> Self
    where
        E: Clone,
    {
        Self {
            reasons: self
                .reasons
                .iter()
                .filter(|r| matches!(r, Reason::Fail(_)))
                .cloned()
                .collect(),
            annotations: self.annotations.clone(),
        }
    }

    /// Keep only  reasons (narrow to defects).
    pub fn strip_failures_and_interrupts(&self) -> Self
    where
        E: Clone,
    {
        Self {
            reasons: self
                .reasons
                .iter()
                .filter(|r| matches!(r, Reason::Die(_)))
                .cloned()
                .collect(),
            annotations: self.annotations.clone(),
        }
    }

    /// Keep only  reasons (narrow to interruptions).
    pub fn strip_failures_and_dies(&self) -> Self
    where
        E: Clone,
    {
        Self {
            reasons: self
                .reasons
                .iter()
                .filter(|r| matches!(r, Reason::Interrupt { .. }))
                .cloned()
                .collect(),
            annotations: self.annotations.clone(),
        }
    }
}

/// Annotation and mapping support (`Cause.annotate`, `Cause.map`):
/// annotations are metadata, so narrowing keeps them.
impl<E> Cause<E> {
    /// Attach a context annotation, replacing any earlier value for `key`.
    pub fn annotate(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        if let Some(slot) = self.annotations.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value.into();
        } else {
            self.annotations.push((key, value.into()));
        }
        self
    }

    /// Read an annotation by key (`None` when absent).
    pub fn annotation(&self, key: &str) -> Option<&str> {
        self.annotations
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Map typed failures, preserving annotations and other reasons.
    pub fn map<F, U>(self, f: F) -> Cause<U>
    where
        F: Fn(E) -> U,
    {
        Cause {
            reasons: self
                .reasons
                .into_iter()
                .map(|r| match r {
                    Reason::Fail(e) => Reason::Fail(f(e)),
                    Reason::Die(d) => Reason::Die(d),
                    Reason::Interrupt { fiber_id } => Reason::Interrupt { fiber_id },
                })
                .collect(),
            annotations: self.annotations,
        }
    }

    /// Build from an explicit reason list (`Cause.fromReasons`). An empty
    /// list is [`Cause::empty`], not a distinct state.
    pub fn from_reasons(reasons: Vec<Reason<E>>) -> Self {
        Self {
            reasons,
            annotations: Vec::new(),
        }
    }

    /// `Cause.hasFails`: at least one [`Reason::Fail`].
    pub fn has_fails(&self) -> bool {
        self.reasons.iter().any(is_fail_reason)
    }

    /// `Cause.hasDies`: at least one [`Reason::Die`].
    pub fn has_dies(&self) -> bool {
        self.reasons.iter().any(is_die_reason)
    }

    /// `Cause.hasInterrupts`: at least one [`Reason::Interrupt`].
    pub fn has_interrupts(&self) -> bool {
        self.reasons.iter().any(is_interrupt_reason)
    }

    /// `Cause.hasInterruptsOnly`: non-empty and every reason an interrupt.
    /// An empty cause is NOT interrupt-only (pinned by the "false for empty
    /// cause" row).
    pub fn has_interrupts_only(&self) -> bool {
        !self.reasons.is_empty() && self.reasons.iter().all(is_interrupt_reason)
    }

    /// `Cause.squash`: the reason a caller sees, in the pin's priority —
    /// first `Fail`, else first `Die`, else the interrupt-only or empty
    /// summary. Values are handed back by clone so the cause stays intact.
    pub fn squash(&self) -> Squash<E>
    where
        E: Clone,
    {
        if let Some(e) = self.failure() {
            return Squash::Fail(e.clone());
        }
        if let Some(defect) = self.defects().first() {
            return Squash::Die((*defect).clone());
        }
        if self.has_interrupts_only() {
            return Squash::Interrupted;
        }
        if self.reasons.is_empty() {
            return Squash::Empty;
        }
        unreachable!("a non-empty cause has at least one reason kind");
    }
}

/// `Cause.squash`'s outcome. `Interrupted` and `Empty` mirror the pin's
/// two summary errors ("All fibers interrupted without error", "Empty
/// cause"); the port carries them as distinct variants rather than `String`
/// messages so a caller cannot confuse them with a defect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Squash<E> {
    /// The first typed failure, by value.
    Fail(E),
    /// The first defect, by value.
    Die(Defect),
    /// Interrupt-only cause: the pin's "All fibers interrupted without error".
    Interrupted,
    /// Empty cause: the pin's "Empty cause".
    Empty,
}

impl<E> Default for Cause<E> {
    fn default() -> Self {
        Self::empty()
    }
}
