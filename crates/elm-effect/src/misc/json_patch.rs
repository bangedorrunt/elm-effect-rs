//! `JsonPatch` port (`JsonPatch.ts` at the pin).
//!
//! RFC 6902 operations over [`Json`](super::json_pointer::Json):
//! [`apply`] runs a patch, [`diff`] derives one. Object keys iterate in
//! sorted order (the shared model stores them sorted), so generated patches
//! are deterministic like the tag's stable-key diffs.
//!
//! Residuals: the tag's `get` names this [`diff`]; string-level patch
//! rendering is out of scope (patches are data, applied directly).

use super::json::Json;
use super::json_pointer::{escape_token, get, parse, set};

/// One RFC 6902 operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Add `value` at `path` (`-` appends to arrays).
    Add { path: String, value: Json },
    /// Remove the value at `path`.
    Remove { path: String },
    /// Replace the value at `path` (must exist).
    Replace { path: String, value: Json },
    /// Move the value from `from` to `path`.
    Move { from: String, path: String },
    /// Copy the value from `from` to `path`.
    Copy { from: String, path: String },
    /// Check the value at `path` equals `value`.
    Test { path: String, value: Json },
}

fn remove_at(doc: &Json, path: &[String]) -> Option<Json> {
    if path.is_empty() {
        return None;
    }
    match doc {
        Json::Obj(map) => {
            if path.len() == 1 {
                if !map.contains_key(&path[0]) {
                    return None;
                }
                let mut next = map.clone();
                next.remove(&path[0]);
                return Some(Json::Obj(next));
            }
            let child = map.get(&path[0])?;
            let mut next = map.clone();
            next.insert(path[0].clone(), remove_at(child, &path[1..])?);
            Some(Json::Obj(next))
        }
        Json::Arr(items) => {
            if path.len() == 1 {
                let i: usize = path[0].parse().ok()?;
                if i >= items.len() {
                    return None;
                }
                let mut next = items.clone();
                next.remove(i);
                return Some(Json::Arr(next));
            }
            let i: usize = path[0].parse().ok()?;
            let child = items.get(i)?;
            let mut next = items.clone();
            next[i] = remove_at(child, &path[1..])?;
            Some(Json::Arr(next))
        }
        _ => None,
    }
}

/// Apply `ops` in order. `Err` names the first failing operation; the
/// document is unchanged on failure (operations rebuild values).
pub fn apply(doc: &Json, ops: &[Op]) -> Result<Json, String> {
    let mut cur = doc.clone();
    for op in ops {
        cur = match op {
            Op::Add { path, value } => {
                let tokens = parse(path);
                set(&cur, &tokens, value.clone())
                    .ok_or_else(|| format!("add: cannot apply at {path}"))?
            }
            Op::Remove { path } => {
                let tokens = parse(path);
                remove_at(&cur, &tokens).ok_or_else(|| format!("remove: missing {path}"))?
            }
            Op::Replace { path, value } => {
                let tokens = parse(path);
                if get(&cur, &tokens).is_none() {
                    return Err(format!("replace: missing {path}"));
                }
                set(&cur, &tokens, value.clone())
                    .ok_or_else(|| format!("replace: cannot apply at {path}"))?
            }
            Op::Move { from, path } => {
                let ft = parse(from);
                let moved = get(&cur, &ft)
                    .cloned()
                    .ok_or_else(|| format!("move: missing {from}"))?;
                let without =
                    remove_at(&cur, &ft).ok_or_else(|| format!("move: missing {from}"))?;
                let tt = parse(path);
                set(&without, &tt, moved).ok_or_else(|| format!("move: cannot apply at {path}"))?
            }
            Op::Copy { from, path } => {
                let ft = parse(from);
                let copied = get(&cur, &ft)
                    .cloned()
                    .ok_or_else(|| format!("copy: missing {from}"))?;
                let tt = parse(path);
                set(&cur, &tt, copied).ok_or_else(|| format!("copy: cannot apply at {path}"))?
            }
            Op::Test { path, value } => {
                let tokens = parse(path);
                match get(&cur, &tokens) {
                    Some(actual) if actual == value => cur,
                    _ => return Err(format!("test: mismatch at {path}")),
                }
            }
        };
    }
    Ok(cur)
}

/// Derive a patch turning `old` into `new`: identical values give no ops;
/// objects diff per key in sorted order; arrays diff per index with removals
/// in descending index order (no shifting); anything else is one root
/// replace. `__proto__` is an ordinary key.
pub fn diff(old: &Json, new: &Json) -> Vec<Op> {
    diff_at(old, new, String::new())
}

fn child(base: &str, token: String) -> String {
    format!("{base}/{}", escape_token(&token))
}

fn diff_at(old: &Json, new: &Json, path: String) -> Vec<Op> {
    if old == new {
        return Vec::new();
    }
    match (old, new) {
        (Json::Obj(a), Json::Obj(b)) => {
            let mut ops = Vec::new();
            for k in a.keys() {
                match b.get(k) {
                    None => ops.push(Op::Remove {
                        path: child(&path, k.clone()),
                    }),
                    Some(nv) => ops.extend(diff_at(&a[k], nv, child(&path, k.clone()))),
                }
            }
            for (k, v) in b.iter() {
                if !a.contains_key(k) {
                    ops.push(Op::Add {
                        path: child(&path, k.clone()),
                        value: v.clone(),
                    });
                }
            }
            ops
        }
        (Json::Arr(a), Json::Arr(b)) => {
            let mut ops = Vec::new();
            let common = a.len().min(b.len());
            for (i, (o, n)) in a.iter().zip(b.iter()).enumerate().take(common) {
                ops.extend(diff_at(o, n, format!("{path}/{i}")));
            }
            if b.len() > a.len() {
                for (i, v) in b.iter().enumerate().skip(a.len()) {
                    ops.push(Op::Add {
                        path: format!("{path}/{i}"),
                        value: v.clone(),
                    });
                }
            } else {
                for i in (b.len()..a.len()).rev() {
                    ops.push(Op::Remove {
                        path: format!("{path}/{i}"),
                    });
                }
            }
            ops
        }
        _ => vec![Op::Replace {
            path,
            value: new.clone(),
        }],
    }
}
