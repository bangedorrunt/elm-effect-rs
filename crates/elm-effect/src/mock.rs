//! `Layer.mock`: a partial test service whose omitted members fail loudly.
//!
//! The pin hands `Layer.mock` a partial object and a Proxy fills every omitted
//! key with a value that dies as an `UnimplementedError` when it is used as an
//! effect, a channel, or a method. Rust cannot read a struct's fields, so the
//! port names the omitted members and fills them through [`Mock`];
//! [`Unimplemented`] is the placeholder, and every shape of it fails.

use crate::Defect;
use crate::channel::Channel;
use crate::effect::Effect;
use crate::take::Take;

/// What exercising a missing member raises (`Layer.test.ts:452`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("UnimplementedError: {service}.{member} is not implemented")]
pub struct UnimplementedError {
    /// The service the member belongs to.
    pub service: &'static str,
    /// The member the implementation left out.
    pub member: &'static str,
}

/// A member a mock layer's implementation left out.
///
/// Every shape fails: [`effect`](Unimplemented::effect) dies when it runs,
/// [`call`](Unimplemented::call) answers that effect, and
/// [`channel`](Unimplemented::channel) fails every take that carries items.
#[derive(Clone, Copy, Debug)]
pub struct Unimplemented {
    service: &'static str,
    member: &'static str,
}

impl Unimplemented {
    /// The placeholder for `member` of `service`.
    pub fn new(service: &'static str, member: &'static str) -> Self {
        Self { service, member }
    }

    /// The error this member fails with.
    pub fn error(&self) -> UnimplementedError {
        UnimplementedError {
            service: self.service,
            member: self.member,
        }
    }

    /// The defect an effect or a method raises.
    pub fn defect(&self) -> Defect {
        Defect::new(self.error().to_string())
    }

    /// The member as an effect: it dies as soon as it runs.
    pub fn effect<A: Send + 'static, E: Send + 'static, R: crate::Requirement>(
        &self,
    ) -> Effect<A, E, R> {
        Effect::die(self.defect())
    }

    /// The member as a method: calling it answers
    /// [`effect`](Unimplemented::effect).
    pub fn call<A: Send + 'static, E: Send + 'static, R: crate::Requirement>(
        &self,
    ) -> impl Fn() -> Effect<A, E, R> + Send + Sync + 'static {
        let defect = self.defect();
        move || Effect::die(defect.clone())
    }

    /// The member as a channel: a take that carries items dies with the
    /// member's defect, an upstream failure passes through, and the end stays
    /// the end. The channel's own `E` is free, because a defect never needed
    /// one.
    pub fn channel<A: Send + 'static, E: Send + 'static, B: Send + 'static>(
        &self,
    ) -> Channel<A, E, B> {
        let defect = self.defect();
        Channel::from_fn(move |take| match take {
            Take::Items(_) => vec![Take::die(defect.clone())],
            Take::Fail(cause) => vec![Take::Fail(cause)],
            Take::End => vec![Take::End],
        })
    }
}

/// A service a mock layer can fill in.
///
/// Implement it beside the service: `fill_missing` replaces each member named
/// in `missing` with the matching [`Unimplemented`] placeholder, which is what
/// the pin's Proxy does for an omitted key. The layer calls it once, when it
/// builds.
pub trait Mock: Sized {
    /// Fill the omitted members and answer the whole service.
    fn fill_missing(self, service: &'static str, missing: &'static [&'static str]) -> Self;
}
