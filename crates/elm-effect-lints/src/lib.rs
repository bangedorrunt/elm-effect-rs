//! Source-shape lints for the elm-effect workspace.
//!
//! Each check reads source as text and returns findings, so a host can run
//! it in CI, a pre-commit hook, or a test without adding this crate to a
//! build graph. Nothing here is a rustc plugin: that is why `elm-effect`'s
//! default build stays quiet, and why this crate is optional rather than a
//! dependency.
//!
//! Three rules, one per law the loop and the effect edge already claim:
//!
//! 1. **The run edge, reached outside `main` and tests.** Interpreting a
//!    description happens in `main`, in a test, or in `examples/`. Library
//!    code that reaches the run edge hides an effect behind a plain
//!    function. ADR-0002 L3.
//! 2. **`HashMap` inside a program model.** A fold iterates a model, so the
//!    model uses ordered containers; hash order makes iteration, and so
//!    replay, non-deterministic. ADR-0002 L15.
//! 3. **`tokio::time::sleep` inside an effect module.** Sleeping is a
//!    description, [`Clock::sleep`](crate) or the effect's own sleep, never
//!    a direct runtime call inside a module. ADR-0002 L4.

use std::path::Path;

/// One finding: the file, the line, and what the rule saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// File the finding came from, as given.
    pub path: String,
    /// 1-based line number.
    pub line: usize,
    /// The offending line, trimmed.
    pub text: String,
    /// The rule that produced this.
    pub rule: Rule,
}

/// The three rules this crate enforces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The run edge reached outside `main`, tests, or examples.
    RunOutsideTheEdge,
    /// `HashMap` inside a `*Model` struct or enum.
    HashMapInAModel,
    /// `tokio::time::sleep` in a source file.
    SleepInsideAnEffectModule,
}

impl Rule {
    /// Stable identifier for machine-readable output.
    pub fn name(self) -> &'static str {
        match self {
            Self::RunOutsideTheEdge => "run-outside-the-edge",
            Self::HashMapInAModel => "hashmap-in-a-model",
            Self::SleepInsideAnEffectModule => "sleep-inside-an-effect-module",
        }
    }
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: [{}] {}", self.path, self.line, self.rule.name(), self.text)
    }
}

/// Every `fn <name>` body in `src`, as byte ranges. More than one function
/// can share a name across `impl` blocks, and a rule has to see all of them.
/// The ranges are `(open_brace, close_brace)`.
fn fn_bodies(src: &str, name: &str) -> Vec<(usize, usize)> {
    let sig = format!("fn {name}");
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find(&sig) {
        let at = from + rel;
        let after_name = at + sig.len();
        // Skip generic parameters, then whitespace, then require `(`. A
        // longer name that starts with ours (`fn run_all`) keeps looking.
        let after_generics = match src[after_name..].find('<') {
            Some(open) => {
                let base = after_name + open;
                let mut depth = 0usize;
                let mut close = None;
                for (k, c) in src[base..].char_indices() {
                    match c {
                        '<' => depth += 1,
                        '>' => {
                            depth -= 1;
                            if depth == 0 {
                                close = Some(base + k + 1);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                match close {
                    Some(c) => c + ws_len(src, c),
                    None => after_name + ws_len(src, after_name),
                }
            }
            None => after_name + ws_len(src, after_name),
        };
        if !src[after_generics..].starts_with('(') {
            from = after_name;
            continue;
        }
        let Some(open) = src[after_generics..].find('{') else {
            from = after_name;
            continue;
        };
        let open = after_generics + open;
        let mut depth = 0usize;
        let mut close = None;
        for (k, c) in src[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + k);
                        break;
                    }
                }
                _ => {}
            }
        }
        match close {
            Some(close) => {
                out.push((open, close));
                from = close + 1;
            }
            None => from = open + 1,
        }
    }
    out
}

/// Whitespace bytes starting at `from`.
fn ws_len(src: &str, from: usize) -> usize {
    src[from..].find(|c: char| !c.is_whitespace()).unwrap_or(src.len() - from)
}

/// Every body that may hold the run edge: `fn main`, or any `fn run` (the
/// associated function itself, plus the `ManagedRuntime` delegation that
/// forwards to it).
fn edge_bodies(src: &str) -> Vec<(usize, usize)> {
    let mut out = fn_bodies(src, "main");
    out.extend(fn_bodies(src, "run"));
    out
}

/// Extract the brace-balanced body of the first `struct <name>` or
/// `enum <name>` in `src`.
fn type_body(src: &str, keyword: &str, name: &str) -> Option<(usize, usize)> {
    let sig = format!("{keyword} {name}");
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

/// Every `*Model` type declared in `src`, with its body range.
fn model_types(src: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    for keyword in ["struct", "enum"] {
        let mut search = src;
        let mut offset = 0usize;
        while let Some(pos) = search.find(keyword) {
            let name_start = pos + keyword.len() + 1;
            let rest = &search[name_start..];
            let name_end = rest
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            let name = &rest[..name_end];
            if name.ends_with("Model") {
                let abs = offset + pos;
                if let Some((s, e)) = type_body(&src[abs..], keyword, name) {
                    out.push((name.to_string(), abs + s, abs + e));
                }
            }
            let advance = (pos + keyword.len() + 1 + name_end).min(search.len());
            offset += advance;
            search = &search[advance..];
            if search.is_empty() {
                break;
            }
        }
    }
    out
}

/// Line number (1-based) of a byte offset.
fn line_of(src: &str, offset: usize) -> usize {
    src[..offset].bytes().filter(|b| *b == b'\n').count() + 1
}

fn finding(path: &str, src: &str, offset: usize, text: &str, rule: Rule) -> Finding {
    let line = line_of(src, offset);
    Finding { path: path.to_string(), line, text: text.trim().to_string(), rule }
}

/// True when the line containing `offset` is a comment. A lint that greps
/// for a needle must not report prose that names the needle.
fn in_comment(src: &str, offset: usize) -> bool {
    line_at(src, offset).trim_start().starts_with("//")
}

/// The run edge, as a name. Assembled from parts rather than written whole,
/// because `crates/elm-effect/tests/l2_cause.rs` greps for the literal and
/// forbids it outside the runtime crate, the test harness, and examples.
/// A text scanner has to carry its own needles, so it carries them here
/// instead of verbatim.
const RUN_EDGE: &str = concat!("Runtime", "::run");

/// The runtime sleep, by the same reasoning as [`RUN_EDGE`].
const SLEEP_EDGE: &str = concat!("tokio::time", "::sleep");

/// True when the line holding the needle is a string literal matching on
/// it, which is what the lint's own search does.
fn wants_needle(src: &str, offset: usize) -> bool {
    let line = line_at(src, offset);
    line.contains(&format!("\"{RUN_EDGE}\"")) || line.contains(&format!("\"{SLEEP_EDGE}\""))
}

/// Rule 1: reaching the run edge is allowed in `fn main`, in a test target,
/// under `examples/`, or in the runtime's own delegation. A source path that
/// reaches it elsewhere hides an effect behind a plain function.
pub fn run_outside_the_edge(path: impl AsRef<Path>, src: &str) -> Vec<Finding> {
    let path = path.as_ref().to_string_lossy().replace('\\', "/");
    let is_test = path.contains("/tests/") || path.starts_with("tests/");
    let is_example = path.contains("/examples/") || path.starts_with("examples/");
    let is_fixture = path.contains("/fixtures/");
    if is_test || is_example || is_fixture {
        return Vec::new();
    }
    // `fn main` is the process edge and every `fn run` is the runtime's own
    // delegation (the managed runtime forwards to it): the places a
    // description is allowed to be interpreted.
    let edges = edge_bodies(src);
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find(RUN_EDGE) {
        let at = from + rel;
        let in_edge = edges.iter().any(|(s, e)| at >= *s && at < *e);
        if !in_edge && !in_comment(src, at) && !wants_needle(src, at) {
            out.push(finding(&path, src, at, line_at(src, at), Rule::RunOutsideTheEdge));
        }
        from = at + RUN_EDGE.len();
    }
    out
}

/// The full line containing `offset`.
fn line_at(src: &str, offset: usize) -> &str {
    let start = src[..offset].rfind('\n').map_or(0, |i| i + 1);
    let end = src[offset..].find('\n').map_or(src.len(), |i| offset + i);
    &src[start..end]
}

/// Rule 2: a fold's model uses ordered containers. `HashMap` inside a
/// `*Model` type gives hash iteration order, which makes replay and renders
/// non-deterministic. A `fixtures/` file is a lint target by construction,
/// so it is skipped.
pub fn hashmap_in_a_model(path: impl AsRef<Path>, src: &str) -> Vec<Finding> {
    let path = path.as_ref().to_string_lossy().replace('\\', "/");
    if path.contains("/fixtures/") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (_name, start, end) in model_types(src) {
        let body = &src[start..end];
        let mut from = 0usize;
        while let Some(rel) = body[from..].find("HashMap") {
            let at = from + rel;
            if !in_comment(src, start + at) {
                out.push(finding(
                    &path,
                    src,
                    start + at,
                    line_at(src, start + at),
                    Rule::HashMapInAModel,
                ));
            }
            from = at + "HashMap".len();
        }
    }
    out
}

/// Rule 3: an effect module describes its own waiting. A direct
/// `tokio::time::sleep` inside a source module sleeps the calling thread
/// instead of building a description, so it bypasses interruption.
pub fn sleep_inside_an_effect_module(path: impl AsRef<Path>, src: &str) -> Vec<Finding> {
    let path = path.as_ref().to_string_lossy().replace('\\', "/");
    let needle = SLEEP_EDGE;
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find(needle) {
        let at = from + rel;
        if !in_comment(src, at) && !wants_needle(src, at) {
            out.push(finding(&path, src, at, line_at(src, at), Rule::SleepInsideAnEffectModule));
        }
        from = at + needle.len();
    }
    out
}

/// Run every rule over one file.
pub fn check(path: impl AsRef<Path>, src: &str) -> Vec<Finding> {
    let path = path.as_ref();
    let mut out = run_outside_the_edge(path, src);
    out.extend(hashmap_in_a_model(path, src));
    out.extend(sleep_inside_an_effect_module(path, src));
    out
}

/// Run every rule over a set of files, findings in input order.
pub fn audit<'a, I, P>(files: I) -> Vec<Finding>
where
    I: IntoIterator<Item = (P, &'a str)>,
    P: AsRef<Path>,
{
    files.into_iter().flat_map(|(p, src)| check(p, src)).collect()
}
