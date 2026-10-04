//! `Pull.ts` port: a repeatable one-take source.
//!
//! Each `pull` yields one [`Take`]: items until the source is spent, then
//! `End` forever. An unbounded source stays a pull loop — never materialize
//! it; consumers stop pulling when they have enough.

use super::take::Take;

/// A repeatable pull source.
pub struct Pull<A, E> {
    next: Box<dyn FnMut() -> Take<A, E> + Send>,
}

impl<A, E> Pull<A, E>
where
    A: Send + 'static,
    E: Send + 'static,
{
    /// Build from a pull closure.
    pub fn from_fn(next: impl FnMut() -> Take<A, E> + Send + 'static) -> Self {
        Self {
            next: Box::new(next),
        }
    }

    /// One item vector, then `End` forever.
    pub fn of(items: Vec<A>) -> Self
    where
        A: Clone,
    {
        let mut iter = items.into_iter();
        let mut done = false;
        Self::from_fn(move || {
            if done {
                return Take::End;
            }
            match iter.next() {
                Some(first) => {
                    let mut chunk = vec![first];
                    chunk.extend(&mut iter);
                    done = true;
                    Take::Items(chunk)
                }
                None => {
                    done = true;
                    Take::End
                }
            }
        })
    }

    /// Ends immediately.
    pub fn empty() -> Self {
        Self::from_fn(|| Take::End)
    }

    /// Fails immediately, on every pull.
    pub fn fail(error: E) -> Self
    where
        E: Clone,
    {
        Self::from_fn(move || Take::Fail(error.clone()))
    }

    /// Pull once.
    pub fn pull(&mut self) -> Take<A, E> {
        (self.next)()
    }
}
