//! `HttpMiddleware.compression` port: negotiation, eligibility, gzip/deflate.
//!
//! Ports `packages/effect/src/http/HttpMiddleware.ts` (`compression`) and
//! `internal/compression.ts` (`negotiate`, `parseAcceptEncoding`,
//! `defaultCompressible`, `varyWith`). Only gzip/deflate bodies are actually
//! transformed (`flate2`, default level 6 like the TS defaults); `br`/`zstd`
//! join negotiation so server preference order stays testable, but this crate
//! ships no brotli/zstd codec and a negotiated-but-unencodable algorithm
//! leaves the body alone (with `Vary` stamped).

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};

/// Algorithms named by `Accept-Encoding` (`CompressionAlgorithm` in TS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompressionAlgorithm {
    Gzip,
    Deflate,
    Br,
    Zstd,
}

impl CompressionAlgorithm {
    /// Wire name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
            Self::Deflate => "deflate",
            Self::Br => "br",
            Self::Zstd => "zstd",
        }
    }

    /// Parse a wire name (already lowercased by the caller).
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "gzip" => Self::Gzip,
            "deflate" => Self::Deflate,
            "br" => Self::Br,
            "zstd" => Self::Zstd,
            _ => return None,
        })
    }
}

/// This crate's encoders: gzip + deflate (the web platform's pair in TS).
pub fn supported_algorithms() -> HashSet<CompressionAlgorithm> {
    HashSet::from([CompressionAlgorithm::Gzip, CompressionAlgorithm::Deflate])
}

/// Server preference order. Mirrors the TS default `["br", "gzip", "deflate"]`
/// (`zstd` stays opt-in there too).
pub fn default_algorithms() -> Vec<CompressionAlgorithm> {
    vec![
        CompressionAlgorithm::Br,
        CompressionAlgorithm::Gzip,
        CompressionAlgorithm::Deflate,
    ]
}

/// Options for [`apply_compression`] (the `compression(options)` argument).
pub struct CompressionOptions {
    /// Server preference order; negotiation picks the first accepted entry
    /// the platform supports.
    pub algorithms: Vec<CompressionAlgorithm>,
    /// Minimum body size when the length is known; unknown lengths always
    /// compress. Defaults to `1024`.
    pub min_size: usize,
    /// Replaces the default content-type predicate.
    pub compressible: fn(&str) -> bool,
}

impl Default for CompressionOptions {
    fn default() -> Self {
        Self {
            algorithms: default_algorithms(),
            min_size: 1024,
            compressible: default_compressible,
        }
    }
}

/// Default content-type predicate: `text/*`, JSON/XML/JS/WASM, SVG, and
/// `+json`/`+xml` suffixes (TS `defaultCompressible`).
pub fn default_compressible(content_type: &str) -> bool {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mime.starts_with("text/") {
        return true;
    }
    match mime.as_str() {
        "application/json"
        | "application/javascript"
        | "application/xml"
        | "image/svg+xml"
        | "application/wasm" => {
            return true;
        }
        _ => {}
    }
    mime.ends_with("+json") || mime.ends_with("+xml")
}

/// Parse `Accept-Encoding` into coding → qvalue. `None` when the header is
/// absent, empty, or any member is malformed (TS returns `undefined`, and the
/// caller treats the header as absent).
pub fn parse_accept_encoding(header: &str) -> Option<HashMap<String, f64>> {
    if header.trim().is_empty() {
        return None;
    }
    let mut accepted = HashMap::new();
    for part in header.split(',') {
        // TS lowercases and collapses `[ \t]*;[ \t]*` to `;`.
        let member: Vec<&str> = part.split(';').map(str::trim).collect();
        let coding = member[0].to_ascii_lowercase();
        if !is_token(&coding) {
            return None;
        }
        let quality = if member.len() == 1 {
            1.0
        } else if member.len() == 2 && member[1].starts_with("q=") {
            parse_q(&member[1][2..])?
        } else {
            return None;
        };
        accepted.insert(coding, quality);
    }
    Some(accepted)
}

fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// `q=` grammar: `0` with up to 3 decimals, or `1` with up to 3 zeros.
fn parse_q(text: &str) -> Option<f64> {
    let (head, tail) = match text.split_once('.') {
        Some(pair) => pair,
        None => (text, ""),
    };
    match head {
        "0" => {
            if tail.len() > 3 || !tail.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
        }
        "1" => {
            if tail.len() > 3 || !tail.bytes().all(|b| b == b'0') {
                return None;
            }
        }
        _ => return None,
    }
    text.parse().ok()
}

/// Pick the first preferred algorithm the platform supports with q > 0.
/// Explicit codings beat `*`; q-values only gate, server order ranks.
pub fn negotiate(
    header: Option<&str>,
    preferred: &[CompressionAlgorithm],
    supported: &HashSet<CompressionAlgorithm>,
) -> Option<CompressionAlgorithm> {
    let accepted = parse_accept_encoding(header?)?;
    for algorithm in preferred {
        if !supported.contains(algorithm) {
            continue;
        }
        let quality = accepted
            .get(algorithm.name())
            .copied()
            .or_else(|| accepted.get("*").copied());
        if quality.is_some_and(|q| q > 0.0) {
            return Some(*algorithm);
        }
    }
    None
}

/// Append a `Vary` dimension unless `*` or the dimension (case-insensitive)
/// is already listed; the existing value keeps its spelling (TS `varyWith`).
pub fn vary_with(existing: Option<&str>, dimension: &str) -> String {
    match existing {
        None => dimension.to_string(),
        Some(vary) => {
            let members: Vec<&str> = vary.split(',').map(str::trim).collect();
            let wanted = dimension.to_ascii_lowercase();
            if members
                .iter()
                .any(|m| *m == "*" || m.to_ascii_lowercase() == wanted)
            {
                vary.to_string()
            } else {
                format!("{vary}, {dimension}")
            }
        }
    }
}

/// Strict content-length parse: all digits and a safe integer, else `None`
/// (TS `parseContentLength`).
pub fn parse_content_length(value: &str) -> Option<usize> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let parsed: u64 = value.parse().ok()?;
    if parsed > 9_007_199_254_740_991 {
        return None;
    }
    usize::try_from(parsed).ok()
}

/// `Cache-Control: no-transform` guard (TS `noTransformRegex`).
fn has_no_transform(cache_control: &str) -> bool {
    let lower = cache_control.to_ascii_lowercase();
    let needle = "no-transform";
    let bytes = lower.as_bytes();
    let mut start = 0;
    while let Some(offset) = lower[start..].find(needle) {
        let from = start + offset;
        let to = from + needle.len();
        let before_ok = from == 0 || matches!(bytes[from - 1], b' ' | b'\t' | b'\n' | b'\r' | b',');
        let after_ok =
            to == bytes.len() || matches!(bytes[to], b' ' | b'\t' | b'\n' | b'\r' | b',' | b';');
        if before_ok && after_ok {
            return true;
        }
        start = from + 1;
    }
    false
}

/// Gzip-encode (level 6, the TS default).
pub fn compress_gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("gzip encode");
    encoder.finish().expect("gzip finish")
}

/// Gzip-decode; `None` on corrupt input.
pub fn decompress_gzip(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).ok()?;
    Some(out)
}

/// Deflate-encode (level 6, the TS default).
pub fn compress_deflate(bytes: &[u8]) -> Vec<u8> {
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("deflate encode");
    encoder.finish().expect("deflate finish")
}

/// Deflate-decode; `None` on corrupt input.
pub fn decompress_deflate(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = flate2::read::DeflateDecoder::new(bytes);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).ok()?;
    Some(out)
}

/// A response body for compression: known bytes, an untransformable raw
/// (TS `Raw` with a null body), or empty (`Empty` skips silently).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompBody {
    Bytes(Vec<u8>),
    RawNull,
    Empty,
}

/// Minimal response shape the middleware transforms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: CompBody,
}

impl CompResponse {
    /// Body bytes, or empty for non-byte bodies.
    pub fn bytes(&self) -> Vec<u8> {
        match &self.body {
            CompBody::Bytes(bytes) => bytes.clone(),
            CompBody::RawNull | CompBody::Empty => Vec::new(),
        }
    }

    /// First header value for `name` (case-insensitive), if present.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Set a header, replacing any existing entry case-insensitively.
    pub fn set_header(&mut self, name: &str, value: &str) {
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.headers.push((name.to_owned(), value.to_owned()));
    }

    /// Remove a header case-insensitively; true when something was removed.
    pub fn remove_header(&mut self, name: &str) -> bool {
        let before = self.headers.len();
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.headers.len() != before
    }
}

/// Run the compression middleware over one response: status and encoding
/// guards, content-type and size eligibility, negotiation, then gzip/deflate.
/// `Vary: Accept-Encoding` stamps every eligible-but-skipped response.
/// HEAD answers carry GET headers with no body.
pub fn apply_compression(
    options: &CompressionOptions,
    supported: &HashSet<CompressionAlgorithm>,
    method: &str,
    accept_encoding: Option<&str>,
    response: CompResponse,
) -> CompResponse {
    let mut res = response;
    if res.status < 200 || res.status == 204 || res.status == 206 || res.status == 304 {
        return head_body(method, res);
    }
    if let Some(encoding) = res.header("content-encoding").map(str::to_owned) {
        if encoding.trim().eq_ignore_ascii_case("identity") {
            res.remove_header("content-encoding");
        }
        return head_body(method, res);
    }
    if !matches!(res.body, CompBody::Bytes(_)) {
        return head_body(method, res);
    }
    if res.header("cache-control").is_some_and(has_no_transform) {
        return head_body(method, res);
    }
    let content_type = res.header("content-type").map(str::to_owned);
    let Some(content_type) = content_type else {
        return head_body(method, res);
    };
    if !(options.compressible)(&content_type) {
        return head_body(method, res);
    }
    let Some(algorithm) = negotiate(accept_encoding, &options.algorithms, supported) else {
        stamp_vary(&mut res);
        return head_body(method, res);
    };
    // Known length wins; a stream-style body would consult the header here.
    // Malformed lengths parse to `None` (unknown) and always compress.
    let known = match &res.body {
        CompBody::Bytes(bytes) => Some(bytes.len()),
        _ => res.header("content-length").and_then(parse_content_length),
    };
    if known.is_some_and(|len| len < options.min_size) {
        stamp_vary(&mut res);
        return head_body(method, res);
    }
    let plain = match &res.body {
        CompBody::Bytes(bytes) => bytes.clone(),
        _ => return head_body(method, res),
    };
    let compressed = match algorithm {
        CompressionAlgorithm::Gzip => compress_gzip(&plain),
        CompressionAlgorithm::Deflate => compress_deflate(&plain),
        // Negotiated but unencodable here: leave the body, keep Vary.
        CompressionAlgorithm::Br | CompressionAlgorithm::Zstd => {
            stamp_vary(&mut res);
            return head_body(method, res);
        }
    };
    res.body = CompBody::Bytes(compressed);
    res.remove_header("content-length");
    res.set_header("content-encoding", algorithm.name());
    let vary = vary_with(res.header("vary"), "Accept-Encoding");
    res.set_header("vary", &vary);
    if let Some(etag) = res.header("etag").map(str::to_owned) {
        if !etag.starts_with("W/") {
            res.set_header("etag", &format!("W/{etag}"));
        }
    }
    head_body(method, res)
}

fn stamp_vary(res: &mut CompResponse) {
    let vary = vary_with(res.header("vary"), "Accept-Encoding");
    res.set_header("vary", &vary);
}

fn head_body(method: &str, mut res: CompResponse) -> CompResponse {
    if method.eq_ignore_ascii_case("HEAD") {
        res.body = CompBody::Bytes(Vec::new());
    }
    res
}
