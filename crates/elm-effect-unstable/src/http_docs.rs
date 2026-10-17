//! Generated documentation routes (`HttpApiDocumentation`).
//!
//! Ports the doc layer half of `packages/effect/test/httpapi/HttpApiDocumentation.ts`:
//! deferred memoized doc cells (compute once, cache successes, retry defects),
//! HTML-escaped Scalar/Swagger pages, and CDN version encoding. The OpenAPI
//! spec itself is an opaque string here: spec generation belongs to the
//! schema-compiler epic, while every assertion these rows make (call counts,
//! statuses, escaping, roundtrips) holds on the document layer.

use std::sync::Mutex;

/// A deferred memoized document: the first request computes, successes are
/// cached (one transform even for concurrent first requests), and defects
/// are never cached, so the next request retries.
pub struct DocCell {
    cached: Mutex<Option<String>>,
    compute: Box<dyn Fn() -> Result<String, String> + Send + Sync>,
}

impl DocCell {
    /// A document computed by `compute` on first request.
    pub fn new(compute: impl Fn() -> Result<String, String> + Send + Sync + 'static) -> Self {
        Self { cached: Mutex::new(None), compute: Box::new(compute) }
    }

    /// The cached document, computing it when absent. Defects propagate
    /// without caching.
    pub fn get(&self) -> Result<String, String> {
        let mut cached = self.cached.lock().expect("doc cell");
        if let Some(document) = cached.as_ref() {
            return Ok(document.clone());
        }
        let document = (self.compute)()?;
        *cached = Some(document.clone());
        Ok(document)
    }
}

/// Escape text content (`&`, `<`, `>`); quotes stay literal outside attributes.
pub fn escape_html_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for part in text.split_inclusive(&['&', '<', '>']) {
        match part.strip_suffix(['&', '<', '>']) {
            Some(head) => {
                out.push_str(head);
                out.push_str(match part.as_bytes().last() {
                    Some(b'&') => "&amp;",
                    Some(b'<') => "&lt;",
                    _ => "&gt;",
                });
            }
            None => out.push_str(part),
        }
    }
    out
}

/// Escape an attribute value (`&`, `<`, `>`, `"`, `'`).
pub fn escape_html_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for part in value.split_inclusive(&['&', '<', '>', '"', '\'']) {
        match part.strip_suffix(['&', '<', '>', '"', '\'']) {
            Some(head) => {
                out.push_str(head);
                out.push_str(match part.as_bytes().last() {
                    Some(b'&') => "&amp;",
                    Some(b'<') => "&lt;",
                    Some(b'>') => "&gt;",
                    Some(b'"') => "&quot;",
                    _ => "&#39;",
                });
            }
            None => out.push_str(part),
        }
    }
    out
}

/// Percent-encode a URI component, mirroring `encodeURIComponent`: letters,
/// digits, and `-_.!~*'()` stay literal, every other byte becomes `%XX`.
pub fn encode_uri_component(value: &str) -> String {
    const UNRESERVED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()";
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if UNRESERVED.contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Escape embedded JSON so no script end-tag sequence survives: `</script`
/// becomes `<\/script`, which decodes back to the same JSON string.
pub fn escape_embedded_json(json: &str) -> String {
    json.replace("</script", "<\\/script")
}

/// A Scalar API-reference page: escaped title and description meta tags, the
/// versioned CDN script, and the embedded spec.
pub fn scalar_page(title: &str, description: Option<&str>, spec_json: &str, version: &str) -> String {
    let mut html = String::from("<!doctype html><html><head>");
    html.push_str(&format!("<title>{}</title>", escape_html_text(title)));
    if let Some(description) = description {
        let escaped = escape_html_attr(description);
        html.push_str(&format!("<meta name=\"description\" content=\"{escaped}\"/>"));
        html.push_str(&format!("<meta name=\"og:description\" content=\"{escaped}\"/>"));
    }
    html.push_str("</head><body>");
    html.push_str(&format!(
        "<script src=\"https://cdn.jsdelivr.net/npm/@scalar/api-reference@{}/dist/browser/standalone.min.js\"></script>",
        encode_uri_component(version)
    ));
    html.push_str(&format!(
        "<script id=\"scalar-data\" type=\"application/json\">{}</script>",
        escape_embedded_json(spec_json)
    ));
    html.push_str("</body></html>");
    html
}

/// A Swagger UI page: the embedded spec plus its loader script.
pub fn swagger_page(_description: Option<&str>, spec_json: &str) -> String {
    format!(
        "<!doctype html><html><body><script id=\"swagger-data\" type=\"application/json\">{}</script><script src=\"swagger-ui.js\"></script></body></html>",
        escape_embedded_json(spec_json)
    )
}
