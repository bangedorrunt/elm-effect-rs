//! The portable runtime half of `Utils.ts`: [`SingleShotGen`].
//!
//! The rest of `Utils.ts` is type-level machinery (`Variance`, `Gen`) and
//! internal hooks (`internalCall`) with no runtime behavior to port; those
//! rows are skipped-upstream in the ledger.

/// A one-shot generator: yields the stored value once, then completes with
/// whatever value is sent back in. The Rust spelling of `SingleShotGen`.
#[derive(Clone, Debug)]
pub struct SingleShotGen<T, A> {
    value: Option<T>,
    yielded: bool,
    back: Option<A>,
}

impl<T, A> SingleShotGen<T, A> {
    /// Build around a value to yield once.
    pub fn new(value: T) -> Self {
        Self { value: Some(value), yielded: false, back: None }
    }

    /// Advance: the first call yields the stored value; later calls
    /// complete with the value handed back in.
    pub fn next<'s>(&'s mut self, sent: A) -> Step<'s, T, A>
    where
        A: Clone,
    {
        if !self.yielded {
            self.yielded = true;
            match &self.value {
                Some(v) => Step::Yield(v),
                None => panic!("SingleShotGen yields its value once"),
            }
        } else {
            self.back = Some(sent.clone());
            Step::Return(sent)
        }
    }
}

/// One step of a [`SingleShotGen`]: a yielded borrow or a final value.
#[derive(Clone, Debug, PartialEq)]
pub enum Step<'a, T, A> {
    /// The single yielded value (borrowed; the generator keeps ownership).
    Yield(&'a T),
    /// Completion with the value sent back in.
    Return(A),
}
