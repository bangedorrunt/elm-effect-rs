//! `Pull.ts` port: a repeatable one-take source.
//!
//! Each `pull` yields one [`Take`]: items until the source is spent, then
//! `End` forever. An unbounded source stays a pull loop — never materialize
//! it; consumers stop pulling when they have enough.

use super::Defect;
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

    /// One value, then end (`succeed`, `of` single).
    pub fn single(value: A) -> Self
    where
        A: Clone,
    {
        Self::of(vec![value])
    }

    /// From a slice, cloned (`fromArray` shape).
    pub fn from_slice(values: &[A]) -> Self
    where
        A: Clone,
    {
        Self::of(values.to_vec())
    }

    /// From an iterator in bounded chunks (`fromIterator`, `fromIterable`
    /// shapes): pulls `chunk_size` items at a time, never materializing
    /// the rest — unbounded inputs stay streams.
    pub fn from_iter<I>(iter: I, chunk_size: usize) -> Self
    where
        I: Iterator<Item = A> + Send + 'static,
    {
        use std::sync::{Arc, Mutex};
        let iter = Arc::new(Mutex::new(iter));
        let chunk = chunk_size.max(1);
        Self::from_fn(move || {
            let mut guard = iter.lock().unwrap();
            let mut out = Vec::with_capacity(chunk);
            for _ in 0..chunk {
                match guard.next() {
                    Some(item) => out.push(item),
                    None => break,
                }
            }
            if out.is_empty() {
                Take::End
            } else {
                Take::Items(out)
            }
        })
    }

    /// Fails immediately, on every pull, with one typed error.
    pub fn fail(error: E) -> Self
    where
        E: Clone,
    {
        Self::from_fn(move || Take::fail(error.clone()))
    }

    /// Fails immediately, on every pull, with a defect. Never a typed error:
    /// the sink reports it as a `Die`, not as the stream's `E`.
    pub fn die(defect: Defect) -> Self
    where
        Defect: Clone,
    {
        Self::from_fn(move || Take::die(defect.clone()))
    }

    /// Bracket with a fallible release (`acquireUseRelease` failure shapes):
    /// `release` runs exactly once, on the first terminal take. A release
    /// failure surfaces as `Fail` on the pull AFTER the terminal take: the
    /// stream's own ending is delivered first, then the release error.
    pub fn using_try<R>(
        resource: R,
        mut pull: impl FnMut(&mut R) -> Take<A, E> + Send + 'static,
        release: impl FnOnce(R) -> Result<(), E> + Send + 'static,
    ) -> Self
    where
        R: Send + 'static,
        E: Send + 'static,
    {
        let mut state = Some((resource, Some(release)));
        let mut release_error: Option<E> = None;
        Self::from_fn(move || {
            if let Some(error) = release_error.take() {
                return Take::fail(error);
            }
            let Some((mut resource, release)) = state.take() else {
                return Take::End;
            };
            match pull(&mut resource) {
                terminal @ (Take::End | Take::Fail(_)) => {
                    if let Some(release) = release
                        && let Err(error) = release(resource)
                    {
                        release_error = Some(error);
                    }
                    terminal
                }
                take => {
                    state = Some((resource, release));
                    take
                }
            }
        })
    }
    /// Bracket a resource around pulls (`acquireRelease` shapes):
    /// `release` runs exactly once, on the first terminal take
    /// (`End` or `Fail`), before that take is delivered. Later pulls yield
    /// `End`.
    pub fn using<R>(
        resource: R,
        mut pull: impl FnMut(&mut R) -> Take<A, E> + Send + 'static,
        release: impl FnOnce(R) + Send + 'static,
    ) -> Self
    where
        R: Send + 'static,
    {
        let mut state = Some((resource, Some(release)));
        Self::from_fn(move || {
            let Some((mut resource, release)) = state.take() else {
                return Take::End;
            };
            match pull(&mut resource) {
                terminal @ (Take::End | Take::Fail(_)) => {
                    if let Some(release) = release {
                        release(resource);
                    }
                    terminal
                }
                take => {
                    state = Some((resource, release));
                    take
                }
            }
        })
    }

    /// Pull once.
    pub fn pull(&mut self) -> Take<A, E> {
        (self.next)()
    }
}
