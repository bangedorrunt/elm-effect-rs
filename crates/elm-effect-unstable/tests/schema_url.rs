//! `Schema.test.ts:5991` `URLFromString`: text in, a parsed URL as the value,
//! and the URL's own rendering on the way out.

use elm_effect_unstable::http::Url;
use elm_effect_unstable::url_from_string;
use elm_effect_schema::Unknown;

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

#[test]
fn a_url_string_decodes_to_a_url_and_renders_back() {
    let schema = url_from_string();

    assert_eq!(
        schema.decode(text("https://effect.website")).unwrap(),
        Url::parse("https://effect.website").expect("a valid URL")
    );

    let refused = schema
        .decode(text("123"))
        .expect_err("`123` is not an absolute URL")
        .to_string();
    assert!(refused.contains("a valid URL string"), "{refused}");

    let url = Url::parse("https://effect.website").expect("a valid URL");
    assert_eq!(
        schema.try_encode(&url).unwrap(),
        text("https://effect.website/"),
        "an empty path renders as `/`, which is the pin's URL.toString()"
    );
}
