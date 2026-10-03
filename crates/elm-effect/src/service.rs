//! Law L5 (tags part): the `R` channel as service tags.
//!
//! A service tag is a zero-sized type with a string id and an associated
//! service type. Providing removes the tag at the type level: `provide`
//! strips the HEAD requirement, so tuples are ordered by provision
//! (`Effect<A, E, (Database, Clock)>` provides `Database` first). This is a
//! deliberate ceiling, not a limitation to route around: tuple impls run to
//! 16 via macro, and `Cons<H, T>` is the escape beyond that.
//!
//! A missing service is a compile error, never a runtime lookup: there is no
//! `TypeId` map anywhere here. (A `DynamicServices` plugin host may exist one
//! day; it would never satisfy a static tag.)

use std::any::Any;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::effect::sealed;
use crate::{Effect, Requirement};

/// A service tag: zero-sized, with a string id and an associated service
/// type. Services are shared by clone between provider and fiber: wrap a
/// non-cloneable resource in an [`Arc`] and serve that.
pub trait ServiceTag: Send + 'static {
    type Service: Clone + Send + Sync + 'static;
    const ID: &'static str;
}

impl<T: ServiceTag> sealed::Sealed for T {}
impl<T: ServiceTag> Requirement for T {}

/// Proof that requirement `R` carries tag `T` at its head, leaving `Rest`.
/// Implemented for tuples (macro, arities 1..=16) and [`Cons`].
pub trait Provide<T, Rest>: Requirement {}

/// Head-cons escape beyond the 16-tuple ceiling.
pub struct Cons<H, T>(PhantomData<fn() -> (H, T)>);

impl<H: Requirement, T: Requirement> sealed::Sealed for Cons<H, T> {}
impl<H: Requirement, T: Requirement> Requirement for Cons<H, T> {}
impl<H: ServiceTag, T: Requirement> Provide<H, T> for Cons<H, T> {}

/// A bare tag provides itself, leaving no requirements.
impl<T: ServiceTag> Provide<T, ()> for T {}

macro_rules! requirement_tuples {
    ($(($($T:ident),+), $Head:ident, ($($Rest:ident),*));+) => {
        $(
            impl<$($T: Requirement),+> sealed::Sealed for ($($T,)+) {}
            impl<$($T: Requirement),+> Requirement for ($($T,)+) {}
            impl<$Head: ServiceTag $(, $Rest: Requirement)*> Provide<$Head, ($($Rest,)*)> for ($($T,)+) {}
        )+
    };
}

requirement_tuples! {
    (T0), T0, ();
    (T0, T1), T0, (T1);
    (T0, T1, T2), T0, (T1, T2);
    (T0, T1, T2, T3), T0, (T1, T2, T3);
    (T0, T1, T2, T3, T4), T0, (T1, T2, T3, T4);
    (T0, T1, T2, T3, T4, T5), T0, (T1, T2, T3, T4, T5);
    (T0, T1, T2, T3, T4, T5, T6), T0, (T1, T2, T3, T4, T5, T6);
    (T0, T1, T2, T3, T4, T5, T6, T7), T0, (T1, T2, T3, T4, T5, T6, T7);
    (T0, T1, T2, T3, T4, T5, T6, T7, T8), T0, (T1, T2, T3, T4, T5, T6, T7, T8);
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9), T0, (T1, T2, T3, T4, T5, T6, T7, T8, T9);
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10), T0, (T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11), T0, (T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12), T0, (T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13), T0, (T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13);
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14), T0, (T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14);
    (T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15), T0, (T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15)
}

/// Provided services, threaded through interpretation. Maps tag id to an
/// erased service value; inner provisions shadow outer ones.
pub(crate) type Services = Arc<HashMap<&'static str, Arc<dyn Any + Send + Sync>>>;

pub(crate) fn empty_services() -> Services {
    Arc::new(HashMap::new())
}

/// Read an effect's services merged over its parent scope: inner wins.
pub(crate) fn merged_services(outer: &Services, own: &Services) -> Services {
    if own.is_empty() {
        Arc::clone(outer)
    } else if outer.is_empty() {
        Arc::clone(own)
    } else {
        let mut map = (**outer).clone();
        map.extend(own.iter().map(|(k, v)| (*k, Arc::clone(v))));
        Arc::new(map)
    }
}

/// Use the service behind tag `T`: the returned description requires `T`
/// in `R`. Free function (not an `Effect` method) so no `Self` params need
/// inferring at the call site.
pub fn use_service<T: ServiceTag>() -> Effect<T::Service, std::convert::Infallible, T> {
    Effect {
        step: crate::effect::Step::Use {
            id: T::ID,
            read: |any| {
                Arc::downcast::<T::Service>(any)
                    .ok()
                    .map(|arc| (*arc).clone())
            },
        },
        services: empty_services(),
        marker: PhantomData,
    }
}

impl<A: Send + 'static, E: Send + 'static, R: Requirement> Effect<A, E, R> {
    /// Provide tag `T`'s service, removing `T` from the requirement tuple at
    /// the type level. The value is stored in the effect's service map;
    /// `Use` nodes resolve it at interpretation.
    pub fn provide<T, Rest>(self, service: T::Service) -> Effect<A, E, Rest>
    where
        T: ServiceTag,
        R: Provide<T, Rest>,
        Rest: Requirement,
    {
        let mut map = (*self.services).clone();
        map.insert(T::ID, Arc::new(service));
        Effect {
            step: self.step,
            services: Arc::new(map),
            marker: PhantomData,
        }
    }

    /// Provide two head tags at once (type-level only here; `Layer` wiring
    /// composes N-wise in the layer bead).
    pub fn provide_merge<T0, T1, Mid, Rest>(
        self,
        s0: T0::Service,
        s1: T1::Service,
    ) -> Effect<A, E, Rest>
    where
        T0: ServiceTag,
        T1: ServiceTag,
        R: Provide<T0, Mid>,
        Mid: Provide<T1, Rest> + Requirement,
        Rest: Requirement,
    {
        self.provide::<T0, Mid>(s0).provide::<T1, Rest>(s1)
    }
}

/// Sequential composition over descriptions. Each `let` binds a success
/// value; typed failures, defects, and interrupts all short-circuit through
/// `flat_map` by construction. (A postfix `?` sugar was tried and dropped:
/// `expr` fragments absorb a trailing `?`, which makes `?`-arms ambiguous
/// in `macro_rules`, and the desugaring is identical anyway.)
#[macro_export]
macro_rules! effect {
    ($e:expr) => {
        $e
    };
    (let $p:pat = $e:expr; $($rest:tt)*) => {
        ($e).flat_map(move |$p| $crate::effect! { $($rest)* })
    };
    ($e:expr; $($rest:tt)*) => {
        ($e).flat_map(move |_| $crate::effect! { $($rest)* })
    };
}
