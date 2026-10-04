//! `Filter.ts` port: composable checks that narrow or transform.
//!
//! A [`Filter`] takes an input and returns `Ok(pass)` or `Err(fail)`.
//! Builders cover predicates ([`from_predicate`]), exact values
//! ([`equals_strict`]), string/number/boolean shapes, property presence
//! ([`has`]), and tag checks ([`instance_of`] by tag name: no runtime
//! classes in `std`). No test rows name this module; the surface stays
//! narrow on purpose.

/// `Filter<Input, Pass, Fail>`: check that can also transform.
pub struct Filter<Input, Pass, Fail> {
    run: Box<dyn Fn(Input) -> Result<Pass, Fail>>,
}

impl<Input, Pass, Fail> Filter<Input, Pass, Fail> {
    /// Build from a raw function (`Filter.make`).
    pub fn make(run: impl Fn(Input) -> Result<Pass, Fail> + 'static) -> Self {
        Self { run: Box::new(run) }
    }

    /// Run the check.
    pub fn run(&self, input: Input) -> Result<Pass, Fail> {
        (self.run)(input)
    }

    /// Map the failure (`mapFail`).
    pub fn map_fail<Fail2>(
        self,
        f: impl Fn(Fail) -> Fail2 + 'static,
    ) -> Filter<Input, Pass, Fail2>
    where
        Input: 'static,
        Pass: 'static,
        Fail: 'static,
    {
        Filter::make(move |input| self.run(input).map_err(&f))
    }

    /// Plain predicate view (`toPredicate`): pass becomes `true`.
    pub fn to_predicate(self) -> impl Fn(Input) -> bool
    where
        Input: 'static,
        Pass: 'static,
        Fail: 'static,
    {
        move |input| self.run(input).is_ok()
    }
}

/// Pass inputs satisfying the predicate, failing with the input itself.
pub fn from_predicate<T: 'static>(
    check: impl Fn(&T) -> bool + 'static,
) -> Filter<T, T, T> {
    Filter::make(move |input: T| if check(&input) { Ok(input) } else { Err(input) })
}

/// Exact strict equality (`equalsStrict`, `NaN`-aware like `Equal`).
pub fn equals_strict<T: PartialEq + Clone + 'static>(expected: T) -> Filter<T, T, String> {
    Filter::make(move |input: T| {
        if input == expected {
            Ok(input)
        } else {
            Err("did not equal expected value".to_string())
        }
    })
}

/// String shape check (`Filter.string`).
pub fn string(input: &str) -> Result<String, String> {
    Ok(input.to_string())
}

/// Property presence (`Filter.has`): the map must contain `key`.
pub fn has<V: Clone>(
    map: &std::collections::BTreeMap<String, V>,
    key: &str,
) -> Result<V, String> {
    map.get(key).cloned().ok_or_else(|| format!("missing property: {key}"))
}

/// Tag check (`Filter.instanceOf` by tag name).
pub fn instance_of(tag: &str, actual: &str) -> Result<(), String> {
    if tag == actual {
        Ok(())
    } else {
        Err(format!("expected tag {tag}, got {actual}"))
    }
}
