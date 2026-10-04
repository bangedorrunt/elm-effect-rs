//! `Optic.ts` behavior port at `effect@4.0.0` (pin `67ba4e46`).
//!
//! Optics are values that compose: [`Iso`], [`Lens`], [`Prism`],
//! [`Optional`], [`Traversal`], with `get` / `set` / `replace` / `modify`
//! and the fallible `get_result` / `replace_result`. Composition narrows
//! toward the weaker side (anything composed with a [`Traversal`] is a
//! [`Traversal`]; anything composed with an [`Optional`] is at most an
//! [`Optional`]).
//!
//! Deliberate divergences, kept on purpose: struct keys and tuple elements
//! are built with explicit closures (Rust has no string-keyed fields, so
//! there is no `.key("a")` syntax); fallible failures are [`OpticError`],
//! not Schema issues; `undefined`-hole and prototype/`__proto__` rows have
//! no Rust object model and are `skipped-upstream` in the ledger.

use std::collections::HashMap;
use std::sync::Arc;

/// Why a fallible optic operation failed. Ordinary data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpticError {
    /// Nothing is focused here (missing key, index, or variant).
    MissingFocus(String),
    /// A focused value failed a `check` / `refine` predicate.
    FailedCheck(String),
    /// A traversal replacement did not match the focus count.
    LengthMismatch { got: usize, want: usize },
}

impl std::fmt::Display for OpticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFocus(where_) => write!(f, "missing focus at {where_}"),
            Self::FailedCheck(msg) => write!(f, "{msg}"),
            Self::LengthMismatch { got, want } => {
                write!(f, "each: replacement length mismatch: {got} !== {want}")
            }
        }
    }
}

impl std::error::Error for OpticError {}

type Get<S, A> = Arc<dyn Fn(&S) -> A + Send + Sync>;
type Set<S, A> = Arc<dyn Fn(A, &S) -> S + Send + Sync>;
type Preview<S, A> = Arc<dyn Fn(&S) -> Option<A> + Send + Sync>;
type Review<S, A> = Arc<dyn Fn(A) -> S + Send + Sync>;
type GetAll<S, A> = Arc<dyn Fn(&S) -> Vec<A> + Send + Sync>;
type ModifyAll<S, A> = Arc<dyn Fn(&dyn Fn(A) -> A, &S) -> S + Send + Sync>;

fn arc<S, A, F>(f: F) -> Arc<dyn Fn(&S) -> A + Send + Sync>
where
    F: Fn(&S) -> A + Send + Sync + 'static,
{
    Arc::new(f)
}

/// An isomorphism: lossless in both directions.
#[derive(Clone)]
pub struct Iso<S, A> {
    to: Get<S, A>,
    from: Arc<dyn Fn(A) -> S + Send + Sync>,
}

impl<S, A> Iso<S, A>
where
    S: Clone + Send + Sync + 'static,
    A: Clone + Send + Sync + 'static,
{
    /// Build from a forward and backward function.
    pub fn new(
        to: impl Fn(&S) -> A + Send + Sync + 'static,
        from: impl Fn(A) -> S + Send + Sync + 'static,
    ) -> Self {
        Self {
            to: arc(to),
            from: Arc::new(from),
        }
    }

    /// The identity isomorphism (same type both sides).
    pub fn id() -> Iso<S, S>
    where
        S: Clone + Send + Sync + 'static,
    {
        Iso::new(|s: &S| s.clone(), |s: S| s)
    }

    /// Read through the isomorphism.
    pub fn get(&self, source: &S) -> A {
        (self.to)(source)
    }

    /// Write without reading a source.
    pub fn set(&self, value: A) -> S {
        (self.from)(value)
    }

    /// Iso composed with iso stays an iso.
    pub fn compose_iso<B>(&self, other: &Iso<A, B>) -> Iso<S, B>
    where
        B: Clone + Send + Sync + 'static,
    {
        let outer_to = Arc::clone(&self.to);
        let outer_from = Arc::clone(&self.from);
        let inner_to = Arc::clone(&other.to);
        let inner_from = Arc::clone(&other.from);
        Iso {
            to: Arc::new(move |s: &S| inner_to(&outer_to(s))),
            from: Arc::new(move |b: B| outer_from(inner_from(b))),
        }
    }

    /// Iso in front of a lens is a lens.
    pub fn compose_lens<B>(&self, other: &Lens<A, B>) -> Lens<S, B>
    where
        B: Clone + Send + Sync + 'static,
    {
        let to_get = Arc::clone(&self.to);
        let to_set = Arc::clone(&self.to);
        let from = Arc::clone(&self.from);
        let get = Arc::clone(&other.get);
        let set = Arc::clone(&other.set);
        Lens {
            get: Arc::new(move |s: &S| get(&to_get(s))),
            set: Arc::new(move |b: B, s: &S| from(set(b, &to_set(s)))),
        }
    }

    /// Narrow to an [`Optional`].
    pub fn as_optional(&self) -> Optional<S, A>
    where
        S: Clone,
        A: Clone,
    {
        let to = Arc::clone(&self.to);
        let from = Arc::clone(&self.from);
        Optional {
            get_opt: Arc::new(move |s: &S| Some(to(s))),
            set: Arc::new(move |a: A, _: &S| from(a)),
        }
    }
}

/// A lens: exactly one focus, always present.
#[derive(Clone)]
pub struct Lens<S, A> {
    get: Get<S, A>,
    set: Set<S, A>,
}

impl<S, A> Lens<S, A>
where
    S: Clone + Send + Sync + 'static,
    A: Clone + Send + Sync + 'static,
{
    /// Build from a getter and a setter closure (the Rust spelling of `.key`).
    pub fn field(
        get: impl Fn(&S) -> A + Send + Sync + 'static,
        set: impl Fn(A, &S) -> S + Send + Sync + 'static,
    ) -> Self {
        Self {
            get: arc(get),
            set: Arc::new(set),
        }
    }

    /// Read the focus.
    pub fn get(&self, source: &S) -> A {
        (self.get)(source)
    }

    /// Write the focus.
    pub fn replace(&self, value: A, source: &S) -> S {
        (self.set)(value, source)
    }

    /// Map the focus.
    pub fn modify(&self, f: impl Fn(A) -> A, source: &S) -> S
    where
        S: Clone,
    {
        let a = (self.get)(source);
        (self.set)(f(a), source)
    }

    /// Read through a composed optic without naming an intermediate.
    pub fn get_result(&self, source: &S) -> Result<A, OpticError> {
        Ok((self.get)(source))
    }

    /// A lens write never fails.
    pub fn replace_result(&self, value: A, source: &S) -> Result<S, OpticError> {
        Ok((self.set)(value, source))
    }

    /// Lens composed with lens stays a lens.
    pub fn compose<B>(&self, other: &Lens<A, B>) -> Lens<S, B>
    where
        B: Clone + Send + Sync + 'static,
    {
        let get_o = Arc::clone(&self.get);
        let get_o2 = Arc::clone(&self.get);
        let set_o = Arc::clone(&self.set);
        let get_i = Arc::clone(&other.get);
        let set_i = Arc::clone(&other.set);
        Lens {
            get: Arc::new(move |s: &S| get_i(&get_o(s))),
            set: Arc::new(move |b: B, s: &S| {
                let a = get_o2(s);
                set_o(set_i(b, &a), s)
            }),
        }
    }

    /// Lens in front of an iso is a lens.
    pub fn compose_iso<B>(&self, other: &Iso<A, B>) -> Lens<S, B>
    where
        B: Clone + Send + Sync + 'static,
    {
        let get_o = Arc::clone(&self.get);
        let set_o = Arc::clone(&self.set);
        let to = Arc::clone(&other.to);
        let from = Arc::clone(&other.from);
        Lens {
            get: Arc::new(move |s: &S| to(&get_o(s))),
            set: Arc::new(move |b: B, s: &S| set_o(from(b), s)),
        }
    }

    /// Narrow to an [`Optional`].
    pub fn as_optional(&self) -> Optional<S, A>
    where
        S: Clone,
        A: Clone,
    {
        let get = Arc::clone(&self.get);
        let set = Arc::clone(&self.set);
        Optional {
            get_opt: Arc::new(move |s: &S| Some(get(s))),
            set: Arc::new(move |a: A, s: &S| set(a, s)),
        }
    }
}

/// A prism: zero or one focus, with a total constructor back.
#[derive(Clone)]
pub struct Prism<S, A> {
    preview: Preview<S, A>,
    review: Review<S, A>,
}

impl<S, A> Prism<S, A>
where
    S: Clone + Send + Sync + 'static,
    A: Clone + Send + Sync + 'static,
{
    /// Build from a preview and a review closure.
    pub fn new(
        preview: impl Fn(&S) -> Option<A> + Send + Sync + 'static,
        review: impl Fn(A) -> S + Send + Sync + 'static,
    ) -> Self {
        Self {
            preview: Arc::new(preview),
            review: Arc::new(review),
        }
    }

    /// Read the focus, or fail.
    pub fn get_result(&self, source: &S) -> Result<A, OpticError> {
        (self.preview)(source).ok_or_else(|| OpticError::MissingFocus("prism".into()))
    }

    /// Build without reading a source.
    pub fn set(&self, value: A) -> S {
        (self.review)(value)
    }

    /// Write the focus; a missed focus leaves the source unchanged.
    pub fn replace(&self, value: A, source: &S) -> S
    where
        S: Clone,
    {
        match (self.preview)(source) {
            Some(_) => (self.review)(value),
            None => source.clone(),
        }
    }

    /// Fallible write.
    pub fn replace_result(&self, value: A, source: &S) -> Result<S, OpticError>
    where
        S: Clone,
    {
        match (self.preview)(source) {
            Some(_) => Ok((self.review)(value)),
            None => Err(OpticError::MissingFocus("prism".into())),
        }
    }

    /// Map a present focus; absent stays absent.
    pub fn modify(&self, f: impl Fn(A) -> A, source: &S) -> S
    where
        S: Clone,
    {
        match (self.preview)(source) {
            Some(a) => (self.review)(f(a)),
            None => source.clone(),
        }
    }

    /// Prism composed with prism stays a prism.
    pub fn compose<B>(&self, other: &Prism<A, B>) -> Prism<S, B>
    where
        B: Clone + Send + Sync + 'static,
    {
        let prev_o = Arc::clone(&self.preview);
        let rev_o = Arc::clone(&self.review);
        let prev_i = Arc::clone(&other.preview);
        let rev_i = Arc::clone(&other.review);
        Prism {
            preview: Arc::new(move |s: &S| prev_o(s).and_then(|a| prev_i(&a))),
            review: Arc::new(move |b: B| rev_o(rev_i(b))),
        }
    }

    /// Prism in front of an iso is a prism.
    pub fn compose_iso<B>(&self, other: &Iso<A, B>) -> Prism<S, B>
    where
        B: Clone + Send + Sync + 'static,
    {
        let prev = Arc::clone(&self.preview);
        let rev = Arc::clone(&self.review);
        let to = Arc::clone(&other.to);
        let from = Arc::clone(&other.from);
        Prism {
            preview: Arc::new(move |s: &S| prev(s).map(|a| to(&a))),
            review: Arc::new(move |b: B| rev(from(b))),
        }
    }

    /// Narrow to an [`Optional`].
    pub fn as_optional(&self) -> Optional<S, A>
    where
        S: Clone,
        A: Clone,
    {
        let preview = Arc::clone(&self.preview);
        let preview2 = Arc::clone(&self.preview);
        let review = Arc::clone(&self.review);
        Optional {
            get_opt: preview,
            set: Arc::new(move |a: A, s: &S| match preview2(s) {
                Some(_) => review(a),
                None => s.clone(),
            }),
        }
    }

    /// A lens narrowed by a predicate, with an explicit rebuild for review.
    /// This is the honest spelling of `Lens::check`: the setter needs the
    /// base source, so review takes it from a successful preview instead of
    /// inventing one.
    pub fn check_lens(
        base: S,
        lens: &Lens<S, A>,
        pred: impl Fn(&A) -> bool + Send + Sync + 'static,
    ) -> Prism<S, A>
    where
        S: Clone,
        A: Clone,
    {
        let get = Arc::clone(&lens.get);
        let set = Arc::clone(&lens.set);
        Prism {
            preview: Arc::new(move |s: &S| {
                let a = get(s);
                pred(&a).then_some(a)
            }),
            review: Arc::new(move |a: A| set(a, &base)),
        }
    }
}

/// An optional: zero or one focus, with a setter that keeps the source on miss.
#[derive(Clone)]
pub struct Optional<S, A> {
    get_opt: Preview<S, A>,
    set: Set<S, A>,
}

impl<S, A> Optional<S, A>
where
    S: Clone + Send + Sync + 'static,
    A: Clone + Send + Sync + 'static,
{
    /// Build from a preview and a setter closure.
    pub fn new(
        get_opt: impl Fn(&S) -> Option<A> + Send + Sync + 'static,
        set: impl Fn(A, &S) -> S + Send + Sync + 'static,
    ) -> Self {
        Self {
            get_opt: Arc::new(get_opt),
            set: Arc::new(set),
        }
    }

    /// Read the focus, or fail.
    pub fn get_result(&self, source: &S) -> Result<A, OpticError> {
        (self.get_opt)(source).ok_or_else(|| OpticError::MissingFocus("optional".into()))
    }

    /// Write; a missed focus leaves the source unchanged.
    pub fn replace(&self, value: A, source: &S) -> S
    where
        S: Clone,
    {
        match (self.get_opt)(source) {
            Some(_) => (self.set)(value, source),
            None => source.clone(),
        }
    }

    /// Fallible write; a missed focus is an error and keeps nothing.
    pub fn replace_result(&self, value: A, source: &S) -> Result<S, OpticError> {
        match (self.get_opt)(source) {
            Some(_) => Ok((self.set)(value, source)),
            None => Err(OpticError::MissingFocus("optional".into())),
        }
    }

    /// Map a present focus; absent stays absent.
    pub fn modify(&self, f: impl Fn(A) -> A, source: &S) -> S
    where
        S: Clone,
    {
        match (self.get_opt)(source) {
            Some(a) => (self.set)(f(a), source),
            None => source.clone(),
        }
    }

    /// Optional composed with optional stays optional.
    pub fn compose<B>(&self, other: &Optional<A, B>) -> Optional<S, B>
    where
        B: Clone + Send + Sync + 'static,
    {
        let get_o = Arc::clone(&self.get_opt);
        let get_o2 = Arc::clone(&self.get_opt);
        let set_o = Arc::clone(&self.set);
        let get_i = Arc::clone(&other.get_opt);
        let set_i = Arc::clone(&other.set);
        Optional {
            get_opt: Arc::new(move |s: &S| get_o(s).and_then(|a| get_i(&a))),
            set: Arc::new(move |b: B, s: &S| match get_o2(s) {
                Some(a) => set_o(set_i(b, &a), s),
                None => s.clone(),
            }),
        }
    }

    /// Optional in front of an iso stays optional.
    pub fn compose_iso<B>(&self, other: &Iso<A, B>) -> Optional<S, B>
    where
        B: Clone + Send + Sync + 'static,
    {
        let get_o = Arc::clone(&self.get_opt);
        let get_o2 = Arc::clone(&self.get_opt);
        let set_o = Arc::clone(&self.set);
        let to = Arc::clone(&other.to);
        let from = Arc::clone(&other.from);
        Optional {
            get_opt: Arc::new(move |s: &S| get_o(s).map(|a| to(&a))),
            set: Arc::new(move |b: B, s: &S| match get_o2(s) {
                Some(_) => set_o(from(b), s),
                None => s.clone(),
            }),
        }
    }
}

/// A traversal: any number of foci, visited in order.
#[derive(Clone)]
pub struct Traversal<S, A> {
    get_all: GetAll<S, A>,
    modify_all: ModifyAll<S, A>,
}

impl<S, A> Traversal<S, A>
where
    S: Clone + Send + Sync + 'static,
    A: Clone + Send + Sync + 'static,
{
    /// Build from a collect-all and a modify-all closure.
    pub fn new(
        get_all: impl Fn(&S) -> Vec<A> + Send + Sync + 'static,
        modify_all: impl Fn(&dyn Fn(A) -> A, &S) -> S + Send + Sync + 'static,
    ) -> Self {
        Self {
            get_all: Arc::new(get_all),
            modify_all: Arc::new(modify_all),
        }
    }

    /// Collect every focus in order.
    pub fn get_all(&self, source: &S) -> Vec<A> {
        (self.get_all)(source)
    }

    /// Map every focus.
    pub fn modify_all(&self, f: impl Fn(A) -> A, source: &S) -> S {
        (self.modify_all)(&f, source)
    }

    /// Replace every focus; the replacement count must match the focus count.
    pub fn replace_all(&self, values: Vec<A>, source: &S) -> Result<S, OpticError>
    where
        A: Clone,
    {
        let current = (self.get_all)(source);
        if values.len() != current.len() {
            return Err(OpticError::LengthMismatch {
                got: values.len(),
                want: current.len(),
            });
        }
        let at = std::sync::atomic::AtomicUsize::new(0);
        Ok((self.modify_all)(
            &|_| {
                let i = at.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                values[i].clone()
            },
            source,
        ))
    }

    /// Traversal composed with traversal stays a traversal.
    pub fn compose<B>(&self, other: &Traversal<A, B>) -> Traversal<S, B>
    where
        B: Clone + Send + Sync + 'static,
    {
        let get_o = Arc::clone(&self.get_all);
        let mod_o = Arc::clone(&self.modify_all);
        let get_i = Arc::clone(&other.get_all);
        let mod_i = Arc::clone(&other.modify_all);
        Traversal {
            get_all: Arc::new(move |s: &S| get_o(s).iter().flat_map(|a| get_i(a)).collect()),
            modify_all: Arc::new(move |f: &dyn Fn(B) -> B, s: &S| {
                let inner = Arc::clone(&mod_i);
                let g = |a: A| inner(f, &a);
                mod_o(&g, s)
            }),
        }
    }

    /// Traverse every element of a `Vec` (the Rust spelling of `forEach`).
    pub fn vec_each() -> Traversal<Vec<A>, A>
    where
        A: Clone + Send + Sync + 'static,
    {
        Traversal::new(
            |v: &Vec<A>| v.clone(),
            |f: &dyn Fn(A) -> A, v: &Vec<A>| v.iter().cloned().map(f).collect(),
        )
    }

    /// Traverse the values of a `HashMap` in key order.
    pub fn map_values<K>() -> Traversal<HashMap<K, A>, A>
    where
        K: Eq + std::hash::Hash + Clone + Ord + Send + Sync + 'static,
        A: Clone + Send + Sync + 'static,
    {
        Traversal::new(
            |m: &HashMap<K, A>| {
                let mut keys: Vec<&K> = m.keys().collect();
                keys.sort();
                keys.into_iter().map(|k| m[k].clone()).collect()
            },
            |f: &dyn Fn(A) -> A, m: &HashMap<K, A>| {
                m.iter().map(|(k, v)| (k.clone(), f(v.clone()))).collect()
            },
        )
    }

    /// Keep only foci passing `pred` (the Rust spelling of a checked `forEach`).
    pub fn filter(&self, pred: impl Fn(&A) -> bool + Send + Sync + 'static) -> Traversal<S, A> {
        let pred: Arc<dyn Fn(&A) -> bool + Send + Sync> = Arc::new(pred);
        let get = Arc::clone(&self.get_all);
        let modify = Arc::clone(&self.modify_all);
        let pred_get = Arc::clone(&pred);
        let pred_mod = Arc::clone(&pred);
        Traversal {
            get_all: Arc::new(move |s: &S| get(s).into_iter().filter(|a| pred_get(a)).collect()),
            modify_all: Arc::new(move |f: &dyn Fn(A) -> A, s: &S| {
                let g = |a: A| {
                    if pred_mod(&a) { f(a) } else { a }
                };
                modify(&g, s)
            }),
        }
    }
}

/// Focus an `Option` value: `Some` is present, `None` is absent.
pub fn some<B>() -> Prism<Option<B>, B>
where
    B: Clone + Send + Sync + 'static,
{
    Prism::new(|s: &Option<B>| s.clone(), Option::Some)
}

/// Focus the `None` case, carrying unit.
pub fn none<B>() -> Prism<Option<B>, ()>
where
    B: Clone + Send + Sync + 'static,
{
    Prism::new(|s: &Option<B>| s.is_none().then_some(()), |()| None)
}

/// Focus the `Ok` case of a `Result`.
pub fn success<B, E>() -> Prism<Result<B, E>, B>
where
    B: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    Prism::new(|s: &Result<B, E>| s.clone().ok(), Result::Ok)
}

/// Focus the `Err` case of a `Result`.
pub fn failure<B, E>() -> Prism<Result<B, E>, E>
where
    B: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + 'static,
{
    Prism::new(|s: &Result<B, E>| s.clone().err(), Result::Err)
}

/// Focus a `HashMap` entry by key (the Rust spelling of `.at` on records).
pub fn at_map<K, B>(key: K) -> Optional<HashMap<K, B>, B>
where
    K: Eq + std::hash::Hash + Clone + Send + Sync + 'static,
    B: Clone + Send + Sync + 'static,
{
    let key_get = key.clone();
    Optional::new(
        move |m: &HashMap<K, B>| m.get(&key_get).cloned(),
        move |a: B, m: &HashMap<K, B>| {
            let mut next = m.clone();
            next.insert(key.clone(), a);
            next
        },
    )
}

/// Exact-optional `Vec` element: reads miss out of range; writing `Some`
/// at `len` extends; writing `None` truncates the tail from the index.
pub fn at_vec_exact<B>(index: usize) -> Optional<Vec<B>, Option<B>>
where
    B: Clone + Send + Sync + 'static,
{
    Optional::new(
        move |v: &Vec<B>| Some(v.get(index).cloned()),
        move |opt: Option<B>, v: &Vec<B>| {
            let mut next = v.clone();
            match opt {
                Some(a) => {
                    if index < next.len() {
                        next[index] = a;
                    } else if index == next.len() {
                        next.push(a);
                    }
                }
                None => next.truncate(index),
            }
            next
        },
    )
}

/// Focus a `Vec` element by index (the Rust spelling of `.at` on arrays).
/// Out-of-range reads miss; writes leave the source unchanged.
pub fn at_vec<B>(index: usize) -> Optional<Vec<B>, B>
where
    B: Clone + Send + Sync + 'static,
{
    Optional::new(
        move |v: &Vec<B>| v.get(index).cloned(),
        move |a: B, v: &Vec<B>| {
            let mut next = v.clone();
            if let Some(slot) = next.get_mut(index) {
                *slot = a;
            }
            next
        },
    )
}
