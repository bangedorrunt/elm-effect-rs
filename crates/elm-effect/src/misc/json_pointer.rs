//! `JsonPointer` port (`JsonPointer.ts` at the pin).
//!
//! RFC 6901 token escaping (`~` to `~0`, `/` to `~1`, in that order) plus
//! the tag's URI-fragment helpers, and pointer evaluation over
//! [`Json`](crate::json::Json): [`parse`] splits a pointer string into
//! tokens, [`get`] resolves them, [`set`] returns an updated value.
//!
//! Residuals: none structural; `decodeUriFragment` percent-decoding covers
//! `%XX` ASCII escapes (full UTF-8 sequences resolve to the replacement
//! character when malformed, like the tag's `decodeURIComponent` throw
//! mapped to `None`).

pub use super::json::Json;

/// Escape one reference token (`~` to `~0`, then `/` to `~1`).
pub fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// Unescape one reference token (`~1` to `/`, then `~0` to `~`).
pub fn unescape_token(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

/// Escape a token for a URI fragment.
pub fn format_uri_fragment_token(token: &str) -> String {
    percent_encode(&escape_token(token))
}

/// Decode one percent-encoded byte triple. Returns `None` on malformed input.
fn decode_triple(bytes: &[u8], i: usize) -> Option<u8> {
    if i + 2 >= bytes.len() {
        return None;
    }
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    Some(hex(bytes[i + 1])? << 4 | hex(bytes[i + 2])?)
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decode a URI fragment to its raw pointer string. `None` on malformed
/// percent-encoding.
pub fn decode_uri_fragment(fragment: &str) -> Option<String> {
    let bytes = fragment.as_bytes();
    let mut raw: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            raw.push(decode_triple(bytes, i)?);
            i += 3;
        } else {
            raw.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(raw).ok()
}

/// Parse a `#`-prefixed URI fragment into tokens. `None` when the fragment
/// is malformed or missing its `#`.
pub fn parse_uri_fragment(fragment: &str) -> Option<Vec<String>> {
    let raw = fragment.strip_prefix('#')?;
    let decoded = decode_uri_fragment(raw)?;
    Some(parse(&decoded))
}

/// Format tokens as a `#`-prefixed URI fragment.
pub fn format_uri_fragment(path: &[String]) -> String {
    let mut out = String::from("#");
    for token in path {
        out.push('/');
        out.push_str(&format_uri_fragment_token(token));
    }
    out
}

/// Split a JSON Pointer string (`""`, `"/a/b"`, `"/a~1b"`) into tokens.
/// The empty pointer addresses the whole document.
pub fn parse(pointer: &str) -> Vec<String> {
    if pointer.is_empty() {
        return Vec::new();
    }
    pointer
        .strip_prefix('/')
        .unwrap_or(pointer)
        .split('/')
        .map(unescape_token)
        .collect()
}

/// Format tokens as a pointer string.
pub fn format(path: &[String]) -> String {
    let mut out = String::new();
    for token in path {
        out.push('/');
        out.push_str(&escape_token(token));
    }
    out
}

/// Resolve `path` against `doc`. Object keys match by string; array tokens
/// match by decimal index. `None` on any miss.
pub fn get<'a>(doc: &'a Json, path: &[String]) -> Option<&'a Json> {
    let mut cur = doc;
    for token in path {
        cur = match (cur, token.parse::<usize>()) {
            (Json::Arr(items), Ok(i)) => items.get(i)?,
            (Json::Obj(map), _) => map.get(token)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Return a copy of `doc` with `path` set to `value`. Missing objects along
/// the way are created; a missing array index appends only at the end
/// (`"-"` also appends). Returns `None` when the path cannot apply.
pub fn set(doc: &Json, path: &[String], value: Json) -> Option<Json> {
    if path.is_empty() {
        return Some(value);
    }
    match doc {
        Json::Obj(map) => {
            let mut next = map.clone();
            let child = next
                .get(&path[0])
                .cloned()
                .unwrap_or(Json::Obj(Default::default()));
            next.insert(path[0].clone(), set(&child, &path[1..], value)?);
            Some(Json::Obj(next))
        }
        Json::Arr(items) => {
            let mut next = items.clone();
            if path[0] == "-" {
                if path.len() == 1 {
                    next.push(value);
                    return Some(Json::Arr(next));
                }
                let child = Json::Obj(Default::default());
                next.push(set(&child, &path[1..], value)?);
                return Some(Json::Arr(next));
            }
            let i: usize = path[0].parse().ok()?;
            if i > next.len() {
                return None;
            }
            if i == next.len() {
                if path.len() == 1 {
                    next.push(value);
                    return Some(Json::Arr(next));
                }
                let child = Json::Obj(Default::default());
                next.push(set(&child, &path[1..], value)?);
                return Some(Json::Arr(next));
            }
            next[i] = set(&next[i], &path[1..], value)?;
            Some(Json::Arr(next))
        }
        _ => None,
    }
}
