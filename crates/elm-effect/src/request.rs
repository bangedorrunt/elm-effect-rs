//! `Request` at the pin: a request names work a resolver performs, and
//! `request` submits one.
//!
//! This is the direct-execution core: a submission runs the resolver once, for
//! its own batch of one. The batching executor (a shared queue that groups
//! concurrent submissions, `batchN`, and the cache) is not here yet, so a
//! resolver never sees more than one entry and the rows that count batches
//! cannot be ported on top of it. The shapes that do not count batches — a
//! tagged resolver preserving typed errors, defects, and interrupts, and
//! answering every submitted request — are what this module ports.

use std::marker::PhantomData;
use std::sync::Arc;

use crate::{Cause, Defect, Effect};

/// A request: a value that names work, plus the types of its answer and its
/// failure.
///
/// `Clone` because a resolver receives copies of what was submitted (the pin's
/// `Entry.request` is the submitted value); `tag` because a tagged resolver
/// dispatches on it.
pub trait Request: Clone + Send + Sync + 'static {
    /// What the resolver answers for this request.
    type Success: Send + 'static;
    /// How the resolver fails this request.
    type Error: Send + 'static;
    /// The tag a [`TaggedResolver`] dispatches on.
    fn tag(&self) -> &'static str;
}

/// One submitted request, as the resolver sees it.
#[derive(Clone, Debug)]
pub struct Entry<R: Request> {
    /// The request itself.
    pub request: R,
}

impl<R: Request> Entry<R> {
    /// Wrap a request for submission.
    pub fn new(request: R) -> Self {
        Self { request }
    }
}

/// A resolver for `R`: one call per batch, answering every entry in order.
///
/// A single `Cause` covers the whole batch: a resolver that fails fails every
/// entry it was given, which is the pin's `fromEffectTagged` shape. A per-entry
/// answer arrives with the batching executor.
pub trait Resolver<R: Request>: Send + Sync + 'static {
    /// Answer one batch. `Ok` carries one value per entry, in entry order.
    fn run(&self, entries: &[Entry<R>]) -> Result<Vec<R::Success>, Cause<R::Error>>;
}

/// The step `request` builds: run the resolver for this one request.
struct RequestStep<R: Request, Res: Resolver<R>> {
    resolver: Arc<Res>,
    request: R,
    marker: PhantomData<fn() -> R>,
}

impl<R, Res> crate::effect::ErasedStep<R::Success, R::Error> for RequestStep<R, Res>
where
    R: Request,
    Res: Resolver<R>,
{
    fn run_erased(
        self: Box<Self>,
        _scope: &crate::service::Services,
    ) -> Result<R::Success, Cause<R::Error>> {
        let Self {
            resolver, request, ..
        } = *self;
        match resolver.run(&[Entry::new(request)]) {
            Ok(mut values) => match values.len() {
                1 => Ok(values.remove(0)),
                _ => Err(Cause::die(Defect::new(
                    "a resolver answered a different number of requests than it was given",
                ))),
            },
            Err(cause) => Err(cause),
        }
    }
}

/// `Effect.request` at the pin: submit `request` to `resolver` and answer with
/// its result.
pub fn request<R, Res>(resolver: Arc<Res>, request: R) -> Effect<R::Success, R::Error>
where
    R: Request,
    Res: Resolver<R>,
{
    Effect {
        step: crate::effect::Step::Suspend(Box::new(RequestStep {
            resolver,
            request,
            marker: PhantomData,
        })),
        services: crate::service::empty_services(),
        marker: PhantomData,
    }
}

/// The handler a [`TaggedResolver`] runs for one tag: the requests carrying
/// that tag, in submission order, answered with one value each.
pub type Handler<R> =
    Box<dyn Fn(&[R]) -> Result<Vec<<R as Request>::Success>, Cause<<R as Request>::Error>> + Send + Sync>;

/// `RequestResolver.fromEffectTagged` at the pin: a resolver built from one
/// handler per tag.
///
/// Each handler receives only the requests carrying its own tag, in submission
/// order, and its answer is reassembled into entry order. A missing handler is
/// a defect, not a silent skip: the port's fail-closed answer to a tag nobody
/// claimed.
pub struct TaggedResolver<R: Request> {
    handlers: Vec<(&'static str, Handler<R>)>,
}

impl<R: Request> Default for TaggedResolver<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Request> TaggedResolver<R> {
    /// A resolver with no handlers yet.
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    /// Claim one tag.
    pub fn on<F>(mut self, tag: &'static str, handler: F) -> Self
    where
        F: Fn(&[R]) -> Result<Vec<R::Success>, Cause<R::Error>> + Send + Sync + 'static,
    {
        self.handlers.push((tag, Box::new(handler)));
        self
    }
}

impl<R: Request> Resolver<R> for TaggedResolver<R> {
    fn run(&self, entries: &[Entry<R>]) -> Result<Vec<R::Success>, Cause<R::Error>> {
        let mut answers: Vec<Option<R::Success>> = entries.iter().map(|_| None).collect();
        let mut tags: Vec<&'static str> = Vec::new();
        for entry in entries {
            let tag = entry.request.tag();
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }
        for tag in tags {
            let Some((_, handler)) = self.handlers.iter().find(|(claimed, _)| *claimed == tag) else {
                return Err(Cause::die(Defect::new(format!(
                    "no handler for request tag: {tag}"
                ))));
            };
            let indexes: Vec<usize> = entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.request.tag() == tag)
                .map(|(index, _)| index)
                .collect();
            let batch: Vec<R> = indexes
                .iter()
                .map(|index| entries[*index].request.clone())
                .collect();
            let values = handler(&batch)?;
            if values.len() != indexes.len() {
                return Err(Cause::die(Defect::new(format!(
                    "resolver for `{tag}` answered {} requests, was given {}",
                    values.len(),
                    indexes.len()
                ))));
            }
            for (index, value) in indexes.into_iter().zip(values) {
                answers[index] = Some(value);
            }
        }
        Ok(answers
            .into_iter()
            .map(|slot| slot.expect("every entry answered"))
            .collect())
    }
}
