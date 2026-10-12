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
use std::sync::{Arc, Mutex};

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

/// Provided services, threaded through interpretation.
///
/// Tags accumulate with conflict detection: provisioning the same tag twice
/// with distinct values poisons it, and any run of the poisoned build Dies
/// (never last-writer-wins). Re-providing the SAME shared value (a layer's
/// memo) is fine, told apart by pointer equality.
/// References override by replacement instead: innermost wins, `R` unchanged.
#[derive(Clone, Default)]
pub(crate) struct ServiceMaps {
    tags: HashMap<&'static str, TagEntry>,
    refs: HashMap<&'static str, Arc<dyn Any + Send + Sync>>,
}

#[derive(Clone)]
struct TagEntry {
    value: Arc<dyn Any + Send + Sync>,
    conflict: bool,
}

pub(crate) type Services = Arc<ServiceMaps>;

pub(crate) fn empty_services() -> Services {
    Arc::new(ServiceMaps::default())
}

impl ServiceMaps {
    /// Insert one tag provision. Same shared value re-provided: no-op.
    /// Distinct value over an existing one: poison (ambiguous).
    pub(crate) fn insert_tag(&mut self, id: &'static str, value: Arc<dyn Any + Send + Sync>) {
        match self.tags.get(&id) {
            None => {
                self.tags.insert(
                    id,
                    TagEntry {
                        value,
                        conflict: false,
                    },
                );
            }
            Some(existing) if Arc::ptr_eq(&existing.value, &value) => {}
            Some(_) => {
                if let Some(entry) = self.tags.get_mut(&id) {
                    entry.conflict = true;
                }
            }
        }
    }

    /// Insert one tag provision as an override: replaces any existing value
    /// and never poisons. For the ambient scope tag `Effect::scoped` mints,
    /// a second distinct value is the intended layering (an inner scope
    /// shadows an outer one for its own subtree), not the ambiguity
    /// `insert_tag` rejects.
    pub(crate) fn replace_tag(&mut self, id: &'static str, value: Arc<dyn Any + Send + Sync>) {
        self.tags.insert(
            id,
            TagEntry {
                value,
                conflict: false,
            },
        );
    }

    /// Insert one reference override. Replaces: innermost wins.
    pub(crate) fn insert_ref(&mut self, id: &'static str, value: Arc<dyn Any + Send + Sync>) {
        self.refs.insert(id, value);
    }

    /// First conflicted tag id, if any.
    pub(crate) fn conflict(&self) -> Option<&'static str> {
        self.conflicted_ids().into_iter().next()
    }

    /// Every tag id already found ambiguous.
    pub(crate) fn conflicted_ids(&self) -> Vec<&'static str> {
        self.tags
            .iter()
            .filter_map(|(id, entry)| entry.conflict.then_some(*id))
            .collect()
    }

    /// Mark an existing tag entry ambiguous without touching its value. For
    /// merges: `insert_tag` can only spot a conflict BETWEEN two sides, so a
    /// conflict that lived inside one side has to be carried over or the
    /// merged build quietly succeeds where the law says Die.
    pub(crate) fn poison(&mut self, id: &'static str) {
        if let Some(entry) = self.tags.get_mut(&id) {
            entry.conflict = true;
        }
    }

    /// Cloned tag value, if present (conflicts observed separately).
    pub(crate) fn get_tag(&self, id: &'static str) -> Option<Arc<dyn Any + Send + Sync>> {
        self.tags.get(id).map(|entry| Arc::clone(&entry.value))
    }

    /// Every tag provision, for layer merges. The conflict flag rides along:
    /// a replay that takes only the value cannot rediscover a conflict that
    /// lived inside this map, and a dropped conflict is a build that quietly
    /// succeeds where the law says Die.
    pub(crate) fn tag_list(&self) -> Vec<(&'static str, Arc<dyn Any + Send + Sync>, bool)> {
        self.tags
            .iter()
            .map(|(id, entry)| (*id, Arc::clone(&entry.value), entry.conflict))
            .collect()
    }

    /// Cloned reference override, innermost wins after merging.
    pub(crate) fn get_ref(&self, id: &'static str) -> Option<Arc<dyn Any + Send + Sync>> {
        self.refs.get(id).map(Arc::clone)
    }

    /// Total provisioned entries, for debug summaries.
    pub(crate) fn len(&self) -> usize {
        self.tags.len() + self.refs.len()
    }
}

/// Read an effect's services merged over its parent scope: provisions
/// accumulate (conflicts accounted), references shadow outward-in.
pub(crate) fn merged_services(outer: &Services, own: &Services) -> Services {
    if outer.tags.is_empty() && outer.refs.is_empty() {
        return Arc::clone(own);
    }
    if own.tags.is_empty() && own.refs.is_empty() {
        return Arc::clone(outer);
    }
    // `outer` arrives by clone, so its own conflict flags survive; `own`'s
    // replay through `tag_list`, which carries them.
    let mut merged = (**outer).clone();
    for (id, value, conflicted) in own.tag_list() {
        merged.insert_tag(id, value);
        if conflicted {
            merged.poison(id);
        }
    }
    for (id, value) in own.refs.iter().map(|(k, v)| (*k, Arc::clone(v))) {
        merged.insert_ref(id, value);
    }
    Arc::new(merged)
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
        let mut maps = (*self.services).clone();
        maps.insert_tag(T::ID, Arc::new(service));
        Effect {
            step: self.step,
            services: Arc::new(maps),
            marker: PhantomData,
        }
    }

    /// Provide two head tags at once (type-level only here; `Layer` recipes
    /// compose N-wise).
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

    /// Close tag `T` from a runtime's built services rather than from a
    /// value in hand: `Effect<A, E, T>` becomes `Effect<A, E, ()>`.
    ///
    /// This is the primitive a static handler needs. `provide` needs a
    /// value, and a trait method such as `Interpret::interpret` has no way
    /// to be handed one, so `use_service::<T>()` inside one used to be a
    /// dead end: the tag could never be closed, and the runtime built by
    /// `ManagedRuntime::make` was never consulted at all.
    ///
    /// Fail-closed: a tag the layer did not build is not a silent `None`. It
    /// surfaces as `missing service: <id>` at the run edge, so a runtime
    /// booted with the wrong layer refuses rather than running with a
    /// default. (Providing one tag twice with distinct values is already
    /// impossible through the type-level API, which is why the `ambiguous
    /// tag` branch you can still hit through a layer merge has no public
    /// witness here.)
    pub fn provide_from<T, Rest>(
        self,
        services: &crate::BuiltServices,
    ) -> Effect<A, E, Rest>
    where
        T: ServiceTag,
        R: Provide<T, Rest>,
        Rest: Requirement,
    {
        let merged = crate::service::merged_services(&services.env(), &self.services);
        Effect {
            step: self.step,
            services: merged,
            marker: PhantomData,
        }
    }

    /// Override an ambient [`Reference`] for this effect and its children:
    /// innermost wins, `R` unchanged. Replaces any outer override.
    pub fn provide_reference<T>(self, reference: &'static Reference<T>, value: T) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        let mut maps = (*self.services).clone();
        maps.insert_ref(reference.id, Arc::new(value));
        Effect {
            step: self.step,
            services: Arc::new(maps),
            marker: self.marker,
        }
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

/// Memoized constructors by tag id: the first build wins per instance, and
/// every later request shares the same value (pointer-equal). Separate
/// instances memoize separately: sharing starts once the constructor is
/// wrapped, not at the bare factory function.
pub struct MemoMap {
    built: Mutex<HashMap<&'static str, Arc<dyn Any + Send + Sync>>>,
}

/// Erase a shared value for the memo map.
/// (Function boundary: the coercion site `Arc::clone` expressions cannot use.)
fn erase_arc<T: Send + Sync + 'static>(arc: Arc<T>) -> Arc<dyn Any + Send + Sync> {
    arc
}

impl MemoMap {
    pub fn new() -> Self {
        Self {
            built: Mutex::new(HashMap::new()),
        }
    }

    /// Build once per id; later calls with the same id share the first value
    /// without re-running `build`.
    pub fn get_or_build<T>(&self, id: &'static str, build: impl FnOnce() -> T) -> Arc<T>
    where
        T: Send + Sync + 'static,
    {
        if let Some(shared) = self.built.lock().unwrap().get(id) {
            return Arc::downcast::<T>(Arc::clone(shared))
                .expect("memo type mismatch: id reused for another type");
        }
        let value: Arc<T> = Arc::new(build());
        self.built
            .lock()
            .unwrap()
            .insert(id, erase_arc(Arc::clone(&value)));
        value
    }
}

impl Default for MemoMap {
    fn default() -> Self {
        Self::new()
    }
}

/// An ambient value with a default (log level, locale): read anywhere with
/// no requirement, overridden for a sub-effect with
/// [`Effect::provide_reference`]. Overrides replace: innermost wins.
/// Domain code never reads environment variables here; configuration is
/// parsed at startup into a value the layer closes over.
pub struct Reference<T: Clone + Send + Sync + 'static> {
    pub id: &'static str,
    pub default: T,
}

/// Read an ambient reference: the innermost override, else the default.
/// Requires nothing (`R` stays as it is).
pub fn read_reference<T>(
    reference: &'static Reference<T>,
) -> Effect<T, std::convert::Infallible, ()>
where
    T: Clone + Send + Sync + 'static,
{
    Effect {
        step: crate::effect::Step::ReadRef {
            id: reference.id,
            apply: Box::new(move |found| {
                found
                    .and_then(|any| Arc::downcast::<T>(any).ok())
                    .map(|arc| (*arc).clone())
                    .unwrap_or_else(|| reference.default.clone())
            }),
        },
        services: empty_services(),
        marker: PhantomData,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_tag_shadows_where_insert_tag_poisons() {
        // The ambient scope tag needs innermost-wins. `insert_tag` poisons a
        // tag carrying two distinct values, which is right for a service tag
        // whose provision should be unique, and wrong for the scope: a build
        // that already provisioned one (a layer build inserts every tag it
        // built into the effect's map) must be shadowed, not poisoned.
        let outer: Arc<dyn Any + Send + Sync> = Arc::new(1u64);
        let inner: Arc<dyn Any + Send + Sync> = Arc::new(2u64);

        let mut inserted = ServiceMaps::default();
        inserted.insert_tag("test/tag", Arc::clone(&outer));
        inserted.insert_tag("test/tag", Arc::clone(&inner));
        assert_eq!(inserted.conflict(), Some("test/tag"));

        let mut replaced = ServiceMaps::default();
        replaced.insert_tag("test/tag", Arc::clone(&outer));
        replaced.replace_tag("test/tag", Arc::clone(&inner));
        assert_eq!(replaced.conflict(), None);
        // Innermost wins: a `Use` node reads the newer value.
        let read = replaced.get_tag("test/tag").expect("still provisioned");
        assert_eq!(*read.downcast::<u64>().expect("u64"), 2);
    }
}
