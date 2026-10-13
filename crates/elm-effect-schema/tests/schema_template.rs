//! Slice 3: template-literal anchors with greedy backtracking.
//! Ports Schema.test.ts anchor rows by exact call.

use elm_effect_schema::{Schema, SchemaError, TemplateParser, TemplatePart, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

/// A literal part, which the pin writes as the bare string in its part list.
fn anchor(value: &str) -> TemplatePart {
    TemplatePart::Anchor(value.to_string())
}

fn matched() -> String {
    "expected a string matching template literal parts at $, got no match".to_string()
}

fn decode_ok(schema: &Schema<String>, input: &str) -> Option<String> {
    schema.decode(text(input)).ok()
}

#[test]
fn greedy_segmentation_at_repeated_anchors() {
    let schema = Schema::template(vec![
        TemplatePart::AnyString,
        TemplatePart::Anchor(":".to_string()),
        TemplatePart::AnyString,
    ]);
    assert_eq!(
        schema.decode(text("a:b:c")),
        Ok(vec![text("a:b"), text(":"), text("c")])
    );
}

#[test]
fn backtracks_from_an_invalid_literal_anchor() {
    let schema = Schema::template(vec![
        TemplatePart::AnyString,
        TemplatePart::Anchor(":".to_string()),
        TemplatePart::NonEmptyString,
        TemplatePart::Anchor("x".to_string()),
    ]);
    assert_eq!(
        schema.decode(text("a:b:x")),
        Ok(vec![text("a"), text(":"), text("b:"), text("x")])
    );
}

#[test]
fn backtracks_across_an_empty_literal_anchor() {
    let schema = Schema::template(vec![
        TemplatePart::AnyString,
        TemplatePart::Anchor(String::new()),
        TemplatePart::NonEmptyString,
    ]);
    assert_eq!(
        schema.decode(text("a")),
        Ok(vec![text(""), text(""), text("a")])
    );
}

#[test]
fn adjacent_spans_split_greedily() {
    let schema = Schema::template(vec![
        TemplatePart::NonEmptyString,
        TemplatePart::AnyString,
    ]);
    assert_eq!(schema.decode(text("a")), Ok(vec![text("a"), text("")]));
}

#[test]
fn single_anchor_matches_whole() {
    let schema = Schema::template(vec![TemplatePart::Anchor("a".to_string())]);
    assert_eq!(schema.decode(text("a")), Ok(vec![text("a")]));
    assert!(schema.decode(text("b")).is_err());
}

#[test]
fn anchors_with_spaces_roundtrip() {
    let schema = Schema::template(vec![
        TemplatePart::Anchor("a".to_string()),
        TemplatePart::Anchor(" ".to_string()),
        TemplatePart::Anchor("b".to_string()),
    ]);
    let captures = vec![text("a"), text(" "), text("b")];
    assert_eq!(schema.decode(text("a b")), Ok(captures.clone()));
    assert_eq!(schema.encode(&captures), text("a b"));
}

#[test]
fn int_span_captures_integer_prefix() {
    let schema = Schema::template(vec![
        TemplatePart::Int,
        TemplatePart::Anchor("a".to_string()),
    ]);
    assert_eq!(
        schema.decode(text("1a")),
        Ok(vec![Unknown::Number(1.0), text("a")])
    );
    assert!(schema.decode(text("a")).is_err());
    assert_eq!(schema.encode(&vec![Unknown::Number(1.0), text("a")]), text("1a"));
}

// Schema.test.ts TemplateLiteral rows. The pin's `TemplateLiteral(parts)` is a
// `Schema<string>`: it admits text matching every part and hands the same text
// back, so these assert the string, not a capture list. Its sibling
// `TemplateLiteralParser` is the capture form and lives in `Schema::template`.

#[test]
fn template_literal_exposes_its_parts() {
    // Schema.test.ts:6229 — the constructed schema exposes the parts it was
    // built from, in order.
    let parts = vec![anchor("a"), TemplatePart::AnyString];
    let parser = TemplateParser::new(parts.clone());
    assert_eq!(parser.parts(), parts.as_slice());
}

#[test]
fn template_literal_non_empty_string_part() {
    // Schema.test.ts:6235 — a non-empty string part accepts its own minimum.
    let schema = Schema::template_literal(vec![
        TemplatePart::NonEmptyString,
        TemplatePart::AnyString,
    ]);
    assert_eq!(decode_ok(&schema, "a"), Some("a".to_string()));
}

#[test]
fn template_literal_single_anchor() {
    // Schema.test.ts:6260 — a lone literal part admits only itself, and the two
    // refusals are different: the wrong type, and a string that is not the part.
    let schema = Schema::template_literal(vec![anchor("a")]);
    assert_eq!(decode_ok(&schema, "a"), Some("a".to_string()));
    assert_eq!(schema.decode(text("ab")).unwrap_err().to_string(), matched());
    assert_eq!(schema.decode(text("")).unwrap_err().to_string(), matched());
    assert_eq!(
        schema.decode(Unknown::Null).unwrap_err().to_string(),
        "expected string at $, got null"
    );
}

#[test]
fn template_literal_three_anchors() {
    // Schema.test.ts:6277 — adjacent literal parts are one exact string, so a
    // second space is a different string.
    let schema = Schema::template_literal(vec![anchor("a"), anchor(" "), anchor("b")]);
    assert_eq!(decode_ok(&schema, "a b"), Some("a b".to_string()));
    assert_eq!(schema.decode(text("a  b")).unwrap_err().to_string(), matched());
}

#[test]
fn template_literal_bracketed_string() {
    // Schema.test.ts:6290 — literals around a string part must both be present.
    let schema = Schema::template_literal(vec![anchor("["), TemplatePart::AnyString, anchor("]")]);
    assert_eq!(decode_ok(&schema, "[a]"), Some("[a]".to_string()));
    assert_eq!(schema.decode(text("a")).unwrap_err().to_string(), matched());
}

#[test]
fn template_literal_anchor_then_string() {
    // Schema.test.ts:6303 — the string part after the literal may be empty, so
    // the literal alone matches while a non-string input still does not.
    let schema = Schema::template_literal(vec![anchor("a"), TemplatePart::AnyString]);
    assert_eq!(decode_ok(&schema, "a"), Some("a".to_string()));
    assert_eq!(decode_ok(&schema, "ab"), Some("ab".to_string()));
    assert_eq!(schema.decode(text("")).unwrap_err().to_string(), matched());
    assert_eq!(
        schema.decode(Unknown::Null).unwrap_err().to_string(),
        "expected string at $, got null"
    );
}

#[test]
fn template_literal_number_part() {
    // Schema.test.ts:6321 — a Number part reads a sign, a fraction and either
    // exponent case, and refuses digits that do not form one.
    let schema = Schema::template_literal(vec![anchor("a"), TemplatePart::Number]);
    for accepted in [
        "a1",
        "a+1",
        "a1.2",
        "a-1.401298464324817e-45",
        "a1.401298464324817e-45",
        "a+1.401298464324817e-45",
        "a-1.401298464324817e+45",
        "a1.401298464324817E+45",
        "a+1.401298464324817E+45",
    ] {
        assert_eq!(decode_ok(&schema, accepted), Some(accepted.to_string()), "{accepted}");
    }
    for refused in ["", "aa"] {
        assert_eq!(schema.decode(text(refused)).unwrap_err().to_string(), matched(), "{refused}");
    }
    assert_eq!(
        schema.decode(Unknown::Null).unwrap_err().to_string(),
        "expected string at $, got null"
    );
}

#[test]
fn template_literal_digits_part() {
    // Schema.test.ts:6358 — a BigInt part is an optional minus and digits: no
    // plus sign and no fraction.
    let schema = Schema::template_literal(vec![anchor("a"), TemplatePart::Digits]);
    for accepted in ["a0", "a1", "a-1"] {
        assert_eq!(decode_ok(&schema, accepted), Some(accepted.to_string()), "{accepted}");
    }
    for refused in ["", "aa", "a1.2", "a+1"] {
        assert_eq!(schema.decode(text(refused)).unwrap_err().to_string(), matched(), "{refused}");
    }
}

#[test]
fn template_literal_single_string_part() {
    // Schema.test.ts:6389 — one string part accepts every string, whitespace and
    // the empty string included.
    let schema = Schema::template_literal(vec![TemplatePart::AnyString]);
    for accepted in ["a", "ab", "", "\n", "\r", "\r\n", "\t"] {
        assert_eq!(decode_ok(&schema, accepted), Some(accepted.to_string()), "{accepted:?}");
    }
}

#[test]
fn template_literal_newline_then_string() {
    // Schema.test.ts:6403 — a newline is an ordinary literal part.
    let schema = Schema::template_literal(vec![anchor("\n"), TemplatePart::AnyString]);
    assert_eq!(decode_ok(&schema, "\n"), Some("\n".to_string()));
    assert_eq!(decode_ok(&schema, "\na"), Some("\na".to_string()));
    assert_eq!(schema.decode(text("a")).unwrap_err().to_string(), matched());
}

#[test]
fn template_literal_multiline_anchor_then_string() {
    // Schema.test.ts:6416 — a literal part may itself span lines and end in a
    // space, which the string part then continues.
    let schema = Schema::template_literal(vec![anchor("a\nb "), TemplatePart::AnyString]);
    assert_eq!(decode_ok(&schema, "a\nb "), Some("a\nb ".to_string()));
    assert_eq!(decode_ok(&schema, "a\nb c"), Some("a\nb c".to_string()));
}

#[test]
fn template_literal_string_between_anchors() {
    // Schema.test.ts:6425 — a string part between two literals backtracks to
    // the split that leaves the trailing literal room, and encoding is the same
    // string back.
    let schema = Schema::template_literal(vec![anchor("a"), TemplatePart::AnyString, anchor("b")]);
    for accepted in ["ab", "acb", "abb"] {
        assert_eq!(decode_ok(&schema, accepted), Some(accepted.to_string()), "{accepted}");
    }
    for refused in ["", "a", "b"] {
        assert_eq!(schema.decode(text(refused)).unwrap_err().to_string(), matched(), "{refused}");
    }
    assert_eq!(schema.encode(&"acb".to_string()), text("acb"));
}

#[test]
fn template_literal_two_string_parts_around_an_anchor() {
    // Schema.test.ts:6450 — two string parts either side of a literal split
    // greedily and then give characters back.
    let schema = Schema::template_literal(vec![
        anchor("a"),
        TemplatePart::AnyString,
        anchor("b"),
        TemplatePart::AnyString,
    ]);
    for accepted in ["ab", "acb", "acbd"] {
        assert_eq!(decode_ok(&schema, accepted), Some(accepted.to_string()), "{accepted}");
    }
    for refused in ["a", "b"] {
        assert_eq!(schema.decode(text(refused)).unwrap_err().to_string(), matched(), "{refused}");
    }
}

#[test]
fn template_literal_union_of_literals_part() {
    // Schema.test.ts:6468 — a Literals part is one alternative per value, so
    // only those values (here each followed by `_id`) are admitted.
    let schema = Schema::template_literal(vec![
        TemplatePart::OneOf(vec![
            vec![anchor("welcome_email")],
            vec![anchor("email_heading")],
            vec![anchor("footer_title")],
            vec![anchor("footer_sendoff")],
        ]),
        anchor("_id"),
    ]);
    for accepted in [
        "welcome_email_id",
        "email_heading_id",
        "footer_title_id",
        "footer_sendoff_id",
    ] {
        assert_eq!(decode_ok(&schema, accepted), Some(accepted.to_string()), "{accepted}");
    }
    assert_eq!(schema.decode(text("_id")).unwrap_err().to_string(), matched());
}

#[test]
fn template_literal_literal_zero_part() {
    // Schema.test.ts:6486 — the number literal `0` in a part list is the text
    // `0`, not the Number part.
    let schema = Schema::template_literal(vec![TemplatePart::AnyString, anchor("0")]);
    assert_eq!(decode_ok(&schema, "a0"), Some("a0".to_string()));
    assert_eq!(schema.decode(text("a")).unwrap_err().to_string(), matched());
}

#[test]
fn template_literal_literal_bigint_part() {
    // Schema.test.ts:6498 — the bigint literal `1n` is the text `1`.
    let schema = Schema::template_literal(vec![TemplatePart::AnyString, anchor("1")]);
    assert_eq!(decode_ok(&schema, "a1"), Some("a1".to_string()));
    assert_eq!(schema.decode(text("a")).unwrap_err().to_string(), matched());
}

#[test]
fn template_literal_literals_part() {
    // Schema.test.ts:6510 — `("a" | 0)` accepts either alternative, and the
    // leading string part still has to leave the alternative room.
    let schema = Schema::template_literal(vec![
        TemplatePart::AnyString,
        TemplatePart::OneOf(vec![vec![anchor("a")], vec![anchor("0")]]),
    ]);
    assert_eq!(decode_ok(&schema, "a0"), Some("a0".to_string()));
    assert_eq!(decode_ok(&schema, "aa"), Some("aa".to_string()));
    assert_eq!(schema.decode(text("b")).unwrap_err().to_string(), matched());
}

#[test]
fn template_literal_union_parts() {
    // Schema.test.ts:6523 — `(string | 1) + (number | true)` takes either
    // alternative of each part, which is what makes `atrue`, `-2` and `10.1`
    // three different splits of the same schema.
    let schema = Schema::template_literal(vec![
        TemplatePart::OneOf(vec![vec![TemplatePart::AnyString], vec![anchor("1")]]),
        TemplatePart::OneOf(vec![vec![TemplatePart::Number], vec![anchor("true")]]),
    ]);
    for accepted in ["atrue", "-2", "10.1"] {
        assert_eq!(decode_ok(&schema, accepted), Some(accepted.to_string()), "{accepted}");
    }
    assert_eq!(schema.decode(text("")).unwrap_err().to_string(), matched());
}

#[test]
fn template_literal_nested_template_part() {
    // Schema.test.ts:6540 — a union alternative may be a whole nested template,
    // so `c${...}d` accepts both `ced` and the nested `a...b` shape.
    let schema = Schema::template_literal(vec![
        anchor("c"),
        TemplatePart::OneOf(vec![
            vec![anchor("a"), TemplatePart::AnyString, anchor("b")],
            vec![anchor("e")],
        ]),
        anchor("d"),
    ]);
    for accepted in ["ced", "cabd", "casbd", "ca  bd"] {
        assert_eq!(decode_ok(&schema, accepted), Some(accepted.to_string()), "{accepted}");
    }
    assert_eq!(schema.decode(text("")).unwrap_err().to_string(), matched());
}

#[test]
fn template_literal_nested_literals_part() {
    // Schema.test.ts:6557 — `<` + a nested template of a literal and a literals
    // part + `>` admits exactly its two values.
    let schema = Schema::template_literal(vec![
        anchor("<"),
        TemplatePart::OneOf(vec![vec![
            anchor("h"),
            TemplatePart::OneOf(vec![vec![anchor("1")], vec![anchor("2")]]),
        ]]),
        anchor(">"),
    ]);
    assert_eq!(decode_ok(&schema, "<h1>"), Some("<h1>".to_string()));
    assert_eq!(decode_ok(&schema, "<h2>"), Some("<h2>".to_string()));
    assert_eq!(schema.decode(text("<h3>")).unwrap_err().to_string(), matched());
}

#[test]
fn template_literal_anchor_then_non_empty_string() {
    // Schema.test.ts:6570 — `"a" + check` is a literal followed by a
    // non-empty string part, so the literal alone is not enough.
    let schema = Schema::template_literal(vec![anchor("a"), TemplatePart::NonEmptyString]);
    assert_eq!(decode_ok(&schema, "ab"), Some("ab".to_string()));
    assert_eq!(schema.decode(text("")).unwrap_err().to_string(), matched());
    assert_eq!(schema.decode(text("a")).unwrap_err().to_string(), matched());
    assert_eq!(
        schema.decode(Unknown::Null).unwrap_err().to_string(),
        "expected string at $, got null"
    );
    // The mismatch kind is the template one, not the type one.
    assert!(matches!(
        schema.decode(text("a")).unwrap_err(),
        SchemaError::TypeMismatch { expected, .. } if expected.contains("template")
    ));
}
