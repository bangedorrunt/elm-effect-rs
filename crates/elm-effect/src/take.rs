//! `Take.ts` port: one pull outcome, items, end of stream, or failure.

use super::{Cause, Defect};

/// One pull from a source.
///
/// A failure carries the whole [`Cause`], as the pin's `Take` does: a defect
/// is not a typed error, so a channel that dies must not have to name an `E`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Take<A, E> {
    /// A bounded chunk of items.
    Items(Vec<A>),
    /// The stream ended cleanly.
    End,
    /// The stream failed, typed error or defect.
    Fail(Cause<E>),
}

impl<A, E> Take<A, E> {
    /// A failure with one typed error.
    pub fn fail(error: E) -> Self {
        Take::Fail(Cause::fail(error))
    }

    /// A failure with one defect. Never a typed error.
    pub fn die(defect: Defect) -> Self {
        Take::Fail(Cause::die(defect))
    }

    /// Map items, passing `End`/`Fail` through whole.
    pub fn map<B>(self, f: impl FnMut(A) -> B) -> Take<B, E> {
        match self {
            Take::Items(items) => Take::Items(items.into_iter().map(f).collect()),
            Take::End => Take::End,
            Take::Fail(cause) => Take::Fail(cause),
        }
    }

    /// True only for the end marker.
    pub fn is_end(&self) -> bool {
        matches!(self, Take::End)
    }
}
