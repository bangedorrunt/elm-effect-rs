//! Foundation port at `effect@4.0.0` (pin `67ba4e46`): `Predicate`, `Function`,
//! `Equal`, `Equivalence`, `Order`, `Ordering`, `Hash`.
//!
//! Deliberate divergence, kept on purpose: [`Equal`] treats `NaN` as equal to
//! `NaN`; [`f64`]'s `PartialEq` stays IEEE. The two traits answer different
//! questions, so `equal_nan_is_nan` uses [`Equal`], never `==`.
//!
//! `pipe`/`flow` are macros here: Rust has no variadic function, and the
//! macros compose exactly as the pin's functions do. `dual` stays out of
//! scope: one constructor serving both call orders needs runtime arity, which
//! a typed Rust API does not have.
//!
//! Also out of scope on purpose: `Date`/`RegExp`/`Map`/`Set` hashing (no
//! universal object graph in `std`), type-guard refinements (Rust matches do
//! that).
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

// Ordering

/// Flip an ordering: less-than becomes greater-than and back.
pub fn reverse_ordering(ordering: Ordering) -> Ordering {
    ordering.reverse()
}

/// Fold an ordering into a value.
pub fn match_ordering<T>(
    ordering: Ordering,
    on_less_than: impl FnOnce() -> T,
    on_equal: impl FnOnce() -> T,
    on_greater_than: impl FnOnce() -> T,
) -> T {
    match ordering {
        Ordering::Less => on_less_than(),
        Ordering::Equal => on_equal(),
        Ordering::Greater => on_greater_than(),
    }
}

/// First non-equal verdict wins.
pub fn combine_ordering(first: Ordering, second: Ordering) -> Ordering {
    match first {
        Ordering::Equal => second,
        decided => decided,
    }
}

// Predicate

/// A test on `T`: a shared, repeatable boolean function.
/// Shared step-function type for predicates.
type PredicateFn<T> = Arc<dyn Fn(&T) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct Predicate<T> {
    run: PredicateFn<T>,
}

impl<T> Predicate<T> {
    /// Build from a function.
    pub fn make(run: impl Fn(&T) -> bool + Send + Sync + 'static) -> Self {
        Self { run: Arc::new(run) }
    }

    /// Test a value.
    pub fn test(&self, value: &T) -> bool {
        (self.run)(value)
    }

    /// Always true.
    pub fn always() -> Self {
        Self::make(|_| true)
    }

    /// Always false.
    pub fn never() -> Self {
        Self::make(|_| false)
    }
}

impl<T> std::ops::Not for Predicate<T>
where
    T: 'static,
{
    type Output = Self;

    /// Negate via `!predicate`.
    fn not(self) -> Self {
        Self::make(move |value| !self.test(value))
    }
}

impl<T> Predicate<T> {
    /// Both must hold.
    pub fn and(self, other: Self) -> Self
    where
        T: 'static,
    {
        Self::make(move |value| self.test(value) && other.test(value))
    }

    /// Either must hold.
    pub fn or(self, other: Self) -> Self
    where
        T: 'static,
    {
        Self::make(move |value| self.test(value) || other.test(value))
    }

    /// Negation (`Predicate.not`). The name is the pin's; implementing
    /// `std::ops::Not` would give the same spelling a different receiver.
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Self
    where
        T: 'static,
    {
        Self::make(move |value| !self.test(value))
    }

    /// Exactly one holds.
    pub fn xor(self, other: Self) -> Self
    where
        T: 'static,
    {
        Self::make(move |value| self.test(value) ^ other.test(value))
    }

    /// All hold (vacuously true when empty).
    pub fn every(predicates: Vec<Self>) -> Self
    where
        T: 'static,
    {
        Self::make(move |value| predicates.iter().all(|p| p.test(value)))
    }

    /// At least one holds (false when empty).
    pub fn some(predicates: Vec<Self>) -> Self
    where
        T: 'static,
    {
        Self::make(move |value| predicates.iter().any(|p| p.test(value)))
    }

    /// `Predicate.Tuple`: every element predicate must hold for the element
    /// at its position. Checking stops at the first failure.
    pub fn tuple(predicates: Vec<Self>) -> Predicate<Vec<T>>
    where
        T: 'static,
    {
        Predicate::make(move |values: &Vec<T>| {
            if values.len() != predicates.len() {
                return false;
            }
            predicates
                .iter()
                .zip(values.iter())
                .all(|(predicate, value)| predicate.test(value))
        })
    }

    /// `Predicate.Struct`: every field predicate must hold for the field of
    /// the same name. Extra keys are ignored, and the walk stops at the
    /// first failing field. Fields are matched by name, so the input is a
    /// name-keyed record rather than a positional tuple.
    pub fn struct_(fields: Vec<(String, Self)>) -> Predicate<std::collections::BTreeMap<String, T>>
    where
        T: 'static,
    {
        Predicate::make(move |record: &std::collections::BTreeMap<String, T>| {
            fields.iter().all(|(name, predicate)| {
                record.get(name).is_some_and(|value| predicate.test(value))
            })
        })
    }

    /// Test the input through a projection first.
    pub fn map_input<U>(self, f: impl Fn(&U) -> T + Send + Sync + 'static) -> Predicate<U>
    where
        T: 'static,
        U: Send + Sync + 'static,
    {
        Predicate::make(move |value| self.test(&f(value)))
    }
}

impl<T> Predicate<Option<T>> {
    /// Holds for `Some`.
    pub fn is_some() -> Self {
        Self::make(|value| value.is_some())
    }

    /// Holds for `None`.
    pub fn is_none() -> Self {
        Self::make(|value| value.is_none())
    }
}

// Function

/// Return the argument unchanged.
pub fn identity<A>(a: A) -> A {
    a
}

/// A thunk returning `value` (cloned per call).
pub fn constant<A: Clone>(value: A) -> impl Fn() -> A {
    move || value.clone()
}

/// A thunk returning true.
pub fn const_true() -> impl Fn() -> bool {
    || true
}

/// A thunk returning false.
pub fn const_false() -> impl Fn() -> bool {
    || false
}

/// Flip two arguments.
pub fn flip<A, B, C>(f: impl Fn(A, B) -> C) -> impl Fn(B, A) -> C {
    move |b, a| f(a, b)
}

/// `g` after `f`.
pub fn compose<A, B, C>(f: impl Fn(A) -> B, g: impl Fn(B) -> C) -> impl Fn(A) -> C {
    move |a| g(f(a))
}

/// Nothing (`None`), mirroring `constNull`.
pub fn const_null<T>() -> Option<T> {
    None
}

/// Reverse a curried two-argument function. Needs `Clone` because the
/// outer function is shared across applications, and `B: Clone` because
/// each flipped call replays its captured second argument.
pub fn flip_curried<A, B, C, F, G>(f: F) -> impl Fn(B) -> Box<dyn Fn(A) -> C>
where
    F: Fn(A) -> G + Clone + 'static,
    G: Fn(B) -> C + 'static,
    A: 'static,
    B: Clone + 'static,
    C: 'static,
{
    move |b| {
        let f = f.clone();
        Box::new(move |a| {
            let again = b.clone();
            f(a)(again)
        }) as Box<dyn Fn(A) -> C>
    }
}

/// Make a two-argument function take its arguments as one tuple.
pub fn tupled<A, B, C, F>(f: F) -> impl Fn((A, B)) -> C
where
    F: Fn(A, B) -> C,
{
    move |(a, b)| f(a, b)
}

/// Make a tuple-accepting function take two arguments.
pub fn untupled<A, B, C, F>(f: F) -> impl Fn(A, B) -> C
where
    F: Fn((A, B)) -> C,
{
    move |a, b| f((a, b))
}

/// Diverging function for impossible branches.
pub fn absurd<T>(_: std::convert::Infallible) -> T {
    panic!("Called `absurd` function which should be uncallable")
}

/// Apply a value to a function: `apply(a, f)` is `f(a)`.
///
/// The pin curries this as `apply(a)(f)`; Rust has no currying, so the value
/// stays first, the same order the pin's two applications read in.
pub fn apply<A, B>(value: A, f: impl Fn(A) -> B) -> B {
    f(value)
}

/// A thunk returning the unit value.
///
/// The pin has two names for this, `constUndefined` and `constVoid`, because
/// JavaScript spells the value `undefined`; Rust spells it `()`.
pub fn const_unit() -> impl Fn() {
    || ()
}

/// Discard the first argument and return the second (`SK`).
pub fn sk<A, B>(_first: A, second: B) -> B {
    second
}

/// A call that must be unreachable, mirroring the pin's `hole<T>()`.
///
/// # Panics
///
/// Always, with the pin's sentence. The type system cannot rule out every
/// branch, so the branch that is left gets a named failure instead of a wrong
/// value.
pub fn hole<T>() -> T {
    panic!("Called `hole` function which should be uncallable")
}

/// Memoize a computation by `Arc` identity, the pin's `WeakMap` key.
///
/// Structurally equal values do not share an entry. The cache keeps a clone of
/// the key, so its address cannot be reused by a later allocation; that is
/// what makes the pointer a sound key.
///
/// ponytail: entries live as long as the memoizer, since a strong key cannot
/// be collected. An LRU or a `Weak`-keyed store if a long-lived process
/// memoizes many transient values.
pub fn memoize<A, O>(f: impl Fn(&A) -> O) -> impl Fn(&Arc<A>) -> O
where
    O: Clone,
{
    let cache: RefCell<HashMap<usize, (Arc<A>, O)>> = RefCell::new(HashMap::new());
    move |a: &Arc<A>| {
        let key = Arc::as_ptr(a) as usize;
        if let Some((_, cached)) = cache.borrow().get(&key) {
            return cached.clone();
        }
        let out = f(a);
        cache
            .borrow_mut()
            .insert(key, (Arc::clone(a), out.clone()));
        out
    }
}

/// Memoize an idempotent transform, caching each output as a fixed point.
///
/// Calling the result with an input or with the output it produced returns the
/// output without running `f` again.
pub fn memoize_idempotent<A>(f: impl Fn(&Arc<A>) -> Arc<A>) -> impl Fn(&Arc<A>) -> Arc<A> {
    let cache: RefCell<HashMap<usize, (Arc<A>, Arc<A>)>> = RefCell::new(HashMap::new());
    move |a: &Arc<A>| {
        let key = Arc::as_ptr(a) as usize;
        if let Some((_, cached)) = cache.borrow().get(&key) {
            return Arc::clone(cached);
        }
        let out = f(a);
        let out_key = Arc::as_ptr(&out) as usize;
        let mut cache = cache.borrow_mut();
        cache.insert(key, (Arc::clone(a), Arc::clone(&out)));
        if out_key != key {
            cache.insert(out_key, (Arc::clone(&out), Arc::clone(&out)));
        }
        out
    }
}

/// Apply a value through a chain of functions, left to right.
///
/// `pipe!(x)` is `x`; `pipe!(x, f, g)` is `g(f(x))`. The pin's `pipe` is
/// variadic, and a Rust function is not, so the chain is a macro.
#[macro_export]
macro_rules! pipe {
    ($value:expr $(,)?) => {
        $value
    };
    ($value:expr, $first:expr $(,)?) => {
        ($first)($value)
    };
    ($value:expr, $first:expr, $($rest:expr),+ $(,)?) => {
        $crate::pipe!(($first)($value), $($rest),+)
    };
}

/// Compose functions left to right into one function.
///
/// `flow!(f)` is `f`; `flow!(f, g)` is `|x| g(f(x))`. A head that takes more
/// than one argument takes them as one tuple, since a Rust closure has a fixed
/// arity: `flow!(|(a, b): (i32, i32)| a + b, double)((1, 2))`.
#[macro_export]
macro_rules! flow {
    ($f:expr $(,)?) => {
        $f
    };
    ($f:expr, $g:expr $(,)?) => {
        move |a| ($g)(($f)(a))
    };
    ($f:expr, $g:expr, $($rest:expr),+ $(,)?) => {
        $crate::flow!(move |a| ($g)(($f)(a)), $($rest),+)
    };
}

// Hash

/// Structural hash with the pin's exact formulas (MurmurHash3 finalizer,
/// djb2 strings, canonical-`NaN` floats). Implemented alongside [`Equal`]:
/// equal values hash equally.
pub trait Hash {
    /// The structural hash.
    fn hash_value(&self) -> u32;
}

/// MurmurHash3 32-bit finalizer.
fn mix(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85ebca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2ae35);
    h ^ (h >> 16)
}

/// Keep hashes in the non-negative 31-bit range.
fn optimize(n: u32) -> u32 {
    (n & 0xbfffffff) | ((n >> 1) & 0x40000000)
}

/// Mix two hashes into one.
pub fn combine_hash(first: u32, second: u32) -> u32 {
    mix(first
        .wrapping_mul(0x9e3779b1)
        .wrapping_add(second.wrapping_mul(0x85ebca6b)))
}

/// Hash a string: djb2 over UTF-16 units from the end, then optimized.
/// BMP-identical to the pin; astral characters hash by scalar value here.
pub fn hash_str(s: &str) -> u32 {
    let mut units: Vec<u16> = s.encode_utf16().collect();
    let mut h: u32 = 5381;
    while let Some(unit) = units.pop() {
        h = h.wrapping_mul(33) ^ u32::from(unit);
    }
    optimize(h)
}

/// Hash a float: integers take the fast path; anything else hashes its
/// big-endian halves, with every `NaN` normalized first.
pub fn hash_f64(n: f64) -> u32 {
    if n.fract() == 0.0 && (-2147483648.0..=2147483647.0).contains(&n) {
        return optimize(n as i32 as u32);
    }
    let canonical = if n.is_nan() { f64::NAN } else { n };
    let bits = canonical.to_bits();
    combine_hash((bits >> 32) as u32, bits as u32)
}

impl Hash for bool {
    fn hash_value(&self) -> u32 {
        optimize(mix(hash_str(if *self { "true" } else { "false" })))
    }
}

impl Hash for i32 {
    fn hash_value(&self) -> u32 {
        optimize(*self as u32)
    }
}

impl Hash for u32 {
    fn hash_value(&self) -> u32 {
        optimize(*self)
    }
}

impl Hash for i64 {
    fn hash_value(&self) -> u32 {
        hash_f64(*self as f64)
    }
}

impl Hash for u64 {
    fn hash_value(&self) -> u32 {
        hash_f64(*self as f64)
    }
}

impl Hash for f32 {
    fn hash_value(&self) -> u32 {
        hash_f64(f64::from(*self))
    }
}

impl Hash for f64 {
    fn hash_value(&self) -> u32 {
        hash_f64(*self)
    }
}

impl Hash for String {
    fn hash_value(&self) -> u32 {
        hash_str(self)
    }
}

impl Hash for str {
    fn hash_value(&self) -> u32 {
        hash_str(self)
    }
}

impl<A: Hash> Hash for Option<A> {
    fn hash_value(&self) -> u32 {
        match self {
            None => optimize(mix(hash_str("None"))),
            Some(value) => combine_hash(hash_str("Some"), value.hash_value()),
        }
    }
}

impl<A: Hash> Hash for Vec<A> {
    fn hash_value(&self) -> u32 {
        let mut h = 6151u32;
        for element in self {
            h = combine_hash(h, element.hash_value());
        }
        optimize(h)
    }
}

impl<A: Hash, B: Hash> Hash for (A, B) {
    fn hash_value(&self) -> u32 {
        combine_hash(self.0.hash_value(), self.1.hash_value())
    }
}

// Equal

/// Structural equality with its own law, separate from `PartialEq`:
/// `NaN` equals `NaN`, while [`f64`]'s `==` stays IEEE.
pub trait Equal: Hash {
    /// Whether both values are structurally equal.
    fn equal(&self, other: &Self) -> bool;
}

impl Equal for bool {
    fn equal(&self, other: &Self) -> bool {
        self == other
    }
}

impl Equal for i32 {
    fn equal(&self, other: &Self) -> bool {
        self == other
    }
}

impl Equal for u32 {
    fn equal(&self, other: &Self) -> bool {
        self == other
    }
}

impl Equal for i64 {
    fn equal(&self, other: &Self) -> bool {
        self == other
    }
}

impl Equal for u64 {
    fn equal(&self, other: &Self) -> bool {
        self == other
    }
}

impl Equal for f32 {
    fn equal(&self, other: &Self) -> bool {
        self == other || (self.is_nan() && other.is_nan())
    }
}

impl Equal for f64 {
    fn equal(&self, other: &Self) -> bool {
        self == other || (self.is_nan() && other.is_nan())
    }
}

impl Equal for String {
    fn equal(&self, other: &Self) -> bool {
        self == other
    }
}

impl<A: Equal> Equal for Option<A> {
    fn equal(&self, other: &Self) -> bool {
        match (self, other) {
            (None, None) => true,
            (Some(a), Some(b)) => a.equal(b),
            _ => false,
        }
    }
}

impl<A: Equal> Equal for Vec<A> {
    fn equal(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().zip(other).all(|(a, b)| a.equal(b))
    }
}

impl<A: Equal, B: Equal> Equal for (A, B) {
    fn equal(&self, other: &Self) -> bool {
        self.0.equal(&other.0) && self.1.equal(&other.1)
    }
}

// Keyed collections. The pin's `Equal.equals` compares a JavaScript `Map`
// and `Set` entry by entry, independent of insertion order, so these read
// as the port's ordered spellings of the same two shapes.

impl<K: Equal + Ord, V: Equal> Hash for std::collections::BTreeMap<K, V> {
    fn hash_value(&self) -> u32 {
        // Order-independent: a `Map` built in either insertion order hashes
        // the same, which is what the equal-values-hash-equally law needs.
        self.iter().fold(0u32, |acc, (key, value)| {
            acc.wrapping_add(combine_hash(key.hash_value(), value.hash_value()))
        })
    }
}

impl<K: Equal + Ord, V: Equal> Equal for std::collections::BTreeMap<K, V> {
    fn equal(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        // One-to-one matching rather than a zip: two keys that compare equal
        // need not sort together, and a key may not be reused by two entries.
        let mut matched = vec![false; other.len()];
        let others: Vec<(&K, &V)> = other.iter().collect();
        'entries: for (key, value) in self {
            for (index, (other_key, other_value)) in others.iter().enumerate() {
                if !matched[index] && key.equal(other_key) && value.equal(other_value) {
                    matched[index] = true;
                    continue 'entries;
                }
            }
            return false;
        }
        true
    }
}

impl<K: Equal + Ord> Hash for std::collections::BTreeSet<K> {
    fn hash_value(&self) -> u32 {
        self.iter()
            .fold(0u32, |acc, key| acc.wrapping_add(key.hash_value()))
    }
}

impl<K: Equal + Ord> Equal for std::collections::BTreeSet<K> {
    fn equal(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        // Same one-to-one rule as the map's keys, for the same reason.
        let mut matched = vec![false; other.len()];
        let others: Vec<&K> = other.iter().collect();
        'members: for key in self {
            for (index, other_key) in others.iter().enumerate() {
                if !matched[index] && key.equal(other_key) {
                    matched[index] = true;
                    continue 'members;
                }
            }
            return false;
        }
        true
    }
}

// Equivalence

/// An equivalence relation on `T`: a shared, repeatable boolean test.
/// Shared equivalence-test type.
type EquivalenceFn<T> = Arc<dyn Fn(&T, &T) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct Equivalence<T> {
    is_equivalent: EquivalenceFn<T>,
}

impl<T> Equivalence<T> {
    /// Build from a test.
    pub fn make(test: impl Fn(&T, &T) -> bool + Send + Sync + 'static) -> Self {
        Self {
            is_equivalent: Arc::new(test),
        }
    }

    /// Test a pair.
    pub fn is_equivalent(&self, first: &T, second: &T) -> bool {
        (self.is_equivalent)(first, second)
    }

    /// Both must hold.
    pub fn combine(self, other: Self) -> Self
    where
        T: 'static,
    {
        Self::make(move |a, b| self.is_equivalent(a, b) && other.is_equivalent(a, b))
    }

    /// Every member must hold.
    pub fn combine_all(members: Vec<Self>) -> Self
    where
        T: 'static,
    {
        Self::make(move |a, b| members.iter().all(|m| m.is_equivalent(a, b)))
    }

    /// Test the inputs through a projection first.
    pub fn map_input<U>(self, f: impl Fn(&U) -> T + Send + Sync + 'static) -> Equivalence<U>
    where
        T: 'static,
    {
        Equivalence::make(move |a, b| self.is_equivalent(&f(a), &f(b)))
    }
}

impl<T: PartialEq> Equivalence<T> {
    /// Strict (`==`) equivalence.
    pub fn strict() -> Self {
        Self::make(|a, b| a == b)
    }
}

/// String equivalence.
pub fn string_equivalence() -> Equivalence<String> {
    Equivalence::make(|a, b| a == b)
}

/// Number equivalence: `NaN` is equivalent to `NaN`.
pub fn number_equivalence() -> Equivalence<f64> {
    Equivalence::make(|a: &f64, b: &f64| a == b || (a.is_nan() && b.is_nan()))
}

/// Boolean equivalence.
pub fn boolean_equivalence() -> Equivalence<bool> {
    Equivalence::make(|a, b| a == b)
}

/// Pairwise tuple equivalence.
pub fn tuple_equivalence<A: Equal, B: Equal>() -> Equivalence<(A, B)> {
    Equivalence::make(|a: &(A, B), b: &(A, B)| a.equal(b))
}

/// Element-wise vector equivalence (order and length matter).
pub fn array_equivalence<A: Equal>() -> Equivalence<Vec<A>> {
    Equivalence::make(|a: &Vec<A>, b: &Vec<A>| a.equal(b))
}

// Order

/// A total preorder on `T`: a shared, repeatable comparison.
/// Shared comparison type.
type CompareFn<T> = Arc<dyn Fn(&T, &T) -> Ordering + Send + Sync>;

#[derive(Clone)]
pub struct Order<T> {
    compare: CompareFn<T>,
}

impl<T> Order<T> {
    /// Build from a comparator.
    pub fn make(compare: impl Fn(&T, &T) -> Ordering + Send + Sync + 'static) -> Self {
        Self {
            compare: Arc::new(compare),
        }
    }

    /// Compare two values.
    pub fn compare(&self, first: &T, second: &T) -> Ordering {
        (self.compare)(first, second)
    }

    /// Flip the direction.
    pub fn reverse(self) -> Self
    where
        T: 'static,
    {
        Self::make(move |a, b| self.compare(a, b).reverse())
    }

    /// First non-equal verdict wins.
    pub fn combine(self, other: Self) -> Self
    where
        T: 'static,
    {
        Self::make(move |a, b| combine_ordering(self.compare(a, b), other.compare(a, b)))
    }

    /// Compare through a projection first.
    pub fn map_input<U>(self, f: impl Fn(&U) -> T + Send + Sync + 'static) -> Order<U>
    where
        T: 'static,
    {
        Order::make(move |a, b| self.compare(&f(a), &f(b)))
    }

    /// The smaller value (ties go to the second, per the pin).
    pub fn min(&self, first: T, second: T) -> T
    where
        T: Clone,
    {
        match self.compare(&first, &second) {
            Ordering::Greater => second,
            _ => first,
        }
    }

    /// The larger value (ties go to the first, per the pin).
    pub fn max(&self, first: T, second: T) -> T
    where
        T: Clone,
    {
        match self.compare(&first, &second) {
            Ordering::Less => second,
            _ => first,
        }
    }

    /// Clamp between both bounds.
    pub fn clamp(&self, value: T, minimum: T, maximum: T) -> T
    where
        T: Clone,
    {
        self.max(self.min(value, maximum), minimum)
    }

    /// Whether the first is strictly less.
    pub fn is_less_than(&self, first: &T, second: &T) -> bool {
        matches!(self.compare(first, second), Ordering::Less)
    }

    /// Whether the first is less or ties (ties report `Greater` under the
    /// pin, so this is true only for strictly smaller values there too).
    pub fn is_less_than_or_equal(&self, first: &T, second: &T) -> bool {
        !matches!(self.compare(first, second), Ordering::Greater)
    }

    /// Whether the first is less or equal (never equal under the pin's
    /// number/string orders, which return `Greater` on ties).
    pub fn is_greater_than(&self, first: &T, second: &T) -> bool {
        matches!(self.compare(first, second), Ordering::Greater)
    }

    /// Whether the first is greater or ties.
    pub fn is_greater_than_or_equal(&self, first: &T, second: &T) -> bool {
        !matches!(self.compare(first, second), Ordering::Less)
    }

    /// Whether the value sits inside both bounds.
    pub fn is_between(&self, value: &T, minimum: &T, maximum: &T) -> bool {
        self.is_greater_than_or_equal(value, minimum) && self.is_less_than_or_equal(value, maximum)
    }

    /// Fold many criteria: first non-equal verdict wins.
    pub fn combine_all(members: Vec<Self>) -> Self
    where
        T: 'static,
    {
        Self::make(move |a, b| {
            let mut out = Ordering::Equal;
            for member in &members {
                out = combine_ordering(out, member.compare(a, b));
                if out != Ordering::Equal {
                    break;
                }
            }
            out
        })
    }
}

/// String order (byte-wise; ties report `Greater`, per the pin).
pub fn string_order() -> Order<String> {
    Order::make(|a: &String, b: &String| {
        if a < b {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    })
}

/// Number order: `NaN` equals `NaN`, `NaN` sorts below everything, ties
/// report `Greater`, per the pin.
pub fn number_order() -> Order<f64> {
    Order::make(|a: &f64, b: &f64| {
        if a.is_nan() && b.is_nan() {
            Ordering::Equal
        } else if a.is_nan() {
            Ordering::Less
        } else if b.is_nan() {
            Ordering::Greater
        } else if a < b {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    })
}

/// Boolean order (`false` below `true`; ties report `Greater`, per the pin).
pub fn boolean_order() -> Order<bool> {
    Order::make(|a: &bool, b: &bool| {
        if a < b {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    })
}

/// Order from Rust's own [`Ord`].
pub fn ord_order<T: Ord>() -> Order<T> {
    Order::make(|a: &T, b: &T| a.cmp(b))
}

// Data

/// A tagged struct: a `_tag` discriminator plus a value. The `TaggedClass`
/// side of the pin; tagged unions (`taggedEnum`) are plain Rust enums
/// matched with [`Matcher`] or `match`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tagged<A> {
    /// The discriminator.
    pub tag: &'static str,
    /// The payload.
    pub value: A,
}

/// Build a tagged struct.
pub fn tagged<A>(tag: &'static str, value: A) -> Tagged<A> {
    Tagged { tag, value }
}

impl<A: Equal> Equal for Tagged<A> {
    fn equal(&self, other: &Self) -> bool {
        self.tag == other.tag && self.value.equal(&other.value)
    }
}

impl<A: Hash> Hash for Tagged<A> {
    fn hash_value(&self) -> u32 {
        combine_hash(hash_str(self.tag), self.value.hash_value())
    }
}

// Match

/// A value matcher: ordered cases plus a fallback. `when`-style guards come
/// from [`Predicate`]; field matching from [`Matcher::discriminator`].
/// One matcher case: a guard plus its output function.
type MatchCase<I, O> = (Predicate<I>, Box<dyn Fn(&I) -> O + Send + Sync>);

pub struct Matcher<I, O> {
    input: I,
    cases: Vec<MatchCase<I, O>>,
}

impl<I, O> Matcher<I, O> {
    /// Match this input value.
    pub fn value(input: I) -> Self {
        Self {
            input,
            cases: Vec::new(),
        }
    }

    /// Add a case: when `predicate` holds, `then` decides the output.
    pub fn when(
        mut self,
        predicate: Predicate<I>,
        then: impl Fn(&I) -> O + Send + Sync + 'static,
    ) -> Self
    where
        I: 'static,
    {
        self.cases.push((predicate, Box::new(then)));
        self
    }

    /// Add a case when either predicate holds.
    pub fn when_or(
        self,
        first: Predicate<I>,
        second: Predicate<I>,
        then: impl Fn(&I) -> O + Send + Sync + 'static,
    ) -> Self
    where
        I: 'static,
    {
        self.when(first.or(second), then)
    }

    /// Add a case when both predicates hold.
    pub fn when_and(
        self,
        first: Predicate<I>,
        second: Predicate<I>,
        then: impl Fn(&I) -> O + Send + Sync + 'static,
    ) -> Self
    where
        I: 'static,
    {
        self.when(first.and(second), then)
    }

    /// Add a case on a projected value (e.g. a `_tag` field).
    pub fn discriminator<V>(
        self,
        get: impl Fn(&I) -> V + Send + Sync + 'static,
        value: V,
        then: impl Fn(&I) -> O + Send + Sync + 'static,
    ) -> Self
    where
        I: 'static,
        V: PartialEq + Send + Sync + 'static,
    {
        self.when(Predicate::make(move |input| get(input) == value), then)
    }

    /// Add a case when the projected string starts with `prefix`.
    pub fn discriminator_starts_with(
        self,
        get: impl Fn(&I) -> String + Send + Sync + 'static,
        prefix: &str,
        then: impl Fn(&I) -> O + Send + Sync + 'static,
    ) -> Self
    where
        I: 'static,
    {
        let prefix = prefix.to_string();
        self.when(
            Predicate::make(move |input| get(input).starts_with(&prefix)),
            then,
        )
    }

    /// The first matching case, if any.
    pub fn option(self) -> Option<O> {
        for (predicate, then) in &self.cases {
            if predicate.test(&self.input) {
                return Some(then(&self.input));
            }
        }
        None
    }

    /// The first matching case, or the default.
    pub fn or_else(self, default: O) -> O {
        self.option().unwrap_or(default)
    }

    /// The first matching case. Panics when nothing matches, like the pin's
    /// `exhaustive` throwing on a non-exhaustive match.
    pub fn exhaustive(self) -> O {
        self.option().expect("non-exhaustive match")
    }
}

// Filter

/// A narrowing check: `I` in, `P` or `F` out. Composes with [`and`](Filter::and)
/// / [`or`](Filter::or); unknown-narrowing (`string`, `number`) is `match`
/// territory in Rust and intentionally absent here.
/// A narrowing check function.
type FilterFn<I, P, F> = Arc<dyn Fn(&I) -> Result<P, F> + Send + Sync>;

#[derive(Clone)]
pub struct Filter<I, P, F> {
    run: FilterFn<I, P, F>,
}

impl<I, P, F> Filter<I, P, F> {
    /// Build from a function.
    pub fn make(run: impl Fn(&I) -> Result<P, F> + Send + Sync + 'static) -> Self {
        Self { run: Arc::new(run) }
    }

    /// Run the check.
    pub fn check(&self, input: &I) -> Result<P, F> {
        (self.run)(input)
    }

    /// Pass inputs satisfying the predicate, fail the rest with the input.
    pub fn from_predicate(predicate: Predicate<I>) -> Filter<I, I, I>
    where
        I: Clone + 'static,
    {
        Filter::make(move |input| {
            if predicate.test(input) {
                Ok(input.clone())
            } else {
                Err(input.clone())
            }
        })
    }

    /// Read the pass channel as a predicate (fails map to false).
    pub fn to_predicate(self) -> Predicate<I>
    where
        I: Send + Sync + 'static,
        P: Send + Sync + 'static,
        F: Send + Sync + 'static,
    {
        Predicate::make(move |input| self.check(input).is_ok())
    }

    /// Map the fail channel.
    pub fn map_fail<G>(self, f: impl Fn(F) -> G + Send + Sync + 'static) -> Filter<I, P, G>
    where
        I: 'static,
        P: 'static,
        F: 'static,
    {
        Filter::make(move |input| self.check(input).map_err(&f))
    }

    /// Both must pass.
    pub fn and(self, other: Self) -> Self
    where
        I: Clone + 'static,
        P: 'static,
        F: 'static,
    {
        Filter::make(move |input| {
            self.check(input)?;
            other.check(input)
        })
    }

    /// Either may pass; both failures return the second one.
    pub fn or(self, other: Self) -> Self
    where
        I: 'static,
        P: 'static,
        F: 'static,
    {
        Filter::make(move |input| match self.check(input) {
            Ok(pass) => Ok(pass),
            Err(_) => other.check(input),
        })
    }

    /// Pass only values equal to `value`.
    pub fn equals(value: P) -> Filter<P, P, P>
    where
        P: PartialEq + Clone + Send + Sync + 'static,
    {
        Filter::make(move |input: &P| {
            if input == &value {
                Ok(input.clone())
            } else {
                Err(input.clone())
            }
        })
    }
}

// Combiner

/// An associative combination. Laws live with the caller: combine twice in
/// either grouping and get the same result.
/// An associative combination function.
type CombineFn<A> = Arc<dyn Fn(&A, &A) -> A + Send + Sync>;

#[derive(Clone)]
pub struct Combiner<A> {
    combine: CombineFn<A>,
}

impl<A> Combiner<A> {
    /// Build from a function.
    pub fn make(combine: impl Fn(&A, &A) -> A + Send + Sync + 'static) -> Self {
        Self {
            combine: Arc::new(combine),
        }
    }

    /// Combine two values.
    pub fn combine(&self, first: &A, second: &A) -> A {
        (self.combine)(first, second)
    }

    /// Combine with swapped arguments.
    pub fn flip(self) -> Self
    where
        A: 'static,
    {
        Self::make(move |a, b| self.combine(b, a))
    }

    /// Combine under an order: the smaller value wins.
    pub fn min(order: Order<A>) -> Self
    where
        A: Clone + Send + Sync + 'static,
    {
        Self::make(move |a, b| order.min(a.clone(), b.clone()))
    }

    /// Combine under an order: the larger value wins.
    pub fn max(order: Order<A>) -> Self
    where
        A: Clone + Send + Sync + 'static,
    {
        Self::make(move |a, b| order.max(a.clone(), b.clone()))
    }

    /// Always the first value.
    pub fn first() -> Self
    where
        A: Clone + Send + Sync + 'static,
    {
        Self::make(|a, _| a.clone())
    }

    /// Always the last value.
    pub fn last() -> Self
    where
        A: Clone + Send + Sync + 'static,
    {
        Self::make(|_, b| b.clone())
    }

    /// Always this value.
    pub fn constant(value: A) -> Self
    where
        A: Clone + Send + Sync + 'static,
    {
        Self::make(move |_: &A, _: &A| value.clone())
    }

    /// Join strings with a middle separator.
    pub fn intercalate(middle: &str) -> Combiner<String> {
        let middle = middle.to_string();
        Combiner::make(move |a, b| format!("{a}{middle}{b}"))
    }
}

// Reducer

/// A combiner with an empty element: folding starts somewhere neutral.
#[derive(Clone)]
pub struct Reducer<A> {
    empty: A,
    combiner: Combiner<A>,
}

impl<A: Clone + 'static> Reducer<A> {
    /// Build from an empty value and a combination.
    pub fn make(empty: A, combine: impl Fn(&A, &A) -> A + Send + Sync + 'static) -> Self {
        Self {
            empty,
            combiner: Combiner::make(combine),
        }
    }

    /// The neutral element.
    pub fn empty(&self) -> A {
        self.empty.clone()
    }

    /// Combine two values.
    pub fn combine(&self, first: &A, second: &A) -> A {
        self.combiner.combine(first, second)
    }

    /// Fold a slice left to right, starting from empty.
    pub fn fold(&self, values: &[A]) -> A {
        let mut out = self.empty();
        for value in values {
            out = self.combine(&out, value);
        }
        out
    }

    /// Combine with swapped arguments.
    pub fn flip(self) -> Self {
        Self {
            empty: self.empty,
            combiner: self.combiner.flip(),
        }
    }
}

// Differ

/// Patch-based updates: compute a patch between two values, combine patches
/// in order, and apply a patch later.
/// Patch functions for a differ.
type DiffFn<T, Patch> = Arc<dyn Fn(&T, &T) -> Patch + Send + Sync>;
type CombinePatchFn<Patch> = Arc<dyn Fn(&Patch, &Patch) -> Patch + Send + Sync>;
type PatchFn<T, Patch> = Arc<dyn Fn(&T, &Patch) -> T + Send + Sync>;

#[derive(Clone)]
pub struct Differ<T, Patch> {
    empty: Patch,
    diff: DiffFn<T, Patch>,
    combine: CombinePatchFn<Patch>,
    patch: PatchFn<T, Patch>,
}

impl<T, Patch> Differ<T, Patch> {
    /// Build from parts.
    pub fn make(
        empty: Patch,
        diff: impl Fn(&T, &T) -> Patch + Send + Sync + 'static,
        combine: impl Fn(&Patch, &Patch) -> Patch + Send + Sync + 'static,
        patch: impl Fn(&T, &Patch) -> T + Send + Sync + 'static,
    ) -> Self
    where
        T: 'static,
        Patch: Clone + 'static,
    {
        Self {
            empty,
            diff: Arc::new(diff),
            combine: Arc::new(combine),
            patch: Arc::new(patch),
        }
    }

    /// The empty patch.
    pub fn empty(&self) -> Patch
    where
        Patch: Clone,
    {
        self.empty.clone()
    }

    /// The patch from `old` to `new`.
    pub fn diff(&self, old: &T, new: &T) -> Patch {
        (self.diff)(old, new)
    }

    /// Combine two patches in order.
    pub fn combine(&self, first: &Patch, second: &Patch) -> Patch {
        (self.combine)(first, second)
    }

    /// Apply a patch.
    pub fn patch(&self, old: &T, patch: &Patch) -> T {
        (self.patch)(old, patch)
    }
}

impl<T> Differ<T, Option<T>>
where
    T: Equal + Clone + 'static,
{
    /// A differ that always replaces: the patch is the new value when
    /// unequal, empty otherwise.
    pub fn replace() -> Self {
        Differ::make(
            None,
            |old, new: &T| (!old.equal(new)).then(|| new.clone()),
            |_, second: &Option<T>| second.clone(),
            |old, patch: &Option<T>| patch.clone().unwrap_or_else(|| old.clone()),
        )
    }
}

impl<T: Equal, E: Equal> Equal for Result<T, E> {
    /// `Ok` compares values, `Err` compares errors; mixed sides differ.
    /// Cross-bead residual: lets `Result` participate in structural equality.
    fn equal(&self, other: &Self) -> bool {
        match (self, other) {
            (Ok(a), Ok(b)) => a.equal(b),
            (Err(a), Err(b)) => a.equal(b),
            _ => false,
        }
    }
}

impl<T: Hash, E: Hash> Hash for Result<T, E> {
    fn hash_value(&self) -> u32 {
        match self {
            Ok(value) => combine_hash(hash_str("Ok"), value.hash_value()),
            Err(error) => combine_hash(hash_str("Err"), error.hash_value()),
        }
    }
}
