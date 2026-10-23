//! The `Arbitrary.test.ts` rows that construct strings from a pattern or from a
//! code-point bound (`:171`–`:1010`).
//!
//! Generation runs through `Arbitrary.schema`, so a sample is only a sample if
//! the schema decodes it: a pattern, a length bound, and a code-point bound are
//! all enforced on the way back in. The assertions then name the shape itself
//! (length in UTF-16 units, code-point count, the text a branch produces), so a
//! generator that merely agreed with its own matcher would still be caught.

use elm_effect_schema::pattern::{utf16_len, Pattern};
use elm_effect_schema::{Schema, Unknown};
use elm_effect_unstable::arbitrary::{Gen, Rng};

fn json(text: &str) -> String {
    serde_json::to_string(text).expect("a JSON string")
}

/// The `isPattern` argument as a repr prints it.
fn pattern_arg(source: &str, flags: &str) -> Vec<String> {
    vec![format!("new RegExp({}, {})", json(source), json(flags))]
}

fn string_with(checks: Vec<(&str, Vec<String>)>) -> Schema<Unknown> {
    let mut result = Schema::as_unknown(Schema::string());
    for (name, args) in checks {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        result = Schema::checked(result, name, refs, vec![]);
    }
    result
}

fn pattern(source: &str, flags: &str) -> Schema<Unknown> {
    string_with(vec![("isPattern", pattern_arg(source, flags))])
}

fn bound(value: usize) -> Vec<String> {
    vec![value.to_string()]
}

fn text(value: &Unknown) -> &str {
    match value {
        Unknown::Text(text) => text,
        other => panic!("expected text, got {other:?}"),
    }
}

/// A seed from the row's own seed text, so the rows keep their names.
fn seed_of(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[derive(Debug, PartialEq, Eq)]
struct SampleError {
    generated: usize,
    discards: usize,
}

/// `sampleEffect`: one attempt per draw, discards counted.
fn sample(
    generator: &Gen<Unknown>,
    count: usize,
    max_discards: usize,
    seed: u64,
) -> Result<Vec<String>, SampleError> {
    let mut values = Vec::new();
    let mut discards = 0usize;
    let mut attempt = 0u64;
    while values.len() < count {
        let mut rng = Rng::new(seed ^ attempt.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        match generator.try_sample(&mut rng) {
            Some(value) => values.push(text(&value).to_string()),
            None => {
                discards += 1;
                if discards > max_discards {
                    return Err(SampleError {
                        generated: values.len(),
                        discards,
                    });
                }
            }
        }
        attempt += 1;
    }
    Ok(values)
}

fn samples(schema: &Schema<Unknown>, count: usize, seed: &str) -> Vec<String> {
    let generator = schema_(schema);
    sample(&generator, count, 200, seed_of(seed)).expect("the derivation must construct a value")
}

fn schema_(schema: &Schema<Unknown>) -> Gen<Unknown> {
    elm_effect_unstable::arbitrary::schema(schema).expect("the schema derives an arbitrary")
}

/// `Schema.is(schema)` for one value.
fn accepts(schema: &Schema<Unknown>, value: &str) -> bool {
    schema.decode(Unknown::Text(value.to_string())).is_ok()
}

struct Falsified {
    initial: String,
    shrunk: String,
}

/// `checkEffect` with the usual shrink loop: the first candidate that still
/// fails the property is adopted, until no candidate does.
fn check(
    generator: &Gen<Unknown>,
    property: impl Fn(&str) -> bool,
    seed: u64,
) -> Falsified {
    let mut attempt = 0u64;
    let initial = loop {
        let mut rng = Rng::new(seed ^ attempt.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        if let Some(value) = generator.try_sample(&mut rng) {
            break text(&value).to_string();
        }
        attempt += 1;
        assert!(attempt < 64, "no draw survived");
    };
    assert!(!property(&initial), "the first draw must fail");
    let mut shrunk = initial.clone();
    loop {
        let candidates = generator.shrink(&Unknown::Text(shrunk.clone()));
        let next = candidates
            .into_iter()
            .map(|candidate| text(&candidate).to_string())
            .find(|candidate| !property(candidate));
        match next {
            Some(candidate) => shrunk = candidate,
            None => break,
        }
    }
    Falsified { initial, shrunk }
}

// ---------------------------------------------------------------------------
// Construction from a pattern
// ---------------------------------------------------------------------------

/// Arbitrary.test.ts:171 — every pattern in the regular subset, including
/// counted repeats, alternation, escapes and escapes of a supplementary code
/// point, produces only strings the pattern matches.
#[test]
fn constructs_strings_for_the_regular_pattern_subset() {
    let cases: &[(&str, &str)] = &[
        ("^[A-Z]{3}[0-9]{3}$", ""),
        ("^(?:foo|bar)-[^0-9]\\s?\\w+$", ""),
        ("^a+?$", ""),
        ("^\\x41\\u0042\\cC$", ""),
        ("^\\u{1f600}{2}$", "u"),
        ("^effect$", "y"),
    ];
    for (index, (source, flags)) in cases.iter().enumerate() {
        let schema = pattern(source, flags);
        let values = samples(&schema, 20, &format!("regular-pattern-{index}"));
        let compiled = Pattern::parse(source, flags).expect("the source is in the subset");
        assert_eq!(values.len(), 20);
        for value in &values {
            assert!(
                compiled.is_match(value),
                "{source} generated {value:?} which it does not match"
            );
            assert!(accepts(&schema, value), "{source} generated {value:?}");
        }
    }
    // The shapes themselves, so the matcher and the generator cannot agree on
    // a mistake.
    let anchor = Pattern::parse("^[A-Z]{3}[0-9]{3}$", "").expect("a pattern");
    assert!(anchor.is_match("ABC123"));
    assert!(!anchor.is_match("ABC12"));
    assert!(!anchor.is_match("abc123"));
    let escaped = Pattern::parse("^\\x41\\u0042\\cC$", "").expect("a pattern");
    assert!(escaped.is_match("AB\u{3}"));
    assert!(!escaped.is_match("ABC"));
}

/// Arbitrary.test.ts:191 — code-point bounds and UTF-16 length bounds are
/// normalized against each other, so a value satisfies both.
#[test]
fn constructs_strings_with_code_point_and_utf16_length_bounds() {
    let cases: Vec<Schema<Unknown>> = vec![
        string_with(vec![("isMinCodePoints", bound(128))]),
        string_with(vec![("isMaxCodePoints", bound(0))]),
        string_with(vec![("isBetweenCodePoints", vec!["2".into(), "4".into()])]),
        string_with(vec![
            ("isMinCodePoints", bound(3)),
            ("isMinCodePoints", bound(4)),
            ("isMaxCodePoints", bound(4)),
        ]),
        string_with(vec![
            ("isMinCodePoints", bound(4)),
            ("isMaxCodePoints", bound(5)),
            ("isMaxCodePoints", bound(4)),
        ]),
        string_with(vec![
            ("isBetweenLength", vec!["8".into(), "8".into()]),
            ("isBetweenCodePoints", vec!["4".into(), "4".into()]),
        ]),
        string_with(vec![
            ("isMaxLength", bound(4)),
            ("isMinCodePoints", bound(4)),
        ]),
        string_with(vec![
            ("isPattern", pattern_arg("^\\u{1f600}{4}$", "u")),
            ("isBetweenCodePoints", vec!["4".into(), "4".into()]),
        ]),
    ];
    let expected_code_points: &[(usize, Option<usize>)] = &[
        (128, None),
        (0, Some(0)),
        (2, Some(4)),
        (4, Some(4)),
        (4, Some(4)),
        (4, Some(4)),
        (4, Some(4)),
        (4, Some(4)),
    ];
    for (index, schema) in cases.iter().enumerate() {
        let values = samples(schema, 20, &format!("code-points-{index}"));
        let (minimum, maximum) = expected_code_points[index];
        for value in &values {
            let count = value.chars().count();
            assert!(
                count >= minimum,
                "case {index}: {value:?} has {count} code points, wanted at least {minimum}"
            );
            if let Some(maximum) = maximum {
                assert!(
                    count <= maximum,
                    "case {index}: {value:?} has {count} code points, wanted at most {maximum}"
                );
            }
            assert!(accepts(schema, value), "case {index}: {value:?}");
        }
    }
}

/// Arbitrary.test.ts:215 — a variable-length pattern above the default size
/// reaches the length the code-point floor demands.
#[test]
fn generates_variable_length_supplementary_patterns_above_the_default_size() {
    let schema = string_with(vec![
        ("isPattern", pattern_arg("^\\u{1f600}+$", "u")),
        ("isMinCodePoints", bound(128)),
    ]);
    let values = samples(&schema, 5, "42");
    for value in &values {
        assert!(value.chars().count() >= 128, "{value:?}");
        assert!(value.chars().all(|c| c == '😀'), "{value:?}");
    }
}

/// Arbitrary.test.ts:226 — a code-point window pins the repetition count, so
/// the pattern's own length analysis has to reach it.
#[test]
fn generates_variable_length_patterns_with_code_point_bounds_at_size_zero() {
    let cases = vec![
        string_with(vec![
            ("isPattern", pattern_arg("^\\u{1f600}+$", "u")),
            ("isBetweenCodePoints", vec!["4".into(), "4".into()]),
            ("isMaxLength", bound(8)),
        ]),
        string_with(vec![
            ("isPattern", pattern_arg("^a+$", "")),
            ("isBetweenCodePoints", vec!["4".into(), "4".into()]),
            ("isMaxLength", bound(4)),
        ]),
        string_with(vec![
            ("isPattern", pattern_arg("^(?:a|\\u{1f600})+$", "u")),
            ("isBetweenCodePoints", vec!["4".into(), "4".into()]),
            ("isMaxLength", bound(6)),
        ]),
    ];
    for (index, schema) in cases.iter().enumerate() {
        let values = samples(schema, 20, &format!("code-point-pattern-{index}"));
        for value in &values {
            assert_eq!(value.chars().count(), 4, "case {index}: {value:?}");
            assert!(accepts(schema, value), "case {index}: {value:?}");
        }
    }
}

/// Arbitrary.test.ts:253 — of two alternatives of different UTF-16 length, only
/// the one inside the code-point ceiling is constructible, and it is the one
/// every sample uses.
#[test]
fn reaches_longer_pattern_alternatives_under_a_maximum_code_point_bound() {
    let schema = string_with(vec![
        (
            "isPattern",
            pattern_arg("^(?:a{129}|\\u{1f600}{128})$", "u"),
        ),
        ("isMaxCodePoints", bound(128)),
    ]);
    let generator = schema_(&schema);
    let values = sample(&generator, 10, 100, seed_of("42")).expect("the long alternative generates");
    assert_eq!(values, vec!["😀".repeat(128); 10]);
}

/// Arbitrary.test.ts:270 — a code-point ceiling selects the alternative that
/// fits, and a generous ceiling does not stop a bounded repetition.
#[test]
fn respects_utf16_limits_and_large_code_point_maxima_for_patterns() {
    let patterns = string_with(vec![
        ("isPattern", pattern_arg("^(?:aaa|\\u{1f600}{2})$", "u")),
        ("isMaxCodePoints", bound(2)),
        ("isMaxLength", bound(4)),
    ]);
    let values = samples(&patterns, 10, "42");
    for value in &values {
        assert_eq!(value, "😀😀", "only the supplementary alternative fits");
    }

    let generous = string_with(vec![
        ("isPattern", pattern_arg("^a+$", "")),
        ("isMaxCodePoints", bound(1_000_000)),
    ]);
    let values = samples(&generous, 10, "42");
    for value in &values {
        assert!(accepts(&generous, value), "{value:?}");
        assert!(value.chars().all(|c| c == 'a'), "{value:?}");
    }
}

/// Arbitrary.test.ts:291 — a one-code-point window reaches supplementary
/// characters, and shrinking stays inside it.
#[test]
fn generates_supplementary_code_points_and_preserves_bounds_while_shrinking() {
    let schema = string_with(vec![("isBetweenCodePoints", vec!["1".into(), "1".into()])]);
    let values = samples(&schema, 100, "supplementary-code-points");
    assert!(
        values.iter().any(|value| utf16_len(value) == 2),
        "a supplementary code point is two UTF-16 units"
    );
    for value in &values {
        assert_eq!(value.chars().count(), 1, "{value:?}");
        assert!(accepts(&schema, value), "{value:?}");
    }

    let generator = schema_(&schema);
    let falsified = check(&generator, |_| false, seed_of("code-point-shrink"));
    assert_eq!(falsified.shrunk, "\0");
    assert_eq!(falsified.shrunk.chars().count(), 1);
}

/// Arbitrary.test.ts:313 — contradictory code-point and length bounds have no
/// inhabitant, so the derivation refuses rather than filtering forever.
#[test]
fn rejects_contradictory_code_point_and_utf16_length_constraints() {
    let cases = vec![
        string_with(vec![
            ("isMinCodePoints", bound(3)),
            ("isMaxCodePoints", bound(2)),
        ]),
        string_with(vec![
            ("isMinCodePoints", bound(3)),
            ("isMaxLength", bound(2)),
        ]),
        string_with(vec![
            ("isMinLength", bound(3)),
            ("isMaxCodePoints", bound(1)),
        ]),
    ];
    for schema in &cases {
        let error = elm_effect_unstable::arbitrary::schema(schema)
            .expect_err("contradictory bounds have no value");
        assert!(
            error.to_string().contains("constraints"),
            "the refusal names the constraints: {error}"
        );
    }
}

/// Arbitrary.test.ts:325 — every alternative stays reachable under a length
/// bound, and the generator covers more than one length.
#[test]
fn covers_feasible_regular_expression_alternatives_under_a_length_bound() {
    let schema = string_with(vec![
        (
            "isPattern",
            pattern_arg("^(?:a|[B-D]{2}|[0-9]{3}){16}$", ""),
        ),
        ("isMaxLength", bound(32)),
    ]);
    let values = samples(&schema, 64, "42");
    let lengths: std::collections::HashSet<usize> = values.iter().map(|v| utf16_len(v)).collect();
    assert!(lengths.len() > 1, "the length window must be explored");
    assert!(
        values.iter().any(|value| value.contains('a')),
        "the one-character alternative must appear"
    );
    assert!(
        values.iter().any(|value| {
            value
                .as_bytes()
                .windows(2)
                .any(|window| window.iter().all(|byte| (b'B'..=b'D').contains(byte)))
        }),
        "the two-character alternative must appear"
    );
    assert!(
        values.iter().any(|value| {
            value
                .as_bytes()
                .windows(3)
                .any(|window| window.iter().all(u8::is_ascii_digit))
        }),
        "the three-character alternative must appear"
    );
    for value in &values {
        assert!(accepts(&schema, value), "{value:?}");
    }
}

/// Arbitrary.test.ts:343 — dot-all reaches a line terminator, and a sticky
/// pattern matches at the start without pinning the end.
#[test]
fn honors_dot_all_and_sticky_regular_expression_flags() {
    let dot_all = pattern("^.$", "s");
    let values = samples(&dot_all, 200, "0");
    assert!(
        values
            .iter()
            .any(|value| matches!(value.as_str(), "\n" | "\r" | "\u{2028}" | "\u{2029}")),
        "dot-all must reach a line terminator"
    );

    let sticky = pattern("a", "y");
    let values = samples(&sticky, 100, "sticky-pattern");
    assert!(
        values.iter().any(|value| value.len() > 1),
        "a sticky pattern matches at the start of a longer string"
    );
    for value in &values {
        assert!(value.starts_with('a'), "{value:?}");
    }
}

/// Arbitrary.test.ts:364 — a broad class prefers printable ASCII sometimes and
/// the whole domain other times.
#[test]
fn biases_broad_character_classes_toward_common_characters() {
    let schema = pattern("^.$", "u");
    let values = samples(&schema, 200, "broad-character-class");
    assert!(
        values
            .iter()
            .any(|value| value.chars().next().is_some_and(|c| (' '..='~').contains(&c))),
        "printable ASCII must be reachable"
    );
    assert!(
        values
            .iter()
            .any(|value| value.chars().next().is_some_and(|c| c as u32 > 0x1f600)),
        "the far end of the domain must be reachable"
    );
}

/// Arbitrary.test.ts:377 — the built-in string checks are patterns too, so the
/// derivation constructs for them instead of rejecting until one passes.
#[test]
fn constructs_strings_for_built_in_pattern_checks() {
    type Case = (Schema<Unknown>, fn(&str) -> bool);
    let cases: Vec<Case> = vec![
        (
            string_with(vec![("isTrimmed", vec![])]),
            |value| value.trim() == value,
        ),
        (
            string_with(vec![("isUppercased", vec![])]),
            |value| value.to_uppercase() == value,
        ),
        (
            string_with(vec![("isLowercased", vec![])]),
            |value| value.to_lowercase() == value,
        ),
        (
            string_with(vec![("isCapitalized", vec![])]),
            |value| {
                value
                    .chars()
                    .next()
                    .is_none_or(|first| first.to_uppercase().to_string() == first.to_string())
            },
        ),
        (
            string_with(vec![("isUncapitalized", vec![])]),
            |value| {
                value
                    .chars()
                    .next()
                    .is_none_or(|first| first.to_lowercase().to_string() == first.to_string())
            },
        ),
        (
            string_with(vec![("isStartingWith", vec!["a.b".into()])]),
            |value| value.starts_with("a.b"),
        ),
        (
            string_with(vec![("isEndingWith", vec!["a.b".into()])]),
            |value| value.ends_with("a.b"),
        ),
        (
            string_with(vec![("isIncluding", vec!["a.b".into()])]),
            |value| value.contains("a.b"),
        ),
    ];
    for (index, (schema, property)) in cases.iter().enumerate() {
        let values = samples(schema, 20, &format!("built-in-pattern-{index}"));
        for value in &values {
            assert!(property(value), "case {index}: {value:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Patterns beside length bounds, and outside the subset
// ---------------------------------------------------------------------------

/// Arbitrary.test.ts:833 — an unanchored pattern sits inside generated text,
/// and both an empty prefix and an empty suffix are reachable.
#[test]
fn explores_strings_around_an_unanchored_match() {
    let schema = string_with(vec![("isIncluding", vec!["needle".into()])]);
    let values = samples(&schema, 100, "unanchored-pattern");
    assert!(values.iter().all(|value| value.contains("needle")));
    assert!(values.iter().any(|value| value != "needle"));
    assert!(values.iter().any(|value| value.starts_with("needle")));
    assert!(values.iter().any(|value| value.ends_with("needle")));
}

/// Arbitrary.test.ts:848 — a pattern whose repetitions straddle the length
/// window only produces the lengths the window allows.
#[test]
fn combines_constructive_pattern_generation_with_string_length_constraints() {
    let schema = string_with(vec![
        ("isPattern", pattern_arg("^(ab){3,10}$", "")),
        ("isMinLength", bound(8)),
        ("isMaxLength", bound(12)),
    ]);
    let values = samples(&schema, 100, "pattern-length");
    for value in &values {
        assert!(
            matches!(utf16_len(value), 8 | 10 | 12),
            "the length window admits 8, 10 and 12 only, got {value:?}"
        );
        assert!(accepts(&schema, value), "{value:?}");
    }
}

/// Arbitrary.test.ts:865 — with two patterns, one is the candidate and every
/// one still validates, so a value has to satisfy both.
#[test]
fn uses_every_supported_pattern_as_a_candidate() {
    let schema = string_with(vec![
        ("isPattern", pattern_arg("^[^A-Z]*$", "")),
        ("isPattern", pattern_arg("^0x[0-9a-f]{40}$", "")),
    ]);
    let generator = schema_(&schema);
    let values = sample(&generator, 20, 200, seed_of("multiple-patterns"))
        .expect("the feasible candidate is reachable");
    for value in &values {
        assert_eq!(utf16_len(value), 42, "{value:?}");
        assert!(accepts(&schema, value), "{value:?}");
    }
}

/// Arbitrary.test.ts:880 — an unsupported pattern stays a residual filter, so
/// the supported one drives construction and the filter decides.
#[test]
fn keeps_unsupported_patterns_as_residual_filters() {
    let schema = string_with(vec![
        ("isPattern", pattern_arg("^(?=a)a$", "")),
        ("isPattern", pattern_arg("^a$", "")),
    ]);
    let generator = schema_(&schema);
    let values = sample(&generator, 20, 0, seed_of("unsupported-pattern-candidate"))
        .expect("the supported pattern generates a value the filter accepts");
    assert_eq!(values, vec!["a"; 20]);
}

/// Arbitrary.test.ts:895 — a residual filter no value passes exhausts the
/// discards, and the sampler reports how many it spent.
#[test]
fn bounds_fallback_filtering_for_unsupported_constructs() {
    let schema = pattern("^(?=a)b$", "");
    let generator = schema_(&schema);
    assert_eq!(
        sample(&generator, 1, 2, seed_of("unsupported-pattern")),
        Err(SampleError {
            generated: 0,
            discards: 3
        })
    );
}

/// Arbitrary.test.ts:915 — a permissive unsupported pattern falls back to the
/// generic string generator, whose values the filter accepts.
#[test]
fn uses_generic_generation_for_permissive_unsupported_patterns() {
    let schema = pattern("^(?=)[ -~]*$", "");
    let generator = schema_(&schema);
    let values = sample(&generator, 20, 20, seed_of("unsupported-permissive-pattern"))
        .expect("the generic generator stays inside the pattern");
    for value in &values {
        assert!(value.chars().all(|c| (' '..='~').contains(&c)), "{value:?}");
        assert!(accepts(&schema, value), "{value:?}");
    }
}

/// Arbitrary.test.ts:927 — a pattern and a length window that cannot meet have
/// no value, so the sampler exhausts its discards.
#[test]
fn bounds_incompatible_pattern_and_length_constraints() {
    let cases = vec![
        string_with(vec![
            ("isPattern", pattern_arg("^(aa)+$", "")),
            ("isMinLength", bound(3)),
            ("isMaxLength", bound(3)),
        ]),
        string_with(vec![
            ("isPattern", pattern_arg("^$", "")),
            ("isMinLength", bound(1)),
        ]),
    ];
    for schema in &cases {
        let generator = schema_(schema);
        assert_eq!(
            sample(&generator, 1, 2, seed_of("incompatible-pattern-length")),
            Err(SampleError {
                generated: 0,
                discards: 3
            })
        );
    }
}

/// Arbitrary.test.ts:960 — shrinking a pattern-bound string keeps the pattern
/// and stops at the first value the property accepts.
#[test]
fn keeps_pattern_constraints_while_shrinking_strings() {
    let schema = string_with(vec![
        ("isPattern", pattern_arg("^a+$", "")),
        ("isMaxLength", bound(8)),
    ]);
    let generator = schema_(&schema);
    let falsified = check(&generator, |value| utf16_len(value) < 4, seed_of("pattern-shrink"));
    assert_eq!(falsified.shrunk, "aaaa");
    assert!(accepts(&schema, &falsified.shrunk));
}

/// Arbitrary.test.ts:978 — without a pattern, a fixed-length string shrinks
/// every UTF-16 unit toward null and never leaves its length.
#[test]
fn shrinks_fixed_length_string_code_units_toward_null() {
    let schema = string_with(vec![
        ("isMinLength", bound(3)),
        ("isMaxLength", bound(3)),
    ]);
    let generator = schema_(&schema);
    let falsified = check(&generator, |_| false, seed_of("string-code-unit-shrink"));
    assert_ne!(falsified.initial, "\0\0\0");
    assert_eq!(falsified.shrunk, "\0\0\0");
}

/// Arbitrary.test.ts:994 — a repetition shrinks to its minimum, not by
/// character deletion.
#[test]
fn structurally_shrinks_regular_expression_repetitions() {
    let schema = string_with(vec![
        ("isPattern", pattern_arg("^a*b$", "")),
        ("isMinLength", bound(1)),
        ("isMaxLength", bound(8)),
    ]);
    let generator = schema_(&schema);
    // The first draw may already be the minimum, which has nothing to shrink;
    // a repetition is what this row is about, so find one.
    let mut attempt = 0u64;
    loop {
        let mut rng = Rng::new(seed_of("structural-pattern-shrink") ^ attempt);
        let value = generator.try_sample(&mut rng).expect("a value");
        let initial = text(&value).to_string();
        if initial == "b" {
            attempt += 1;
            assert!(attempt < 64, "no draw used the repetition");
            continue;
        }
        let mut shrunk = initial.clone();
        loop {
            let next = generator
                .shrink(&Unknown::Text(shrunk.clone()))
                .into_iter()
                .map(|candidate| text(&candidate).to_string())
                .next();
            match next {
                Some(candidate) => shrunk = candidate,
                None => break,
            }
        }
        assert_eq!(shrunk, "b");
        assert!(accepts(&schema, &shrunk));
        break;
    }
}

/// Arbitrary.test.ts:1010 — an alternation shrinks to its first branch.
#[test]
fn structurally_shrinks_regular_expression_alternatives_to_the_first_one() {
    let schema = pattern("^(foo|bar|baz)$", "");
    let generator = schema_(&schema);
    let mut attempt = 0u64;
    loop {
        let mut rng = Rng::new(seed_of("structural-alternative-shrink") ^ attempt);
        let value = generator.try_sample(&mut rng).expect("a value");
        let initial = text(&value).to_string();
        if initial == "foo" {
            attempt += 1;
            assert!(attempt < 64, "no draw used a later alternative");
            continue;
        }
        let mut shrunk = initial.clone();
        loop {
            let next = generator
                .shrink(&Unknown::Text(shrunk.clone()))
                .into_iter()
                .map(|candidate| text(&candidate).to_string())
                .next();
            match next {
                Some(candidate) => shrunk = candidate,
                None => break,
            }
        }
        assert_eq!(shrunk, "foo");
        break;
    }
}
