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
    let schema = Schema::template(vec![TemplatePart::NonEmptyString, TemplatePart::AnyString]);
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
    assert_eq!(
        schema.encode(&vec![Unknown::Number(1.0), text("a")]),
        text("1a")
    );
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
    let schema =
        Schema::template_literal(vec![TemplatePart::NonEmptyString, TemplatePart::AnyString]);
    assert_eq!(decode_ok(&schema, "a"), Some("a".to_string()));
}

#[test]
fn template_literal_single_anchor() {
    // Schema.test.ts:6260 — a lone literal part admits only itself, and the two
    // refusals are different: the wrong type, and a string that is not the part.
    let schema = Schema::template_literal(vec![anchor("a")]);
    assert_eq!(decode_ok(&schema, "a"), Some("a".to_string()));
    assert_eq!(
        schema.decode(text("ab")).unwrap_err().to_string(),
        matched()
    );
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
    assert_eq!(
        schema.decode(text("a  b")).unwrap_err().to_string(),
        matched()
    );
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
        assert_eq!(
            decode_ok(&schema, accepted),
            Some(accepted.to_string()),
            "{accepted}"
        );
    }
    for refused in ["", "aa"] {
        assert_eq!(
            schema.decode(text(refused)).unwrap_err().to_string(),
            matched(),
            "{refused}"
        );
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
        assert_eq!(
            decode_ok(&schema, accepted),
            Some(accepted.to_string()),
            "{accepted}"
        );
    }
    for refused in ["", "aa", "a1.2", "a+1"] {
        assert_eq!(
            schema.decode(text(refused)).unwrap_err().to_string(),
            matched(),
            "{refused}"
        );
    }
}

#[test]
fn template_literal_single_string_part() {
    // Schema.test.ts:6389 — one string part accepts every string, whitespace and
    // the empty string included.
    let schema = Schema::template_literal(vec![TemplatePart::AnyString]);
    for accepted in ["a", "ab", "", "\n", "\r", "\r\n", "\t"] {
        assert_eq!(
            decode_ok(&schema, accepted),
            Some(accepted.to_string()),
            "{accepted:?}"
        );
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
        assert_eq!(
            decode_ok(&schema, accepted),
            Some(accepted.to_string()),
            "{accepted}"
        );
    }
    for refused in ["", "a", "b"] {
        assert_eq!(
            schema.decode(text(refused)).unwrap_err().to_string(),
            matched(),
            "{refused}"
        );
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
        assert_eq!(
            decode_ok(&schema, accepted),
            Some(accepted.to_string()),
            "{accepted}"
        );
    }
    for refused in ["a", "b"] {
        assert_eq!(
            schema.decode(text(refused)).unwrap_err().to_string(),
            matched(),
            "{refused}"
        );
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
        assert_eq!(
            decode_ok(&schema, accepted),
            Some(accepted.to_string()),
            "{accepted}"
        );
    }
    assert_eq!(
        schema.decode(text("_id")).unwrap_err().to_string(),
        matched()
    );
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
        assert_eq!(
            decode_ok(&schema, accepted),
            Some(accepted.to_string()),
            "{accepted}"
        );
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
        assert_eq!(
            decode_ok(&schema, accepted),
            Some(accepted.to_string()),
            "{accepted}"
        );
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
    assert_eq!(
        schema.decode(text("<h3>")).unwrap_err().to_string(),
        matched()
    );
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

// Schema.test.ts TemplateLiteralParser rows (:6599-:6795). The parser form
// yields one capture per part, and a part's capture follows the part's own kind
// rather than the shape of the text: a literals part captures its value, a
// nested parser captures its list, and a nested template literal captures the
// text it matched. Encoding runs each capture back through its part, so the
// refusals name the part's index.

#[test]
fn template_parser_exposes_its_parts() {
    // :6599 — the parser exposes the parts it was built from, in order.
    let parts = vec![anchor("a"), TemplatePart::AnyString];
    let parser = TemplateParser::new(parts.clone());
    assert_eq!(parser.parts(), parts.as_slice());
}

#[test]
fn parser_int_then_anchor() {
    // :6667 `Int + "a"`: the integer part takes the integer prefix and the
    // anchor takes the rest. A fraction is not this part's text, and encoding a
    // fraction back through it is a refusal at the part's own index.
    let schema = Schema::template(vec![TemplatePart::Int, anchor("a")]);
    assert_eq!(
        schema.decode(text("1a")),
        Ok(vec![Unknown::Number(1.0), text("a")])
    );
    assert!(schema.decode(text("1.1a")).is_err());
    assert_eq!(
        schema.encode(&vec![Unknown::Number(1.0), text("a")]),
        text("1a")
    );
    let refusal = schema
        .try_encode(&vec![Unknown::Number(1.1), text("a")])
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("[0]"), "the part is named: {refusal}");
    assert!(
        refusal.contains("an integer"),
        "and what it wants: {refusal}"
    );
}

#[test]
fn parser_int_part_then_string_leaves_the_fraction() {
    // :6687 `Number.check(isInt()) + String`: an integer-checked number part
    // takes `1` and the string part takes what is left, fraction included.
    let schema = Schema::template(vec![TemplatePart::Int, TemplatePart::AnyString]);
    assert_eq!(
        schema.decode(text("1.2")),
        Ok(vec![Unknown::Number(1.0), text(".2")])
    );
}

#[test]
fn parser_number_then_anchor_then_non_empty_string() {
    // :6695 `NumberFromString + "a" + NonEmptyString`: the third part has to
    // take something, and encoding an empty one back is a refusal at index 2.
    let schema = Schema::template(vec![
        TemplatePart::Number,
        anchor("a"),
        TemplatePart::NonEmptyString,
    ]);
    assert_eq!(
        schema.decode(text("100ab")),
        Ok(vec![Unknown::Number(100.0), text("a"), text("b")])
    );
    assert_eq!(
        schema.decode(text("100ab23a")),
        Ok(vec![Unknown::Number(100.0), text("a"), text("b23a")])
    );
    assert!(schema.decode(text("-ab")).is_err(), "no number to read");
    assert_eq!(
        schema.encode(&vec![Unknown::Number(100.0), text("a"), text("b")]),
        text("100ab")
    );
    let refusal = schema
        .try_encode(&vec![Unknown::Number(100.0), text("a"), text("")])
        .unwrap_err()
        .to_string();
    assert!(
        refusal.contains("[2]") && refusal.contains("length of at least 1"),
        "the empty third part is named: {refusal}"
    );
}

#[test]
fn parser_anchor_then_literals_part() {
    // :6717 `"h" + (1 | 2 | 3)`: a literals part matches the text its values
    // spell and captures the value, so the capture is the number 1.
    let schema = Schema::template(vec![
        anchor("h"),
        TemplatePart::Literals(vec![
            Unknown::Number(1.0),
            Unknown::Number(2.0),
            Unknown::Number(3.0),
        ]),
    ]);
    assert_eq!(
        schema.decode(text("h1")),
        Ok(vec![text("h"), Unknown::Number(1.0)])
    );
    assert!(schema.decode(text("h4")).is_err());
    assert_eq!(
        schema.encode(&vec![text("h"), Unknown::Number(3.0)]),
        text("h3")
    );
    // A value outside the set is refused at the part.
    let refusal = schema
        .try_encode(&vec![text("h"), Unknown::Number(4.0)])
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("[1]"), "the part is named: {refusal}");
}

#[test]
fn parser_union_with_a_nested_parser_part() {
    // :6725 `"c" + (`a${string}b` | "e") + "d"`: the union alternative that is
    // a whole nested parser captures that parser's list, and the alternative
    // that is a literal captures its text.
    let nested = TemplateParser::new(vec![anchor("a"), TemplatePart::NonEmptyString, anchor("b")]);
    let schema = Schema::template(vec![
        anchor("c"),
        TemplatePart::OneOf(vec![
            vec![TemplatePart::NestedParser(Box::new(nested))],
            vec![anchor("e")],
        ]),
        anchor("d"),
    ]);
    assert_eq!(
        schema.decode(text("ca bd")),
        Ok(vec![
            text("c"),
            Unknown::Array(vec![text("a"), text(" "), text("b")]),
            text("d")
        ])
    );
    assert_eq!(
        schema.decode(text("ced")),
        Ok(vec![text("c"), text("e"), text("d")])
    );
    assert!(
        schema.decode(text("cabd")).is_err(),
        "the nested part needs a body"
    );
    assert!(
        schema.decode(text("ed")).is_err(),
        "the leading `c` is required"
    );
}

#[test]
fn parser_union_with_a_nested_checked_int_part() {
    // :6746 the same shape with `Finite.check(isInt())` inside: the nested
    // capture keeps the number, and the alternative is a real alternative, so
    // `ced` still takes the literal branch.
    let nested = TemplateParser::new(vec![anchor("a"), TemplatePart::Int, anchor("b")]);
    let schema = Schema::template(vec![
        anchor("c"),
        TemplatePart::OneOf(vec![
            vec![TemplatePart::NestedParser(Box::new(nested))],
            vec![anchor("e")],
        ]),
        anchor("d"),
    ]);
    assert_eq!(
        schema.decode(text("ced")),
        Ok(vec![text("c"), text("e"), text("d")])
    );
    assert_eq!(
        schema.decode(text("ca1bd")),
        Ok(vec![
            text("c"),
            Unknown::Array(vec![text("a"), Unknown::Number(1.0), text("b")]),
            text("d")
        ])
    );
    assert!(
        schema.decode(text("ca1.1bd")).is_err(),
        "the part is an integer"
    );
    assert!(schema.decode(text("ca-bd")).is_err(), "a sign needs digits");
}

#[test]
fn parser_nested_template_literal_part_captures_text() {
    // :6770 `"<" + `h${1 | 2}` + ">"`: the nested part is a `TemplateLiteral`,
    // a string schema, so it captures the text it matched rather than a list.
    let nested = vec![
        anchor("h"),
        TemplatePart::Literals(vec![Unknown::Number(1.0), Unknown::Number(2.0)]),
    ];
    let schema = Schema::template(vec![
        anchor("<"),
        TemplatePart::OneOf(vec![nested]),
        anchor(">"),
    ]);
    assert_eq!(
        schema.decode(text("<h1>")),
        Ok(vec![text("<"), text("h1"), text(">")])
    );
    assert_eq!(
        schema.decode(text("<h2>")),
        Ok(vec![text("<"), text("h2"), text(">")])
    );
    assert!(schema.decode(text("<h3>")).is_err());
}

#[test]
fn parser_nested_parser_part_captures_a_list() {
    // :6783 `"<" + readonly ["h", 1 | 2] + ">"`: the nested part is a
    // `TemplateLiteralParser`, a tuple schema, so it captures its own list.
    let nested = TemplateParser::new(vec![
        anchor("h"),
        TemplatePart::Literals(vec![Unknown::Number(1.0), Unknown::Number(2.0)]),
    ]);
    let schema = Schema::template(vec![
        anchor("<"),
        TemplatePart::NestedParser(Box::new(nested)),
        anchor(">"),
    ]);
    assert_eq!(
        schema.decode(text("<h1>")),
        Ok(vec![
            text("<"),
            Unknown::Array(vec![text("h"), Unknown::Number(1.0)]),
            text(">")
        ])
    );
    assert_eq!(
        schema.decode(text("<h2>")),
        Ok(vec![
            text("<"),
            Unknown::Array(vec![text("h"), Unknown::Number(2.0)]),
            text(">")
        ])
    );
    assert!(schema.decode(text("<h3>")).is_err());
    // The list is what encodes back, one capture per nested part.
    assert_eq!(
        schema.encode(&vec![
            text("<"),
            Unknown::Array(vec![text("h"), Unknown::Number(2.0)]),
            text(">")
        ]),
        text("<h2>")
    );
}

/// TemplateLiteral.test.ts :157 — greedy segmentation needs no tuple
/// round-trip: encode concatenates the captures (so a capture that already
/// holds a colon survives) and decode re-splits greedily.
#[test]
fn parser_keeps_greedy_segmentation_without_tuple_roundtrips() {
    let schema = Schema::template(vec![
        TemplatePart::AnyString,
        anchor(":"),
        TemplatePart::AnyString,
    ]);
    assert_eq!(
        schema.encode(&vec![text("a"), text(":"), text("b:c")]),
        text("a:b:c")
    );
    assert_eq!(
        schema.decode(text("a:b:c")),
        Ok(vec![text("a:b"), text(":"), text("c")])
    );
}
