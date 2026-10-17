//! Law L2: causes classify every failure as [`Reason::Fail`], [`Reason::Die`],
//! or [`Reason::Interrupt`]. A cause is a vector of reasons: no `Empty`,
//! `Sequential`, or `Parallel` variants, and [`Cause::empty`] is an empty
//! vector, not a special case.

use std::backtrace::Backtrace;

use crate::context::Tag;

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
    ///
    /// This is an accumulator seed only (parallel folds collect siblings
    /// into it): it must never reach the fold as a completed failure, and
    /// no producer emits it on a failure path. [`Failure::of_cause`] treats
    /// an empty cause as a bug in the runtime's bookkeeping and reports it
    /// loudly instead of guessing.
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

    /// Interrupting fibers, in order (`Cause.interruptors`).
    pub fn interruptors(&self) -> Vec<Option<FiberId>> {
        self.reasons
            .iter()
            .filter_map(|r| match r {
                Reason::Interrupt { fiber_id } => Some(*fiber_id),
                _ => None,
            })
            .collect()
    }

    /// The defined interrupting fiber IDs (`Cause.interruptors`'s set form):
    /// `None` entries are excluded, and the result is ordered and deduplicated
    /// so a caller sees a set rather than the raw per-reason list.
    pub fn interruptor_ids(&self) -> Vec<FiberId> {
        let mut ids: Vec<FiberId> = self
            .interruptors()
            .into_iter()
            .flatten()
            .collect();
        ids.sort_by_key(|id| id.0);
        ids.dedup();
        ids
    }

    /// `Cause.findFail`: the first [`Reason::Fail`], or a unit failure when
    /// the cause holds none (the pin's `Result<T, void>`).
    pub fn find_fail(&self) -> Option<&Reason<E>> {
        self.reasons.iter().find(|r| is_fail_reason(r))
    }

    /// `Cause.findError`: the first typed error value.
    pub fn find_error(&self) -> Option<&E> {
        self.failure()
    }

    /// `Cause.findErrorOption`: `Some` with the first typed error value,
    /// `None` when the cause holds no `Fail` (empty causes included).
    pub fn find_error_option(&self) -> Option<&E> {
        self.failure()
    }

    /// `Cause.findDie`: the first [`Reason::Die`].
    pub fn find_die(&self) -> Option<&Reason<E>> {
        self.reasons.iter().find(|r| is_die_reason(r))
    }

    /// `Cause.findDefect`: the first defect value.
    pub fn find_defect(&self) -> Option<&Defect> {
        self.defects().first().copied()
    }

    /// `Cause.findInterrupt`: the first [`Reason::Interrupt`].
    pub fn find_interrupt(&self) -> Option<&Reason<E>> {
        self.reasons.iter().find(|r| is_interrupt_reason(r))
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

    /// `Cause.combine`: merge two causes. The tag dedups reasons by
    /// (Equal, Hash); a `Vec`'s `PartialEq` is the same observable rule, so
    /// unequal reasons never merge, an equal one does, and the first
    /// occurrence keeps its place. Combining with an empty cause returns the
    /// other unchanged, and mixed reason kinds all land in one flat list.
    pub fn combine(self, other: Self) -> Self
    where
        E: PartialEq,
    {
        let mut reasons = self.reasons;
        for reason in other.reasons {
            if !reasons.contains(&reason) {
                reasons.push(reason);
            }
        }
        let mut annotations = self.annotations;
        annotations.extend(other.annotations);
        Self { reasons, annotations }
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

/// `Cause.NoSuchElementError` and its siblings. The tag models each as its own
/// class extending `TaggedError(name)`, so each carries an optional message
/// and is an `Error`. This port uses one enum rather than five structs because
/// the guards below must answer "is this one of ours, and which" about a single
/// value, and a five-way `match` is the same discriminator the tag's five
/// classes give it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CauseError {
    /// `Cause.NoSuchElementError`: an element that was not there.
    NoSuchElement(Option<String>),
    /// `Cause.TimeoutError`: an operation that ran out of time.
    Timeout(Option<String>),
    /// `Cause.IllegalArgumentError`: a caller passed an unusable argument.
    IllegalArgument(Option<String>),
    /// `Cause.ExceededCapacityError`: a bounded structure was full.
    ExceededCapacity(Option<String>),
    /// `Cause.UnknownError`: an untyped failure wrapped for the error channel,
    /// with the original kept as text so it survives the boundary.
    Unknown {
        /// The wrapper's message, when the caller supplied one.
        message: Option<String>,
        /// The original failure, rendered. The tag carries `unknown` here; a
        /// Rust boundary cannot, so the value crosses as its rendering.
        cause: String,
    },
}

impl CauseError {
    /// `Cause.NoSuchElementErrorTypeId`.
    pub const NO_SUCH_ELEMENT_TYPE_ID: &'static str = "~effect/Cause/NoSuchElementError";
    /// `Cause.TimeoutErrorTypeId`.
    pub const TIMEOUT_TYPE_ID: &'static str = "~effect/Cause/TimeoutError";
    /// `Cause.IllegalArgumentErrorTypeId`.
    pub const ILLEGAL_ARGUMENT_TYPE_ID: &'static str = "~effect/Cause/IllegalArgumentError";
    /// `Cause.ExceededCapacityErrorTypeId`.
    pub const EXCEEDED_CAPACITY_TYPE_ID: &'static str = "~effect/Cause/ExceededCapacityError";
    /// `Cause.UnknownErrorTypeId`.
    pub const UNKNOWN_TYPE_ID: &'static str = "~effect/Cause/UnknownError";

    /// `new Cause.NoSuchElementError(message?)`.
    pub fn no_such_element(message: impl Into<String>) -> Self {
        Self::NoSuchElement(Some(message.into()))
    }

    /// `new Cause.NoSuchElementError()`.
    pub fn no_such_element_bare() -> Self {
        Self::NoSuchElement(None)
    }

    /// `new Cause.TimeoutError(message?)`.
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::Timeout(Some(message.into()))
    }

    /// `new Cause.TimeoutError()`.
    pub fn timeout_bare() -> Self {
        Self::Timeout(None)
    }

    /// `new Cause.IllegalArgumentError(message?)`.
    pub fn illegal_argument(message: impl Into<String>) -> Self {
        Self::IllegalArgument(Some(message.into()))
    }

    /// `new Cause.IllegalArgumentError()`.
    pub fn illegal_argument_bare() -> Self {
        Self::IllegalArgument(None)
    }

    /// `new Cause.ExceededCapacityError(message?)`.
    pub fn exceeded_capacity(message: impl Into<String>) -> Self {
        Self::ExceededCapacity(Some(message.into()))
    }

    /// `new Cause.ExceededCapacityError()`.
    pub fn exceeded_capacity_bare() -> Self {
        Self::ExceededCapacity(None)
    }

    /// `new Cause.UnknownError(cause, message?)`.
    pub fn unknown(cause: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Unknown {
            message: Some(message.into()),
            cause: cause.into(),
        }
    }

    /// `new Cause.UnknownError(cause)`.
    pub fn unknown_bare(cause: impl Into<String>) -> Self {
        Self::Unknown {
            message: None,
            cause: cause.into(),
        }
    }

    /// The `_tag` this value renders as.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::NoSuchElement(_) => "NoSuchElementError",
            Self::Timeout(_) => "TimeoutError",
            Self::IllegalArgument(_) => "IllegalArgumentError",
            Self::ExceededCapacity(_) => "ExceededCapacityError",
            Self::Unknown { .. } => "UnknownError",
        }
    }

    /// The `message`, which the tag's constructors leave absent when unset.
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::NoSuchElement(m) | Self::Timeout(m) | Self::IllegalArgument(m) => m.as_deref(),
            Self::ExceededCapacity(m) => m.as_deref(),
            Self::Unknown { message, .. } => message.as_deref(),
        }
    }

    /// The wrapped failure, for [`CauseError::Unknown`].
    pub fn cause(&self) -> Option<&str> {
        match self {
            Self::Unknown { cause, .. } => Some(cause),
            _ => None,
        }
    }

    /// `error.name` in the tag, which its `TaggedError` sets from the tag.
    pub fn name(&self) -> &'static str {
        self.tag()
    }

    /// True for [`CauseError::NoSuchElement`], mirroring
    /// `Cause.isNoSuchElementError`.
    pub fn is_no_such_element(&self) -> bool {
        matches!(self, Self::NoSuchElement(_))
    }

    /// True for [`CauseError::Timeout`], mirroring `Cause.isTimeoutError`.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout(_))
    }

    /// True for [`CauseError::IllegalArgument`], mirroring
    /// `Cause.isIllegalArgumentError`.
    pub fn is_illegal_argument(&self) -> bool {
        matches!(self, Self::IllegalArgument(_))
    }

    /// True for [`CauseError::ExceededCapacity`], mirroring
    /// `Cause.isExceededCapacityError`.
    pub fn is_exceeded_capacity(&self) -> bool {
        matches!(self, Self::ExceededCapacity(_))
    }

    /// True for [`CauseError::Unknown`], mirroring `Cause.isUnknownError`.
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown { .. })
    }
}

impl std::fmt::Display for CauseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.message() {
            Some(message) => write!(f, "{}: {}", self.tag(), message),
            None => write!(f, "{}", self.tag()),
        }
    }
}

impl std::error::Error for CauseError {}

/// The tag guards an arbitrary value against one error class. Rust has no
/// untyped value, so the guard takes a borrowed `Any` and an `Option` slot: the
/// `None` case is the tag's `null`/`undefined`, which its guard answers `false`
/// for. `Some("nope")` and `Some(42)` are `false` too, so the same call answers
/// every assertion the tag's row makes.
macro_rules! cause_error_guard {
    ($name:ident, $variant:pat) => {
        /// `Cause::is*Error` over a possibly-absent borrowed value.
        pub fn $name(value: Option<&dyn std::any::Any>) -> bool {
            value
                .and_then(|value| value.downcast_ref::<CauseError>())
                .is_some_and(|error| matches!(error, $variant))
        }
    };
}

cause_error_guard!(is_no_such_element_error, CauseError::NoSuchElement(_));
cause_error_guard!(is_timeout_error, CauseError::Timeout(_));
cause_error_guard!(is_illegal_argument_error, CauseError::IllegalArgument(_));
cause_error_guard!(is_exceeded_capacity_error, CauseError::ExceededCapacity(_));
cause_error_guard!(is_unknown_error, CauseError::Unknown { .. });

/// `Cause.Done`: a graceful completion signal carried through the error
/// channel, with an optional payload. `V` is the payload type; the tag
/// defaults it to `void`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done<V = ()> {
    /// The signal's payload, absent for `Cause.Done()`.
    pub value: Option<V>,
}

impl<V> Done<V> {
    /// `Cause.DoneTypeId`.
    pub const TYPE_ID: &'static str = "~effect/Cause/Done";

    /// `Cause.Done()`.
    pub fn empty() -> Self {
        Self { value: None }
    }

    /// `Cause.Done(value)`.
    pub fn new(value: V) -> Self {
        Self { value: Some(value) }
    }

    /// The `_tag` this value renders as.
    pub fn tag(&self) -> &'static str {
        "Done"
    }
}

/// `Cause.isDone` over a possibly-absent borrowed value, with the same
/// `None`-means-nullish contract as the guards above.
pub fn is_done<V: 'static>(value: Option<&dyn std::any::Any>) -> bool {
    value
        .and_then(|value| value.downcast_ref::<Done<V>>())
        .is_some()
}

/// `Reason`'s own `_tag`, which the tag's reason objects carry as a field.
impl<E> Reason<E> {
    /// `Fail`, `Die`, or `Interrupt`.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Fail(_) => "Fail",
            Self::Die(_) => "Die",
            Self::Interrupt { .. } => "Interrupt",
        }
    }
}

/// `Cause.makeFailReason`: build a reason directly rather than through a cause.
pub fn make_fail_reason<E>(error: E) -> Reason<E> {
    Reason::Fail(error)
}

/// `Cause.makeDieReason`.
pub fn make_die_reason<E>(defect: Defect) -> Reason<E> {
    Reason::Die(defect)
}

/// `Cause.makeInterruptReason`. The tag takes a raw fiber id and defaults it to
/// `undefined`; this port's `Option<FiberId>` is the same distinction.
pub fn make_interrupt_reason<E>(fiber_id: Option<FiberId>) -> Reason<E> {
    Reason::Interrupt { fiber_id }
}

impl<E> Cause<E> {
    /// `Cause.filterInterruptors`: the interrupting fiber ids when this cause
    /// interrupts, else the cause itself. The failure value is the cause, which
    /// is what the tag puts in the error channel.
    pub fn filter_interruptors(&self) -> Result<Vec<FiberId>, Self>
    where
        E: Clone,
    {
        if !self.is_interrupted() {
            return Err(self.clone());
        }
        Ok(self.interruptor_ids())
    }

    /// `Cause.annotations`: every attached annotation. This port's reasons
    /// carry none, so this is the whole set the tag merges.
    pub fn annotation_pairs(&self) -> &[(String, String)] {
        &self.annotations
    }

    /// `Cause.prettyErrors`: one `Error` per reason, suitable for logging or
    /// rethrowing. A typed failure and a defect each become one error; an
    /// interrupt-only cause becomes an interrupt error; an empty cause yields
    /// nothing.
    pub fn pretty_errors(&self) -> Vec<Box<dyn std::error::Error + Send + Sync>>
    where
        E: std::fmt::Display + Send + Sync + 'static,
    {
        let mut errors: Vec<Box<dyn std::error::Error + Send + Sync>> = Vec::new();
        for reason in &self.reasons {
            match reason {
                Reason::Fail(error) => {
                    errors.push(Box::new(PrettyError(format!("{error}"))));
                }
                Reason::Die(defect) => {
                    errors.push(Box::new(PrettyError(defect.message.clone())));
                }
                Reason::Interrupt { fiber_id } => {
                    errors.push(Box::new(PrettyError(match fiber_id {
                        Some(id) => format!("fiber {} interrupted", id.0),
                        None => "interrupted".to_owned(),
                    })));
                }
            }
        }
        errors
    }

    /// `Cause.pretty`: the whole cause as one string.
    pub fn pretty(&self) -> String
    where
        E: std::fmt::Debug,
    {
        self.to_string()
    }
}

/// One rendered reason from [`Cause::pretty_errors`].
#[derive(Debug)]
struct PrettyError(String);

impl std::fmt::Display for PrettyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PrettyError {}

/// `Cause.StackTrace`: the service the tag reads to render a defect's stack.
/// The port keeps the tag as a service so a caller can substitute a renderer,
/// which is the reason the tag models it as a context service at all.
pub struct StackTraceService;

/// `Cause.StackTrace`.
pub const STACK_TRACE: Tag<StackTraceService> = Tag::new("effect/Cause/StackTrace");

/// `Cause.InterruptorStackTrace`: the same, for the interrupting fiber's stack.
pub struct InterruptorStackTraceService;

/// `Cause.InterruptorStackTrace`.
pub const INTERRUPTOR_STACK_TRACE: Tag<InterruptorStackTraceService> =
    Tag::new("effect/Cause/InterruptorStackTrace");
