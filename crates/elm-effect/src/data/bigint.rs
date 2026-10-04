//! `BigInt.ts` port: arbitrary-precision integers.
//!
//! [`Big`] wraps [`num_bigint::BigInt`]. Rows covered: global-constructor
//! re-export ([`from_int`] family), fallible [`divide`] (none on zero),
//! integer [`sqrt`], [`to_number`] inside the safe-integer range only
//! (none outside), [`from_number`] for safe integral floats, plus the
//! `ReducerMultiply`/`CombinerMax`/`CombinerMin` instances.

use num_bigint::{BigInt as NBI, Sign};
use num_traits::{One, Signed, Zero};

/// Arbitrary-precision integer (`BigInt`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Big(pub NBI);

/// Safe-integer bounds (`Number.MAX_SAFE_INTEGER`).
pub const MAX_SAFE: i64 = 9_007_199_254_740_991;
/// Safe-integer lower bound.
pub const MIN_SAFE: i64 = -9_007_199_254_740_991;

impl Big {
    /// Zero.
    pub fn zero() -> Self {
        Self(NBI::zero())
    }

    /// One (the multiply identity).
    pub fn one() -> Self {
        Self(NBI::one())
    }

    /// From a machine integer.
    pub fn from_int(n: i64) -> Self {
        Self(NBI::from(n))
    }

    /// From a decimal string (`BigInt("123")`), none when malformed.
    pub fn from_string(s: &str) -> Option<Self> {
        s.trim().parse::<NBI>().ok().map(Self)
    }

    /// From a float: some only for safe integral values.
    pub fn from_number(x: f64) -> Option<Self> {
        if !x.is_finite() || x.fract() != 0.0 || !(-9_007_199_254_740_992.0..9_007_199_254_740_992.0).contains(&x) {
            return None;
        }
        // `x as i64` is exact inside the safe range.
        Some(Self::from_int(x as i64))
    }

    /// To float: some only inside the safe-integer range.
    pub fn to_number(&self) -> Option<f64> {
        let as_i128: Option<i128> = self.0.to_string().parse().ok();
        match as_i128 {
            Some(v) if (MIN_SAFE as i128..=MAX_SAFE as i128).contains(&v) => Some(v as f64),
            _ => None,
        }
    }

    /// Fallible division: none on a zero divisor.
    pub fn divide(&self, other: &Self) -> Option<Self> {
        if other.0.is_zero() {
            None
        } else {
            Some(Self(&self.0 / &other.0))
        }
    }

    /// Integer square root (floor).
    pub fn sqrt(&self) -> Self {
        Self(self.0.sqrt())
    }

    /// Add.
    pub fn add(&self, other: &Self) -> Self {
        Self(&self.0 + &other.0)
    }

    /// Multiply.
    pub fn multiply(&self, other: &Self) -> Self {
        Self(&self.0 * &other.0)
    }

    /// Negate.
    pub fn negate(&self) -> Self {
        Self(-&self.0)
    }

    /// Absolute value.
    pub fn abs(&self) -> Self {
        Self(self.0.abs())
    }

    /// Non-negative greatest common divisor (zero for 0,0).
    pub fn gcd(&self, other: &Self) -> Self {
        use num_integer::Integer;
        Self(self.0.gcd(&other.0))
    }

    /// Non-negative least common multiple (zero when either is zero).
    pub fn lcm(&self, other: &Self) -> Self {
        use num_integer::Integer;
        Self(self.0.lcm(&other.0))
    }

    /// Inclusive range membership.
    pub fn between(&self, min: &Self, max: &Self) -> bool {
        self >= min && self <= max
    }

    /// Sign: -1, 0, 1.
    pub fn sign(&self) -> i8 {
        match self.0.sign() {
            Sign::Minus => -1,
            Sign::NoSign => 0,
            Sign::Plus => 1,
        }
    }

    /// Decimal rendering.
    pub fn to_string_decimal(&self) -> String {
        self.0.to_string()
    }

    /// Multiply combine with one as identity (`ReducerMultiply`).
    pub fn reducer_multiply(values: impl IntoIterator<Item = Self>) -> Self {
        values.into_iter().fold(Self::one(), |a, b| a.multiply(&b))
    }

    /// Larger of two (`CombinerMax`).
    pub fn max_of(a: &Self, b: &Self) -> Self {
        a.clone().max(b.clone())
    }

    /// Smaller of two (`CombinerMin`).
    pub fn min_of(a: &Self, b: &Self) -> Self {
        a.clone().min(b.clone())
    }
}

impl std::fmt::Display for Big {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
