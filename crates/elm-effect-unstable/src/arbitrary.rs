//! Arbitrary value generation (`arbitrary/Arbitrary` at the pin): seeded
//! generators with local shrinking.
//!
//! A [`Gen`] pairs generation with shrink candidates; [`Rng`] is a seeded
//! xorshift stream, so samples are deterministic per seed
//! ("generates deterministic samples"). Bounds reject out-of-range
//! construction; integers shrink toward the failure boundary; composition
//! runs through `map`/`filter`/`tuple`/`record`/`option`/`array`.
//!
//! Residuals (fast-check internals, out of timebox): model-based command
//! shrinking, UTF-16 pattern generation, constraint pushing into
//! primitives, schema-derived shrinkers, and replay orchestration.

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

/// Sampler: stream in, value out.
type SampleFn<T> = std::sync::Arc<dyn Fn(&mut Rng) -> T + Send + Sync>;
/// Shrinker: failing value in, smaller candidates out.
type ShrinkFn<T> = std::sync::Arc<dyn Fn(&T) -> Vec<T> + Send + Sync>;

/// A generator: sampling plus shrink candidates toward a boundary.
#[derive(Clone)]
pub struct Gen<T> {
    run: SampleFn<T>,
    shrink: ShrinkFn<T>,
}

impl<T> Gen<T> {
    /// Build from a sampler and a shrinker.
    pub fn new(
        run: impl Fn(&mut Rng) -> T + Send + Sync + 'static,
        shrink: impl Fn(&T) -> Vec<T> + Send + Sync + 'static,
    ) -> Self {
        Self {
            run: std::sync::Arc::new(run),
            shrink: std::sync::Arc::new(shrink),
        }
    }

    /// Sample one value.
    pub fn sample(&self, rng: &mut Rng) -> T {
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
