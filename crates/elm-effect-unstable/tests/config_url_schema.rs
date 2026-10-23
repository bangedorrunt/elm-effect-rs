//! `Config.schema` over the URL codec: the two Config.test.ts rows that need a
//! declaration whose wire is a string, and a nested one at that.
//!
//! `Schema.URL` lives in this crate because `elm-effect-schema` cannot name
//! `Url`; the config bridge is in `elm-effect`, so the seam between them is the
//! `IntoConfigValue` impl beside the codec. Without it `Config.schema` cannot be
//! called with a URL at all, nested or not.

#![cfg(feature = "schema")]

use std::collections::BTreeMap;

use elm_effect::config::{Config, Value};
use elm_effect::config_provider::Provider;
use elm_effect_schema::{Field, Schema, SchemaError, Unknown};
use elm_effect_unstable::http::Url;
use elm_effect_unstable::schema_url::url_from_string;

fn provider(pairs: &[(&str, &str)]) -> Provider {
    Provider::from_pairs(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string())),
    )
}

fn invalid_url() -> SchemaError {
    SchemaError::TypeMismatch {
        path: "$".to_string(),
        expected: "a valid URL string",
        got: "string",
    }
}

/// The typed spelling of the pin's `Schema.Struct({ a: Schema.URL })`: this
/// port's struct constructors erase their fields to `Unknown`, so a struct
/// whose decoded value keeps the `Url` is a `convert` over the erased struct.
fn nested_url() -> Schema<BTreeMap<String, Url>> {
    Schema::convert(
        Schema::struct_shape(vec![Field::required(
            "a",
            Schema::as_unknown(url_from_string()),
        )]),
        |fields| {
            let text = match fields.get("a") {
                Some(Unknown::Text(text)) => text.clone(),
                other => {
                    return Err(SchemaError::TypeMismatch {
                        path: "$.a".to_string(),
                        expected: "a valid URL string",
                        got: other.map_or("missing", Unknown::kind),
                    });
                }
            };
            let url = Url::parse(&text).ok_or_else(|| invalid_url().at("$.a"))?;
            Ok([("a".to_string(), url)].into_iter().collect())
        },
        |urls: &BTreeMap<String, Url>| {
            urls.iter()
                .map(|(key, url)| (key.clone(), Unknown::Text(url.render())))
                .collect()
        },
    )
}

/// Config.test.ts:1965 `supports declarations with concrete StringTree
/// encodings`. `Schema.URL` is one: the wire is its text, so the bridge reads
/// the provider's string and the config value is the URL's rendering.
#[test]
fn a_url_declaration_carries_a_concrete_string_wire() {
    let config = Config::schema(&url_from_string(), Some("value".to_string()));
    assert_eq!(
        config.load(&provider(&[("value", "https://example.com")])),
        Ok(Value::Str("https://example.com/".to_string())),
        "the empty path prints as /, the way URL.toString() does"
    );
    assert!(
        config
            .load(&provider(&[("value", "not a url")]))
            .is_err_and(|errors| errors.len() == 1),
        "text the codec refuses is a validation failure"
    );
    assert!(
        config.load(&Provider::empty()).is_err(),
        "an absent node is missing input, not a parse"
    );
}

/// Config.test.ts:2772 `decodes nested URL values`. The declaration sits inside
/// a struct, so the bridge has to materialize the field's string wire and hand
/// the decoded URL back through the config value.
#[test]
fn nested_url_values_decode_through_the_bridge() {
    let config = Config::schema(&nested_url(), None);
    assert_eq!(
        config.load(&provider(&[("a", "https://example.com")])),
        Ok(Value::Map(vec![(
            "a".to_string(),
            Value::Str("https://example.com/".to_string())
        )])),
        "the nested declaration decodes through its own wire"
    );
    assert!(
        config.load(&provider(&[("a", "not a url")])).is_err(),
        "the nested codec's refusal is a validation failure"
    );
    assert!(
        config.load(&Provider::empty()).is_err(),
        "an absent field is missing input"
    );
}
