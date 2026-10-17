//! `Schema.toDifferJsonPatch` port: diff two encoded wire values into a JSON
//! Patch, apply a patch back, decode the result.
//!
//! A [`Differ`] wraps a [`Schema<T>`]: `diff` encodes both sides and diffs the
//! wire forms; `patch` encodes the old value, applies the ops, and decodes the
//! result. Numbers compare with JavaScript `Object.is` semantics (0 and -0
//! differ, NaN equals NaN), which is the one place this is not plain `==`.
//! Everything takes `&Unknown` and builds anew, so inputs are never mutated.

use super::{Schema, SchemaError, Unknown};

/// One patch operation: `add`, `remove`, or `replace` at a pointer path.
#[derive(Clone, Debug, PartialEq)]
pub enum PatchOp {
    /// Insert `value` at `path` (`-` appends to arrays).
    Add {
        /// Pointer string the operation targets.
        path: String,
        /// Value to insert.
        value: Unknown,
    },
    /// Delete the value at `path` (never the root).
    Remove {
        /// Pointer string the operation targets.
        path: String,
    },
    /// Overwrite the value at `path` (`""` replaces the root).
    Replace {
        /// Pointer string the operation targets.
        path: String,
        /// Replacement value.
        value: Unknown,
    },
}

/// A patch: ops applied in order, later ones seeing earlier results.
pub type JsonPatch = Vec<PatchOp>;

/// Why [`apply`] failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchError {
    /// The pointer string does not parse.
    BadPath(String),
    /// A step on the way down is missing or not a container.
    Unresolvable(String),
    /// The target itself is missing or illegal (remove at the root, `-` for
    /// replace/remove, replace of an absent key).
    BadTarget(String),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadPath(p) => write!(f, "bad pointer: {p}"),
            Self::Unresolvable(p) => write!(f, "cannot resolve: {p}"),
            Self::BadTarget(p) => write!(f, "bad target: {p}"),
        }
    }
}

impl std::error::Error for PatchError {}

/// Why [`Differ::patch`] failed: the ops did not apply, or the patched wire
/// value did not decode.
#[derive(Clone, Debug, PartialEq)]
pub enum DifferError {
    /// The patch did not apply to the encoded old value.
    Patch(PatchError),
    /// The patched wire value did not decode.
    Schema(SchemaError),
}

impl std::fmt::Display for DifferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Patch(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DifferError {}

/// Wire equality with JavaScript `Object.is` semantics: numbers compare by
/// bits, so 0 and -0 differ and NaN equals NaN; containers compare deeply.
fn wire_equal(first: &Unknown, second: &Unknown) -> bool {
    match (first, second) {
        (Unknown::Number(a), Unknown::Number(b)) => a.to_bits() == b.to_bits(),
        (Unknown::Array(a), Unknown::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| wire_equal(x, y))
        }
        (Unknown::Object(a), Unknown::Object(b)) => {
            a.len() == b.len()
                && a.keys().eq(b.keys())
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|w| wire_equal(v, w)))
        }
        _ => first == second,
    }
}

fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn parse(path: &str) -> Option<Vec<String>> {
    if path.is_empty() {
        return Some(vec![]);
    }
    if !path.starts_with('/') {
        return None;
    }
    path[1..]
        .split('/')
        .map(|token| {
            if token.contains('~') {
                let mut out = String::with_capacity(token.len());
                let mut chars = token.chars();
                while let Some(c) = chars.next() {
                    if c == '~' {
                        match chars.next() {
                            Some('0') => out.push('~'),
                            Some('1') => out.push('/'),
                            _ => return None,
                        }
                    } else {
                        out.push(c);
                    }
                }
                Some(out)
            } else {
                Some(token.to_string())
            }
        })
        .collect()
}

/// The patch turning `old` into `new`: empty when wire-equal, per-key in
/// sorted order for objects, per-index (removals highest first) for arrays,
/// one root-or-branch replace for anything else.
pub fn diff(old: &Unknown, new: &Unknown) -> JsonPatch {
    let mut ops = vec![];
    diff_at(old, new, String::new(), &mut ops);
    ops
}

fn diff_at(old: &Unknown, new: &Unknown, path: String, ops: &mut JsonPatch) {
    if wire_equal(old, new) {
        return;
    }
    match (old, new) {
        (Unknown::Object(a), Unknown::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let child = format!("{path}/{}", escape(key));
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => diff_at(x, y, child, ops),
                    (None, Some(v)) => ops.push(PatchOp::Add {
                        path: child,
                        value: v.clone(),
                    }),
                    (Some(_), None) => ops.push(PatchOp::Remove { path: child }),
                    (None, None) => {}
                }
            }
        }
        (Unknown::Array(a), Unknown::Array(b)) => {
            let shared = a.len().min(b.len());
            for (i, (x, y)) in a.iter().zip(b.iter()).enumerate().take(shared) {
                diff_at(x, y, format!("{path}/{i}"), ops);
            }
            for (i, v) in b.iter().enumerate().skip(a.len()) {
                ops.push(PatchOp::Add {
                    path: format!("{path}/{i}"),
                    value: v.clone(),
                });
            }
            for i in (b.len()..a.len()).rev() {
                ops.push(PatchOp::Remove {
                    path: format!("{path}/{i}"),
                });
            }
        }
        _ => ops.push(PatchOp::Replace {
            path,
            value: new.clone(),
        }),
    }
}

/// Apply `ops` in order; the input is never mutated. An empty patch hands the
/// document back unchanged, a root add/replace hands back the provided value,
/// and a root remove is refused.
pub fn apply(doc: &Unknown, patch: &[PatchOp]) -> Result<Unknown, PatchError> {
    let mut current = doc.clone();
    for op in patch {
        current = apply_one(&current, op)?;
    }
    Ok(current)
}

fn apply_one(doc: &Unknown, op: &PatchOp) -> Result<Unknown, PatchError> {
    let path = match op {
        PatchOp::Add { path, .. } | PatchOp::Remove { path } | PatchOp::Replace { path, .. } => {
            path
        }
    };
    let tokens = parse(path).ok_or_else(|| PatchError::BadPath(path.clone()))?;
    if tokens.is_empty() {
        return match op {
            PatchOp::Add { value, .. } | PatchOp::Replace { value, .. } => Ok(value.clone()),
            PatchOp::Remove { .. } => Err(PatchError::BadTarget("remove at root".to_string())),
        };
    }
    apply_at(doc, &tokens, op)
}

fn apply_at(doc: &Unknown, tokens: &[String], op: &PatchOp) -> Result<Unknown, PatchError> {
    let (last, parents) = tokens.split_last().expect("non-empty checked by caller");
    let mut current = doc.clone();
    let mut cursor = &mut current;
    for step in parents {
        cursor = match cursor {
            Unknown::Object(map) => map
                .get_mut(step)
                .ok_or_else(|| PatchError::Unresolvable(step.clone()))?,
            Unknown::Array(items) => {
                let i: usize = step
                    .parse()
                    .map_err(|_| PatchError::Unresolvable(step.clone()))?;
                items
                    .get_mut(i)
                    .ok_or_else(|| PatchError::Unresolvable(step.clone()))?
            }
            _ => return Err(PatchError::Unresolvable(step.clone())),
        };
        if !matches!(cursor, Unknown::Object(_) | Unknown::Array(_)) {
            return Err(PatchError::Unresolvable(step.clone()));
        }
    }
    match (cursor, op) {
        (Unknown::Object(map), PatchOp::Add { value, .. }) => {
            map.insert(last.clone(), value.clone());
            Ok(current)
        }
        (Unknown::Object(map), PatchOp::Replace { value, .. }) => {
            if !map.contains_key(last) {
                return Err(PatchError::BadTarget(last.clone()));
            }
            map.insert(last.clone(), value.clone());
            Ok(current)
        }
        (Unknown::Object(map), PatchOp::Remove { .. }) => {
            if map.remove(last).is_none() {
                return Err(PatchError::BadTarget(last.clone()));
            }
            Ok(current)
        }
        (Unknown::Array(items), PatchOp::Add { value, .. }) => {
            if last == "-" {
                items.push(value.clone());
            } else {
                let i: usize = last
                    .parse()
                    .map_err(|_| PatchError::BadTarget(last.clone()))?;
                if i > items.len() {
                    return Err(PatchError::BadTarget(last.clone()));
                }
                items.insert(i, value.clone());
            }
            Ok(current)
        }
        (Unknown::Array(items), PatchOp::Replace { value, .. }) => {
            if last == "-" {
                return Err(PatchError::BadTarget("-".to_string()));
            }
            let i: usize = last
                .parse()
                .map_err(|_| PatchError::BadTarget(last.clone()))?;
            *items
                .get_mut(i)
                .ok_or_else(|| PatchError::BadTarget(last.clone()))? = value.clone();
            Ok(current)
        }
        (Unknown::Array(items), PatchOp::Remove { .. }) => {
            if last == "-" {
                return Err(PatchError::BadTarget("-".to_string()));
            }
            let i: usize = last
                .parse()
                .map_err(|_| PatchError::BadTarget(last.clone()))?;
            if i >= items.len() {
                return Err(PatchError::BadTarget(last.clone()));
            }
            items.remove(i);
            Ok(current)
        }
        _ => Err(PatchError::Unresolvable(last.clone())),
    }
}

/// A schema-bound differ (`Schema.toDifferJsonPatch`): `diff` encodes both
/// sides and diffs the wire forms; `patch` encodes the old value, applies the
/// ops, and decodes the result back.
pub struct Differ<T> {
    schema: Schema<T>,
}

impl<T: Clone> Differ<T> {
    /// Bind a schema.
    pub fn new(schema: Schema<T>) -> Self {
        Self { schema }
    }

    /// The bound schema.
    pub fn schema(&self) -> &Schema<T> {
        &self.schema
    }

    /// The patch turning `old` into `new` on the wire, or the reason either
    /// side is unrepresentable.
    pub fn diff(&self, old: &T, new: &T) -> Result<JsonPatch, DifferError> {
        let old_wire = self.schema.try_encode(old).map_err(DifferError::Schema)?;
        let new_wire = self.schema.try_encode(new).map_err(DifferError::Schema)?;
        Ok(diff(&old_wire, &new_wire))
    }

    /// Apply `patch` to `old` and decode the result.
    pub fn patch(&self, old: &T, patch: &[PatchOp]) -> Result<T, DifferError> {
        let wired = self.schema.try_encode(old).map_err(DifferError::Schema)?;
        let patched = apply(&wired, patch).map_err(DifferError::Patch)?;
        self.schema.decode(patched).map_err(DifferError::Schema)
    }
}
