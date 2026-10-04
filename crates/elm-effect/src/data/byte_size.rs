//! `ByteSize.ts` port: branded byte counts.
//!
//! A [`ByteSize`] wraps an integer byte count. Constructors take whole
//! numbers ([`bytes`], [`kilobytes`] and friends scale by SI/IEC units);
//! fractional inputs are accepted only when the byte count is exact
//! (`1.5 kB` is fine, `0.1 KiB` throws), negatives throw, `-0` is zero.
//! [`from_input`] parses strict SI/IEC strings (`"4MB"`, `"2 mebibytes"`,
//! `"0B"`); ambiguous, negative, or fractional-byte strings are none.
//! Numbers convert only inside the safe-integer range; ordering and
//! checked arithmetic come free on the wrapper.

/// Byte count (`ByteSize`, branded bigint upstream).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteSize(pub i128);

/// Bad constructor or parse input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByteSizeError(pub String);

impl std::fmt::Display for ByteSizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ByteSize: {}", self.0)
    }
}

impl std::error::Error for ByteSizeError {}

const MAX_SAFE: i128 = 9_007_199_254_740_991;

impl ByteSize {
    /// Zero bytes.
    pub fn zero() -> Self {
        Self(0)
    }

    /// Raw byte count; negatives rejected.
    pub fn bytes(n: i128) -> Result<Self, ByteSizeError> {
        if n < 0 {
            return Err(ByteSizeError(format!("negative bytes: {n}")));
        }
        Ok(Self(n))
    }

    /// Whole f64 count; fractions, `NaN`, infinities, and unsafe values fail.
    pub fn bytes_float(x: f64) -> Result<Self, ByteSizeError> {
        if !x.is_finite() || x.fract() != 0.0 || !(0.0..=(MAX_SAFE as f64)).contains(&x) {
            return Err(ByteSizeError(format!("bad byte count: {x}")));
        }
        Ok(Self(x as i128))
    }

    /// Scale a validated count by a unit multiplier.
    fn scaled(x: f64, unit: i128, name: &str) -> Result<Self, ByteSizeError> {
        if !x.is_finite() || x < 0.0 {
            return Err(ByteSizeError(format!("bad {name} count: {x}")));
        }
        let raw = x * unit as f64;
        if raw.fract() != 0.0 || raw > MAX_SAFE as f64 {
            return Err(ByteSizeError(format!("inexact {name} count: {x}")));
        }
        Ok(Self(raw as i128))
    }

    /// SI kilobytes.
    pub fn kilobytes(x: f64) -> Result<Self, ByteSizeError> {
        Self::scaled(x, 1_000, "kilobytes")
    }
    /// SI megabytes.
    pub fn megabytes(x: f64) -> Result<Self, ByteSizeError> {
        Self::scaled(x, 1_000_000, "megabytes")
    }
    /// SI gigabytes.
    pub fn gigabytes(x: f64) -> Result<Self, ByteSizeError> {
        Self::scaled(x, 1_000_000_000, "gigabytes")
    }
    /// Binary kibibytes.
    pub fn kibibytes(x: f64) -> Result<Self, ByteSizeError> {
        Self::scaled(x, 1_024, "kibibytes")
    }
    /// Binary mebibytes.
    pub fn mebibytes(x: f64) -> Result<Self, ByteSizeError> {
        Self::scaled(x, 1_048_576, "mebibytes")
    }
    /// Binary gibibytes.
    pub fn gibibytes(x: f64) -> Result<Self, ByteSizeError> {
        Self::scaled(x, 1_073_741_824, "gibibytes")
    }

    /// To machine number: some only inside the safe-integer range.
    pub fn to_number(&self) -> Option<i64> {
        i64::try_from(self.0).ok().filter(|v| (*v as i128) <= MAX_SAFE)
    }

    /// Checked add.
    pub fn checked_add(&self, other: &Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    /// Checked subtract (no negative results).
    pub fn checked_sub(&self, other: &Self) -> Option<Self> {
        self.0.checked_sub(other.0).filter(|v| *v >= 0).map(Self)
    }

    /// Canonical rendering (`"1500 B"`, exact, no unit promotion).
    pub fn format(&self) -> String {
        format!("{} B", self.0)
    }

    /// Parse strict strings: `<number><unit>` with SI (`B`, `kB`, `MB`,
    /// ...) or IEC (`KiB`, `MiB`, ...) units, long names allowed
    /// (`"2 mebibytes"`); whitespace between number and unit allowed.
    pub fn from_input(s: &str) -> Option<Self> {
        parse_bytes(s.trim())
    }

    /// Parse, panicking on garbage.
    pub fn from_input_unsafe(s: &str) -> Self {
        Self::from_input(s).unwrap_or_else(|| panic!("ByteSize invalid: {s}"))
    }
}

impl std::fmt::Display for ByteSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.format())
    }
}

fn unit_factor(unit: &str) -> Option<i128> {
    Some(match unit {
        "B" | "byte" | "bytes" => 1,
        "kB" | "kilobyte" | "kilobytes" => 1_000,
        "MB" | "megabyte" | "megabytes" => 1_000_000,
        "GB" | "gigabyte" | "gigabytes" => 1_000_000_000,
        "TB" | "terabyte" | "terabytes" => 1_000_000_000_000,
        "KiB" | "kibibyte" | "kibibytes" => 1_024,
        "MiB" | "mebibyte" | "mebibytes" => 1_048_576,
        "GiB" | "gibibyte" | "gibibytes" => 1_073_741_824,
        "TiB" | "tebibyte" | "tebibytes" => 1_099_511_627_776,
        _ => return None,
    })
}

fn parse_bytes(s: &str) -> Option<ByteSize> {
    if s.is_empty() {
        return None;
    }
    // Split leading number from trailing unit.
    let mut split = s.len();
    for (i, b) in s.bytes().enumerate() {
        if !(b.is_ascii_digit() || b == b'.' || b == b'+' || (b == b'-' && i == 0)) {
            split = i;
            break;
        }
    }
    let (num, unit) = (s[..split].trim(), s[split..].trim());
    // A bare number is ambiguous (no unit): reject.
    if unit.is_empty() {
        return None;
    }
    if num.starts_with('-') {
        return None;
    }
    let factor = unit_factor(unit)?;
    let value: f64 = num.parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let raw = value * factor as f64;
    if raw.fract() != 0.0 || raw > MAX_SAFE as f64 {
        return None;
    }
    Some(ByteSize(raw as i128))
}
