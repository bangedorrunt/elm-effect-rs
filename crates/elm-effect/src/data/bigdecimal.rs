//! `BigDecimal.ts` port: mantissa/scale decimals.
//!
//! A [`Decimal`] is a [`num_bigint::BigInt`] mantissa plus an `i64` scale:
//! value = mantissa / 10^scale. [`normalize`] strips trailing mantissa
//! zeros (zero canonicalizes); equality, order, and hashing all run on
//! normalized forms, so `1.0 == 1.00`. Division is exact when it
//! terminates, else truncated to 100 significant digits, like the
//! original. [`format`] prints plain text unless `|scale| >= 16`, when it
//! switches to [`to_exponential`]. Scales must fit `i64` arithmetic;
//! out-of-range construction is an error, never wrapping.

use num_bigint::{BigInt as NBI, Sign};
use num_traits::{One, Signed, Zero};

/// Arbitrary-precision decimal (`BigDecimal`).
#[derive(Clone, Debug)]
pub struct Decimal {
    mantissa: NBI,
    scale: i64,
}

/// Scale or value out of range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecimalError(pub String);

impl std::fmt::Display for DecimalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BigDecimal: {}", self.0)
    }
}

impl std::error::Error for DecimalError {}

/// Division precision: significant digits when inexact.
pub const DIV_PRECISION: usize = 100;

/// Rounding mode (`RoundingMode`): halves variants break ties as named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundMode {
    /// Toward positive infinity.
    Ceil,
    /// Toward negative infinity.
    Floor,
    /// Toward zero (truncate).
    ToZero,
    /// Away from zero.
    FromZero,
    /// Ties toward positive infinity.
    HalfCeil,
    /// Ties toward negative infinity.
    HalfFloor,
    /// Ties toward zero.
    HalfToZero,
    /// Ties away from zero.
    HalfFromZero,
    /// Ties to even.
    HalfEven,
    /// Ties to odd.
    HalfOdd,
}

impl Decimal {
    /// Build from mantissa and scale; scale must be a safe integer.
    pub fn make(mantissa: NBI, scale: i64) -> Result<Self, DecimalError> {
        // `i64` already bounds the scale; reject absurd magnitudes that
        // would explode padding.
        if scale.abs() > 1_000_000 {
            return Err(DecimalError(format!("unsafe scale: {scale}")));
        }
        Ok(Self { mantissa, scale })
    }

    /// Zero.
    pub fn zero() -> Self {
        Self {
            mantissa: NBI::zero(),
            scale: 0,
        }
    }

    /// One.
    pub fn one() -> Self {
        Self {
            mantissa: NBI::one(),
            scale: 0,
        }
    }

    /// Mantissa accessor (for tests).
    pub fn mantissa(&self) -> &NBI {
        &self.mantissa
    }

    /// Scale accessor.
    pub fn scale(&self) -> i64 {
        self.scale
    }

    /// Canonical form: trailing mantissa zeros folded into the scale;
    /// zero becomes `0/0`.
    pub fn normalize(&self) -> Self {
        if self.mantissa.is_zero() {
            return Self::zero();
        }
        let mut digits = self.mantissa.to_string();
        let negative = digits.starts_with('-');
        if negative {
            digits.remove(0);
        }
        let mut end = digits.len();
        while end > 0 && digits.as_bytes()[end - 1] == b'0' {
            end -= 1;
        }
        let stripped = if negative {
            format!("-{0}", &digits[..end])
        } else {
            digits[..end].to_string()
        };
        let dropped = (digits.len() - end) as i64;
        let mantissa: NBI = stripped.parse().unwrap_or_else(|_| self.mantissa.clone());
        Self {
            mantissa,
            scale: self.scale - dropped,
        }
    }

    /// True for zero.
    pub fn is_zero(&self) -> bool {
        self.mantissa.is_zero()
    }

    /// Sign: -1, 0, 1.
    pub fn sign(&self) -> i8 {
        match self.mantissa.sign() {
            Sign::Minus => -1,
            Sign::NoSign => 0,
            Sign::Plus => 1,
        }
    }

    /// True when the value is integral.
    pub fn is_integer(&self) -> bool {
        let n = self.normalize();
        n.scale <= 0
    }

    /// Align two values to a common scale (panics on overflow: scales are
    /// validated at construction).
    fn aligned(&self, other: &Self) -> (NBI, NBI, i64) {
        let scale = self.scale.max(other.scale);
        let ten = NBI::from(10);
        let up = |m: &NBI, from: i64| {
            let mut v = m.clone();
            for _ in from..scale {
                v *= &ten;
            }
            v
        };
        (
            up(&self.mantissa, self.scale),
            up(&other.mantissa, other.scale),
            scale,
        )
    }

    /// Add.
    pub fn add(&self, other: &Self) -> Self {
        let (a, b, scale) = self.aligned(other);
        Self {
            mantissa: a + b,
            scale,
        }
    }

    /// Sum many.
    pub fn sum_all(iter: impl IntoIterator<Item = Self>) -> Self {
        iter.into_iter().fold(Self::zero(), |a, b| a.add(&b))
    }

    /// Subtract.
    pub fn subtract(&self, other: &Self) -> Self {
        let (a, b, scale) = self.aligned(other);
        Self {
            mantissa: a - b,
            scale,
        }
    }

    /// Multiply (scales add).
    pub fn multiply(&self, other: &Self) -> Self {
        Self {
            mantissa: &self.mantissa * &other.mantissa,
            scale: self.scale + other.scale,
        }
    }

    /// Multiply many.
    pub fn multiply_all(iter: impl IntoIterator<Item = Self>) -> Self {
        iter.into_iter().fold(Self::one(), |a, b| a.multiply(&b))
    }

    /// Exact-or-truncated division: none on zero divisor.
    pub fn divide(&self, other: &Self) -> Option<Self> {
        if other.mantissa.is_zero() {
            return None;
        }
        if self.mantissa.is_zero() {
            return Some(Self::zero());
        }
        if self.mantissa == other.mantissa {
            let _ = self.scale - other.scale;
            return Some(Self {
                mantissa: NBI::one(),
                scale: self.scale - other.scale,
            });
        }
        Some(divide_precise(
            &self.mantissa,
            &other.mantissa,
            self.scale - other.scale,
        ))
    }

    /// Truncating remainder: none on zero divisor.
    pub fn remainder(&self, other: &Self) -> Option<Self> {
        if other.mantissa.is_zero() {
            return None;
        }
        let (a, b, scale) = self.aligned(other);
        Some(Self {
            mantissa: a % b,
            scale,
        })
    }

    /// Remainder, panicking on zero divisor.
    pub fn remainder_unsafe(&self, other: &Self) -> Self {
        self.remainder(other).expect("BigDecimal remainder by zero")
    }

    /// Negate.
    pub fn negate(&self) -> Self {
        Self {
            mantissa: -&self.mantissa,
            scale: self.scale,
        }
    }

    /// Absolute value.
    pub fn abs(&self) -> Self {
        Self {
            mantissa: self.mantissa.abs(),
            scale: self.scale,
        }
    }

    /// Value comparison.
    pub fn compare(&self, other: &Self) -> std::cmp::Ordering {
        let (a, b, _) = self.aligned(other);
        a.cmp(&b)
    }

    /// Between, inclusive.
    pub fn between(&self, min: &Self, max: &Self) -> bool {
        self >= min && self <= max
    }

    /// From an integer mantissa (`fromBigInt`, scale 0).
    pub fn from_bigint(mantissa: NBI) -> Self {
        Self { mantissa, scale: 0 }
    }

    /// From a float: none for non-finite values.
    pub fn from_number(x: f64) -> Option<Self> {
        if !x.is_finite() {
            return None;
        }
        Self::from_string(&format!("{x:?}")).or_else(|| Self::from_string(&x.to_string()))
    }

    /// True for negative values.
    pub fn is_negative(&self) -> bool {
        self.sign() < 0
    }

    /// True for positive values.
    pub fn is_positive(&self) -> bool {
        self.sign() > 0
    }

    /// Digit at the given scale (`digitAt`): the `10^-position` place,
    /// zero when the representation is shorter.
    pub fn digit_at(&self, position: i64) -> u8 {
        if self.scale < position {
            return 0;
        }
        let div = Self::pow10((self.scale - position) as u32);
        let digit: NBI = (self.mantissa.abs() / div) % 10;
        digit.to_string().bytes().next().unwrap_or(b'0') - b'0'
    }

    /// Round to `scale` decimal places under `mode`.
    pub fn round(&self, scale: i64, mode: RoundMode) -> Self {
        let shift = self.scale - scale;
        if shift <= 0 {
            // Already at least as precise: pad with zeros.
            let mut m = self.mantissa.clone();
            for _ in 0..(-shift) {
                m *= 10;
            }
            return Self { mantissa: m, scale };
        }
        let divisor = Self::pow10(shift as u32);
        let negative = self.mantissa.sign() == Sign::Minus;
        let mag = self.mantissa.abs();
        let (mut q, r) = (mag.clone() / divisor.clone(), mag % &divisor);
        let nonzero = !r.is_zero();
        let doubled: NBI = r.clone() * 2;
        let half_cmp = doubled.cmp(&divisor);
        let up = match mode {
            RoundMode::Ceil => nonzero && !negative,
            RoundMode::Floor => nonzero && negative,
            RoundMode::ToZero => false,
            RoundMode::FromZero => nonzero,
            RoundMode::HalfCeil => {
                half_cmp == std::cmp::Ordering::Greater
                    || (half_cmp == std::cmp::Ordering::Equal && !negative)
            }
            RoundMode::HalfFloor => {
                half_cmp == std::cmp::Ordering::Greater
                    || (half_cmp == std::cmp::Ordering::Equal && negative)
            }
            RoundMode::HalfToZero => half_cmp == std::cmp::Ordering::Greater,
            RoundMode::HalfFromZero => half_cmp != std::cmp::Ordering::Less,
            RoundMode::HalfEven => {
                half_cmp == std::cmp::Ordering::Greater
                    || (half_cmp == std::cmp::Ordering::Equal && (q.clone() % 2 != NBI::zero()))
            }
            RoundMode::HalfOdd => {
                half_cmp == std::cmp::Ordering::Greater
                    || (half_cmp == std::cmp::Ordering::Equal && (q.clone() % 2 == NBI::zero()))
            }
        };
        if up {
            q += 1;
        }
        let mantissa = if negative { -q } else { q };
        Self { mantissa, scale }
    }

    /// To float, precision loss accepted (`toNumberUnsafe`).
    pub fn to_number_unsafe(&self) -> f64 {
        self.format().parse().unwrap_or(f64::NAN)
    }

    /// Parse decimal text (`fromString`): `""` is zero, `e` exponents and
    /// `+`/`-` signs accepted, anything else is none.
    pub fn from_string(s: &str) -> Option<Self> {
        if s.is_empty() {
            return Some(Self::zero());
        }
        let (base, exp) = match s.split_once(['e', 'E']) {
            Some((b, trail)) => {
                if b.is_empty() {
                    return None;
                }
                let exp: i64 = trail.parse().ok()?;
                if !trail
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '+' || c == '-')
                {
                    return None;
                }
                if trail.trim_start_matches(['+', '-']).is_empty() {
                    return None;
                }
                (b, exp)
            }
            None => (s, 0),
        };
        let (digits, offset) = match base.split_once('.') {
            Some((lead, trail)) => {
                let lead = if lead.is_empty() || lead == "+" || lead == "-" {
                    format!("{lead}0")
                } else {
                    lead.to_string()
                };
                (format!("{lead}{trail}"), trail.len() as i64)
            }
            None => (base.to_string(), 0),
        };
        if digits.is_empty()
            || !digits
                .trim_start_matches(['+', '-'])
                .bytes()
                .all(|b| b.is_ascii_digit())
            || digits.trim_start_matches(['+', '-']).is_empty()
        {
            return None;
        }
        let scale = offset - exp;
        if scale.abs() > 1_000_000 {
            return None;
        }
        let mantissa: NBI = digits.parse().ok()?;
        Some(Self { mantissa, scale })
    }

    /// Parse, panicking on garbage.
    pub fn make_unsafe(s: &str) -> Self {
        Self::from_string(s).unwrap_or_else(|| panic!("BigDecimal invalid: {s}"))
    }

    /// Plain or scientific rendering (`format`).
    pub fn format(&self) -> String {
        let n = self.normalize();
        if n.scale.abs() >= 16 {
            return n.to_exponential();
        }
        let negative = n.mantissa.sign() == Sign::Minus;
        let mut absolute = n.mantissa.abs().to_string();
        let digits = if n.scale > 0 {
            while absolute.len() < (n.scale + 1) as usize {
                absolute.insert(0, '0');
            }
            let point = absolute.len() - n.scale as usize;
            format!("{0}.{1}", &absolute[..point], &absolute[point..])
        } else {
            format!("{absolute}{0}", "0".repeat((-n.scale) as usize))
        };
        if negative {
            format!("-{digits}")
        } else {
            digits
        }
    }

    /// Scientific rendering (`toExponential`).
    pub fn to_exponential(&self) -> String {
        if self.is_zero() {
            return "0e+0".to_string();
        }
        let n = self.normalize();
        let digits = n.mantissa.to_string();
        let (head, tail, point) = if digits.starts_with('-') {
            (&digits[..2], &digits[2..], 2)
        } else {
            (&digits[..1], &digits[1..], 1)
        };
        let _ = point;
        let exp = tail.len() as i64 - n.scale;
        if tail.is_empty() {
            format!("{head}e{0}{exp}", if exp >= 0 { "+" } else { "" })
        } else {
            format!("{head}.{tail}e{0}{exp}", if exp >= 0 { "+" } else { "" })
        }
    }

    fn pow10(n: u32) -> NBI {
        let mut v = NBI::from(1);
        for _ in 0..n {
            v *= 10;
        }
        v
    }

    /// Hash-stable key: normalized rendering.
    pub fn hash_key(&self) -> String {
        let n = self.normalize();
        format!("{0}/{1}", n.mantissa, n.scale)
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for Decimal {}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.compare(other)
    }
}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.format())
    }
}

fn divide_precise(num: &NBI, den: &NBI, scale: i64) -> Decimal {
    let negate = (num < &NBI::zero()) != (den < &NBI::zero());
    let (mut num, den) = (num.abs(), den.abs());
    let ten = NBI::from(10);
    let mut scale = scale;
    while num < den {
        num *= &ten;
        scale += 1;
    }
    let mut quotient = num.clone() / den.clone();
    let mut remainder = num % den.clone();
    if remainder.is_zero() {
        if negate {
            quotient = -quotient;
        }
        return Decimal {
            mantissa: quotient,
            scale,
        };
    }
    let mut count = quotient.to_string().len();
    remainder *= &ten;
    while !remainder.is_zero() && count < DIV_PRECISION {
        let q = remainder.clone() / den.clone();
        let r = remainder % &den;
        quotient = quotient * &ten + q;
        remainder = r * &ten;
        count += 1;
        scale += 1;
    }
    if !remainder.is_zero() {
        // Round the final digit half-up on the next dropped digit.
        let next = remainder / den;
        let first = next.to_string().bytes().next().unwrap_or(b'0');
        if first >= b'5' {
            quotient += 1;
        }
    }
    if negate {
        quotient = -quotient;
    }
    Decimal {
        mantissa: quotient,
        scale,
    }
}
