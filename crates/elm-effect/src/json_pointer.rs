//! `JsonPointer.ts` behavior port: RFC 6901 token escaping, pointer
//! parsing/formatting, and URI-fragment parsing/formatting.
//!
//! [`parse`] accepts `""` (the document root) and pointers starting with
//! `/`; anything else is [`PointerError::NotAPointer`]. Percent-decoding
//! uses a self-contained UTF-8 codec. Rust `str` cannot hold unpaired
//! surrogates, so the spec's unpaired-surrogate rows are skipped-upstream.
// This file belongs to the parked port campaign: a test reaches it through
// a `#[path]` include to port specific rows, so most of its surface is
// unused by any one test target. Warnings about the unexercised remainder
// describe the test's coverage, not a defect in the crate, so they are
// allowed here rather than fixed one by one.
#![allow(dead_code)]
use super::json::Json;

/// Why pointer parsing or resolution failed. Ordinary data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PointerError {
    /// The pointer does not start with `/` (and is not empty).
    NotAPointer(String),
    /// A token holds an invalid escape (`~` not followed by `0` or `1`).
    BadEscape(String),
    /// Not a URI fragment, or it needs percent-encoding it does not have.
    BadFragment(String),
    /// No value lives at this path in the document.
    Missing(String),
}

impl std::fmt::Display for PointerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAPointer(p) => write!(f, "not a JSON pointer: {p}"),
            Self::BadEscape(t) => write!(f, "invalid escape in token: {t}"),
            Self::BadFragment(g) => write!(f, "not a URI fragment: {g}"),
            Self::Missing(p) => write!(f, "no value at pointer: {p}"),
        }
    }
}

impl std::error::Error for PointerError {}

/// Escape one reference token: `~` becomes `~0`, then `/` becomes `~1`.
/// The order matters: replacing `~` first prevents double-escaping.
pub fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// Unescape one reference token: `~1` becomes `/`, then `~0` becomes `~`.
/// The order matters: `~01` decodes to the literal `~1`, never to `/`.
pub fn unescape_token(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

/// Validate escapes inside one token: every `~` needs a `0` or `1` after it.
fn check_escapes(token: &str) -> Result<(), PointerError> {
    let bytes = token.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'~' {
            match bytes.get(i + 1) {
                Some(b'0') | Some(b'1') => i += 2,
                _ => return Err(PointerError::BadEscape(token.into())),
            }
        } else {
            i += 1;
        }
    }
    Ok(())
}

/// Split a pointer string into unescaped tokens: `""` is the root (no
/// tokens); otherwise the string must start with `/`.
pub fn parse(pointer: &str) -> Result<Vec<String>, PointerError> {
    if pointer.is_empty() {
        return Ok(vec![]);
    }
    if !pointer.starts_with('/') {
        return Err(PointerError::NotAPointer(pointer.into()));
    }
    pointer[1..]
        .split('/')
        .map(|raw| {
            check_escapes(raw)?;
            Ok(unescape_token(raw))
        })
        .collect()
}

/// Join tokens into a pointer string, escaping each one first.
/// No tokens formats as `""` (the document root).
pub fn format(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|t| format!("/{}", escape_token(t)))
        .collect()
}

/// Percent-decode `%XX` runs into bytes, then UTF-8. Fails on stray `%`,
/// short runs, non-hex digits, and invalid UTF-8.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Percent-encode everything outside the URI unreserved set plus a few
/// structurally safe characters. Encoding runs after [`escape_token`].
fn percent_encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Parse a URI fragment (`#` or `#/a~1b`) into path tokens.
/// Percent-decoding runs before the pointer split; `#` alone is the root.
/// Returns `None` for non-fragments and undecodable input.
pub fn parse_uri_fragment(fragment: &str) -> Option<Vec<String>> {
    let rest = fragment.strip_prefix('#')?;
    if rest.is_empty() {
        return Some(vec![]);
    }
    // Characters that must arrive percent-encoded reject the fragment.
    if rest.contains(['#', ' ', '<', '>', '"', '{', '}', '|', '\\', '^', '`']) {
        return None;
    }
    let decoded = percent_decode(rest)?;
    if decoded.is_empty() {
        return Some(vec![]);
    }
    parse(&decoded).ok()
}

/// Format path tokens as a URI fragment: escape each token, then
/// percent-encode. The empty path formats as `#`.
pub fn format_uri_fragment(tokens: &[String]) -> String {
    let mut out = String::from("#");
    for token in tokens {
        out.push('/');
        out.push_str(&percent_encode(&escape_token(token)));
    }
    out
}

/// Resolve parsed tokens against a document: objects by key, arrays by
/// index. `-` never resolves. Missing steps are [`PointerError::Missing`].
pub fn resolve<'a>(doc: &'a Json, tokens: &[String]) -> Result<&'a Json, PointerError> {
    let mut current = doc;
    for token in tokens {
        current = match current {
            Json::Obj(map) => map
                .get(token)
                .ok_or_else(|| PointerError::Missing(token.clone()))?,
            Json::Arr(items) => {
                let index: usize = token
                    .parse()
                    .map_err(|_| PointerError::Missing(token.clone()))?;
                items
                    .get(index)
                    .ok_or_else(|| PointerError::Missing(token.clone()))?
            }
            _ => return Err(PointerError::Missing(token.clone())),
        };
    }
    Ok(current)
}
