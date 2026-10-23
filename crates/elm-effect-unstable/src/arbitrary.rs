//! Arbitrary value generation (`arbitrary/Arbitrary` at the pin): seeded
//! generators with local shrinking.
//!
//! A [`Gen`] pairs generation with shrink candidates; [`Rng`] is a seeded
//! xorshift stream, so samples are deterministic per seed
//! ("generates deterministic samples"). Bounds reject out-of-range
//! construction; integers shrink toward the failure boundary; composition
//! runs through `map`/`filter`/`tuple`/`record`/`option`/`array`.
//!
//! [`schema`] derives a generator from a [`Schema`], which is the pin's
//! `Arbitrary.schema`. It constructs wire values from the schema's shape and
//! decodes them, so what it samples is by construction something the schema
//! accepts; a shape or a check it cannot push into a generator is refused with
//! the pin's own message rather than filtered after the fact.
//!
//! Residuals (fast-check internals, out of timebox): model-based command
//! shrinking, UTF-16 pattern generation, and replay orchestration.

use elm_effect_schema::pattern::{self, Pattern, utf16_len};
use elm_effect_schema::{Schema, SchemaKind, Unknown};

/// Seeded xorshift64 stream. Same seed gives the same sequence.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Seed the stream (zero seeds to a fixed nonzero fallback).
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x9E3779B97F4A7C15 } else { seed },
        }
    }

    /// Next `u64`.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Value below `bound` (panics on zero, like an empty domain).
    pub fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "empty domain");
        self.next_u64() % bound
    }
}

/// Sampler: stream in, value out, `None` when the draw was discarded.
type SampleFn<T> = std::sync::Arc<dyn Fn(&mut Rng) -> Option<T> + Send + Sync>;
/// Shrinker: failing value in, smaller candidates out.
type ShrinkFn<T> = std::sync::Arc<dyn Fn(&T) -> Vec<T> + Send + Sync>;

/// A generator: sampling plus shrink candidates toward a boundary.
#[derive(Clone)]
pub struct Gen<T> {
    run: SampleFn<T>,
    shrink: ShrinkFn<T>,
}

impl<T> std::fmt::Debug for Gen<T> {
    /// A generator is two closures, so there is nothing useful to print; the
    /// implementation exists so a `Result<Gen<_>, _>` can be unwrapped in a
    /// test without the value standing in for the verdict.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Gen(..)")
    }
}

impl<T> Gen<T> {
    /// Build from a sampler and a shrinker.
    pub fn new(
        run: impl Fn(&mut Rng) -> T + Send + Sync + 'static,
        shrink: impl Fn(&T) -> Vec<T> + Send + Sync + 'static,
    ) -> Self {
        Self::fallible(move |rng| Some(run(rng)), shrink)
    }

    /// Build from a sampler that may discard a draw.
    fn fallible(
        run: impl Fn(&mut Rng) -> Option<T> + Send + Sync + 'static,
        shrink: impl Fn(&T) -> Vec<T> + Send + Sync + 'static,
    ) -> Self {
        Self {
            run: std::sync::Arc::new(run),
            shrink: std::sync::Arc::new(shrink),
        }
    }

    /// Sample one value. A draw that is discarded is retried, because a caller
    /// that asked for a value has nothing else to do with a discard.
    pub fn sample(&self, rng: &mut Rng) -> T {
        for _ in 0..64 {
            if let Some(value) = (self.run)(rng) {
                return value;
            }
        }
        panic!("the generator produced no value the schema accepts")
    }

    /// One attempt: `None` when this draw was discarded (a constraint the
    /// generator cannot construct its way into). A sampler that reports
    /// discards, rather than retrying, uses this.
    pub fn try_sample(&self, rng: &mut Rng) -> Option<T> {
        (self.run)(rng)
    }

    /// Deterministic sample of `n` values from `seed`.
    pub fn sample_n(&self, seed: u64, n: usize) -> Vec<T>
    where
        T: Clone,
    {
        let mut rng = Rng::new(seed);
        (0..n).map(|_| self.sample(&mut rng)).collect()
    }

    /// Shrink candidates for a failing value.
    pub fn shrink(&self, value: &T) -> Vec<T> {
        (self.shrink)(value)
    }

    /// Map outputs.
    pub fn map<U>(&self, f: impl Fn(T) -> U + Send + Sync + 'static) -> Gen<U>
    where
        T: Clone + Send + Sync + 'static,
        U: Send + Sync + 'static,
    {
        let base = self.clone();
        Gen::new(move |rng| f(base.sample(rng)), |_| Vec::new())
    }

    /// Keep only outputs satisfying `predicate` (retries internally,
    /// bounded so a filter-all-out predicate terminates).
    pub fn filter(&self, predicate: impl Fn(&T) -> bool + Send + Sync + 'static) -> Gen<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        let base = self.clone();
        Gen::new(
            move |rng| {
                for _ in 0..1024 {
                    let value = base.sample(rng);
                    if predicate(&value) {
                        return value;
                    }
                }
                base.sample(rng)
            },
            |_| Vec::new(),
        )
    }
}

/// Constant generator (no shrinking).
pub fn constant<T: Clone + Send + Sync + 'static>(value: T) -> Gen<T> {
    Gen::new(move |_| value.clone(), |_| Vec::new())
}

/// Booleans.
pub fn boolean() -> Gen<bool> {
    Gen::new(
        |rng| rng.next_u64() & 1 == 1,
        |v| if *v { vec![false] } else { vec![] },
    )
}

/// Integers in `[min, max]` (rejects inverted bounds); shrinks down
/// toward `min`.
pub fn integer(min: i64, max: i64) -> Gen<i64> {
    assert!(min <= max, "integer bounds outside the valid range");
    Gen::new(
        move |rng| {
            let span = (max as u64).wrapping_sub(min as u64).wrapping_add(1);
            if span == 0 {
                // Full range: raw bits reinterpreted.
                rng.next_u64() as i64
            } else {
                min.wrapping_add((rng.next_u64() % span) as i64)
            }
        },
        move |v| {
            let mut out = Vec::new();
            if *v > min {
                out.push(min);
            }
            if *v > min.saturating_add(1) {
                out.push(*v - (*v - min) / 2);
            }
            out
        },
    )
}

/// Floating-point bounds with optional exclusivity.
/// Mirrors `Schema.Number` range checks: NaN is reachable only when a side
/// is unbounded, and exclusive ends snap to the adjacent representable
/// value (so adjacent-exclusive bounds yield their sole interior value).
pub fn number(min: f64, max: f64, exclusive_min: bool, exclusive_max: bool) -> Gen<f64> {
    assert!(
        min <= max && !min.is_nan() && !max.is_nan(),
        "number bounds outside the valid range"
    );
    let lo = if exclusive_min { min.next_up() } else { min };
    let hi = if exclusive_max { max.next_down() } else { max };
    assert!(lo <= hi, "number bounds exclude every value");
    let open = !min.is_finite() || !max.is_finite();
    Gen::new(
        move |rng| {
            // A singleton interval answers with its one value; the
            // unbounded-side NaN wildcard stays out of it, so [v, v]
            // is exact (Arbitrary.test.ts:1772). A zero interval
            // answers with both signed zeros (:1758).
            if lo == hi {
                return if lo == 0.0 && rng.next_u64() & 1 == 1 {
                    -0.0
                } else {
                    lo
                };
            }
            if open && rng.next_u64() % 64 == 0 {
                return f64::NAN;
            }
            let t = (rng.next_u64() as f64) / (u64::MAX as f64);
            lo + (hi - lo) * t
        },
        move |v| {
            let mut out = Vec::new();
            if *v > lo {
                out.push(lo);
                let mid = lo + (*v - lo) / 2.0;
                if mid < *v {
                    out.push(mid);
                }
            }
            out
        },
    )
}

/// ASCII lowercase strings of `len` chars.
pub fn ascii_string(len: usize) -> Gen<String> {
    Gen::new(
        move |rng| {
            (0..len)
                .map(|_| (b'a' + (rng.next_u64() % 26) as u8) as char)
                .collect()
        },
        |_| Vec::new(),
    )
}

/// Hex strings of `len` nibbles.
pub fn hex_string(len: usize) -> Gen<String> {
    const DIGITS: &[u8] = b"0123456789abcdef";
    Gen::new(
        move |rng| {
            (0..len)
                .map(|_| DIGITS[(rng.next_u64() % 16) as usize] as char)
                .collect()
        },
        |_| Vec::new(),
    )
}

/// Optional values (`None` half the time).
pub fn option<T: Clone + Send + Sync + 'static>(inner: Gen<T>) -> Gen<Option<T>> {
    Gen::new(
        move |rng| {
            if rng.next_u64() & 1 == 1 {
                Some(inner.sample(rng))
            } else {
                None
            }
        },
        |_| Vec::new(),
    )
}

/// Pairs.
pub fn tuple<A, B>(first: Gen<A>, second: Gen<B>) -> Gen<(A, B)>
where
    A: Clone + Send + Sync + 'static,
    B: Clone + Send + Sync + 'static,
{
    Gen::new(
        move |rng| (first.sample(rng), second.sample(rng)),
        |_| Vec::new(),
    )
}

/// Vectors with length in `[min_len, max_len]` (rejects inverted bounds).
pub fn array<T: Clone + Send + Sync + 'static>(
    inner: Gen<T>,
    min_len: usize,
    max_len: usize,
) -> Gen<Vec<T>> {
    assert!(min_len <= max_len, "invalid length bounds");
    Gen::new(
        move |rng| {
            let span = (max_len - min_len + 1) as u64;
            let len = min_len + rng.below(span) as usize;
            (0..len).map(|_| inner.sample(rng)).collect()
        },
        |_| Vec::new(),
    )
}

// ---------------------------------------------------------------------------
// Schema-derived arbitrary (the pin's `Arbitrary.schema`)
// ---------------------------------------------------------------------------

/// Why a schema has no derivable arbitrary: the pin's `Unable to derive an
/// arbitrary for {what}`, plus the pointer it was walking when it gave up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeriveError {
    what: String,
    path: String,
}

impl DeriveError {
    fn new(what: impl Into<String>, path: &str) -> Self {
        Self {
            what: what.into(),
            path: path.to_string(),
        }
    }
}

impl std::fmt::Display for DeriveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Unable to derive an arbitrary for {}", self.what)?;
        if self.path != "$" {
            write!(f, " at {}", self.path)?;
        }
        Ok(())
    }
}

impl std::error::Error for DeriveError {}

/// The pin's `Number.MAX_SAFE_INTEGER`, which is also the largest integer this
/// port's `Int` accepts.
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

/// `Arbitrary.schema`: a generator whose samples the schema decodes.
///
/// The pin builds the typed value directly because each constructor carries a
/// `toArbitrary` annotation; this port has no such annotation, so the value is
/// built in its wire form and decoded through the schema it came from. A check
/// the derivation cannot push into a generator refuses the whole schema, which
/// is what keeps every sample decodable instead of filtered.
pub fn schema<T>(schema: &Schema<T>) -> Result<Gen<T>, DeriveError>
where
    T: Clone + Send + Sync + 'static,
{
    let wire = derive(&schema.kind(), "$")?;
    let sink = schema.clone();
    let encoder = schema.clone();
    let shrinker = wire.clone();
    // Each draw is one attempt: a value the schema refuses is that attempt's
    // discard, which is how a residual check (a pattern outside the subset)
    // behaves in the pin. `Gen::sample` retries; a discard-reporting sampler
    // calls `try_sample`.
    Ok(Gen::fallible(
        move |rng| sink.decode(wire.try_sample(rng)?).ok(),
        move |decoded| {
            let Ok(wire_value) = encoder.try_encode(decoded) else {
                return Vec::new();
            };
            shrinker
                .shrink(&wire_value)
                .into_iter()
                .filter_map(|candidate| encoder.decode(candidate).ok())
                .collect()
        },
    ))
}

/// One shape's generator, in wire form.
fn derive(kind: &SchemaKind, path: &str) -> Result<Gen<Unknown>, DeriveError> {
    let mut checks: Vec<CheckRef<'_>> = Vec::new();
    let mut base = kind;
    while let SchemaKind::Check {
        base: inner,
        name,
        args,
        annotation,
    } = base
    {
        checks.push(CheckRef {
            name,
            args: args.as_slice(),
            annotation: annotation.as_slice(),
        });
        base = inner;
    }
    // Application order, so merging bounds reads the outermost check last.
    checks.reverse();
    match base {
        SchemaKind::Number => numeric(&checks, path),
        SchemaKind::String => text(&checks, path),
        SchemaKind::Boolean => Ok(boolean().map(Unknown::Bool)),
        SchemaKind::Literal(value) => Ok(constant(value.clone())),
        SchemaKind::Literals(values) => {
            let values = values.clone();
            Ok(Gen::new(
                move |rng| values[rng.below(values.len() as u64) as usize].clone(),
                |_| Vec::new(),
            ))
        }
        SchemaKind::Struct(fields) => structure(fields, path),
        SchemaKind::Array(inner) => Ok(varied(derive(inner, &format!("{path}[0]"))?)),
        SchemaKind::Tuple(elements) => tuple_of(elements, path),
        SchemaKind::Union { members, .. } => {
            let members = members
                .iter()
                .enumerate()
                .map(|(index, member)| derive(member, &format!("{path}|{index}")))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Gen::new(
                move |rng| members[rng.below(members.len() as u64) as usize].sample(rng),
                |_| Vec::new(),
            ))
        }
        SchemaKind::Optional(inner) | SchemaKind::OptionalKey(inner) => {
            let inner = derive(inner, path)?;
            Ok(Gen::new(
                move |rng| {
                    if rng.next_u64() & 1 == 1 {
                        inner.sample(rng)
                    } else {
                        Unknown::Null
                    }
                },
                |_| Vec::new(),
            ))
        }
        SchemaKind::Annotated { inner, .. } => derive(inner, path),
        SchemaKind::Opaque("null") | SchemaKind::Opaque("void") => {
            Ok(constant(Unknown::Null))
        }
        SchemaKind::Opaque(name) => Err(DeriveError::new(format!("the `{name}` shape"), path)),
        other => Err(DeriveError::new(
            format!("the `{}` shape", shape_name(other)),
            path,
        )),
    }
}

/// A check's identity while a shape is being derived.
struct CheckRef<'a> {
    name: &'a str,
    args: &'a [String],
    annotation: &'a [(String, String)],
}

fn shape_name(kind: &SchemaKind) -> String {
    match kind {
        SchemaKind::Record(_, _) => "Record".to_string(),
        SchemaKind::StructWithRest(_, _) => "StructWithRest".to_string(),
        SchemaKind::TupleWithRest(_, _) => "TupleWithRest".to_string(),
        SchemaKind::TemplateLiteral(_) => "TemplateLiteral".to_string(),
        SchemaKind::Rest(_) => "Rest".to_string(),
        SchemaKind::Json => "Json".to_string(),
        SchemaKind::Reference(name) => format!("Reference({name})"),
        _ => "this".to_string(),
    }
}

/// `Schema.Number` / `Schema.Int` with the numeric checks pushed into bounds.
fn numeric(checks: &[CheckRef<'_>], path: &str) -> Result<Gen<Unknown>, DeriveError> {
    let is_int = checks.iter().any(|check| check.name == "isInt");
    let what = if is_int {
        "integer constraints"
    } else {
        "number constraints"
    };
    let (minimum, maximum) = number_bounds(checks, is_int, what, path)?;
    if is_int {
        // Every bound is inside the safe range by now, so the cast is exact.
        let lo = minimum.unwrap_or(-MAX_SAFE) as i64;
        let hi = maximum.unwrap_or(MAX_SAFE) as i64;
        return Ok(integer(lo, hi).map(|value| Unknown::Number(value as f64)));
    }
    let mut lo = minimum.unwrap_or(-f64::MAX);
    let mut hi = maximum.unwrap_or(f64::MAX);
    // A one-sided infinite bound that no finite constraint excluded is a legal
    // singleton: the pin generates infinity there, and a clamp to MAX would
    // make the interval empty rather than exact.
    if lo == f64::INFINITY && minimum.is_some() && maximum.is_none() {
        hi = lo;
    }
    if hi == f64::NEG_INFINITY && maximum.is_some() && minimum.is_none() {
        lo = hi;
    }
    if lo > hi {
        return Err(DeriveError::new(what, path));
    }
    Ok(bounded_number(lo, hi).map(Unknown::Number))
}

/// The pin's `numberBounds`: apply each check's bound, refuse a NaN, then
/// normalize the interval the way its generator reads one.
fn number_bounds(
    checks: &[CheckRef<'_>],
    integer: bool,
    what: &str,
    path: &str,
) -> Result<(Option<f64>, Option<f64>), DeriveError> {
    let refuse = || DeriveError::new(what, path);
    let mut minimum: Option<f64> = None;
    let mut maximum: Option<f64> = None;
    let mut exclusive_minimum = false;
    let mut exclusive_maximum = false;
    let mut finite = false;
    for check in checks {
        match check.name {
            "isInt" => {}
            "isFinite" => finite = true,
            "isGreaterThan" | "isGreaterThanOrEqualTo" => {
                let bound = number_arg(check.args, 0).ok_or_else(refuse)?;
                let exclusive = check.name == "isGreaterThan";
                merge_minimum(&mut minimum, &mut exclusive_minimum, bound, exclusive);
            }
            "isLessThan" | "isLessThanOrEqualTo" => {
                let bound = number_arg(check.args, 0).ok_or_else(refuse)?;
                let exclusive = check.name == "isLessThan";
                merge_maximum(&mut maximum, &mut exclusive_maximum, bound, exclusive);
            }
            "isBetween" => {
                let low = number_arg(check.args, 0).ok_or_else(refuse)?;
                let high = number_arg(check.args, 1).ok_or_else(refuse)?;
                merge_minimum(
                    &mut minimum,
                    &mut exclusive_minimum,
                    low,
                    annotation_true(check.annotation, "exclusiveMinimum"),
                );
                merge_maximum(
                    &mut maximum,
                    &mut exclusive_maximum,
                    high,
                    annotation_true(check.annotation, "exclusiveMaximum"),
                );
            }
            // A bound this port cannot push into a generator would have to be
            // filtered after the fact, which is the shim this replaced.
            _ => return Err(refuse()),
        }
    }
    if minimum.is_some_and(f64::is_nan) || maximum.is_some_and(f64::is_nan) {
        return Err(refuse());
    }
    if integer {
        if let Some(value) = minimum {
            minimum = Some(if exclusive_minimum {
                value.floor() + 1.0
            } else {
                value.ceil()
            });
        }
        if let Some(value) = maximum {
            maximum = Some(if exclusive_maximum {
                value.ceil() - 1.0
            } else {
                value.floor()
            });
        }
    } else {
        if exclusive_minimum && minimum == Some(f64::INFINITY) {
            return Err(refuse());
        }
        if exclusive_maximum && maximum == Some(f64::NEG_INFINITY) {
            return Err(refuse());
        }
        if let Some(value) = minimum {
            minimum = Some(if exclusive_minimum {
                value.next_up()
            } else if value == 0.0 {
                -0.0
            } else {
                value
            });
        }
        if let Some(value) = maximum {
            maximum = Some(if exclusive_maximum {
                value.next_down()
            } else {
                value
            });
        }
    }
    if integer || finite {
        if minimum == Some(f64::INFINITY) || maximum == Some(f64::NEG_INFINITY) {
            return Err(refuse());
        }
        if minimum == Some(f64::NEG_INFINITY) {
            minimum = Some(if integer { -MAX_SAFE } else { -f64::MAX });
        }
        if maximum == Some(f64::INFINITY) {
            maximum = Some(if integer { MAX_SAFE } else { f64::MAX });
        }
    }
    if integer {
        if minimum.is_some_and(|value| value > MAX_SAFE)
            || maximum.is_some_and(|value| value < -MAX_SAFE)
        {
            return Err(refuse());
        }
        minimum = minimum.map(|value| value.max(-MAX_SAFE));
        maximum = maximum.map(|value| value.min(MAX_SAFE));
    }
    if let (Some(low), Some(high)) = (minimum, maximum)
        && low > high
    {
        return Err(refuse());
    }
    Ok((minimum, maximum))
}

/// The tighter of two lower bounds; a tie is exclusive if either is.
fn merge_minimum(
    minimum: &mut Option<f64>,
    exclusive: &mut bool,
    bound: f64,
    bound_exclusive: bool,
) {
    match *minimum {
        Some(current) if current > bound => {}
        Some(current) if current == bound => {
            *exclusive = *exclusive || bound_exclusive;
        }
        _ => {
            *minimum = Some(bound);
            *exclusive = bound_exclusive;
        }
    }
}

/// The tighter of two upper bounds; a tie is exclusive if either is.
fn merge_maximum(
    maximum: &mut Option<f64>,
    exclusive: &mut bool,
    bound: f64,
    bound_exclusive: bool,
) {
    match *maximum {
        Some(current) if current < bound => {}
        Some(current) if current == bound => {
            *exclusive = *exclusive || bound_exclusive;
        }
        _ => {
            *maximum = Some(bound);
            *exclusive = bound_exclusive;
        }
    }
}

fn annotation_true(annotation: &[(String, String)], key: &str) -> bool {
    annotation
        .iter()
        .any(|(name, value)| name == key && value == "true")
}

fn number_arg(args: &[String], index: usize) -> Option<f64> {
    args.get(index)?.parse().ok()
}

/// A double in `[lo, hi]`, drawn without the NaN wildcard the schema-free
/// `number` generator uses: a derived sample must satisfy the bound it was
/// derived from, and NaN satisfies none of them.
fn bounded_number(lo: f64, hi: f64) -> Gen<f64> {
    Gen::new(
        move |rng| {
            if lo == hi {
                return lo;
            }
            let t = (rng.next_u64() >> 11) as f64 / ((1u64 << 53) as f64);
            lo * (1.0 - t) + hi * t
        },
        move |value| {
            let mut out = Vec::new();
            if *value > lo {
                out.push(lo);
                let mid = lo + (*value - lo) / 2.0;
                if mid < *value {
                    out.push(mid);
                }
            }
            out
        },
    )
}

/// How often a broad character class prefers a common code point, matching the
/// `pattern` module's factor.
const BIAS_FACTOR: u32 = 2;

/// The size signal a derivation works from. The pin reads it off the run; the
/// port has no run counter, so every derivation uses one fixed size. It is the
/// largest the carried rows ask for, because a smaller one would cap a pattern
/// that a row drives at that size.
const DERIVATION_SIZE: usize = 32;

/// `Schema.String`: its length bounds, its code-point bounds, and the patterns
/// its checks demand, all pushed into one constructor.
///
/// A pattern is constructed rather than filtered into, which is what makes a
/// length- or alternation-bound pattern reachable at all: rejection sampling
/// never finds `^(?:aaa|\u{1f600}{2})$` by accident. A pattern outside the
/// supported subset is dropped here and enforced by the schema as a residual
/// filter, which is the pin's own split.
fn text(checks: &[CheckRef<'_>], path: &str) -> Result<Gen<Unknown>, DeriveError> {
    let refuses = || DeriveError::new("string constraints", path);
    let mut minimum = 0usize;
    let mut maximum: Option<usize> = None;
    let mut minimum_code_points: Option<usize> = None;
    let mut maximum_code_points: Option<usize> = None;
    let mut patterns: Vec<Pattern> = Vec::new();
    for check in checks {
        let bound = |index: usize| -> Result<usize, DeriveError> {
            let value = number_arg(check.args, index).ok_or_else(refuses)?;
            // The pin refuses a bound that is not a non-negative safe integer.
            if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > MAX_SAFE {
                return Err(refuses());
            }
            Ok(value as usize)
        };
        match check.name {
            "isMinLength" => minimum = minimum.max(bound(0)?),
            "isMaxLength" => {
                let value = bound(0)?;
                maximum = Some(maximum.map_or(value, |current| current.min(value)));
            }
            "isLength" => {
                let value = bound(0)?;
                minimum = minimum.max(value);
                maximum = Some(maximum.map_or(value, |current| current.min(value)));
            }
            "isBetweenLength" => {
                let low = bound(0)?;
                let high = bound(1)?;
                minimum = minimum.max(low);
                maximum = Some(maximum.map_or(high, |current| current.min(high)));
            }
            "isMinCodePoints" => {
                let value = bound(0)?;
                minimum_code_points = Some(minimum_code_points.map_or(value, |c| c.max(value)));
            }
            "isMaxCodePoints" => {
                let value = bound(0)?;
                maximum_code_points = Some(maximum_code_points.map_or(value, |c| c.min(value)));
            }
            "isBetweenCodePoints" => {
                let low = bound(0)?;
                let high = bound(1)?;
                minimum_code_points = Some(minimum_code_points.map_or(low, |c| c.max(low)));
                maximum_code_points = Some(maximum_code_points.map_or(high, |c| c.min(high)));
            }
            "isPattern" => {
                let source = check.args.first().and_then(|arg| pattern::parse_regexp_argument(arg));
                if let Some((source, flags)) = source
                    && let Some(compiled) = Pattern::parse(&source, &flags).filter(Pattern::is_constructible)
                {
                    patterns.push(compiled);
                }
            }
            name => {
                let Some((source, flags)) = built_in_pattern(name, check.args) else {
                    return Err(refuses());
                };
                if let Some(compiled) = Pattern::parse(&source, flags).filter(Pattern::is_constructible) {
                    patterns.push(compiled);
                }
            }
        }
    }
    let has_code_points = minimum_code_points.is_some() || maximum_code_points.is_some();
    let mut minimum_cp = minimum_code_points.unwrap_or(0);
    let mut maximum_cp = maximum_code_points.unwrap_or(usize::MAX / 4);
    if has_code_points {
        // A code point is one or two UTF-16 units, so the two bounds constrain
        // each other: a floor of N code points is at least N units, and a
        // ceiling of M code points is at most 2M units.
        minimum = minimum.max(minimum_cp);
        let doubled = maximum_cp.saturating_mul(2);
        maximum = Some(maximum.map_or(doubled, |current| current.min(doubled)));
        minimum_cp = minimum_cp.max(minimum.div_ceil(2));
        maximum_cp = maximum_cp.min(maximum.unwrap_or(doubled));
        if minimum_cp > maximum_cp {
            return Err(refuses());
        }
    }
    if maximum.is_some_and(|current| current < minimum) {
        return Err(refuses());
    }
    // The longest value worth aiming for: the floor, the shortest match, and
    // the size signal. A code-point bound doubles it, because a longer UTF-16
    // alternative can use fewer code points than the shortest match.
    let mut ceiling = minimum
        .max(patterns.iter().map(Pattern::minimum_length).max().unwrap_or(0))
        .max(DERIVATION_SIZE);
    if has_code_points && !patterns.is_empty() {
        ceiling = ceiling.saturating_mul(2);
    }
    let upper = maximum.map_or(ceiling, |current| current.min(ceiling));
    let shrink_patterns = patterns.clone();
    Ok(Gen::fallible(
        move |rng| {
            let mut random = |bound: u32| rng.below(bound.max(1) as u64) as u32;
            let text = if !patterns.is_empty() {
                let index = random_index(&mut random, patterns.len());
                patterns[index].generate(&mut random, minimum, upper)?
            } else if has_code_points {
                code_point_text(&mut random, minimum, upper, minimum_cp, maximum_cp.min(upper))?
            } else {
                plain_text(&mut random, minimum, upper)
            };
            Some(Unknown::Text(text))
        },
        move |value| {
            let Unknown::Text(text) = value else {
                return Vec::new();
            };
            match shrink_patterns.iter().find(|pattern| pattern.is_match(text)) {
                Some(pattern) => pattern
                    .shrink(text, minimum)
                    .into_iter()
                    .map(Unknown::Text)
                    .collect(),
                None => shrink_text(text, minimum)
                    .into_iter()
                    .map(Unknown::Text)
                    .collect(),
            }
        },
    ))
}

/// The pattern a built-in string check demands: each is a fixed `isPattern` in
/// the pin, which is why a derivation can construct for them at all.
fn built_in_pattern(name: &str, args: &[String]) -> Option<(String, &'static str)> {
    let pattern = match name {
        "isTrimmed" => "^\\S[\\s\\S]*\\S$|^\\S$|^$".to_string(),
        "isUppercased" => "^[^a-z]*$".to_string(),
        "isLowercased" => "^[^A-Z]*$".to_string(),
        "isCapitalized" => "^(?:[^a-z][\\s\\S]*)?$".to_string(),
        "isUncapitalized" => "^(?:[^A-Z][\\s\\S]*)?$".to_string(),
        "isStartingWith" => format!("^{}", escape_pattern(args.first()?)),
        "isEndingWith" => format!("{}$", escape_pattern(args.first()?)),
        "isIncluding" => escape_pattern(args.first()?),
        _ => return None,
    };
    Some((pattern, ""))
}

/// `RegExp.escape`: quote every metacharacter so the text matches literally.
fn escape_pattern(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(
            character,
            '/' | '\\' | '^' | '$' | '.' | '|' | '?' | '*' | '+' | '(' | ')' | '[' | ']' | '{'
                | '}'
        ) {
            out.push('\\');
        }
        out.push(character);
    }
    out
}

fn random_index(random: &mut dyn FnMut(u32) -> u32, length: usize) -> usize {
    if length <= 1 {
        0
    } else {
        (random(length as u32) as usize).min(length - 1)
    }
}

fn random_integer(random: &mut dyn FnMut(u32) -> u32, minimum: usize, maximum: usize) -> usize {
    if minimum >= maximum {
        return minimum;
    }
    minimum + (random((maximum - minimum + 1) as u32) as usize).min(maximum - minimum)
}

/// A length in `[minimum, maximum]`, leaning on the two ends.
fn random_length(random: &mut dyn FnMut(u32) -> u32, minimum: usize, maximum: usize) -> usize {
    if minimum >= maximum {
        return minimum;
    }
    if random(2) == 0 {
        return if random(2) == 0 { minimum } else { maximum };
    }
    random_integer(random, minimum, maximum)
}

/// Strings the pin samples before falling back to printable ASCII, because a
/// boundary a reader cares about (empty, whitespace, a digit, a lone unit) is
/// unlikely to be drawn.
const TEXT_EDGE_CASES: &[&str] = &[
    "", " ", "\t", "\n", "\0", "0", "1", "-1", "NaN", "true", "false", "__proto__",
    "constructor", "prototype", "toString", "\u{fffd}", "😀",
];

fn plain_text(random: &mut dyn FnMut(u32) -> u32, minimum: usize, maximum: usize) -> String {
    if random(BIAS_FACTOR) == 0 {
        let eligible: Vec<&&str> = TEXT_EDGE_CASES
            .iter()
            .filter(|value| {
                let length = utf16_len(value);
                length >= minimum && length <= maximum
            })
            .collect();
        if !eligible.is_empty() {
            return (*eligible[random_index(random, eligible.len())]).to_string();
        }
    }
    let length = random_length(random, minimum, maximum);
    (0..length)
        .map(|_| char::from_u32(random_integer(random, 32, 126) as u32).unwrap_or('a'))
        .collect()
}

/// A string drawn by code point, so supplementary characters are reachable
/// even when the UTF-16 length bound is what pins the length.
fn code_point_text(
    random: &mut dyn FnMut(u32) -> u32,
    minimum: usize,
    maximum: usize,
    minimum_code_points: usize,
    maximum_code_points: usize,
) -> Option<String> {
    let count = random_length(random, minimum_code_points, maximum_code_points);
    let low = minimum.max(count);
    let high = maximum.min(count.saturating_mul(2));
    if low > high {
        return None;
    }
    let length = random_length(random, low, high);
    let mut pairs = length - count;
    let mut value = String::new();
    for remaining in (1..=count).rev() {
        if random_integer(random, 1, remaining) <= pairs {
            value.push(char::from_u32(random_integer(random, 0x1_0000, 0x10_ffff) as u32)?);
            pairs -= 1;
        } else {
            value.push(char::from_u32(random_integer(random, 32, 126) as u32)?);
        }
    }
    Some(value)
}

/// The pin's string shrinker: drop the tail down to the floor, then walk each
/// UTF-16 unit toward null.
fn shrink_text(value: &str, minimum: usize) -> Vec<String> {
    let units: Vec<u16> = value.encode_utf16().collect();
    let mut out: Vec<String> = Vec::new();
    if units.len() > minimum {
        out.push(String::from_utf16_lossy(&units[..minimum]));
        out.push(String::from_utf16_lossy(&units[..minimum.max(units.len() / 2)]));
        out.push(String::from_utf16_lossy(&units[..units.len() - 1]));
    }
    for index in 0..units.len() {
        for candidate in halving_steps(units[index] as i64) {
            let mut copy = units.clone();
            copy[index] = candidate as u16;
            out.push(String::from_utf16_lossy(&copy));
        }
    }
    let mut unique: Vec<String> = Vec::new();
    for candidate in out {
        if candidate != value && !unique.contains(&candidate) {
            unique.push(candidate);
        }
    }
    unique
}

/// Values between `value` and `target` by halving the gap, largest first: the
/// pin's `shrinkInteger` with `tryTargetAsap`.
fn halving_steps(value: i64) -> Vec<i64> {
    let mut out = Vec::new();
    let mut gap = value;
    while gap != 0 {
        out.push(value - gap);
        gap /= 2;
    }
    out
}

/// `Schema.Struct`: every field derived from its own shape.
fn structure(fields: &[(String, SchemaKind)], path: &str) -> Result<Gen<Unknown>, DeriveError> {
    let parts = fields
        .iter()
        .map(|(name, kind)| Ok((name.clone(), derive(kind, &format!("{path}.{name}"))?)))
        .collect::<Result<Vec<_>, DeriveError>>()?;
    Ok(Gen::new(
        move |rng| {
            Unknown::Object(
                parts
                    .iter()
                    .map(|(name, part)| (name.clone(), part.sample(rng)))
                    .collect(),
            )
        },
        |_| Vec::new(),
    ))
}

/// `Schema.Tuple`: every position derived.
fn tuple_of(elements: &[SchemaKind], path: &str) -> Result<Gen<Unknown>, DeriveError> {
    let parts = elements
        .iter()
        .enumerate()
        .map(|(index, kind)| derive(kind, &format!("{path}[{index}]")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Gen::new(
        move |rng| Unknown::Array(parts.iter().map(|part| part.sample(rng)).collect()),
        |_| Vec::new(),
    ))
}

/// `Schema.Array`: a short array, because a derivation has no size signal.
fn varied(inner: Gen<Unknown>) -> Gen<Unknown> {
    array(inner, 0, 3).map(Unknown::Array)
}

/// Records as key/value vecs from parallel generators.
pub fn record<K, V>(keys: Gen<K>, values: Gen<V>, len: usize) -> Gen<Vec<(K, V)>>
where
    K: Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    Gen::new(
        move |rng| {
            (0..len)
                .map(|_| (keys.sample(rng), values.sample(rng)))
                .collect()
        },
        |_| Vec::new(),
    )
}
