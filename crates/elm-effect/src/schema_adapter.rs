//! `decodeUnknownOption` and its siblings: what each carrier can hand back.
//!
//! A schema run can fail with a whole [`Cause`], and a cause may hold a defect
//! beside a schema issue. [`Exit`] is the only carrier that can say that;
//! `Option`, `Result`, a throw and a promise rejection can each carry only a
//! bare issue, so a cause with a defect in it is refused with the adapter's own
//! message and the whole cause rides on the refusal (the pin's
//! `new Error(message, { cause })`). Reporting the schema issue instead would
//! lose the defect, and answering `None`/a rejection would pretend the value
//! merely failed validation.
//!
//! The construction adapters keep the same contract in
//! `elm_effect_schema::AdapterError`; these are the decode/encode ones.

use crate::{Cause, Exit};

/// The promise adapter's message when the cause held a defect
/// (`Schema.test.ts:8435`).
pub const PROMISE_MESSAGE: &str = "Promise adapter can only reject schema errors";

/// The option adapter's message (`Schema.test.ts:8482`, `:8504`).
pub const OPTION_MESSAGE: &str = "Option adapter can only return none for schema issues";

/// The result adapter's message (`Schema.test.ts:8566`).
pub const RESULT_MESSAGE: &str = "Result adapter can only return schema issues";

/// The sync adapter's message (`Schema.test.ts:8619`).
pub const SYNC_MESSAGE: &str = "Sync adapter can only throw schema errors";

/// An adapter's refusal: its contract message plus the whole cause it refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterRefusal<E> {
    message: &'static str,
    cause: Cause<E>,
}

impl<E> AdapterRefusal<E> {
    /// A refusal under `message`, carrying the cause that was not a bare issue.
    pub fn new(message: &'static str, cause: Cause<E>) -> Self {
        Self { message, cause }
    }

    /// The adapter's contract message.
    pub fn message(&self) -> &'static str {
        self.message
    }

    /// The cause the adapter refused, defects included.
    pub fn cause(&self) -> &Cause<E> {
        &self.cause
    }
}

impl<E> std::fmt::Display for AdapterRefusal<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message)
    }
}

impl<E: std::fmt::Debug> std::error::Error for AdapterRefusal<E> {}

/// What a carrier that can hand back the value itself reports: the schema
/// issue as it arrived, or the adapter's refusal when the cause held a defect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdapterFailure<E> {
    /// A bare schema issue: the carrier reports it as itself.
    Issue(E),
    /// The cause was not a bare issue, so the adapter refused it.
    Refusal(AdapterRefusal<E>),
}

impl<E> From<AdapterRefusal<E>> for AdapterFailure<E> {
    fn from(refusal: AdapterRefusal<E>) -> Self {
        Self::Refusal(refusal)
    }
}

impl<E: std::fmt::Display> std::fmt::Display for AdapterFailure<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Issue(error) => error.fmt(f),
            Self::Refusal(refusal) => refusal.fmt(f),
        }
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for AdapterFailure<E> {}

// The exit adapter needs no method: [`Exit`] is the carrier that keeps
// everything, so `decodeUnknownExit` is the decode itself.

impl<T, E> Exit<T, E> {
    /// `decodeUnknownOption`: `Some` for a success, `None` for a bare issue.
    pub fn into_option(self) -> Result<Option<T>, AdapterRefusal<E>> {
        match self {
            Exit::Success(value) => Ok(Some(value)),
            Exit::Failure(cause) => match into_issue(cause, OPTION_MESSAGE) {
                Ok(_) => Ok(None),
                Err(refusal) => Err(refusal),
            },
        }
    }

    /// `decodeUnknownResult`: the schema error itself for a bare issue.
    pub fn into_result(self) -> Result<Result<T, E>, AdapterRefusal<E>> {
        match self {
            Exit::Success(value) => Ok(Ok(value)),
            Exit::Failure(cause) => match into_issue(cause, RESULT_MESSAGE) {
                Ok(error) => Ok(Err(error)),
                Err(refusal) => Err(refusal),
            },
        }
    }

    /// `decodeUnknownSync`: the value, or what the sync adapter throws.
    pub fn into_value(self) -> Result<T, AdapterFailure<E>> {
        self.into_sync(SYNC_MESSAGE)
    }

    /// [`into_value`](Self::into_value) with the adapter's own wording.
    ///
    /// The pin's two test files disagree on that wording for the same adapter
    /// (`Schema.test.ts` says "schema errors", `SchemaParser.test.ts` says
    /// "schema issues"), so an entry point passes the text its file asserts.
    pub fn into_value_with(self, message: &'static str) -> Result<T, AdapterFailure<E>> {
        self.into_sync(message)
    }

    /// `decodeUnknownPromise`: the value, or what the promise adapter rejects
    /// with. A rejection and a throw differ in wording, not in what they can
    /// carry, so both go through the same split.
    pub fn into_promise(self) -> Result<T, AdapterFailure<E>> {
        self.into_sync(PROMISE_MESSAGE)
    }

    fn into_sync(self, message: &'static str) -> Result<T, AdapterFailure<E>> {
        match self {
            Exit::Success(value) => Ok(value),
            Exit::Failure(cause) => match into_issue(cause, message) {
                Ok(error) => Err(AdapterFailure::Issue(error)),
                Err(refusal) => Err(AdapterFailure::Refusal(refusal)),
            },
        }
    }
}

/// The single schema issue a cause carries, or the adapter's refusal when the
/// cause is anything else: a defect, a defect beside an issue, two issues, an
/// interruption.
fn into_issue<E>(cause: Cause<E>, message: &'static str) -> Result<E, AdapterRefusal<E>> {
    cause
        .into_single_error()
        .map_err(|cause| AdapterRefusal::new(message, cause))
}
