//! Shared value combinators: ordering, functions, hashing, equality, and
//! order. Everything here is a plain value; no runtime state and no I/O.
//!
//! Deliberate divergence: [`Equal`] treats `NaN` as equal to `NaN`; [`f64`]'s
//! `PartialEq` stays IEEE. The two traits answer different questions, so
//! `equal_nan_is_nan` uses [`Equal`], never `==`.
//!
//! `pipe`/`flow` are macros because Rust has no variadic function. `dual` is
//! out of scope: one constructor serving both call orders needs runtime arity
//! dispatch, which a typed Rust API does not have.
//!
//! `Date`/`RegExp`/`Map`/`Set` hashing is out of scope: `std` has no
//! universal object graph. Type-guard refinements are plain `match` arms.
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;

// Ordering

/// First non-equal verdict wins.
pub fn combine_ordering(first: Ordering, second: Ordering) -> Ordering {
    match first {
        Ordering::Equal => second,
        decided => decided,
    }
}

/// Apply a value through a chain of functions, left to right.
///
/// `pipe!(x)` is `x`; `pipe!(x, f, g)` is `g(f(x))`. Effect 4's `pipe` is
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

/// Structural hash with Effect 4's exact formulas (MurmurHash3 finalizer,
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
/// BMP-identical to Effect 4; astral characters hash by scalar value here.
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

// Keyed collections. Effect 4's `Equal.equals` compares a JavaScript `Map`
// and `Set` entry by entry, independent of insertion order, so these read
// as the crate's ordered spellings of the same two shapes.

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

    /// `Equivalence.Struct`: compare only the named fields. Extra keys are
    /// ignored, which is Effect 4's record semantics; a listed key missing
    /// from either side is not equivalent.
    pub fn struct_(
        fields: Vec<(String, Self)>,
    ) -> Equivalence<std::collections::BTreeMap<String, T>>
    where
        T: 'static,
    {
        Equivalence::make(move |a: &BTreeMap<String, T>, b: &BTreeMap<String, T>| {
            fields
                .iter()
                .all(|(name, equivalence)| match (a.get(name), b.get(name)) {
                    (Some(a), Some(b)) => equivalence.is_equivalent(a, b),
                    (None, None) => true,
                    _ => false,
                })
        })
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

    /// First non-equal verdict wins. The second criterion only runs on a tie,
    /// so an expensive tie-breaker costs nothing when the first decides.
    pub fn combine(self, other: Self) -> Self
    where
        T: 'static,
    {
        Self::make(move |a, b| {
            let first = self.compare(a, b);
            if first == Ordering::Equal {
                other.compare(a, b)
            } else {
                first
            }
        })
    }

    /// Compare through a projection first.
    pub fn map_input<U>(self, f: impl Fn(&U) -> T + Send + Sync + 'static) -> Order<U>
    where
        T: 'static,
    {
        Order::make(move |a, b| self.compare(&f(a), &f(b)))
    }

    /// The smaller value (ties go to the second, per Effect 4).
    pub fn min(&self, first: T, second: T) -> T
    where
        T: Clone,
    {
        match self.compare(&first, &second) {
            Ordering::Greater => second,
            _ => first,
        }
    }

    /// The larger value (ties go to the first, per Effect 4).
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

    /// Whether the first is less or ties.
    pub fn is_less_than_or_equal(&self, first: &T, second: &T) -> bool {
        !matches!(self.compare(first, second), Ordering::Greater)
    }

    /// Whether the first is strictly greater.
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

    /// `Order.Struct`: compare the named fields in declaration order. Extra
    /// keys are ignored; a missing field sorts below a present one.
    pub fn struct_(fields: Vec<(String, Self)>) -> Order<std::collections::BTreeMap<String, T>>
    where
        T: 'static,
    {
        Order::make(move |a: &BTreeMap<String, T>, b: &BTreeMap<String, T>| {
            for (name, order) in &fields {
                let verdict = match (a.get(name), b.get(name)) {
                    (Some(a), Some(b)) => order.compare(a, b),
                    (None, None) => Ordering::Equal,
                    (None, Some(_)) => Ordering::Less,
                    (Some(_), None) => Ordering::Greater,
                };
                if verdict != Ordering::Equal {
                    return verdict;
                }
            }
            Ordering::Equal
        })
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

/// String order (byte-wise).
pub fn string_order() -> Order<String> {
    Order::make(|a: &String, b: &String| a.cmp(b))
}

/// Number order: `NaN` equals `NaN`, `NaN` sorts below everything.
///
/// A comparator must answer `Equal` for equal values: `combine`, `clamp` and
/// `is_between` all read a non-`Less`/non-`Greater` tie as a real tie.
pub fn number_order() -> Order<f64> {
    Order::make(|a: &f64, b: &f64| {
        if a.is_nan() && b.is_nan() {
            Ordering::Equal
        } else if a.is_nan() {
            Ordering::Less
        } else if b.is_nan() {
            Ordering::Greater
        } else {
            a.partial_cmp(b).unwrap_or(Ordering::Equal)
        }
    })
}

/// Boolean order (`false` below `true`).
pub fn boolean_order() -> Order<bool> {
    Order::make(|a: &bool, b: &bool| a.cmp(b))
}

/// Order from Rust's own [`Ord`].
pub fn ord_order<T: Ord>() -> Order<T> {
    Order::make(|a: &T, b: &T| a.cmp(b))
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
}

impl Combiner<String> {
    /// Join strings with a middle separator. This lives on the concrete type
    /// so a bare `Combiner::intercalate(", ")` infers the element type.
    pub fn intercalate(middle: &str) -> Self {
        let middle = middle.to_string();
        Self::make(move |a, b| format!("{a}{middle}{b}"))
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
