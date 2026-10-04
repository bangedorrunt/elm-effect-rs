//! `RegExp.ts` port: the global constructor plus helpers.
//!
//! [`RegExp`] re-exports nothing but names the concept: a pattern string
//! with flags. [`is_regexp`] always holds for values built here (there is
//! no foreign `RegExp` to distinguish in `std`). [`escape`] quotes every
//! regex metacharacter so the result matches literally. No test rows name
//! this module; matching engines belong to whoever adds the `regex`
//! dependency.

/// A regex pattern plus flags (`RegExp` shape, engine-free).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegExp {
    /// Source text.
    pub source: String,
    /// Flags (`"gi"` and friends, kept verbatim).
    pub flags: String,
}

impl RegExp {
    /// Build from source and flags.
    pub fn new(source: impl Into<String>, flags: impl Into<String>) -> Self {
        Self { source: source.into(), flags: flags.into() }
    }

    /// Render `/source/flags`.
    pub fn to_pattern(&self) -> String {
        format!("/{0}/{1}", self.source, self.flags)
    }
}

/// Always true for [`RegExp`] values (`isRegExp`).
pub fn is_regexp(_: &RegExp) -> bool {
    true
}

/// Quote regex metacharacters (`RegExp.escape`).
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '^' | '$' | '.' | '|' | '?' | '*' | '+' | '(' | ')' | '[' | ']' | '{' | '}'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
