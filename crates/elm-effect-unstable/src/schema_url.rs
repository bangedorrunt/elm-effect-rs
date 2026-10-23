//! `Schema.URLFromString`: a URL on the wire is its text, and the value is
//! [`Url`](crate::http::Url).
//!
//! The pin's codec is a `URL` instance both ways; this port's `Url` is the
//! unstable crate's parsed absolute URL, so the schema belongs here beside it
//! rather than in `elm-effect-schema`, which cannot name the type.
//!
//! Rendering normalizes the way the pin's `URL.toString()` does: an empty path
//! prints as `/`, which is why `https://effect.website` encodes back as
//! `https://effect.website/` (`Schema.test.ts:5991`).

use elm_effect_schema::{Schema, SchemaError, Unknown};

use crate::http::Url;

/// The pin's refusal for text that is not an absolute URL.
fn invalid_url() -> SchemaError {
    SchemaError::TypeMismatch {
        path: "$".to_string(),
        expected: "a valid URL string",
        got: "string",
    }
}

/// `Schema.URLFromString`.
pub fn url_from_string() -> Schema<Url> {
    Schema::convert(
        Schema::<String>::string(),
        |text: String| Url::parse(&text).ok_or_else(invalid_url),
        |url: &Url| url.render(),
    )
}

/// The wire text of a URL value, for a caller that holds the `Unknown` side.
pub fn url_wire(url: &Url) -> Unknown {
    Unknown::Text(url.render())
}

/// A URL is its text on the config side, which is what lets `Config.schema`
/// take `Schema.URL` (`Config.test.ts:1965`). The impl lives here rather than in
/// `config.rs` because `Url` is this crate's type: `elm-effect` cannot name it.
#[cfg(feature = "schema")]
impl elm_effect::config::IntoConfigValue for Url {
    fn into_config_value(self) -> elm_effect::config::Value {
        elm_effect::config::Value::Str(self.render())
    }
}
