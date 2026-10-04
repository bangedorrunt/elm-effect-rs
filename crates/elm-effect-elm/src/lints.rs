//! Elm fold lints, runnable by plain `cargo test` (no extra tooling):
//!
//! - `fold_no_ambient_world`: no `SystemTime`, `std::env`, or
//!   `rand::thread_rng` inside an `update` function. Time enters as a
//!   message; the fold stays pure.
//! - `fold_model_no_hashmap`: no `HashMap` inside a `*Model` struct.
//!   Iteration order must be deterministic; `BTreeMap` is the shape.
//!
//! The lints scan source text (fixtures here, the real crate as a bonus).
//! Fixtures live in `tests/fixtures/` and are included as text, never
//! compiled, so they may name crates the workspace does not depend on.

/// Extract the brace-balanced body of the first `fn <name>(` in `src`.
/// Returns the byte range (body_start, body_end) of the function body.
fn fn_body(src: &str, name: &str) -> Option<(usize, usize)> {
    let sig = format!("fn {name}(");
    let start = src.find(&sig)?;
    let open = src[start..].find('{')? + start;
    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open + 1, open + i));
                }
            }
            _ => {}
        }
    }
    None
}

/// Extract the brace-balanced body of the first `struct <name>` in `src`.
fn struct_body(src: &str, name: &str) -> Option<(usize, usize)> {
    let sig = format!("struct {name}");
    let start = src.find(&sig)?;
    let open = src[start..].find('{')? + start;
    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open + 1, open + i));
                }
            }
            _ => {}
        }
    }
    None
}

/// Tokens no fold may touch: wall clocks, process environment, ambient RNG.
const AMBIENT_TOKENS: &[&str] = &["SystemTime", "std::env", "rand::thread_rng"];

/// Hits of [`AMBIENT_TOKENS`] inside the `update` function body.
pub fn fold_no_ambient_world(src: &str) -> Vec<String> {
    let Some((start, end)) = fn_body(src, "update") else {
        return vec![];
    };
    let body = &src[start..end];
    AMBIENT_TOKENS
        .iter()
        .filter(|token| body.contains(**token))
        .map(|token| format!("update uses ambient capability: {token}"))
        .collect()
}

/// `HashMap` inside any `*Model` struct body. Deterministic iteration is a
/// fold invariant; hash order is not.
pub fn fold_model_no_hashmap(src: &str) -> Vec<String> {
    let mut hits = vec![];
    let mut search = src;
    let mut offset = 0usize;
    while let Some(pos) = search.find("struct ") {
        let name_start = pos + "struct ".len();
        let rest = &search[name_start..];
        let name_end = rest
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        if name.ends_with("Model") {
            let abs = offset + pos;
            if let Some((start, end)) = struct_body(&src[abs..], name) {
                let body = &src[abs + start..abs + end];
                if body.contains("HashMap") {
                    hits.push(format!("{name} stores a HashMap; use BTreeMap"));
                }
            }
        }
        let advance = pos + "struct ".len() + name_end.max(1);
        offset += advance;
        search = &search[advance.min(search.len())..];
        if search.is_empty() {
            break;
        }
    }
    hits
}
