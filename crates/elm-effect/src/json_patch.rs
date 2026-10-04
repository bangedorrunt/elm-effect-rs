//! `JsonPatch.ts` behavior port: structural diff ([`get`]) and
//! sequential application ([`apply`]) over the [`Json`] value model.
//!
//! Paths are pointer strings (`""` is the root, `-` appends to arrays).
//! Diff output is deterministic but not guaranteed minimal: object keys in
//! sorted order, array removals highest index first so nothing shifts.
//! Inputs are never mutated: everything takes `&Json` and builds anew, and
//! an empty patch returns a clone of the input.

use super::json::Json;
use super::json_pointer::parse;

/// One patch operation: `add`, `remove`, or `replace` at a pointer path.
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    /// Insert `value` at `path` (`-` appends to arrays).
    Add {
        /// Pointer string the operation targets.
        path: String,
        /// Value to insert.
        value: Json,
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
        value: Json,
    },
}

impl Operation {
    /// The pointer string this operation targets.
    pub fn path(&self) -> &str {
        match self {
            Self::Add { path, .. } | Self::Remove { path } | Self::Replace { path, .. } => path,
        }
    }
}

/// Why [`apply`] failed. Ordinary data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchError {
    /// The pointer string does not parse.
    BadPath(String),
    /// A step on the way down is missing or not a container.
    Unresolvable(String),
    /// The target itself is missing, out of bounds, or illegal (`-` for
    /// replace/remove, remove at the root).
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

/// Join a raw parent path and a raw child key into one pointer string,
/// escaping exactly once on the way out.
fn at(tokens: &[String], key: &str) -> String {
    let mut full = tokens.to_vec();
    full.push(key.into());
    super::json_pointer::format(&full)
}

/// Compute the structural patch turning `old` into `new`. Empty output
/// means identical (an empty patch applies to a clone of the input).
/// Arrays compare by index with no move detection; removals run highest
/// index first; a type change is one root-or-branch `replace`.
pub fn get(old: &Json, new: &Json) -> Vec<Operation> {
    let mut ops = vec![];
    diff(old, new, &vec![], &mut ops);
    ops
}

fn diff(old: &Json, new: &Json, path: &[String], ops: &mut Vec<Operation>) {
    if old.structurally_equal(new) {
        return;
    }
    let here = super::json_pointer::format(path);
    match (old, new) {
        (Json::Obj(a), Json::Obj(b)) => {
            for key in a.keys() {
                let mut child = path.to_vec();
                child.push(key.clone());
                match b.get(key) {
                    None => ops.push(Operation::Remove { path: at(path, key) }),
                    Some(v) => diff(&a[key], v, &child, ops),
                }
            }
            for key in b.keys() {
                if !a.contains_key(key) {
                    ops.push(Operation::Add { path: at(path, key), value: b[key].clone() });
                }
            }
        }
        (Json::Arr(a), Json::Arr(b)) => {
            let shared = a.len().min(b.len());
            for (i, (x, y)) in a.iter().zip(b.iter()).take(shared).enumerate() {
                let mut child = path.to_vec();
                child.push(i.to_string());
                diff(x, y, &child, ops);
            }
            if b.len() > a.len() {
                for (i, v) in b.iter().enumerate().skip(a.len()) {
                    let mut p = path.to_vec();
                    p.push(i.to_string());
                    ops.push(Operation::Add { path: super::json_pointer::format(&p), value: v.clone() });
                }
            } else {
                for i in (b.len()..a.len()).rev() {
                    let mut p = path.to_vec();
                    p.push(i.to_string());
                    ops.push(Operation::Remove { path: super::json_pointer::format(&p) });
                }
            }
        }
        _ => ops.push(Operation::Replace { path: here, value: new.clone() }),
    }
}

/// Apply operations in order; later operations see earlier results.
/// Returns a new document; the input is never mutated.
pub fn apply(patch: &[Operation], doc: &Json) -> Result<Json, PatchError> {
    let mut current = doc.clone();
    for op in patch {
        current = apply_one(op, &current)?;
    }
    Ok(current)
}

fn apply_one(op: &Operation, doc: &Json) -> Result<Json, PatchError> {
    let tokens = parse(op.path()).map_err(|_| PatchError::BadPath(op.path().into()))?;
    match op {
        Operation::Replace { value, .. } if tokens.is_empty() => Ok(value.clone()),
        Operation::Add { value, .. } if tokens.is_empty() => Ok(value.clone()),
        Operation::Remove { .. } if tokens.is_empty() => Err(PatchError::BadTarget("remove at root".into())),
        _ => set_at(doc, &tokens, op),
    }
}

fn set_at(doc: &Json, tokens: &[String], op: &Operation) -> Result<Json, PatchError> {
    let (last, parents) = tokens.split_last().expect("non-empty checked");
    let mut current = doc.clone();
    let mut cursor = &mut current;
    for step in parents {
        cursor = match cursor {
            Json::Obj(map) => map
                .get_mut(step)
                .ok_or_else(|| PatchError::Unresolvable(step.clone()))?,
            Json::Arr(items) => {
                let i: usize = step.parse().map_err(|_| PatchError::Unresolvable(step.clone()))?;
                items.get_mut(i).ok_or_else(|| PatchError::Unresolvable(step.clone()))?
            }
            _ => return Err(PatchError::Unresolvable(step.clone())),
        };
        match cursor {
            Json::Obj(_) | Json::Arr(_) => {}
            _ => return Err(PatchError::Unresolvable(step.clone())),
        }
    }
    match (cursor, op) {
        (Json::Obj(map), Operation::Add { value, .. }) => {
            map.insert(last.clone(), value.clone());
            Ok(current)
        }
        (Json::Obj(map), Operation::Replace { value, .. }) => {
            if !map.contains_key(last) {
                return Err(PatchError::BadTarget(last.clone()));
            }
            map.insert(last.clone(), value.clone());
            Ok(current)
        }
        (Json::Obj(map), Operation::Remove { .. }) => {
            if map.remove(last).is_none() {
                return Err(PatchError::BadTarget(last.clone()));
            }
            Ok(current)
        }
        (Json::Arr(items), Operation::Add { value, .. }) => {
            if last == "-" {
                items.push(value.clone());
            } else {
                let i: usize = last.parse().map_err(|_| PatchError::BadTarget(last.clone()))?;
                if i > items.len() {
                    return Err(PatchError::BadTarget(last.clone()));
                }
                items.insert(i, value.clone());
            }
            Ok(current)
        }
        (Json::Arr(items), Operation::Replace { value, .. }) => {
            if last == "-" {
                return Err(PatchError::BadTarget("-".into()));
            }
            let i: usize = last.parse().map_err(|_| PatchError::BadTarget(last.clone()))?;
            let slot = items.get_mut(i).ok_or_else(|| PatchError::BadTarget(last.clone()))?;
            *slot = value.clone();
            Ok(current)
        }
        (Json::Arr(items), Operation::Remove { .. }) => {
            if last == "-" {
                return Err(PatchError::BadTarget("-".into()));
            }
            let i: usize = last.parse().map_err(|_| PatchError::BadTarget(last.clone()))?;
            if i >= items.len() {
                return Err(PatchError::BadTarget(last.clone()));
            }
            items.remove(i);
            Ok(current)
        }
        _ => Err(PatchError::Unresolvable(last.clone())),
    }
}
