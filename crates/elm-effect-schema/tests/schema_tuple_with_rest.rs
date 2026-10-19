//! `Schema.TupleWithRest`: a leading tuple, one repeatable rest schema, and the
//! fixed trailing positions the pin's rest list carries after that first entry.
//! Binds Schema.test.ts rows 4891, 4910, 4947 and 4979.
//!
//! The rest list is read the way the pin reads it: `[Boolean, String]` means
//! "zero or more booleans, then a final string". The trailing schemas therefore
//! line up from the *end* of the array, which is why the same index is a
//! boolean in one array and a string in a shorter one. A failure names the index
//! the split puts it at, so the short cases below report a different index than
//! a positional tuple would.

use elm_effect_schema::{Schema, Unknown};

fn text(value: &str) -> Unknown {
    Unknown::Text(value.to_string())
}

fn arr(items: Vec<Unknown>) -> Unknown {
    Unknown::Array(items)
}

/// The re-typed `FiniteFromString` a tuple position needs, exactly as the other
/// schema slices spell it.
fn finite() -> Schema<Unknown> {
    Schema::as_unknown(Schema::finite_from_string())
}

fn string() -> Schema<Unknown> {
    Schema::as_unknown(Schema::string())
}

fn boolean() -> Schema<Unknown> {
    Schema::as_unknown(Schema::boolean())
}

/// Run `build` and return the message it panicked with.
fn refusal(build: impl FnOnce()) -> String {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(build));
    std::panic::set_hook(hook);
    caught
        .err()
        .and_then(|payload| {
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
        })
        .expect("the tuple is refused at construction")
}

/// Schema.test.ts:4891 — an optional element cannot follow the rest element, in
/// either spelling of optional. Whatever follows the repeatable schema is a
/// fixed trailing position, so an absent one would leave the schemas after it
/// with no position to occupy.
#[test]
fn tuple_with_rest_refuses_an_optional_element_after_the_rest() {
    for optional in [
        Schema::optional_key_shape(string()),
        Schema::as_unknown(Schema::optional(Schema::string())),
    ] {
        assert_eq!(
            refusal(|| {
                Schema::tuple_with_rest(vec![], vec![boolean(), optional.clone()]);
            }),
            "An optional element cannot follow a rest element. ts(1266)"
        );
    }
    // The repeatable position itself may be optional; only the trailing ones
    // may not.
    let schema = Schema::tuple_with_rest(vec![], vec![Schema::optional_key_shape(string())]);
    assert_eq!(schema.decode(arr(vec![])), Ok(arr(vec![])));
}

/// Schema.test.ts:4910 — `[FiniteFromString, String] + [Boolean, String]`: the
/// repeatable boolean absorbs everything between the leading tuple and the
/// final string, so a three-element array has no boolean at all and a four or
/// five-element one has one or two.
#[test]
fn tuple_with_rest_reads_the_rest_from_both_ends() {
    let schema = Schema::tuple_with_rest(vec![finite(), string()], vec![boolean(), string()]);

    assert_eq!(
        schema.decode(arr(vec![text("1"), text("a"), text("b")])),
        Ok(arr(vec![text("1"), text("a"), text("b")]))
    );
    assert_eq!(
        schema.decode(arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            text("b")
        ])),
        Ok(arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            text("b")
        ]))
    );
    assert_eq!(
        schema.decode(arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            Unknown::Bool(true),
            text("b")
        ])),
        Ok(arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            Unknown::Bool(true),
            text("b")
        ]))
    );

    // The trailing string is the first position a short array misses.
    let error = schema
        .decode(arr(vec![text("1"), text("a")]))
        .expect_err("a short array misses the final string");
    assert_eq!(
        error.to_string(),
        "expected required field at $[2], got missing"
    );
    // A boolean window holds booleans only, so a string there is refused even
    // though the same string would be fine at the end of a shorter array.
    let error = schema
        .decode(arr(vec![text("1"), text("a"), text("b"), text("c")]))
        .expect_err("the window element is a boolean");
    assert_eq!(error.to_string(), "expected boolean at $[2], got string");
    let error = schema
        .decode(arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            text("b"),
            text("c"),
        ]))
        .expect_err("the window shifted by one");
    assert_eq!(error.to_string(), "expected boolean at $[3], got string");

    // Encoding walks the same split, so a decoded value goes back to its wire
    // spelling and the round trip is exact.
    assert_eq!(
        schema.try_encode(&arr(vec![text("1"), text("a"), text("b")])),
        Ok(arr(vec![text("1"), text("a"), text("b")]))
    );
    assert_eq!(
        schema.try_encode(&arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            text("b")
        ])),
        Ok(arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            text("b")
        ]))
    );
}

/// Schema.test.ts:4947 — `[FiniteFromString, String] + [Boolean, String,
/// FiniteFromString]`: two trailing positions are required after the window, so
/// a three-element array misses the second of them and a four-element one lands
/// its last value in the numeric position.
#[test]
fn tuple_with_rest_needs_every_trailing_position() {
    let schema = Schema::tuple_with_rest(
        vec![finite(), string()],
        vec![boolean(), string(), finite()],
    );

    assert_eq!(
        schema.decode(arr(vec![text("1"), text("a"), text("b"), text("2")])),
        Ok(arr(vec![text("1"), text("a"), text("b"), text("2")]))
    );
    assert_eq!(
        schema.decode(arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            text("b"),
            text("2")
        ])),
        Ok(arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            text("b"),
            text("2")
        ]))
    );
    assert_eq!(
        schema.decode(arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            Unknown::Bool(true),
            text("b"),
            text("2")
        ])),
        Ok(arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            Unknown::Bool(true),
            text("b"),
            text("2")
        ]))
    );

    for (short, index) in [
        (vec![text("1"), text("a")], 2),
        (vec![text("1"), text("a"), text("b")], 3),
    ] {
        let error = schema
            .decode(arr(short.clone()))
            .expect_err("a short array misses a trailing position");
        assert_eq!(
            error.to_string(),
            format!("expected required field at $[{index}], got missing")
        );
    }
    let error = schema
        .decode(arr(vec![text("1"), text("a"), text("b"), text("c")]))
        .expect_err("the last position is numeric");
    assert_eq!(
        error.to_string(),
        "expected a finite number string at $[3], got string"
    );

    // The two trailing positions encode back to text, the numeric one included.
    assert_eq!(
        schema.try_encode(&arr(vec![text("1"), text("a"), text("b"), text("2")])),
        Ok(arr(vec![text("1"), text("a"), text("b"), text("2")]))
    );
    assert_eq!(
        schema.try_encode(&arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            text("b"),
            text("2")
        ])),
        Ok(arr(vec![
            text("1"),
            text("a"),
            Unknown::Bool(true),
            text("b"),
            text("2")
        ]))
    );
}

/// Schema.test.ts:4979 — `[String] + [Boolean, String, Number, Number]`
/// validates every post-rest index, not just the first: with a one-element
/// leading tuple and three trailing positions, the fourth index is the second
/// number and its failure is what a bad value reports.
#[test]
fn tuple_with_rest_validates_every_post_rest_index() {
    let schema = Schema::tuple_with_rest(
        vec![string()],
        vec![boolean(), string(), finite(), finite()],
    );

    assert_eq!(
        schema.decode(arr(vec![
            text("a"),
            Unknown::Bool(true),
            text("b"),
            text("1"),
            text("2")
        ])),
        Ok(arr(vec![
            text("a"),
            Unknown::Bool(true),
            text("b"),
            text("1"),
            text("2")
        ]))
    );
    let error = schema
        .decode(arr(vec![
            text("a"),
            Unknown::Bool(true),
            text("b"),
            text("1"),
            text("x"),
        ]))
        .expect_err("the last numeric position is checked");
    assert_eq!(
        error.to_string(),
        "expected a finite number string at $[4], got string"
    );
    // Both trailing numbers encode back to text, so the round trip is exact.
    assert_eq!(
        schema.try_encode(&arr(vec![
            text("a"),
            Unknown::Bool(true),
            text("b"),
            text("1"),
            text("2")
        ])),
        Ok(arr(vec![
            text("a"),
            Unknown::Bool(true),
            text("b"),
            text("1"),
            text("2")
        ]))
    );
}
