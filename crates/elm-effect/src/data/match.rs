//! `Match.ts` port: tag and value matchers with fallthrough.
//!
//! A [`Matcher`] is a list of arms tried in order; the first hit wins and
//! a miss falls through to the next arm (or the `otherwise`). [`tag`]
//! matches an exact `_tag`, [`tag_starts_with`] a tag prefix, [`guarded`]
//! an arbitrary predicate over a projected value. Symbol-keyed properties
//! are plain string keys here (no `unique symbol` in `std`).

/// A matchable input: tagged values, nulls, and everything else.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// `{ _tag, ... }` with string fields.
    Tagged {
        /// The `_tag` discriminant.
        tag: String,
        /// Extra string fields (pattern properties read these).
        fields: Vec<(String, String)>,
    },
    /// `null` / missing: every tag arm falls through.
    Null,
    /// Anything else (numbers, plain strings, ...).
    Other(String),
}

impl Input {
    /// Read a pattern property (string or symbol-keyed alike).
    pub fn prop(&self, name: &str) -> Option<&str> {
        match self {
            Self::Tagged { fields, .. } => fields
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str()),
            _ => None,
        }
    }

    /// The `_tag`, if tagged.
    pub fn tag(&self) -> Option<&str> {
        match self {
            Self::Tagged { tag, .. } => Some(tag),
            _ => None,
        }
    }
}

/// One arm: a predicate plus its body.
pub struct Arm<R> {
    check: Box<dyn Fn(&Input) -> bool>,
    body: Box<dyn Fn(&Input) -> R>,
}

/// An ordered arm list with an `otherwise` fallback.
#[derive(Default)]
pub struct Matcher<R> {
    arms: Vec<Arm<R>>,
    otherwise: Option<Box<dyn Fn(&Input) -> R>>,
}

impl<R> Matcher<R> {
    /// Empty matcher; add arms with [`Matcher::arm`].
    pub fn new() -> Self {
        Self {
            arms: Vec::new(),
            otherwise: None,
        }
    }

    /// Push an arm.
    pub fn arm(
        mut self,
        check: impl Fn(&Input) -> bool + 'static,
        body: impl Fn(&Input) -> R + 'static,
    ) -> Self {
        self.arms.push(Arm {
            check: Box::new(check),
            body: Box::new(body),
        });
        self
    }

    /// Fallback when no arm hits.
    pub fn otherwise(mut self, body: impl Fn(&Input) -> R + 'static) -> Self {
        self.otherwise = Some(Box::new(body));
        self
    }

    /// Run: first hit wins, else the fallback (panics with none, like an
    /// non-exhaustive `Match` without `otherwise`).
    pub fn run(&self, input: &Input) -> R {
        for arm in &self.arms {
            if (arm.check)(input) {
                return (arm.body)(input);
            }
        }
        match &self.otherwise {
            Some(f) => f(input),
            None => panic!("non-exhaustive match"),
        }
    }
}

/// Exact-`_tag` arm check (`Match.tag`). `Null` never matches.
pub fn tag(expected: &str) -> impl Fn(&Input) -> bool + '_ {
    move |input| input.tag() == Some(expected)
}

/// `_tag`-prefix arm check (`Match.tagStartsWith`).
pub fn tag_starts_with(prefix: &str) -> impl Fn(&Input) -> bool + '_ {
    move |input| input.tag().is_some_and(|t| t.starts_with(prefix))
}

/// Predicate arm over a projected value (`Match.fn`).
pub fn guarded<A>(
    project: impl Fn(&Input) -> A + 'static,
    check: impl Fn(&A) -> bool + 'static,
) -> impl Fn(&Input) -> bool {
    move |input| check(&project(input))
}
