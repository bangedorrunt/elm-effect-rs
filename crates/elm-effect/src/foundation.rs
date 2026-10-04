//! Foundation port at `effect@4.0.0` (pin `67ba4e46`): `Predicate`, `Function`,
//! `Equal`, `Equivalence`, `Order`, `Ordering`, `Hash`.
//!
//! Deliberate divergence, kept on purpose: [`Equal`] treats `NaN` as equal to
//! `NaN`; [`f64`]'s `PartialEq` stays IEEE. The two traits answer different
//! questions, so `equal_nan_is_nan` uses [`Equal`], never `==`.
//!
//! Out of scope on purpose: `pipe`/`dual` application syntax (use method
//! chains and closures), `Date`/`RegExp`/`Map`/`Set` hashing (no universal
//! object graph in `std`), type-guard refinements (Rust matches do that).

use std::cmp::Ordering;
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
