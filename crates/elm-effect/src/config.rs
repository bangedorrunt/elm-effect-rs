//! `Config.ts` port: parsed-at-startup descriptors.
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
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
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
    /// `or_else` recovers it (`Config.test.ts:65`), and [`Config::load_effect`]
    /// reifies it as the effect's defect so `catch_defect` sees the provider's
    /// own value.
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
    /// how the pin's sentinels survive the value model (`Config.Duration`).
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
                    Err(ProviderError::Defect(defect)) => {
                        Err(vec![ConfigError::Defect { defect }])
                    }
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

    /// `Config.Duration`: the pin's own duration spellings, a count and a unit
    /// name (`"1000 millis"`, `"1 second"`, `"1.5 hours"`) plus the two
    /// infinite sentinels, decoded as milliseconds. The value is the pin's
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

    /// `Config.Array(valueSchema, path[, options])`: a list read from one
    /// separated string (`1,2`) when the node has text, or from its indexed
    /// children otherwise.
    ///
    /// The pin builds this as a union of the array shape and a separated
    /// string shape. The port tries the string form first when the node has
    /// text, which is what Config.test.ts:1892 asserts: `values=1,2` beside
    /// `values_0=3` reads `[1, 2]`, while `Config.schema(Schema.Array)` reads
    /// the indexed child `[3]`.
    #[cfg(feature = "schema")]
    pub fn array_of<T>(value: &elm_effect_schema::Schema<T>, name: Option<String>) -> Self
    where
        T: Clone + IntoConfigValue + Send + Sync + 'static,
    {
        Self::array_with(value, name, ",")
    }

    /// [`array_of`](Config::array_of) with the pin's `separator` option.
    #[cfg(feature = "schema")]
    pub fn array_with<T>(
        value: &elm_effect_schema::Schema<T>,
        name: Option<String>,
        separator: &str,
    ) -> Self
    where
        T: Clone + IntoConfigValue + Send + Sync + 'static,
    {
        let value = value.clone();
        let separator = separator.to_string();
        let name_owned = name.clone();
        Self {
            name,
            run: std::rc::Rc::new(move |provider, prefix| {
                let path = schema_path(prefix, name_owned.as_deref());
                let mut items = Vec::new();
                if let Some(text) = provider.get(&path) {
                    for entry in text.split(separator.as_str()) {
                        if entry.is_empty() {
                            continue;
                        }
                        let decoded = value
                            .decode(scalar_wire(&value.kind(), entry))
                            .map_err(|error| vec![schema_error(path.clone(), error)])?;
                        items.push(decoded.into_config_value());
                    }
                    return Ok(Value::List(items));
                }
                let mut index = 0usize;
                loop {
                    let mut child = path.clone();
                    child.push(index.to_string());
                    let Some(raw) = provider.get(&child) else {
                        break;
                    };
                    let decoded = value
                        .decode(scalar_wire(&value.kind(), &raw))
                        .map_err(|error| vec![schema_error(child, error)])?;
                    items.push(decoded.into_config_value());
                    index += 1;
                }
                if items.is_empty() {
                    Err(vec![ConfigError::Missing { path }])
                } else {
                    Ok(Value::List(items))
                }
            }),
        }
    }

    /// `Config.Record(keySchema, valueSchema, path[, options])`: a key/value
    /// map read from the provider's children, or from one separated string
    /// (`k=v,k=v`) when the node has no children.
    ///
    /// The pin builds this as a union of the record shape and a separated
    /// string shape; the port takes the two forms in the same order, so a
    /// provider holding children never falls back to the string form.
    #[cfg(feature = "schema")]
    pub fn record_of<T>(
        key: &elm_effect_schema::Schema<String>,
        value: &elm_effect_schema::Schema<T>,
        name: Option<String>,
    ) -> Self
    where
        T: Clone + IntoConfigValue + Send + Sync + 'static,
    {
        Self::record_with(key, value, name, ",", "=")
    }

    /// [`record_of`](Config::record_of) with the pin's `separator` and
    /// `keyValueSeparator` options.
    #[cfg(feature = "schema")]
    pub fn record_with<T>(
        key: &elm_effect_schema::Schema<String>,
        value: &elm_effect_schema::Schema<T>,
        name: Option<String>,
        separator: &str,
        key_value_separator: &str,
    ) -> Self
    where
        T: Clone + IntoConfigValue + Send + Sync + 'static,
    {
        let key = key.clone();
        let value = value.clone();
        let separator = separator.to_string();
        let key_value_separator = key_value_separator.to_string();
        let name_owned = name.clone();
        Self {
            name,
            run: std::rc::Rc::new(move |provider, prefix| {
                use elm_effect_schema::Unknown;
                let path = schema_path(prefix, name_owned.as_deref());
                let keys = provider.child_keys(&path);
                if !keys.is_empty() {
                    let mut out = Vec::new();
                    for key_text in keys {
                        let mut child = path.clone();
                        child.push(key_text.clone());
                        let Some(raw) = provider.get(&child) else {
                            continue;
                        };
                        let decoded_key = key
                            .decode(Unknown::Text(key_text))
                            .map_err(|error| vec![schema_error(child.clone(), error)])?;
                        let decoded = value
                            .decode(scalar_wire(&value.kind(), &raw))
                            .map_err(|error| vec![schema_error(child, error)])?;
                        out.push((decoded_key, decoded.into_config_value()));
                    }
                    return Ok(Value::Map(out));
                }
                let Some(text) = provider.get(&path) else {
                    return Err(vec![ConfigError::Missing { path }]);
                };
                // Splitting does not honour quoting or escapes: the pin's
                // `splitKeyValue` does. Keys and values holding the separator
                // need the object form until that lands.
                let mut out = Vec::new();
                for entry in text.split(separator.as_str()) {
                    if entry.is_empty() {
                        continue;
                    }
                    let Some((raw_key, raw_value)) = entry.split_once(key_value_separator.as_str())
                    else {
                        return Err(vec![ConfigError::Invalid {
                            path,
                            got: entry.to_string(),
                            reason: format!("expected key{key_value_separator}value"),
                        }]);
                    };
                    let decoded_key = key
                        .decode(Unknown::Text(raw_key.to_string()))
                        .map_err(|error| vec![schema_error(path.clone(), error)])?;
                    let decoded = value
                        .decode(scalar_wire(&value.kind(), raw_value))
                        .map_err(|error| vec![schema_error(path.clone(), error)])?;
                    out.push((decoded_key, decoded.into_config_value()));
                }
                Ok(Value::Map(out))
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
    /// The pin runs the transform in the fiber that parses the config. A load
    /// here is a bare `Result` with no fiber, so the description is driven on
    /// the calling thread and must complete without suspending: a transform
    /// that sleeps or forks has nowhere to park. A typed failure becomes the
    /// load's failure, which `or_else` recovers from exactly as the pin's
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
                // answering for the provider dying, which the pin forbids
                // (Config.test.ts:65).
                Err(errors) if errors.iter().any(is_defect) => Err(errors),
                Err(_) => (*run_other)(provider, prefix),
            }),
        }
    }

    /// Fall back with the failure in hand (`orElse` with a recovery
    /// function): the fallback sees every [`ConfigError`], but a provider
    /// defect is not offered to it at all — the pin's `orElse` never runs on a
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

/// The pin's `DurationFromString` readings, in milliseconds.
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

// ---------------------------------------------------------------------------
// `Config.schema`: the Schema bridge.
// ---------------------------------------------------------------------------

/// What a decoded schema value becomes on the config side.
///
/// `Config` is one monomorphic descriptor over [`Value`], while a schema
/// decodes to whatever type its codec names. This is the small seam between
/// them: plain data in, plain data out.
#[cfg(feature = "schema")]
pub trait IntoConfigValue {
    /// The config-side spelling of this decoded value.
    fn into_config_value(self) -> Value;
}

#[cfg(feature = "schema")]
impl IntoConfigValue for Value {
    fn into_config_value(self) -> Value {
        self
    }
}

#[cfg(feature = "schema")]
impl IntoConfigValue for String {
    fn into_config_value(self) -> Value {
        Value::Str(self)
    }
}

#[cfg(feature = "schema")]
impl IntoConfigValue for f64 {
    fn into_config_value(self) -> Value {
        Value::Num(self)
    }
}

#[cfg(feature = "schema")]
impl IntoConfigValue for i64 {
    fn into_config_value(self) -> Value {
        Value::Int(self)
    }
}

#[cfg(feature = "schema")]
impl IntoConfigValue for bool {
    fn into_config_value(self) -> Value {
        Value::Bool(self)
    }
}

#[cfg(feature = "schema")]
impl IntoConfigValue for elm_effect_schema::Unknown {
    fn into_config_value(self) -> Value {
        use elm_effect_schema::Unknown;
        match self {
            Unknown::Null => Value::Null,
            Unknown::Undefined => Value::Undefined,
            Unknown::Bool(flag) => Value::Bool(flag),
            Unknown::Number(number) => Value::Num(number),
            Unknown::Text(text) => Value::Str(text),
            Unknown::Array(items) => {
                Value::List(items.into_iter().map(IntoConfigValue::into_config_value).collect())
            }
            Unknown::Object(fields) => Value::Map(
                fields
                    .into_iter()
                    .map(|(name, item)| (name, item.into_config_value()))
                    .collect(),
            ),
        }
    }
}

#[cfg(feature = "schema")]
impl IntoConfigValue for elm_effect_schema::StructValue {
    fn into_config_value(self) -> Value {
        self.to_unknown().into_config_value()
    }
}

#[cfg(feature = "schema")]
impl<T: IntoConfigValue> IntoConfigValue for Vec<T> {
    fn into_config_value(self) -> Value {
        Value::List(self.into_iter().map(IntoConfigValue::into_config_value).collect())
    }
}

#[cfg(feature = "schema")]
impl<T: IntoConfigValue> IntoConfigValue for Option<T> {
    fn into_config_value(self) -> Value {
        match self {
            Some(value) => value.into_config_value(),
            // A schema that decoded `undefined` produced a VALUE, not an absent
            // option: `Config.option` is the wrapper that answers `Value::None`,
            // and keeping the two apart is what lets `UndefinedOr` survive it
            // (Config.test.ts:845).
            None => Value::Undefined,
        }
    }
}

#[cfg(feature = "schema")]
fn scalar_wire(
    kind: &elm_effect_schema::SchemaKind,
    text: &str,
) -> elm_effect_schema::Unknown {
    use elm_effect_schema::{SchemaKind, Unknown};
    match kind {
        SchemaKind::Number => number_wire(text),
        SchemaKind::Boolean => boolean_wire(text),
        SchemaKind::Check { base, .. } => scalar_wire(base, text),
        SchemaKind::Annotated { inner, .. } => scalar_wire(inner, text),
        SchemaKind::OptionalKey(inner) | SchemaKind::Optional(inner) => scalar_wire(inner, text),
        SchemaKind::Literal(expected) => literal_wire(expected, text),
        _ => Unknown::Text(text.to_string()),
    }
}

#[cfg(feature = "schema")]
fn schema_error(path: Vec<String>, error: elm_effect_schema::SchemaError) -> ConfigError {
    ConfigError::Invalid {
        path,
        got: "a schema value".to_string(),
        reason: error.to_string(),
    }
}

#[cfg(feature = "schema")]
fn schema_path(prefix: &[String], name: Option<&str>) -> Vec<String> {
    let mut path = prefix.to_vec();
    if let Some(name) = name.filter(|name| !name.is_empty()) {
        path.extend(name.split('.').map(str::to_string));
    }
    path
}

/// Text to a number wire. Unparseable text survives as a string so the schema
/// reports the mismatch at its own path instead of the bridge guessing.
#[cfg(feature = "schema")]
fn number_wire(text: &str) -> elm_effect_schema::Unknown {
    use elm_effect_schema::Unknown;
    match text {
        "NaN" => Unknown::Number(f64::NAN),
        "Infinity" | "+Infinity" => Unknown::Number(f64::INFINITY),
        "-Infinity" => Unknown::Number(f64::NEG_INFINITY),
        _ => match text.trim().parse::<f64>() {
            Ok(number) => Unknown::Number(number),
            Err(_) => Unknown::Text(text.to_string()),
        },
    }
}

#[cfg(feature = "schema")]
fn boolean_wire(text: &str) -> elm_effect_schema::Unknown {
    use elm_effect_schema::Unknown;
    match text {
        "true" => Unknown::Bool(true),
        "false" => Unknown::Bool(false),
        _ => Unknown::Text(text.to_string()),
    }
}

/// Text that is itself JSON (`null`, `[]`, `{}`, a bare number).
#[cfg(feature = "schema")]
fn json_wire(text: &str) -> Option<elm_effect_schema::Unknown> {
    elm_effect_schema::serde_json::from_str::<elm_effect_schema::serde_json::Value>(text)
        .ok()
        .map(elm_effect_schema::Unknown::from_json)
}

/// One literal's wire spelling, decided by the literal's own type.
#[cfg(feature = "schema")]
fn literal_wire(expected: &elm_effect_schema::Unknown, text: &str) -> elm_effect_schema::Unknown {
    use elm_effect_schema::Unknown;
    match expected {
        Unknown::Number(_) => number_wire(text),
        Unknown::Bool(_) => boolean_wire(text),
        Unknown::Null if text == "null" => Unknown::Null,
        _ => Unknown::Text(text.to_string()),
    }
}

/// A failure while reading a schema's wire shape.
///
/// Two kinds stay apart because Config treats them differently: an opaque kind
/// is invalid input the schema cannot materialize, while a source failure is the
/// provider's own error, which no default or option may recover.
#[cfg(feature = "schema")]
#[derive(Debug)]
enum WireError {
    /// The kind names a constructor this bridge cannot walk.
    Opaque(&'static str),
    /// The provider failed at this path (`ConfigProvider.SourceError`).
    Source { path: Vec<String>, message: String },
    /// The provider died at this path: not a config error any wrapper recovers.
    Defect(Defect),
}

/// Read one path through the source's failure channel.
#[cfg(feature = "schema")]
fn read(provider: &dyn Lookup, path: &[String]) -> Result<Option<String>, WireError> {
    provider.try_get(path).map_err(|error| match error {
        ProviderError::Source(message) => WireError::Source {
            path: path.to_vec(),
            message,
        },
        ProviderError::Defect(defect) => WireError::Defect(defect),
    })
}

/// Build the wire value a schema expects from the provider's text.
///
/// The empty vector is "this path holds nothing", which `Config::schema`
/// reports as absence. More than one wire means the schema offers several
/// shapes at this path (a union); they come back in the schema's own order, so
/// the first one that decodes is the schema's first match.
///
/// A declared field list is walked as fields, which is how an optional key
/// stays absent rather than becoming null. Shapes with no field list (a leaf,
/// an array, a union) are walked by kind.
#[cfg(feature = "schema")]
fn schema_wires(
    schema: &elm_effect_schema::Schema<elm_effect_schema::Unknown>,
    provider: &dyn Lookup,
    path: &[String],
) -> Result<Vec<elm_effect_schema::Unknown>, WireError> {
    if let Some(fields) = schema.fields() {
        return field_wires(fields, provider, path);
    }
    // A struct built from a runtime field list keeps its fields as named
    // children and has no `Field` list, so the child schemas are what carries
    // the shape (and lets a suspended field resolve).
    if matches!(schema.wire_kind(), elm_effect_schema::SchemaKind::Struct(_))
        && let Some(children) = schema.child_schemas()
    {
        return child_wires(children, provider, path);
    }
    if let Some(elements) = schema.elements() {
        return element_wires(elements, provider, path);
    }
    // A suspension resolves to the schema it names. Its kind cannot carry the
    // shape, because a recursive schema does not exist yet when the node is
    // built (Config.test.ts:2735).
    if let Some(resolved) = schema.suspended_schema() {
        return schema_wires(resolved, provider, path);
    }
    // `Schema.Redacted`: the pin's StringTree encoding is the wrapped schema's
    // own encoding, while this port's redacted codec takes the box as its wire.
    // So the path is read as the inner wire and boxed here, which is what lets
    // a redacted Int report its own validation error (Config.test.ts:2444,
    // :2753).
    if matches!(
        schema.wire_kind(),
        elm_effect_schema::SchemaKind::Opaque("redacted")
    ) && let Some(inner) = elm_effect_schema::redacted_value(schema)
    {
        let wires = schema_wires(&inner, provider, path)?;
        return Ok(wires
            .into_iter()
            .map(|wire| {
                let mut boxed = std::collections::BTreeMap::new();
                boxed.insert("value".to_string(), wire);
                elm_effect_schema::Unknown::Object(boxed)
            })
            .collect());
    }
    // An array is walked through its element schema for the same reason: the
    // element may be a suspension, which only the schema can resolve.
    if let Some(element) = schema.array_element_schema() {
        return sequence_wires(
            |child| schema_wires(element, provider, child),
            provider,
            path,
        );
    }
    kind_wires(&schema.wire_kind(), provider, path)
}

#[cfg(feature = "schema")]
fn field_wires(
    fields: &[elm_effect_schema::Field],
    provider: &dyn Lookup,
    path: &[String],
) -> Result<Vec<elm_effect_schema::Unknown>, WireError> {
    use elm_effect_schema::SchemaKind;
    // Nothing at the path means the object is absent, so its fields are never
    // read. Without this a recursive schema descends forever: every level looks
    // exactly like the one above it.
    if nothing_at(provider, path)? {
        return Ok(Vec::new());
    }
    let mut object = std::collections::BTreeMap::new();
    for field in fields {
        let mut child = path.to_vec();
        child.push(field.name().to_string());
        let wires = match field.inner().wire_kind() {
            // An absent optional field is omitted, not null
            // (Config.test.ts:2253, :2261).
            SchemaKind::Optional(inner) | SchemaKind::OptionalKey(inner) => {
                kind_wires(inner.as_ref(), provider, &child)?
            }
            _ => schema_wires(field.inner(), provider, &child)?,
        };
        if let Some(wire) = wires.into_iter().next() {
            object.insert(field.name().to_string(), wire);
        }
    }
    object_or_scalar(object, provider, path)
}

#[cfg(feature = "schema")]
fn child_wires(
    children: &[(String, std::sync::Arc<elm_effect_schema::Schema<elm_effect_schema::Unknown>>)],
    provider: &dyn Lookup,
    path: &[String],
) -> Result<Vec<elm_effect_schema::Unknown>, WireError> {
    if nothing_at(provider, path)? {
        return Ok(Vec::new());
    }
    let mut object = std::collections::BTreeMap::new();
    for (name, child) in children {
        let mut child_path = path.to_vec();
        child_path.push(name.clone());
        if let Some(wire) = schema_wires(child, provider, &child_path)?.into_iter().next() {
            object.insert(name.clone(), wire);
        }
    }
    object_or_scalar(object, provider, path)
}

#[cfg(feature = "schema")]
fn kind_wires(
    kind: &elm_effect_schema::SchemaKind,
    provider: &dyn Lookup,
    path: &[String],
) -> Result<Vec<elm_effect_schema::Unknown>, WireError> {
    use elm_effect_schema::{SchemaKind, Unknown};
    let own = read(provider, path)?;
    Ok(match kind {
        SchemaKind::String => own.clone().map(|text| vec![Unknown::Text(text)]).unwrap_or_default(),
        SchemaKind::Number => own.clone().map(|text| vec![number_wire(&text)]).unwrap_or_default(),
        SchemaKind::Boolean => own.clone()
            .map(|text| vec![boolean_wire(&text)])
            .unwrap_or_default(),
        // `Schema.null()` reports an opaque kind rather than a null one, so the
        // null leaf is recognised by name.
        // `Schema.Undefined` accepts only absence. A present container is a
        // shape it refuses, and a scalar is read so its decode can say so.
        SchemaKind::Opaque("undefined") => {
            if provider.has_children(path) || provider.node(path).is_some() {
                vec![Unknown::Null]
            } else {
                match own.clone() {
                    Some(text) => vec![json_wire(&text).unwrap_or(Unknown::Text(text))],
                    None => vec![Unknown::Undefined],
                }
            }
        }
        SchemaKind::Opaque(name) if *name == "null" => own.clone()
            .map(|text| {
                vec![if text == "null" {
                    Unknown::Null
                } else {
                    Unknown::Text(text)
                }]
            })
            .unwrap_or_default(),
        SchemaKind::Object | SchemaKind::Json => own.clone()
            .and_then(|text| json_wire(&text))
            .map(|value| vec![value])
            .unwrap_or_default(),
        SchemaKind::Literal(expected) => own.clone()
            .map(|text| vec![literal_wire(expected, &text)])
            .unwrap_or_default(),
        SchemaKind::Literals(expected) => own.clone()
            .map(|text| {
                expected
                    .iter()
                    .map(|value| literal_wire(value, &text))
                    .collect()
            })
            .unwrap_or_default(),
        // A key that may be absent only constrains its enclosing struct, so the
        // inner shape decides the wire.
        SchemaKind::OptionalKey(inner) => kind_wires(inner, provider, path)?,
        // A standalone optional reads absence as null, which its schema turns
        // into `None`.
        SchemaKind::Optional(inner) => {
            let wires = kind_wires(inner, provider, path)?;
            if wires.is_empty() {
                vec![Unknown::Null]
            } else {
                wires
            }
        }
        SchemaKind::Check { base, .. } => kind_wires(base, provider, path)?,
        SchemaKind::Annotated { inner, .. } => kind_wires(inner, provider, path)?,
        SchemaKind::Struct(fields) => {
            let mut object = std::collections::BTreeMap::new();
            for (name, field_kind) in fields {
                let mut child = path.to_vec();
                child.push(name.clone());
                let wires = match field_kind {
                    SchemaKind::Optional(inner) | SchemaKind::OptionalKey(inner) => {
                        kind_wires(inner, provider, &child)?
                    }
                    _ => kind_wires(field_kind, provider, &child)?,
                };
                if let Some(wire) = wires.into_iter().next() {
                    object.insert(name.clone(), wire);
                }
            }
            object_or_scalar(object, provider, path)?
        }
        SchemaKind::Array(inner) => sequence_wires(|child| kind_wires(inner, provider, child), provider, path)?,
        SchemaKind::Tuple(kinds) => {
            let mut items = Vec::new();
            for (index, field_kind) in kinds.iter().enumerate() {
                let mut child = path.to_vec();
                child.push(index.to_string());
                match kind_wires(field_kind, provider, &child)?.into_iter().next() {
                    Some(wire) => items.push(wire),
                    None => break,
                }
            }
            positional_or_scalar(items, provider, path)?
        }
        SchemaKind::Union { members, .. } => {
            let mut wires = Vec::new();
            for member in members {
                wires.extend(kind_wires(member, provider, path)?);
            }
            // Object members read the same node: a provider holding both
            // members' keys is ONE object, which is what lets `oneOf` notice
            // both matched (Config.test.ts:2411) and `anyOf` take the first
            // (Config.test.ts:2399). Merging keeps the earlier member's value
            // when two members claim the same key.
            if wires.len() > 1 && wires.iter().all(|wire| matches!(wire, Unknown::Object(_))) {
                let mut merged = std::collections::BTreeMap::new();
                for wire in wires {
                    if let Unknown::Object(fields) = wire {
                        for (name, item) in fields {
                            merged.entry(name).or_insert(item);
                        }
                    }
                }
                vec![Unknown::Object(merged)]
            } else {
                // A numeric-looking value is read as the number before the
                // text, whatever order the union declares: Config.test.ts:2435
                // expects `"1"` to decode as `1` even with String listed
                // first. Only numbers move; the rest keeps declared order, so a
                // scalar member declared first still wins over a struct member
                // materialized from the same node (Config.test.ts:2001).
                wires.sort_by_key(|wire| !matches!(wire, Unknown::Number(_)));
                wires
            }
        }
        SchemaKind::Rest(inner) => kind_wires(inner, provider, path)?,
        // A record reads whatever keys the provider has under the path, and
        // each value through the record's value kind. A key its finite key
        // schema refuses is never read, so a source failing under an unrelated
        // advertised key cannot surface (Config.test.ts:1838).
        SchemaKind::Record(key_kind, value_kind) => {
            let mut object = std::collections::BTreeMap::new();
            for key in provider.child_keys(path) {
                if !key_is_covered(key_kind, &key) {
                    continue;
                }
                let mut child = path.to_vec();
                child.push(key.clone());
                if let Some(wire) = kind_wires(value_kind, provider, &child)?.into_iter().next() {
                    object.insert(key, wire);
                }
            }
            // An explicit object node with no keys is a present empty record
            // (Config.test.ts:729); an array node supplies no keys.
            let present = !object.is_empty()
                || provider.has_children(path)
                || matches!(provider.node(path), Some(super::config_provider::Node::Object));
            if present {
                vec![Unknown::Object(object)]
            } else {
                Vec::new()
            }
        }
        // `Schema.fromJsonString`: the wire is the JSON text itself, so the
        // path is read as a scalar and the codec does the parsing
        // (Config.test.ts:1973).
        SchemaKind::Opaque("fromJsonString") => own
            .map(|text| vec![Unknown::Text(text)])
            .unwrap_or_default(),
        // A declaration or an opaque kind has no wire shape to walk; the config
        // reports the gap rather than pretending the path is absent.
        _ => return Err(WireError::Opaque("the schema's wire shape is not introspectable")),
    })
}

/// An object built from the fields that were found, or the node's own scalar.
///
/// Declared children, or any children at all, make this node an object. A node
/// with unrelated children is the present empty struct of Config.test.ts:2253,
/// not an absent one.
///
/// The probe reads the path itself, through the source's failure channel: an
/// object group asks its own root whether it holds anything, so a source that
/// fails there is a failed read rather than an absent group
/// (Config.test.ts:1245).
#[cfg(feature = "schema")]
fn nothing_at(provider: &dyn Lookup, path: &[String]) -> Result<bool, WireError> {
    Ok(read(provider, path)?.is_none()
        && !provider.has_children(path)
        && provider.node(path).is_none())
}

#[cfg(feature = "schema")]
fn object_or_scalar(
    object: std::collections::BTreeMap<String, elm_effect_schema::Unknown>,
    provider: &dyn Lookup,
    path: &[String],
) -> Result<Vec<elm_effect_schema::Unknown>, WireError> {
    use super::config_provider::Node;
    use elm_effect_schema::Unknown;
    if !object.is_empty() || provider.has_children(path) {
        return Ok(vec![Unknown::Object(object)]);
    }
    match provider.node(path) {
        // An explicit container at the path. An empty object is a present
        // struct the schema validates; an array is not one (Config.test.ts:729).
        Some(Node::Object) => return Ok(vec![Unknown::Object(object)]),
        Some(Node::Array(_)) => return Ok(Vec::new()),
        None => {}
    }
    Ok(match read(provider, path)? {
        Some(text) => match json_wire(&text) {
            // An array node where an object belongs reads as absent
            // (Config.test.ts:1286).
            Some(Unknown::Array(_)) => Vec::new(),
            Some(value) => vec![value],
            None => vec![Unknown::Text(text)],
        },
        None => Vec::new(),
    })
}

/// Indexed children, in order, stopping at the first gap.
#[cfg(feature = "schema")]
fn sequence_wires(
    mut item: impl FnMut(&[String]) -> Result<Vec<elm_effect_schema::Unknown>, WireError>,
    provider: &dyn Lookup,
    path: &[String],
) -> Result<Vec<elm_effect_schema::Unknown>, WireError> {
    use super::config_provider::Node;
    use elm_effect_schema::Unknown;
    // A node that states its length keeps every position up to it: a missing
    // position is `undefined`, not the end of the array (Config.test.ts:1873).
    let declared = match provider.node(path) {
        Some(Node::Array(length)) => Some(length),
        _ => None,
    };
    let mut items = Vec::new();
    let mut index = 0usize;
    loop {
        // A declared length bounds the array: positions past it do not exist,
        // however eager an element schema is to materialize an absence (an
        // `UndefinedOr` element answers every path, holes included).
        if declared.is_some_and(|length| index >= length) {
            break;
        }
        let mut child = path.to_vec();
        child.push(index.to_string());
        match item(&child)?.into_iter().next() {
            Some(wire) => items.push(wire),
            // Inside the declared length, so the position exists and holds
            // `undefined` (Config.test.ts:1873).
            None if declared.is_some() => items.push(Unknown::Undefined),
            None => break,
        }
        index += 1;
    }
    // The node and co-located-scalar rules are the same for an array and an
    // empty tuple, so both read through this one tail.
    positional_or_scalar(items, provider, path)
}

#[cfg(feature = "schema")]
fn positional_or_scalar(
    items: Vec<elm_effect_schema::Unknown>,
    provider: &dyn Lookup,
    path: &[String],
) -> Result<Vec<elm_effect_schema::Unknown>, WireError> {
    use elm_effect_schema::Unknown;
    use super::config_provider::Node;
    if !items.is_empty() {
        return Ok(vec![Unknown::Array(items)]);
    }
    match provider.node(path) {
        // An empty tuple is an explicit empty array node, not an absent one;
        // an object node supplies no positions (Config.test.ts:729).
        Some(Node::Array(_)) => return Ok(vec![Unknown::Array(Vec::new())]),
        Some(Node::Object) => return Ok(Vec::new()),
        None => {}
    }
    Ok(match read(provider, path)? {
        Some(text) => match json_wire(&text) {
            Some(Unknown::Object(_)) => Vec::new(),
            Some(value) => vec![value],
            None => vec![Unknown::Text(text)],
        },
        None => Vec::new(),
    })
}

/// Indexed children through declared element schemas.
#[cfg(feature = "schema")]
fn element_wires(
    elements: &[elm_effect_schema::Schema<elm_effect_schema::Unknown>],
    provider: &dyn Lookup,
    path: &[String],
) -> Result<Vec<elm_effect_schema::Unknown>, WireError> {
    let mut items = Vec::new();
    for (index, element) in elements.iter().enumerate() {
        let mut child = path.to_vec();
        child.push(index.to_string());
        match schema_wires(element, provider, &child)?.into_iter().next() {
            Some(wire) => items.push(wire),
            None => break,
        }
    }
    positional_or_scalar(items, provider, path)
}

/// Whether a record's key schema can name `key` at all.
///
/// Only a finite key schema (`Literal`/`Literals`) can answer no before the
/// read. A checked or patterned key is tested against the key's own text, so
/// the bridge reads it and lets the codec drop it: reading is the only way to
/// know. Skipping a refused key is what keeps a source that fails under an
/// unrelated advertised key from surfacing (Config.test.ts:1838).
#[cfg(feature = "schema")]
fn key_is_covered(kind: &elm_effect_schema::SchemaKind, key: &str) -> bool {
    use elm_effect_schema::{SchemaKind, Unknown};
    match kind {
        SchemaKind::Literal(Unknown::Text(expected)) => expected == key,
        SchemaKind::Literals(values) => values
            .iter()
            .any(|value| matches!(value, Unknown::Text(expected) if expected == key)),
        SchemaKind::Union { members, .. } => {
            members.iter().any(|member| key_is_covered(member, key))
        }
        SchemaKind::Optional(inner) | SchemaKind::OptionalKey(inner) | SchemaKind::Rest(inner) => {
            key_is_covered(inner, key)
        }
        SchemaKind::Check { base, .. } => key_is_covered(base, key),
        SchemaKind::Annotated { inner, .. } => key_is_covered(inner, key),
        // An index signature, or a reading this bridge cannot decide: the key
        // is read and the codec drops it when it does not match.
        _ => true,
    }
}

/// Whether the provider at `path` can supply the representation the schema
/// asks for (`hasProviderInput` in Config.ts): an object for an object shape,
/// an array for an array shape, a scalar for a leaf.
///
/// A decode failure with input present is a validation failure; without it the
/// config is absent, so `withDefault`/`option` recover it. That is the whole
/// difference between an explicit empty container (validated, so its missing
/// fields fail) and an absent path (Config.test.ts:729, :877).
#[cfg(feature = "schema")]
fn has_schema_input(
    schema: &elm_effect_schema::Schema<elm_effect_schema::Unknown>,
    provider: &dyn Lookup,
    path: &[String],
) -> Result<bool, WireError> {
    if schema.fields().is_some() {
        return Ok(object_present(provider, path));
    }
    if schema.elements().is_some() {
        return Ok(array_present(provider, path));
    }
    kind_has_input(&schema.wire_kind(), provider, path)
}

#[cfg(feature = "schema")]
fn kind_has_input(
    kind: &elm_effect_schema::SchemaKind,
    provider: &dyn Lookup,
    path: &[String],
) -> Result<bool, WireError> {
    use elm_effect_schema::SchemaKind;
    Ok(match kind {
        SchemaKind::Struct(_) | SchemaKind::StructWithRest(..) | SchemaKind::Record(..) => {
            object_present(provider, path)
        }
        SchemaKind::Array(_) | SchemaKind::Tuple(_) | SchemaKind::TupleWithRest(..) => {
            array_present(provider, path)
        }
        SchemaKind::Union { members, .. } => {
            for member in members {
                if kind_has_input(member, provider, path)? {
                    return Ok(true);
                }
            }
            false
        }
        SchemaKind::Optional(inner)
        | SchemaKind::OptionalKey(inner)
        | SchemaKind::Rest(inner)
        | SchemaKind::Check { base: inner, .. }
        | SchemaKind::Annotated { inner, .. } => kind_has_input(inner, provider, path)?,
        // Any representation at all is input to an `undefined` schema, which
        // refuses every one of them.
        SchemaKind::Opaque("undefined") => {
            provider.has_children(path)
                || provider.node(path).is_some()
                || read(provider, path)?.is_some()
        }
        // Everything else reads a co-located scalar, `Object`/`Json` included
        // (their text is parsed as JSON), so the scalar is the input.
        _ => read(provider, path)?.is_some(),
    })
}

#[cfg(feature = "schema")]
fn object_present(provider: &dyn Lookup, path: &[String]) -> bool {
    use super::config_provider::Node;
    match provider.node(path) {
        Some(Node::Object) => true,
        Some(Node::Array(_)) => false,
        None => provider.has_children(path),
    }
}

#[cfg(feature = "schema")]
fn array_present(provider: &dyn Lookup, path: &[String]) -> bool {
    use super::config_provider::Node;
    match provider.node(path) {
        Some(Node::Array(_)) => true,
        Some(Node::Object) => false,
        None => provider.has_children(path),
    }
}

/// The pin's refusal for a schema whose StringTree encoding is opaque
/// (`Config.ts`, matched by every opaque row in `Config.test.ts`).
#[cfg(feature = "schema")]
const OPAQUE_ENCODING: &str = "Config.schema does not support opaque StringTree encodings";

/// The opaque encoding a schema carries, by the pin's own name for it.
///
/// A StringTree wire is a string, an array of strings, or an object of strings,
/// so a schema that decodes arbitrary data has no wire to read a config path
/// with. The pin refuses `Any`, `Unknown`, `ObjectKeyword`, `Json`, and
/// `MutableJson` where the config descriptor is built rather than where a
/// provider is read, which is why a nested one is refused too
/// (`Config.test.ts:1936`, `:1944`, `:1951`, `:1958`). `MutableJson` has no
/// constructor in this port.
#[cfg(feature = "schema")]
fn opaque_encoding(
    schema: &elm_effect_schema::Schema<elm_effect_schema::Unknown>,
) -> Option<&'static str> {
    opaque_walk(schema, &mut Vec::new())
}

/// The walk itself. `visiting` holds the resolved schemas already on the path,
/// because a recursive schema below a suspension resolves to the same schema
/// again: without that bound the walk never reaches a leaf.
#[cfg(feature = "schema")]
fn opaque_walk(
    schema: &elm_effect_schema::Schema<elm_effect_schema::Unknown>,
    visiting: &mut Vec<*const elm_effect_schema::Schema<elm_effect_schema::Unknown>>,
) -> Option<&'static str> {
    use elm_effect_schema::SchemaKind;
    // A suspension resolves to the schema it names: its kind is `suspend`, which
    // says nothing about what the thunk will produce.
    if let Some(resolved) = schema.suspended_schema() {
        let key = resolved as *const elm_effect_schema::Schema<elm_effect_schema::Unknown>;
        if visiting.contains(&key) {
            return None;
        }
        visiting.push(key);
        let found = opaque_walk(resolved, visiting);
        visiting.pop();
        return found;
    }
    // `Schema.fromJsonString` carries its inner schema as a codec, not as a
    // nested wire: the wire is the JSON text itself, which is exactly how
    // arbitrary JSON is carried (`Config.test.ts:1965`), so a `Json` inside it
    // is supported rather than opaque.
    if matches!(schema.wire_kind(), SchemaKind::Opaque("fromJsonString")) {
        return None;
    }
    // `Schema.Redacted`'s wire is the wrapped schema's own.
    if let Some(inner) = elm_effect_schema::redacted_value(schema) {
        return opaque_walk(&inner, visiting);
    }
    if let Some(fields) = schema.fields() {
        for field in fields {
            if let Some(opaque) = opaque_walk(field.inner(), visiting) {
                return Some(opaque);
            }
        }
    }
    if let Some(children) = schema.child_schemas() {
        for (_, child) in children {
            if let Some(opaque) = opaque_walk(child, visiting) {
                return Some(opaque);
            }
        }
    }
    if let Some(elements) = schema.elements() {
        for element in elements {
            if let Some(opaque) = opaque_walk(element, visiting) {
                return Some(opaque);
            }
        }
    }
    if let Some(element) = schema.array_element_schema() {
        return opaque_walk(element, visiting);
    }
    opaque_kind(&schema.wire_kind())
}

/// The opaque kind inside a [`SchemaKind`](elm_effect_schema::SchemaKind),
/// walking the positions a wire actually reaches.
#[cfg(feature = "schema")]
fn opaque_kind(kind: &elm_effect_schema::SchemaKind) -> Option<&'static str> {
    use elm_effect_schema::SchemaKind;
    match kind {
        SchemaKind::Opaque("any") => Some("Any"),
        SchemaKind::Opaque("unknown") => Some("Unknown"),
        SchemaKind::Opaque("objectKeyword") => Some("ObjectKeyword"),
        SchemaKind::Json => Some("Json"),
        SchemaKind::Array(inner)
        | SchemaKind::Optional(inner)
        | SchemaKind::OptionalKey(inner)
        | SchemaKind::Rest(inner) => opaque_kind(inner),
        SchemaKind::Check { base, .. } => opaque_kind(base),
        SchemaKind::Annotated { inner, .. } => opaque_kind(inner),
        SchemaKind::Struct(fields) => fields.iter().find_map(|(_, field)| opaque_kind(field)),
        SchemaKind::StructWithRest(fields, rest) => fields
            .iter()
            .find_map(|(_, field)| opaque_kind(field))
            .or_else(|| opaque_kind(rest)),
        SchemaKind::Record(key, value) => opaque_kind(key).or_else(|| opaque_kind(value)),
        SchemaKind::Tuple(items) => items.iter().find_map(opaque_kind),
        SchemaKind::TupleWithRest(items, rest) => items
            .iter()
            .find_map(opaque_kind)
            .or_else(|| rest.iter().find_map(opaque_kind)),
        SchemaKind::Union { members, .. } => members.iter().find_map(opaque_kind),
        SchemaKind::TemplateLiteral(parts) => parts.iter().find_map(opaque_kind),
        // A declaration's encoding lives behind a reference this walk cannot
        // resolve, and every other kind names a wire a StringTree can carry.
        _ => None,
    }
}

#[cfg(feature = "schema")]
impl Config {
    /// `Config.schema(codec, path)`: a config whose value is the provider's
    /// data decoded through `codec`.
    ///
    /// The provider is read through the schema's wire shape (what `kind()`
    /// reports, so a transformation reads its encoded side), then decoded.
    /// Nothing under the path is [`ConfigError::Missing`], which [`option`]
    /// and [`with_default`] recover; data the schema refuses is
    /// [`ConfigError::Invalid`], which they do not. Config.test.ts:696 is that
    /// distinction.
    ///
    /// [`option`]: Config::option
    /// [`with_default`]: Config::with_default
    pub fn schema<T>(codec: &elm_effect_schema::Schema<T>, name: Option<String>) -> Self
    where
        T: Clone + IntoConfigValue + Send + Sync + 'static,
    {
        let walker = elm_effect_schema::Schema::as_unknown(codec.clone());
        // The pin builds the descriptor eagerly and throws on an opaque
        // encoding, so the refusal is the construction's and never a load's.
        // A Rust constructor for a static schema argument has no other way to
        // refuse, so this is the pin's synchronous throw.
        if let Some(opaque) = opaque_encoding(&walker) {
            panic!("{OPAQUE_ENCODING}: Schema.{opaque}");
        }
        let codec = codec.clone();
        let name_owned = name.clone();
        Self {
            name,
            run: std::rc::Rc::new(move |provider, prefix| {
                let path = schema_path(prefix, name_owned.as_deref());
                let wires = match schema_wires(&walker, provider, &path) {
                    Ok(wires) => wires,
                    Err(WireError::Opaque(reason)) => {
                        return Err(vec![ConfigError::Invalid {
                            path: path.clone(),
                            got: "a schema shape".to_string(),
                            reason: reason.to_string(),
                        }]);
                    }
                    // The provider failed: report the read failure itself, not
                    // a shape the bridge could not build from it.
                    Err(WireError::Source { path, message }) => {
                        return Err(vec![ConfigError::Source { path, message }]);
                    }
                    // The provider died: a defect, not a config error the
                    // caller may recover (Config.test.ts:65).
                    Err(WireError::Defect(defect)) => {
                        return Err(vec![ConfigError::Defect { defect }]);
                    }
                };
                if wires.is_empty() {
                    return Err(vec![ConfigError::Missing { path }]);
                }
                // `oneOf` demands exactly one member. The walk materializes a
                // wire per member, so a node holding both a scalar and its
                // object child makes two members match and is ambiguous —
                // which one wire alone cannot show (Config.test.ts:2019).
                if matches!(
                    walker.wire_kind(),
                    elm_effect_schema::SchemaKind::Union {
                        mode: "oneOf",
                        ..
                    }
                ) {
                    let mut matched = Vec::new();
                    for wire in &wires {
                        if let Ok(value) = codec.decode(wire.clone()) {
                            matched.push(value);
                        }
                    }
                    if matched.len() == 1 {
                        let value = matched.pop().expect("one match");
                        return Ok(value.into_config_value());
                    }
                    if matched.len() > 1 {
                        return Err(vec![ConfigError::Invalid {
                            path,
                            got: format!("{} matching members", matched.len()),
                            reason: "the value matches more than one member".to_string(),
                        }]);
                    }
                }
                let relevant = match has_schema_input(&walker, provider, &path) {
                    Ok(relevant) => relevant,
                    Err(WireError::Source { path, message }) => {
                        return Err(vec![ConfigError::Source { path, message }]);
                    }
                    Err(WireError::Defect(defect)) => {
                        return Err(vec![ConfigError::Defect { defect }]);
                    }
                    // `has_schema_input` never refuses a shape, only reads.
                    Err(WireError::Opaque(_)) => false,
                };
                let mut last = String::new();
                let mut last_error = None;
                for wire in wires {
                    last = format!("{wire:?}");
                    match codec.decode(wire) {
                        Ok(value) => return Ok(value.into_config_value()),
                        // The schema's own refusal is the reason the caller
                        // sees: a named check says which check failed
                        // (Config.test.ts:2071), and an inner codec says what
                        // it expected.
                        Err(error) => last_error = Some(error),
                    }
                }
                // The schema refused the input. With a relevant representation
                // present that is a validation failure; without one the config
                // is absent, so a default or an option recovers it. The pin
                // still reports the schema's own message for the absent case;
                // the port reports `Missing`, which carries the same
                // recoverability (Config.test.ts:715, :2499).
                if !relevant {
                    return Err(vec![ConfigError::Missing { path }]);
                }
                Err(vec![ConfigError::Invalid {
                    path,
                    got: last,
                    reason: last_error.map_or_else(
                        || "the value does not match the schema".to_string(),
                        |error| error.to_string(),
                    ),
                }])
            }),
        }
    }
}

/// Log verbosity with `All`/`None` sentinels.
///
/// Ports `packages/effect/src/LogLevel.ts`: levels order
/// `All < Trace < Debug < Info < Warning < Error < Fatal < None`, and the
/// default minimum is `Info`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    /// Everything passes.
    All,
    /// Finest detail.
    Trace,
    /// Debugging detail.
    Debug,
    /// Default minimum.
    #[default]
    Info,
    /// Potential problems.
    Warning,
    /// Errors.
    Error,
    /// Unrecoverable failures.
    Fatal,
    /// Nothing passes.
    None,
}

impl LogLevel {
    /// Parse a level name; `None` when unrecognized.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "All" => Some(Self::All),
            "Trace" => Some(Self::Trace),
            "Debug" => Some(Self::Debug),
            "Info" => Some(Self::Info),
            "Warning" => Some(Self::Warning),
            "Error" => Some(Self::Error),
            "Fatal" => Some(Self::Fatal),
            "None" => Some(Self::None),
            _ => None,
        }
    }

    /// True when a message at `message` passes a `minimum` threshold.
    pub fn passes(self, minimum: Self) -> bool {
        self >= minimum
    }
}
