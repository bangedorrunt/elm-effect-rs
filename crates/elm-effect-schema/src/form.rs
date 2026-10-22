//! `Schema.fromURLSearchParams` and `Schema.fromFormData`.
//!
//! Both take key/value pairs, so the wire here is the encoded text
//! (`a=a&b[c]=bc&e=e0&e=e1`): that is what a query string is, and what a form
//! body posts with `application/x-www-form-urlencoded`. `Unknown` has no
//! `FormData` or `URLSearchParams` object to hold, and the pin's two schemas
//! differ only in the object they accept, never in the pairs inside it.
//!
//! Decoding builds the tree from bracket paths (`SchemaGetter.makeTreeRecord`),
//! so `b[c]` nests, `e[0]` indexes and a repeated `e` collects. Encoding walks
//! the value back out (`collectBracketPathEntries`): an array of leaves repeats
//! its key, an array of objects keeps indexes.
//!
//! ponytail: no percent-decoding or percent-encoding. The rows carry plain
//! text; `%20`, `+`, `&` and `=` inside a value would need a codec pass over
//! each key and value, which is the upgrade path if a row ever shows one.

use crate::transform::{make_tree_record, parse_path};
use crate::{Schema, SchemaError, Unknown};

impl<T> Schema<T> {
    /// `Schema.fromURLSearchParams`: a query string in, `inner`'s value out.
    pub fn from_url_search_params(inner: Schema<T>) -> Self
    where
        T: Send + Sync + 'static,
    {
        url_encoded(inner)
    }

    /// `Schema.fromFormData`: the same pairs and the same codec, named for the
    /// body a form posts.
    pub fn from_form_data(inner: Schema<T>) -> Self
    where
        T: Send + Sync + 'static,
    {
        url_encoded(inner)
    }
}

fn url_encoded<T: Send + Sync + 'static>(inner: Schema<T>) -> Schema<T> {
    let inner = std::sync::Arc::new(inner);
    let decode_inner = inner.clone();
    let encode_inner = inner.clone();
    let kind = inner.repr_kind();
    Schema::new(
        move |unknown| {
            let Unknown::Text(text) = unknown else {
                return Err(SchemaError::TypeMismatch {
                    path: "$".to_string(),
                    expected: "form data text",
                    got: unknown.kind(),
                });
            };
            let entries = text
                .split('&')
                .filter(|pair| !pair.is_empty())
                .map(|pair| {
                    let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
                    (parse_path(key), Unknown::Text(value.to_string()))
                })
                .collect();
            decode_inner.decode(make_tree_record(entries))
        },
        move |value| {
            let encoded = encode_inner.try_encode(value)?;
            Ok(Unknown::Text(print_pairs(&encoded)))
        },
        kind,
    )
}

/// The wire spelling of a tree: `k=v` pairs joined with `&`, one pair per leaf.
fn print_pairs(value: &Unknown) -> String {
    let mut out = Vec::new();
    collect_entries(value, String::new(), &mut out);
    out.join("&")
}

fn collect_entries(value: &Unknown, path: String, out: &mut Vec<String>) {
    match value {
        Unknown::Object(map) if !map.is_empty() => {
            for (key, inner) in map {
                let next = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}[{key}]")
                };
                collect_entries(inner, next, out);
            }
        }
        // A leaf array repeats its own key, which is how the pin writes one
        // (`e=e0&e=e1`); an array of containers keeps indexed paths so the
        // structure survives.
        Unknown::Array(items) if !items.is_empty() => {
            if items.iter().all(is_leaf) {
                for item in items {
                    out.push(format!("{path}={}", leaf_text(item)));
                }
            } else {
                for (index, item) in items.iter().enumerate() {
                    collect_entries(item, format!("{path}[{index}]"), out);
                }
            }
        }
        // An empty object or array writes nothing, the pin's own rule for a
        // flattening that has no leaf to write.
        Unknown::Object(_) | Unknown::Array(_) => {}
        leaf => out.push(format!("{path}={}", leaf_text(leaf))),
    }
}

fn is_leaf(value: &Unknown) -> bool {
    !matches!(value, Unknown::Object(_) | Unknown::Array(_))
}

fn leaf_text(value: &Unknown) -> String {
    match value {
        Unknown::Text(text) => text.clone(),
        Unknown::Number(number) => crate::js_number_text(*number),
        Unknown::Bool(flag) => flag.to_string(),
        Unknown::Null => String::new(),
        // An undefined value writes nothing, the same as a null.
        Unknown::Undefined => String::new(),
        // Containers never reach here: the walk handles them one level up.
        Unknown::Object(_) | Unknown::Array(_) => String::new(),
    }
}
