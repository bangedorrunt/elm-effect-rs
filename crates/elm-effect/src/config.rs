//! Parsed-at-startup descriptors.
//!
//! A [`Config`] describes one value: primitives ([`Config::string`],
//! [`number`](Config::number), [`boolean`](Config::integer) and friends),
//! literals, secrets, redaction, dates, durations, byte sizes, URLs, ports,
//! arrays, records, and the [`option`](Config::option)/[`nested`]/
//! [`with_default`](Config::with_default)/[`or_else`](Config::or_else)/
//! [`map`](Config::map)/[`flat_map`](Config::flat_map) combinators plus
//! [`all`](Config::all) tuples. [`Config::load`] reads one descriptor
//! against a [`Provider`](crate::config_provider::Provider): domain code
//! never touches the environment. Failures are [`ConfigError`] values
//! carrying paths (`Missing` for absence, `Invalid` for bad input).
use super::config_provider::{Lookup, Provider, ProviderError};

// The crate root re-exports `Defect`; a path-included test host provides the
// same name at its own root.
use crate::Defect;

/// Stringly value with its source path.
#[derive(Clone, Debug, PartialEq)]
pub struct Sourced {
    /// Lookup path.
    pub path: Vec<String>,
    /// Raw text (missing when absent).
    pub text: Option<String>,
}

/// Config failure: absent input or undecodable input, with paths.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigError {
    /// No value at this path.
    Missing {
        /// Lookup path.
        path: Vec<String>,
    },
    /// Value present but undecodable.
    Invalid {
        /// Lookup path.
        path: Vec<String>,
        /// What was found.
        got: String,
        /// Why it fails.
        reason: String,
    },
    /// The provider itself failed at this path (`ConfigProvider.SourceError`).
    /// Absence and a failed read are distinct: only absence is recoverable by
    /// a default or an option (Config.test.ts:1021, :1308).
    Source {
        /// Lookup path where the source failed.
        path: Vec<String>,
        /// The source's own message.
        message: String,
    },
    /// The provider died. Not a typed failure: no default, option, or
    /// `or_else` recovers it (`Config.test.ts:65`); [`Config::load`] returns
    /// it rather than pretending the path was absent.
    Defect {
        /// The provider's defect.
        defect: Defect,
    },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { path } => write!(f, "missing config: {}", path.join(".")),
            Self::Invalid { path, got, reason } => {
                write!(
                    f,
                    "invalid config {}: got {got:?}: {reason}",
                    path.join(".")
                )
            }
            Self::Source { path, message } => {
                write!(f, "config source at {}: {message}", path.join("."))
            }
            Self::Defect { defect } => write!(f, "config provider defect: {defect}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Decoded configuration value. `Debug` masks secrets: [`Value::Secret`]
/// prints as `***` and [`Value::Redacted`] never renders content.
#[derive(Clone, PartialEq)]
pub enum Value {
    /// String.
    Str(String),
    /// Number.
    Num(f64),
    /// Integer.
    Int(i64),
    /// Boolean.
    Bool(bool),
    /// Secret (masked in debug).
    Secret(String),
    /// Redacted (never displayed).
    Redacted(String),
    /// Milliseconds count. An infinite duration is an infinite count, which is
    /// how Effect 4's sentinels survive the value model (`Config.Duration`).
    Millis(f64),
    /// JSON null (`Config.schema(Schema.Null)`).
    Null,
    /// `undefined`, which a decode can produce and JSON cannot spell.
    Undefined,
    /// List.
    List(Vec<Value>),
    /// Map.
    Map(Vec<(String, Value)>),
    /// Absent option.
    None,
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Str(s) => write!(f, "Str({s:?})"),
            Self::Num(x) => write!(f, "Num({x:?})"),
            Self::Int(n) => write!(f, "Int({n:?})"),
            Self::Bool(b) => write!(f, "Bool({b:?})"),
            Self::Secret(_) => write!(f, "Secret(***)"),
            Self::Redacted(_) => write!(f, "Redacted(***)"),
            Self::Millis(ms) => write!(f, "Millis({ms:?})"),
            Self::Null => write!(f, "Null"),
            Self::Undefined => write!(f, "Undefined"),
            Self::List(items) => write!(f, "List({items:?})"),
            Self::Map(pairs) => write!(f, "Map({pairs:?})"),
            Self::None => write!(f, "None"),
        }
    }
}

impl Value {
    /// Wrap as present option.
    pub fn some(self) -> Option<Value> {
        Some(self)
    }
}

/// What `map_effect` asks of a transform: run the description and answer the
/// value or the failure it raised.
///
/// Implemented for the effect type in `config_effect.rs` rather than here, so
/// this module never names a `crate::` path and the path-module tests can keep
/// loading it as a plain module tree.
pub trait ConfigEffect {
    /// Drive the transform. `path` is the descriptor's own lookup path, which
    /// an error raised without one carries.
    fn resolve(self, path: &[String]) -> Result<Value, ConfigError>;
}

/// A config descriptor: named lookup plus decode.
#[derive(Clone)]
pub struct Config {
    name: Option<String>,
    run: std::rc::Rc<dyn Fn(&dyn Lookup, &[String]) -> Result<Value, Vec<ConfigError>>>,
}

impl Config {
    fn leaf(
        name: Option<String>,
        f: impl Fn(Option<String>, Vec<String>) -> Result<Value, Vec<ConfigError>> + 'static,
    ) -> Self {
        let name_owned = name.clone();
        Self {
            name,
            run: std::rc::Rc::new(move |provider, prefix| {
                let mut path = prefix.to_vec();
                if let Some(n) = &name_owned {
                    path.push(n.clone());
                }
                match provider.try_get(&path) {
                    Ok(text) => f(text, path),
                    // A failed read is not an absent key: it keeps its own kind
                    // so a default and an option leave it alone. A provider
                    // that died is not a failed read either, so it stays a
                    // defect that `or_else` may not answer for.
                    Err(ProviderError::Source(message)) => {
                        Err(vec![ConfigError::Source { path, message }])
                    }
                    Err(ProviderError::Defect(defect)) => Err(vec![ConfigError::Defect { defect }]),
                }
            }),
        }
    }

    /// Any present string; absence is [`ConfigError::Missing`].
    pub fn string(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) => Ok(Value::Str(t)),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// Non-empty string; preserved empties are invalid.
    pub fn non_empty_string(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) if !t.is_empty() => Ok(Value::Str(t)),
            Some(t) => Err(vec![ConfigError::Invalid {
                path,
                got: t,
                reason: "empty string".to_string(),
            }]),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// f64 (finite or not); garbage is invalid.
    pub fn number(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) => t.parse::<f64>().map(Value::Num).map_err(|_| {
                vec![ConfigError::Invalid {
                    path,
                    got: t,
                    reason: "not a number".to_string(),
                }]
            }),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// Finite f64 only.
    pub fn finite(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) => match t.parse::<f64>() {
                Ok(x) if x.is_finite() => Ok(Value::Num(x)),
                _ => Err(vec![ConfigError::Invalid {
                    path,
                    got: t,
                    reason: "not finite".to_string(),
                }]),
            },
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// Whole integers only.
    pub fn integer(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) => match t.parse::<i64>() {
                Ok(n) => Ok(Value::Int(n)),
                _ => Err(vec![ConfigError::Invalid {
                    path,
                    got: t,
                    reason: "not an integer".to_string(),
                }]),
            },
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// `true`/`false` plus `yes`/`no`, `on`/`off`, `1`/`0`, `y`/`n`.
    pub fn boolean(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) => match t.to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" | "y" => Ok(Value::Bool(true)),
                "false" | "no" | "off" | "0" | "n" => Ok(Value::Bool(false)),
                _ => Err(vec![ConfigError::Invalid {
                    path,
                    got: t,
                    reason: "not a boolean".to_string(),
                }]),
            },
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// Exact literal match.
    pub fn literal(name: Option<String>, expected: &str) -> Self {
        let expected = expected.to_string();
        Self::leaf(name, move |text, path| match text {
            Some(t) if t == expected => Ok(Value::Str(t)),
            Some(t) => Err(vec![ConfigError::Invalid {
                path,
                got: t,
                reason: format!("expected literal {expected:?}"),
            }]),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// One of several string alternatives.
    pub fn literals(name: Option<String>, options: &[&str]) -> Self {
        let options: Vec<String> = options.iter().map(|s| s.to_string()).collect();
        Self::leaf(name, move |text, path| match text {
            Some(t) if options.contains(&t) => Ok(Value::Str(t)),
            Some(t) => Err(vec![ConfigError::Invalid {
                path,
                got: t,
                reason: format!("expected one of {options:?}"),
            }]),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// Secret string (masked on display).
    pub fn secret(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) => Ok(Value::Secret(t)),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// Redacted string (never displayed).
    pub fn redacted(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) => Ok(Value::Redacted(t)),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// Milliseconds (`"150ms"`, `"2s"`, `"1m"`, `"1h"`, plain millis).
    pub fn duration_ms(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) => parse_duration_ms(&t).map(Value::Millis).ok_or_else(|| {
                vec![ConfigError::Invalid {
                    path,
                    got: t,
                    reason: "bad duration".to_string(),
                }]
            }),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// `Config.Duration`: Effect 4's own duration spellings, a count and a unit
    /// name (`"1000 millis"`, `"1 second"`, `"1.5 hours"`) plus the two
    /// infinite sentinels, decoded as milliseconds. The value is Effect 4's
    /// `Duration` in the only unit [`Value`] carries.
    ///
    /// The parser mirrors `Duration::from_input_string` (time.rs) rather than
    /// calling it: this file is included by path from test targets that have no
    /// `time` module, so naming the type here would not compile.
    pub fn duration(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) => parse_pin_duration_ms(&t).map(Value::Millis).ok_or_else(|| {
                vec![ConfigError::Invalid {
                    path,
                    got: t,
                    reason: "a valid Duration string".to_string(),
                }]
            }),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// Strict civil-date check for [`Config::date`]: `YYYY-MM-DD` with real
    /// month/day ranges (leap-aware); a `T`/space time tail may follow.
    fn is_calendar_date(s: &str) -> bool {
        let head = s.split(['T', ' ']).next().unwrap_or("");
        let b = head.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return false;
        }
        let digits = |lo: usize, hi: usize| -> Option<u32> {
            let mut n = 0u32;
            for &c in &b[lo..hi] {
                if !c.is_ascii_digit() {
                    return None;
                }
                n = n * 10 + (c - b'0') as u32;
            }
            Some(n)
        };
        let (Some(y), Some(m), Some(d)) = (digits(0, 4), digits(5, 7), digits(8, 10)) else {
            return false;
        };
        if m == 0 || m > 12 || d == 0 {
            return false;
        }
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let dim = match m {
            2 if leap => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        d <= dim
    }

    /// ISO date string, kept verbatim after real calendar validation
    /// (`YYYY-MM-DD`, leap-aware; an optional `T`/space time tail is
    /// allowed and unchecked beyond presence).
    pub fn date(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) if Self::is_calendar_date(&t) => Ok(Value::Str(t)),
            Some(t) => Err(vec![ConfigError::Invalid {
                path,
                got: t,
                reason: "bad date".to_string(),
            }]),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// Byte count (`"4MB"`, `"2KiB"`, plain bytes).
    pub fn byte_size(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) => parse_bytes(&t).map(Value::Int).ok_or_else(|| {
                vec![ConfigError::Invalid {
                    path,
                    got: t,
                    reason: "bad byte size".to_string(),
                }]
            }),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// URL string (scheme required).
    pub fn url(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) if t.contains("://") => Ok(Value::Str(t)),
            Some(t) => Err(vec![ConfigError::Invalid {
                path,
                got: t,
                reason: "bad url".to_string(),
            }]),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// TCP port 1-65535, decoded as a number.
    pub fn port(name: Option<String>) -> Self {
        Self::leaf(name, |text, path| match text {
            Some(t) => match t.parse::<u16>() {
                Ok(n) if n >= 1 => Ok(Value::Int(n as i64)),
                _ => Err(vec![ConfigError::Invalid {
                    path,
                    got: t,
                    reason: "bad port".to_string(),
                }]),
            },
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// Log level name.
    pub fn log_level(name: Option<String>) -> Self {
        const LEVELS: &[&str] = &["Trace", "Debug", "Info", "Warning", "Error", "Fatal"];
        Self::leaf(name, move |text, path| match text {
            Some(t) if LEVELS.contains(&t.as_str()) => Ok(Value::Str(t)),
            Some(t) => Err(vec![ConfigError::Invalid {
                path,
                got: t,
                reason: "bad log level".to_string(),
            }]),
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// Comma-separated list of an inner string shape.
    pub fn array(name: Option<String>, inner: Config) -> Self {
        Self::leaf(name, move |text, path| match text {
            Some(t) => {
                let mut out = Vec::new();
                let mut errors = Vec::new();
                for (i, part) in t.split(',').enumerate() {
                    let mut sub = path.clone();
                    sub.push(i.to_string());
                    match inner.run_against(Some(part.trim().to_string()), &sub) {
                        Ok(v) => out.push(v),
                        Err(mut e) => errors.append(&mut e),
                    }
                }
                if errors.is_empty() {
                    Ok(Value::List(out))
                } else {
                    Err(errors)
                }
            }
            None => Err(vec![ConfigError::Missing { path }]),
        })
    }

    /// Combine configs positionally (`Config.all` tuple/iterable shape):
    /// every member loads under the same prefix; values keep order,
    /// errors accumulate.
    pub fn all(configs: Vec<Config>) -> Self {
        Self {
            name: None,
            run: std::rc::Rc::new(move |provider, prefix| {
                let mut out = Vec::new();
                let mut errors = Vec::new();
                for c in &configs {
                    match (c.run)(provider, prefix) {
                        Ok(v) => out.push(v),
                        Err(mut e) => errors.append(&mut e),
                    }
                }
                if errors.is_empty() {
                    Ok(Value::List(out))
                } else {
                    Err(group_errors(errors))
                }
            }),
        }
    }

    /// Combine named configs (`Config.all` record / `unwrap` shape):
    /// values keep their keys, errors accumulate.
    pub fn record(fields: Vec<(String, Config)>) -> Self {
        Self {
            name: None,
            run: std::rc::Rc::new(move |provider, prefix| {
                let mut out = Vec::new();
                let mut errors = Vec::new();
                for (key, c) in &fields {
                    match (c.run)(provider, prefix) {
                        Ok(v) => out.push((key.clone(), v)),
                        Err(mut e) => errors.append(&mut e),
                    }
                }
                if errors.is_empty() {
                    Ok(Value::Map(out))
                } else {
                    Err(group_errors(errors))
                }
            }),
        }
    }

    /// Always-failing config.
    pub fn fail(message: &str) -> Self {
        let message = message.to_string();
        Self {
            name: None,
            run: std::rc::Rc::new(move |_, prefix| {
                Err(vec![ConfigError::Invalid {
                    path: prefix.to_vec(),
                    got: String::new(),
                    reason: message.clone(),
                }])
            }),
        }
    }

    /// Provider-independent value.
    pub fn succeed(value: Value) -> Self {
        Self {
            name: None,
            run: std::rc::Rc::new(move |_, _| Ok(value.clone())),
        }
    }

    /// `Config.mapEffect`: transform the decoded value through a described
    /// effect.
    ///
    /// Effect 4 runs the transform in the fiber that parses the config. A load
    /// here is a bare `Result` with no fiber, so the description is driven on
    /// the calling thread and must complete without suspending: a transform
    /// that sleeps or forks has nowhere to park. A typed failure becomes the
    /// load's failure, which `or_else` recovers from exactly as Effect 4's
    /// `orElse` does; a defect has no channel of its own in a load result, so
    /// it is reported as an invalid value at the descriptor's path.
    pub fn map_effect<F, M>(self, f: F) -> Self
    where
        F: Fn(Value) -> M + 'static,
        M: ConfigEffect,
    {
        let name = self.name.clone();
        let failure_name = name.clone();
        let run = self.run.clone();
        Self {
            name,
            run: std::rc::Rc::new(move |provider, prefix| {
                let value = (*run)(provider, prefix)?;
                let mut path = prefix.to_vec();
                if let Some(n) = &failure_name {
                    path.push(n.clone());
                }
                f(value).resolve(&path).map_err(|error| vec![error])
            }),
        }
    }

    /// Map the decoded value.
    pub fn map(self, f: impl Fn(Value) -> Value + 'static) -> Self {
        let name = self.name.clone();
        let run = self.run.clone();
        Self {
            name,
            run: std::rc::Rc::new(move |provider, prefix| (*run)(provider, prefix).map(&f)),
        }
    }

    /// Check the decoded value; failures carry the leaf path (the
    /// wrapped descriptor's name appended to the active prefix).
    pub fn validate(self, f: impl Fn(&Value) -> Result<(), String> + 'static) -> Self {
        let name = self.name.clone();
        let leaf = name.clone();
        let run = self.run.clone();
        Self {
            name,
            run: std::rc::Rc::new(move |provider, prefix| match (*run)(provider, prefix) {
                Ok(value) => match f(&value) {
                    Ok(()) => Ok(value),
                    Err(reason) => {
                        let mut path = prefix.to_vec();
                        if let Some(n) = &leaf {
                            path.push(n.clone());
                        }
                        Err(vec![ConfigError::Invalid {
                            path,
                            got: format!("{value:?}"),
                            reason,
                        }])
                    }
                },
                err => err,
            }),
        }
    }

    /// Chain on the decoded value.
    pub fn flat_map(self, f: impl Fn(Value) -> Config + 'static) -> Self {
        let run = self.run.clone();
        Self {
            name: None,
            run: std::rc::Rc::new(move |provider, prefix| match (*run)(provider, prefix) {
                Ok(v) => {
                    let next = f(v);
                    (next.run)(provider, prefix)
                }
                Err(e) => Err(e),
            }),
        }
    }

    /// Fall back on any failure, preserving provider defects first: a
    /// missing/error result tries the other branch.
    pub fn or_else(self, other: Config) -> Self {
        let run = self.run.clone();
        let run_other = other.run.clone();
        Self {
            name: None,
            run: std::rc::Rc::new(move |provider, prefix| match (*run)(provider, prefix) {
                ok @ Ok(_) => ok,
                // A defect is not a config failure: the fallback would be
                // answering for the provider dying, which Effect 4 forbids
                // (Config.test.ts:65).
                Err(errors) if errors.iter().any(is_defect) => Err(errors),
                Err(_) => (*run_other)(provider, prefix),
            }),
        }
    }

    /// Fall back with the failure in hand (`orElse` with a recovery
    /// function): the fallback sees every [`ConfigError`], but a provider
    /// defect is not offered to it at all — Effect 4's `orElse` never runs on a
    /// defect (`Config.test.ts:65`).
    pub fn or_else_with(self, f: impl Fn(Vec<ConfigError>) -> Config + 'static) -> Self {
        let run = self.run.clone();
        Self {
            name: None,
            run: std::rc::Rc::new(move |provider, prefix| match (*run)(provider, prefix) {
                ok @ Ok(_) => ok,
                Err(errors) if errors.iter().any(is_defect) => Err(errors),
                Err(errors) => {
                    let fallback = f(errors);
                    (fallback.run)(provider, prefix)
                }
            }),
        }
    }

    /// Default on absence only (invalid input still fails).
    pub fn with_default(self, default: Value) -> Self {
        let run = self.run.clone();
        Self {
            name: None,
            run: std::rc::Rc::new(move |provider, prefix| match (*run)(provider, prefix) {
                Err(e) if e.iter().all(|x| matches!(x, ConfigError::Missing { .. })) => {
                    Ok(default.clone())
                }
                other => other,
            }),
        }
    }

    /// Optional: absence becomes [`Value::None`].
    pub fn option(self) -> Self {
        let run = self.run.clone();
        Self {
            name: None,
            run: std::rc::Rc::new(move |provider, prefix| match (*run)(provider, prefix) {
                Err(e) if e.iter().all(|x| matches!(x, ConfigError::Missing { .. })) => {
                    Ok(Value::None)
                }
                other => other,
            }),
        }
    }

    /// Scope lookups under a prefix.
    pub fn nested(self, prefix: &str) -> Self {
        let run = self.run.clone();
        let prefix = prefix.to_string();
        Self {
            name: None,
            run: std::rc::Rc::new(move |provider, outer| {
                let mut full = outer.to_vec();
                full.extend(prefix.split('.').map(str::to_string));
                run(provider, &full)
            }),
        }
    }

    /// Run against one provider with no prefix.
    pub fn load(&self, provider: &dyn Lookup) -> Result<Value, Vec<ConfigError>> {
        (self.run)(provider, &[])
    }

    fn run_against(
        &self,
        text: Option<String>,
        path: &[String],
    ) -> Result<Value, Vec<ConfigError>> {
        let provider = Provider::single(path.to_vec(), text);
        (self.run)(&provider, path)
    }
}

/// Whether a failure is the provider dying, which no wrapper recovers.
fn is_defect(error: &ConfigError) -> bool {
    matches!(error, ConfigError::Defect { .. })
}

/// Group-error selection (`Config.all`): a present-but-undecodable child
/// outranks a missing sibling, so a mixed group reports only the invalid
/// errors (Config.test.ts:990). An all-missing group keeps every missing
/// error, which is what `withDefault` and `option` recover from.
fn group_errors(errors: Vec<ConfigError>) -> Vec<ConfigError> {
    // A provider defect outranks everything: it is the reason nothing could be
    // read, and no wrapper may recover it (Config.test.ts:65).
    if errors.iter().any(is_defect) {
        return errors.into_iter().filter(is_defect).collect();
    }
    // A failed read is reported alone, ahead of a missing or invalid sibling:
    // the failure is the reason the group could not be read, and a wrapper may
    // not recover it (Config.test.ts:1021).
    if errors
        .iter()
        .any(|e| matches!(e, ConfigError::Source { .. }))
    {
        errors
            .into_iter()
            .filter(|e| matches!(e, ConfigError::Source { .. }))
            .collect()
    } else if errors
        .iter()
        .any(|e| matches!(e, ConfigError::Invalid { .. }))
    {
        errors
            .into_iter()
            .filter(|e| matches!(e, ConfigError::Invalid { .. }))
            .collect()
    } else {
        errors
    }
}

fn parse_duration_ms(s: &str) -> Option<f64> {
    let (n, unit) = if let Some(b) = s.strip_suffix("ms") {
        (b, 1.0)
    } else if let Some(b) = s.strip_suffix('s') {
        (b, 1_000.0)
    } else if let Some(b) = s.strip_suffix('m') {
        (b, 60_000.0)
    } else if let Some(b) = s.strip_suffix('h') {
        (b, 3_600_000.0)
    } else {
        (s, 1.0)
    };
    // `1m` vs `1ms` ambiguity: prefer ms above, so a trailing `m` here is minutes.
    n.parse::<f64>().ok().map(|v| v * unit)
}

/// Effect 4's `DurationFromString` readings, in milliseconds.
fn parse_pin_duration_ms(text: &str) -> Option<f64> {
    match text {
        "Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    // A unit is required, singular or plural, with no padding on either side.
    let split = text.find(char::is_whitespace)?;
    let count_text = &text[..split];
    let unit = text[split..].trim_start_matches(char::is_whitespace);
    if unit.is_empty() || unit.contains(char::is_whitespace) {
        return None;
    }
    let count: f64 = count_text.parse().ok()?;
    if !count.is_finite() {
        return None;
    }
    let factor = match unit {
        "nano" | "nanos" => 1.0 / 1_000_000.0,
        "micro" | "micros" => 1.0 / 1_000.0,
        "milli" | "millis" => 1.0,
        "second" | "seconds" => 1_000.0,
        "minute" | "minutes" => 60_000.0,
        "hour" | "hours" => 3_600_000.0,
        "day" | "days" => 86_400_000.0,
        "week" | "weeks" => 604_800_000.0,
        _ => return None,
    };
    Some(count * factor)
}

fn parse_bytes(s: &str) -> Option<i64> {
    let s = s.trim();
    let mut split = s.len();
    for (i, b) in s.bytes().enumerate() {
        if !(b.is_ascii_digit() || b == b'.') {
            split = i;
            break;
        }
    }
    let (num, unit) = (&s[..split], s[split..].trim());
    if num.is_empty() {
        return None;
    }
    let factor: i64 = match unit {
        "" | "B" => 1,
        "kB" => 1_000,
        "MB" => 1_000_000,
        "GB" => 1_000_000_000,
        "KiB" => 1_024,
        "MiB" => 1_048_576,
        "GiB" => 1_073_741_824,
        _ => return None,
    };
    let value: f64 = num.parse().ok()?;
    let raw = value * factor as f64;
    if raw.fract() != 0.0 || raw < 0.0 {
        return None;
    }
    Some(raw as i64)
}

#[allow(unused_imports)]
pub use crate::log_level::LogLevel;
