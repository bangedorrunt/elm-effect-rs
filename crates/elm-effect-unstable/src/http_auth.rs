//! HTTP credential parsing: scheme-split decoding (the asserted core of
//! TS `HttpApiSecurity` credential rows). Service plumbing (`securityDecode`
//! providers) is runtime scaffolding; the rows assert parsed values, pinned
//! here. Password secrecy travels with the logging layer; this module pins
//! the parsed strings.
//!
//! Loads via `#[path]` includes (the .15 precedent): no facade touch.

#[path = "encoding.rs"]
mod encoding;

/// Split `Scheme credentials`: scheme case-insensitive, at least one
/// whitespace run, credentials trimmed. `None` on mismatch-shaped input
/// (no whitespace, wrong scheme): TS yields an empty credential there.
fn split_scheme<'a>(header: &'a str, scheme: &str) -> Option<&'a str> {
    let end = header.find(|c: char| c.is_whitespace())?;
    let (name, rest) = header.split_at(end);
    if !name.eq_ignore_ascii_case(scheme) {
        return None;
    }
    Some(rest.trim())
}

/// Bearer/custom-scheme token: the credentials on scheme match, `None`
/// otherwise.
pub fn decode_scheme_token(header: &str, scheme: &str) -> Option<String> {
    split_scheme(header, scheme).map(str::to_string)
}

/// Basic username + password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicCredential {
    pub username: String,
    pub password: String,
}

fn empty_credential() -> BasicCredential {
    BasicCredential {
        username: String::new(),
        password: String::new(),
    }
}

/// Decode `Basic <base64>` credentials: split at the first colon; any
/// mismatch or undecodable input yields empty/empty (TS behavior).
pub fn decode_basic(header: &str) -> BasicCredential {
    let Some(encoded) = split_scheme(header, "basic") else {
        return empty_credential();
    };
    let Ok(text) = encoding::base64_decode_string(encoded) else {
        return empty_credential();
    };
    match text.split_once(':') {
        Some((user, pass)) => BasicCredential {
            username: user.to_string(),
            password: pass.to_string(),
        },
        // Untested shape: keep the whole value as the username.
        None => BasicCredential {
            username: text,
            password: String::new(),
        },
    }
}
