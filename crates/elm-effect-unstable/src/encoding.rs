//! `encoding/` behavior port: Base64, Base64Url, Hex, and the shared
//! [`EncodingError`]. No third-party codec: the alphabets are small and
//! the spec pins exact error metadata (`kind`, `module`, `input`).
//!
//! - Base64 decode skips ASCII whitespace (`SGVs\r\nbG8=` decodes).
//! - Hex decode accepts mixed case; odd lengths and non-hex fail.
//! - Every decode failure reports [`DecodeKind::Decode`] with the module
//!   name and the original input.

/// Why a decode failed. Only decoding fails in this port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeKind {
    /// Input bytes are malformed.
    Decode,
}

impl DecodeKind {
    /// The spec tag string.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Decode => "Decode",
        }
    }
}

/// An encoding failure with placement metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodingError {
    kind: DecodeKind,
    module: &'static str,
    input: String,
    message: String,
}

impl EncodingError {
    /// Build a decode failure for `module` over `input`.
    pub fn decode(module: &'static str, input: &str, message: impl Into<String>) -> Self {
        Self {
            kind: DecodeKind::Decode,
            module,
            input: input.to_string(),
            message: message.into(),
        }
    }

    /// Failure class tag (`"Decode"` here).
    pub fn kind(&self) -> &'static str {
        self.kind.tag()
    }

    /// Module that rejected the input (`"Base64"`, `"Hex"`, ...).
    pub fn module(&self) -> &'static str {
        self.module
    }

    /// The rejected input, verbatim.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Human message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for EncodingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EncodingError({}): {}", self.module, self.message)
    }
}

impl std::error::Error for EncodingError {}

const B64_STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64_encode(bytes: &[u8], alphabet: &[u8; 64]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.len();
        let mut bits: u32 = 0;
        for (i, byte) in chunk.iter().enumerate() {
            bits |= (*byte as u32) << (16 - 8 * i);
        }
        let chars = if n == 3 { 4 } else { n + 1 };
        for i in 0..chars {
            let shift = 18 - 6 * i;
            out.push(alphabet[((bits >> shift) & 0x3f) as usize] as char);
        }
        for _ in chars..4 {
            out.push('=');
        }
    }
    out
}

fn b64_value(byte: u8, url: bool) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' if !url => Some(62),
        b'/' if !url => Some(63),
        b'-' if url => Some(62),
        b'_' if url => Some(63),
        _ => None,
    }
}

fn b64_decode(input: &str, module: &'static str, url: bool) -> Result<Vec<u8>, EncodingError> {
    let fail = |message: &str| EncodingError::decode(module, input, message);
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.is_empty() {
        return Ok(vec![]);
    }
    if clean.len() % 4 != 0 {
        return Err(fail("invalid length"));
    }
    let mut out = Vec::new();
    for chunk in clean.chunks(4) {
        let pad = chunk.iter().rev().take_while(|b| **b == b'=').count();
        if pad > 2 {
            return Err(fail("invalid padding"));
        }
        let mut payload: u32 = 0;
        for (i, byte) in chunk.iter().enumerate() {
            if *byte == b'=' {
                if i < 4 - pad {
                    return Err(fail("misplaced padding"));
                }
                continue;
            }
            payload = (payload << 6)
                | u32::from(b64_value(*byte, url).ok_or_else(|| fail("invalid character"))?);
        }
        let total_bits = 6 * (4 - pad);
        let bytes = 3 - pad;
        for i in 0..bytes {
            out.push(((payload >> (total_bits - 8 * (i + 1))) & 0xff) as u8);
        }
    }
    Ok(out)
}

/// Standard Base64 encode (bytes or UTF-8 string).
pub fn base64_encode(bytes: &[u8]) -> String {
    b64_encode(bytes, B64_STD)
}

/// URL-safe Base64 encode (`-`/`_` alphabet, padding kept).
pub fn base64_url_encode(bytes: &[u8]) -> String {
    b64_encode(bytes, B64_URL)
}

/// Standard Base64 decode; ASCII whitespace is skipped.
pub fn base64_decode(input: &str) -> Result<Vec<u8>, EncodingError> {
    b64_decode(input, "Base64", false)
}

/// URL-safe Base64 decode.
pub fn base64_url_decode(input: &str) -> Result<Vec<u8>, EncodingError> {
    b64_decode(input, "Base64Url", true)
}

/// Decode to a UTF-8 string.
pub fn base64_decode_string(input: &str) -> Result<String, EncodingError> {
    let bytes = base64_decode(input)?;
    String::from_utf8(bytes).map_err(|_| EncodingError::decode("Base64", input, "invalid UTF-8"))
}

/// Decode URL-safe to a UTF-8 string.
pub fn base64_url_decode_string(input: &str) -> Result<String, EncodingError> {
    let bytes = base64_url_decode(input)?;
    String::from_utf8(bytes).map_err(|_| EncodingError::decode("Base64Url", input, "invalid UTF-8"))
}

/// Lowercase hexadecimal encode (bytes or UTF-8 string).
pub fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    out
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Hexadecimal decode (either case); odd lengths and non-hex fail.
pub fn hex_decode(input: &str) -> Result<Vec<u8>, EncodingError> {
    let fail = |message: &str| EncodingError::decode("Hex", input, message);
    let bytes = input.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(fail("odd length"));
    }
    bytes
        .chunks(2)
        .map(|pair| match (hex_value(pair[0]), hex_value(pair[1])) {
            (Some(hi), Some(lo)) => Ok(hi << 4 | lo),
            _ => Err(fail("invalid character")),
        })
        .collect()
}

/// Decode hex to a UTF-8 string.
pub fn hex_decode_string(input: &str) -> Result<String, EncodingError> {
    let bytes = hex_decode(input)?;
    String::from_utf8(bytes).map_err(|_| EncodingError::decode("Hex", input, "invalid UTF-8"))
}

/// `length` random bytes as lowercase hex (deterministic xorshift seeded
/// from wall time and process id: uniqueness varies, shape never does).
pub fn hex_random(length: usize) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0x243F6A88);
    let mut state = (nanos as u64)
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(std::process::id() as u64)
        | 1;
    let mut bytes = vec![0u8; length];
    for slot in bytes.iter_mut() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *slot = (state >> 33) as u8;
    }
    hex_encode(&bytes)
}
