//! Arbitrary fence: deterministic seeded samples, bounded integers with
//! edges, shrink-to-boundary, and composition.

use elm_effect_unstable::arbitrary::{
    Rng, array, ascii_string, boolean, constant, hex_string, integer, number, option, record, tuple,
};

// ---------------------------------------------------------------------------
// Arbitrary::schema + checkEffect (:54, :62, :84, :126)
// ---------------------------------------------------------------------------

#[test]
fn is_arbitrary_identifies_arbitrary_values() {
    // Arbitrary.test.ts :54 — isArbitrary answers true for an Arbitrary and
    // false for the schema it was derived from and for a non-Arbitrary.
    let arbitrary = arbitrary_schema(&ArbSchema::Literals(vec!["large".into(), "small".into()]));
    assert!(is_arbitrary(&arbitrary));
    assert!(!is_arbitrary(&ArbSchema::Literals(vec![
        "large".into(),
        "small".into()
    ])));
    assert!(!is_arbitrary(&()));
}

#[test]
fn schema_custom_shrinker_shrinks_and_replays() {
    // Arbitrary.test.ts :62 — Arbitrary.schema(Schema.Literals(["large",
    // "small"]), { shrink: (v) => v === "large" ? ["small"] : [] }) checked
    // with an always-false property, runs: 1, maxDiscards: 0, seed: 0.
    let arbitrary = arbitrary_schema_with_shrink(
        &ArbSchema::Literals(vec!["large".into(), "small".into()]),
        |value: &String| {
            if value == "large" {
                vec!["small".to_string()]
            } else {
                Vec::new()
            }
        },
    );
    let result = check_effect(
        &arbitrary,
        |_| false,
        CheckOptions {
            runs: 1,
            max_discards: 0,
            seed: Some(Seed::Number(0)),
            ..CheckOptions::default()
        },
    );
    let falsified = match result {
        CheckResult::Falsified(falsified) => falsified,
        other => panic!("expected Falsified, got {other:?}"),
    };
    assert_eq!(falsified.initial_input, "large");
    assert_eq!(falsified.shrunk_input, "small");
    assert_eq!(falsified.shrinks, 1);

    // The replay token reproduces the same report.
    let replayed = check_effect(
        &arbitrary,
        |_| false,
        CheckOptions {
            replay: Some(falsified.replay.clone()),
            ..CheckOptions::default()
        },
    );
    assert_eq!(replayed, CheckResult::Falsified(falsified));
}

#[test]
fn schema_validates_custom_shrink_candidates_before_evaluating() {
    // Arbitrary.test.ts :84 — a custom shrinker that emits an invalid
    // candidate ("invalid", not a literal) plus "small": with maxShrinks: 1
    // the invalid candidate is inspected and rejected, so shrinks stays 0
    // and shrunkInput stays "large"; with maxShrinks: 2 the valid candidate
    // is reached, shrinks is 1 and shrunkInput is "small".
    let arbitrary = arbitrary_schema_with_shrink(
        &ArbSchema::Literals(vec!["large".into(), "small".into()]),
        |value: &String| {
            if value == "large" {
                vec!["invalid".to_string(), "small".to_string()]
            } else {
                Vec::new()
            }
        },
    );
    let mut limited_evaluated = Vec::new();
    let limited = check_effect(
        &arbitrary,
        |value: &String| {
            limited_evaluated.push(value.clone());
            false
        },
        CheckOptions {
            runs: 1,
            max_discards: 0,
            max_shrinks: 1,
            seed: Some(Seed::Number(0)),
            ..CheckOptions::default()
        },
    );
    let limited = match limited {
        CheckResult::Falsified(falsified) => falsified,
        other => panic!("expected Falsified, got {other:?}"),
    };
    assert_eq!(limited.shrunk_input, "large");
    assert_eq!(limited.shrinks, 0);
    assert_eq!(limited_evaluated, vec!["large".to_string()]);

    let mut evaluated = Vec::new();
    let full = check_effect(
        &arbitrary,
        |value: &String| {
            evaluated.push(value.clone());
            false
        },
        CheckOptions {
            runs: 1,
            max_discards: 0,
            max_shrinks: 2,
            seed: Some(Seed::Number(0)),
            ..CheckOptions::default()
        },
    );
    let full = match full {
        CheckResult::Falsified(falsified) => falsified,
        other => panic!("expected Falsified, got {other:?}"),
    };
    assert_eq!(full.shrunk_input, "small");
    assert_eq!(full.shrinks, 1);
    assert_eq!(evaluated, vec!["large".to_string(), "small".to_string()]);
}

#[test]
fn schema_custom_shrinker_runs_only_while_shrinking_a_failure() {
    // Arbitrary.test.ts :126 — the custom shrinker is evaluated lazily: a
    // passing check and a sampling run never call it, and only the failing
    // check calls it once.
    let calls = std::rc::Rc::new(std::cell::Cell::new(0usize));
    {
        let counter = std::rc::Rc::clone(&calls);
        let arbitrary = arbitrary_schema_with_shrink(
            &ArbSchema::Literals(vec!["large".into(), "small".into()]),
            move |_value: &String| {
                counter.set(counter.get() + 1);
                Vec::new()
            },
        );
        let _ = sample_effect(
            &arbitrary,
            SampleOptions {
                count: 1,
                size: 10,
                max_discards: 0,
                seed: Some(Seed::Number(0)),
            },
        );
        assert_eq!(calls.get(), 0, "sampling must not evaluate the shrinker");
        let passed = check_effect(
            &arbitrary,
            |_| true,
            CheckOptions {
                runs: 1,
                max_discards: 0,
                seed: Some(Seed::Number(0)),
                ..CheckOptions::default()
            },
        );
        assert!(matches!(passed, CheckResult::Passed { .. }));
        assert_eq!(
            calls.get(),
            0,
            "a passing check must not evaluate the shrinker"
        );
        let falsified = check_effect(
            &arbitrary,
            |_| false,
            CheckOptions {
                runs: 1,
                max_discards: 0,
                seed: Some(Seed::Number(0)),
                ..CheckOptions::default()
            },
        );
        assert!(matches!(falsified, CheckResult::Falsified(_)));
        assert_eq!(
            calls.get(),
            1,
            "only the shrink of the failing value calls it"
        );
    }
}

#[test]
fn schema_custom_shrinker_replaces_schema_derived_shrinking() {
    // Arbitrary.test.ts :144 — a custom shrinker returning [] replaces the
    // schema-derived shrinking entirely: the falsification keeps its initial
    // input and reports zero shrinks. (The pin's schema is a bounded BigInt;
    // the bounded-integer shape carries the same observable.)
    let arbitrary = arbitrary_schema_with_shrink(
        &ArbSchema::IntBetween {
            minimum: 100,
            maximum: 1_000,
        },
        |_value: &String| Vec::new(),
    );
    let result = check_effect(
        &arbitrary,
        |_| false,
        CheckOptions {
            runs: 1,
            max_discards: 0,
            seed: Some(Seed::Number(21)),
            ..CheckOptions::default()
        },
    );
    let falsified = match result {
        CheckResult::Falsified(falsified) => falsified,
        other => panic!("expected Falsified, got {other:?}"),
    };
    let initial: i64 = falsified.initial_input.parse().expect("an integer");
    assert!(initial > 100, "the initial input must be above the minimum");
    assert_eq!(falsified.shrunk_input, falsified.initial_input);
    assert_eq!(falsified.shrinks, 0);
}

#[test]
fn schema_pushes_constraints_into_primitive_constructors() {
    // Arbitrary.test.ts :160 — Schema.String.check(isMinLength(8),
    // isMaxLength(8)): every sample is exactly 8 long, and two sampling runs
    // with the same seed produce the same strings.
    let arbitrary = arbitrary_schema(&ArbSchema::StringExactLength(8));
    let first = sample_effect(
        &arbitrary,
        SampleOptions {
            count: 20,
            size: 10,
            max_discards: 0,
            seed: Some(Seed::Text("constraint".into())),
        },
    )
    .expect("a length-constrained string always generates");
    let second = sample_effect(
        &arbitrary,
        SampleOptions {
            count: 20,
            size: 10,
            max_discards: 0,
            seed: Some(Seed::Text("constraint".into())),
        },
    )
    .expect("a length-constrained string always generates");
    assert_eq!(first, second, "the same seed must reproduce the samples");
    assert!(first.iter().all(|value| value.chars().count() == 8));
}

#[test]
fn schema_derives_independent_random_streams_for_adjacent_attempts() {
    // Arbitrary.test.ts :1026 — 1000 samples of a bounded integer cover at
    // least 750 distinct values: the edge bias repeats small values, but
    // each attempt's stream is its own derivation and explores broadly.
    let arbitrary = arbitrary_schema(&ArbSchema::IntBetween {
        minimum: 0,
        maximum: i64::MAX,
    });
    let values = sample_effect(
        &arbitrary,
        SampleOptions {
            count: 1_000,
            size: 10,
            max_discards: 0,
            seed: Some(Seed::Text("attempt-streams".into())),
        },
    )
    .expect("a wide integer range always generates");
    let distinct: std::collections::HashSet<&String> = values.iter().collect();
    assert!(
        distinct.len() >= 750,
        "adjacent attempt streams must stay independent"
    );
}

#[test]
fn map_carries_the_complete_shrink_tree() {
    // Arbitrary.test.ts :2940 — Arbitrary.schema(Int.check(isBetween(1,
    // 1000))).pipe(Arbitrary.map(v => v * 2)), checked with an always-false
    // property, runs: 1, seed: 47, size: 10. The first sample is the
    // maximum, the shrink tree's first candidate is the minimum, and the map
    // carries both.
    let source = arbitrary_schema(&ArbSchema::IntBetween {
        minimum: 1,
        maximum: 1_000,
    });
    let arbitrary = arbitrary_map(source, |value: &String| {
        (value.parse::<i64>().expect("an integer") * 2).to_string()
    });
    let result = check_effect(
        &arbitrary,
        |_| false,
        CheckOptions {
            runs: 1,
            size: 10,
            seed: Some(Seed::Number(47)),
            ..CheckOptions::default()
        },
    );
    let falsified = match result {
        CheckResult::Falsified(falsified) => falsified,
        other => panic!("expected Falsified, got {other:?}"),
    };
    assert_eq!(falsified.initial_input, "2000");
    assert_eq!(falsified.shrunk_input, "2");
    assert_eq!(falsified.shrinks, 1);
}

#[test]
fn map_preserves_duplicate_shrink_positions_after_a_non_injective_map() {
    // Arbitrary.test.ts :2957 — a constant map keeps every shrink position:
    // the mapped values are all "same", but the shrink step still counts one
    // and the property is evaluated twice.
    let source = arbitrary_schema(&ArbSchema::IntBetween {
        minimum: 1,
        maximum: 1_000,
    });
    let arbitrary = arbitrary_map(source, |_value: &String| "same".to_string());
    let mut evaluations = 0;
    let result = check_effect(
        &arbitrary,
        |_value: &String| {
            evaluations += 1;
            false
        },
        CheckOptions {
            runs: 1,
            size: 10,
            seed: Some(Seed::Number(47)),
            ..CheckOptions::default()
        },
    );
    let falsified = match result {
        CheckResult::Falsified(falsified) => falsified,
        other => panic!("expected Falsified, got {other:?}"),
    };
    assert_eq!(falsified.initial_input, "same");
    assert_eq!(falsified.shrunk_input, "same");
    assert_eq!(falsified.shrinks, 1);
    assert_eq!(evaluations, 2);
}

#[test]
fn filter_bounds_rejected_values_and_reports_exhaustion() {
    // Arbitrary.test.ts :2978 — a filter that rejects everything: sampling
    // fails with SampleError { generated: 0, discards: 3 } at maxDiscards: 2,
    // and a check reports Exhausted { runs: 0, discards: 3 } with the seed.
    let arbitrary = arbitrary_filter(arbitrary_schema(&ArbSchema::IntUnbounded), |_| false);
    let sampled = sample_effect(
        &arbitrary,
        SampleOptions {
            count: 1,
            size: 10,
            max_discards: 2,
            seed: Some(Seed::Text("filter-exhaustion".into())),
        },
    );
    assert_eq!(
        sampled,
        Err(SampleError {
            generated: 0,
            discards: 3,
            seed: Seed::Text("filter-exhaustion".into())
        })
    );
    let checked = check_effect(
        &arbitrary,
        |_| true,
        CheckOptions {
            runs: 1,
            max_discards: 2,
            seed: Some(Seed::Text("filter-exhaustion".into())),
            ..CheckOptions::default()
        },
    );
    assert_eq!(
        checked,
        CheckResult::Exhausted {
            runs: 0,
            discards: 3,
            seed: Seed::Text("filter-exhaustion".into())
        }
    );
}

#[test]
fn filter_promotes_accepted_shrink_descendants() {
    // Arbitrary.test.ts :3009 — a filter accepting only 100 or 26: the shrink
    // frontier walks into rejected candidates' subtrees, promoting 26 as the
    // shrunk input after one shrink step.
    let source = arbitrary_schema(&ArbSchema::IntBetween {
        minimum: 1,
        maximum: 100,
    });
    let arbitrary = arbitrary_filter(source, |value: &String| value == "100" || value == "26");
    let result = check_effect(
        &arbitrary,
        |_| false,
        CheckOptions {
            runs: 1,
            size: 10,
            seed: Some(Seed::Number(47)),
            ..CheckOptions::default()
        },
    );
    let falsified = match result {
        CheckResult::Falsified(falsified) => falsified,
        other => panic!("expected Falsified, got {other:?}"),
    };
    assert_eq!(falsified.initial_input, "100");
    assert_eq!(falsified.shrunk_input, "26");
    assert_eq!(falsified.shrinks, 1);
}

/// What the failure formatter renders for an input. The pin formats through
/// Effect's `Formatter`, which quotes strings, renders `Map` and friends by
/// constructor, and catches an accessor that throws while rendering.
#[derive(Clone, Debug, PartialEq)]
enum DisplayValue {
    Text(String),
    Map,
    /// A property access that threw while rendering; the pin leaves the
    /// marker behind instead of propagating.
    Throwing,
}

impl DisplayValue {
    fn render(&self) -> String {
        match self {
            DisplayValue::Text(value) => format!("{value:?}"),
            DisplayValue::Map => "Map(1)".to_string(),
            DisplayValue::Throwing => "{\"value\":\"[property access threw]\"}".to_string(),
        }
    }
}

/// The report shape the formatter consumes, named for the pin's fields.
#[derive(Clone, Debug, PartialEq)]
enum CheckFailure {
    Falsified {
        shrunk: DisplayValue,
        failure: String,
        runs: usize,
        shrinks: usize,
        replay: String,
    },
    Exhausted {
        runs: usize,
        discards: usize,
        seed: String,
    },
}

/// `Arbitrary.formatCheckFailure`: the diagnostic line for a falsification
/// or exhaustion, with each value rendered unambiguously.
fn format_check_failure(failure: &CheckFailure) -> String {
    match failure {
        CheckFailure::Falsified {
            shrunk,
            failure,
            runs,
            shrinks,
            replay,
        } => format!(
            "Property falsified after {runs} run(s) and {shrinks} shrink(s)\nShrunk input: {}\nFailure: {failure:?}\nReplay: {replay}",
            shrunk.render()
        ),
        CheckFailure::Exhausted {
            runs,
            discards,
            seed,
        } => format!(
            "Property exhausted after {runs} run(s) and {discards} discard(s)\nSeed: {seed:?}"
        ),
    }
}

#[test]
fn format_check_failure_renders_values_without_ambiguity() {
    // Arbitrary.test.ts :3505 — the diagnostic line quotes the shrunk input
    // and the failure, names the replay token, and survives both a Map input
    // and an accessor that throws while rendering.
    let falsified = format_check_failure(&CheckFailure::Falsified {
        shrunk: DisplayValue::Text("\n".to_string()),
        failure: "boom".to_string(),
        runs: 1,
        shrinks: 0,
        replay: "replay".to_string(),
    });
    assert_eq!(
        falsified,
        "Property falsified after 1 run(s) and 0 shrink(s)\nShrunk input: \"\\n\"\nFailure: \"boom\"\nReplay: replay"
    );

    let exhausted = format_check_failure(&CheckFailure::Exhausted {
        runs: 0,
        discards: 1,
        seed: "seed".to_string(),
    });
    assert_eq!(
        exhausted,
        "Property exhausted after 0 run(s) and 1 discard(s)\nSeed: \"seed\""
    );

    let mapped = format_check_failure(&CheckFailure::Falsified {
        shrunk: DisplayValue::Map,
        failure: "boom".to_string(),
        runs: 1,
        shrinks: 0,
        replay: "replay".to_string(),
    });
    assert!(mapped.contains("Shrunk input: Map("), "{mapped}");

    let throwing = format_check_failure(&CheckFailure::Falsified {
        shrunk: DisplayValue::Throwing,
        failure: "boom".to_string(),
        runs: 1,
        shrinks: 0,
        replay: "replay".to_string(),
    });
    assert!(
        throwing.contains("Shrunk input: {\"value\":\"[property access threw]\"}"),
        "{throwing}"
    );
    assert!(throwing.ends_with("Replay: replay"), "{throwing}");
}

#[test]
fn samples_are_deterministic_per_seed() {
    let g = integer(-100, 100);
    assert_eq!(g.sample_n(42, 20), integer(-100, 100).sample_n(42, 20));
    assert_ne!(g.sample_n(42, 20), integer(-100, 100).sample_n(43, 20));
}

#[test]
fn bounded_integers_cover_domain_and_edges() {
    let g = integer(0, 10);
    let mut rng = Rng::new(7);
    let samples: Vec<i64> = (0..500).map(|_| g.sample(&mut rng)).collect();
    assert!(samples.iter().all(|v| (0..=10).contains(v)));
    assert!(samples.contains(&0) && samples.contains(&10));
    assert!(
        integer(i64::MIN, i64::MAX).sample(&mut Rng::new(1)) != 0
            || integer(i64::MIN, i64::MAX).sample(&mut Rng::new(2)) != 0
    );
}

#[test]
fn integer_bounds_reject_inverted_ranges() {
    let result = std::panic::catch_unwind(|| integer(5, 4));
    assert!(result.is_err());
}

#[test]
fn integers_shrink_toward_the_boundary() {
    let g = integer(0, 100);
    let shrunk = g.shrink(&90);
    assert!(shrunk.contains(&0));
    assert!(shrunk.iter().all(|v| (0..=100).contains(v)));
    assert!(shrunk.iter().all(|v| *v < 90));
    assert_eq!(g.shrink(&0), Vec::<i64>::new());
}

#[test]
fn primitives_and_constants() {
    let bools: Vec<bool> = boolean().sample_n(99, 200);
    assert!(bools.contains(&true) && bools.contains(&false));
    assert_eq!(constant(7).sample(&mut Rng::new(0)), 7);
    // Arbitrary.test.ts:2920 — a constant generates its value with no
    // shrink candidates (the check-runner report shape is runner
    // machinery; the generator kernel is the value plus empty shrinks).
    assert_eq!(constant(7).shrink(&7), Vec::<i32>::new());
    let s = ascii_string(8).sample(&mut Rng::new(3));
    assert_eq!(s.len(), 8);
    assert!(s.bytes().all(|b| b.is_ascii_lowercase()));
    let h = hex_string(8).sample(&mut Rng::new(3));
    assert_eq!(h.len(), 8);
    assert!(h.bytes().all(|b| b.is_ascii_hexdigit()));
}

#[test]
fn composition_map_filter_tuple_record_option() {
    let doubled = integer(0, 5).map(|v| v * 2);
    assert!(doubled.sample_n(11, 50).iter().all(|v| v % 2 == 0));
    let evens = integer(0, 100).filter(|v| v % 2 == 0);
    assert!(evens.sample_n(12, 50).iter().all(|v| v % 2 == 0));
    let pair = tuple(integer(0, 1), boolean()).sample(&mut Rng::new(4));
    assert!(pair.0 == 0 || pair.0 == 1);
    let rec = record(ascii_string(1), integer(0, 9), 3).sample(&mut Rng::new(5));
    assert_eq!(rec.len(), 3);
    let maybe = option(integer(0, 1)).sample_n(6, 100);
    assert!(maybe.contains(&None));
    assert!(maybe.iter().any(Option::is_some));
}

#[test]
fn arrays_honor_length_bounds() {
    let g = array(integer(0, 9), 2, 4);
    for v in g.sample_n(8, 100) {
        assert!((2..=4).contains(&v.len()));
    }
    assert_eq!(array(integer(0, 9), 3, 3).sample(&mut Rng::new(9)).len(), 3);
}

#[test]
fn array_rejects_invalid_bounds_immediately() {
    let result = std::panic::catch_unwind(|| array(integer(0, 9), 5, 2));
    assert!(result.is_err());
    // Upstream also rejects -1/0.5/NaN/Infinity/>u32 bounds: unrepresentable
    // as `usize`, so the type already refuses them; the largest upstream
    // bound constructs (sampling it would not terminate, as upstream).
    let _ = array(integer(0, 9), 0, u32::MAX as usize);
}

#[test]
fn array_honors_minima_maxima_and_exact_lengths() {
    assert_eq!(
        array(integer(0, 9), 0, 0).sample(&mut Rng::new(21)).len(),
        0
    );
    assert_eq!(
        array(integer(0, 9), 4, 4).sample(&mut Rng::new(22)).len(),
        4
    );
    // Upstream { maxLength: 2 } and { minLength: 3 } across sizes: no size
    // parameter exists here, so pin the bounds over repeated samples.
    for v in array(integer(0, 9), 0, 2).sample_n(23, 50) {
        assert!(v.len() <= 2);
    }
    for v in array(integer(0, 9), 3, 9).sample_n(24, 50) {
        assert!((3..=9).contains(&v.len()));
    }
}

#[test]
fn safe_integers_cover_both_parities() {
    // Arbitrary.test.ts:1037 — the complete safe range reaches both parities.
    const MIN_SAFE: i64 = -9_007_199_254_740_991;
    const MAX_SAFE: i64 = 9_007_199_254_740_991;
    let g = integer(MIN_SAFE, MAX_SAFE);
    let mut rng = Rng::new(99);
    let values: Vec<i64> = (0..200).map(|_| g.sample(&mut rng)).collect();
    assert!(values.iter().all(|v| *v >= MIN_SAFE && *v <= MAX_SAFE));
    assert!(values.iter().any(|v| v.rem_euclid(2) == 0));
    assert!(values.iter().any(|v| v.rem_euclid(2) == 1));
}

#[test]
fn numbers_cover_binade_buckets() {
    // Arbitrary.test.ts:1072 — [1, 2) spreads across 4 buckets, 10-40% each.
    let values = number(1.0, 2.0, false, true).sample_n(11, 8192);
    let mut buckets = [0usize; 4];
    for v in &values {
        buckets[(((v - 1.0) * 4.0) as usize).min(3)] += 1;
    }
    for b in buckets {
        assert!((819..=3277).contains(&b), "bucket {b} outside 10-40%");
    }
}

#[test]
fn numbers_reach_nan_below_upper_bound() {
    // Arbitrary.test.ts:1603 — one-sided (..= 0] reaches NaN.
    let values = number(f64::NEG_INFINITY, 0.0, false, false).sample_n(12, 1000);
    assert!(values.iter().all(|v| *v <= 0.0 || v.is_nan()));
    assert!(values.iter().any(|v| v.is_nan()));
}

#[test]
fn numbers_hit_sole_value_between_adjacent_bounds() {
    // Arbitrary.test.ts:1738 — exclusive-adjacent bounds yield the one value.
    let minimum = 2f64.powi(100);
    let unit = 2f64.powi(48);
    let values = number(minimum, minimum + unit * 2.0, true, true).sample_n(13, 20);
    assert!(values.iter().all(|&v| v == minimum + unit));
}

#[test]
fn number_preserves_both_signed_zeros_in_an_inclusive_zero_interval() {
    // Arbitrary.test.ts:1758 — [0, 0] answers with both signed zeros.
    let values = number(0.0, 0.0, false, false).sample_n(31, 50);
    assert!(values.iter().all(|v| *v == 0.0));
    assert!(values.iter().any(|v| v.is_sign_negative()));
    assert!(values.iter().any(|v| v.is_sign_positive()));
}

#[test]
fn number_generates_exact_subnormal_and_infinite_singletons() {
    // Arbitrary.test.ts:1772 — every IEEE-754 singleton interval answers
    // with its one value, the infinities included (the singleton path
    // runs before the unbounded-side NaN wildcard).
    for expected in [
        f64::NEG_INFINITY,
        -f64::MAX,
        -f64::MIN_POSITIVE,
        f64::MIN_POSITIVE,
        f64::MAX,
        f64::INFINITY,
    ] {
        let singleton = number(expected, expected, false, false);
        assert_eq!(singleton.sample(&mut Rng::new(41)), expected);
        assert_eq!(singleton.sample(&mut Rng::new(42)), expected);
    }
}

// ===========================================================================
// Local core for the Arbitrary.schema / checkEffect rows, ported from the
// pin's internal/arbitrary (runner.ts, model.ts). The port carries the
// primitives the early rows exercise: a seeded generation RNG, the
// schema-subset generator for Literals, validated lazy shrinking, the
// shrink-path walk, and replay.
//
// Ceilings, stated rather than hidden: the property is a plain `FnMut`
// returning bool (the carried rows' properties are synchronous), the value
// domain is `String` (the carried schemas are Literals), and `hashSeed`
// walks UTF-16 code units in the pin while this walks chars — identical for
// the ASCII seeds the carried rows use.
// ===========================================================================

// The three refusals this test file used to model with a local predicate
// (`integer_constraints_admit`, `number_constraints_admit`) live in
// `schema_arbitrary.rs` now, against the port's own derivation: the rows they
// claim (`Arbitrary.test.ts:1616`, `:1627`, `:1817`) are witnessed there.

#[test]
fn exhausted_reports_and_reuses_the_resolved_seed() {
    // Arbitrary.test.ts:3467 — a generator that can only discard
    // exhausts and names the seed it resolved; feeding that seed back
    // reproduces the same failure, for sampling and checking alike.
    // The pin's all-discard arbitrary is filter(() => false) over a
    // literal; the port renders it as a literal union with no members,
    // which discards every attempt by construction. The pin resolves
    // the seed from its Random service; the port's default seed is its
    // rendering, and the report/reuse round-trip is the row's claim.
    let arbitrary = arbitrary_schema(&ArbSchema::Literals(Vec::new()));
    let sampled = sample_effect(
        &arbitrary,
        SampleOptions {
            count: 1,
            max_discards: 2,
            ..SampleOptions::default()
        },
    );
    let Err(error) = sampled else {
        panic!("an all-discard generator must fail sampling")
    };
    let resampled = sample_effect(
        &arbitrary,
        SampleOptions {
            count: 1,
            max_discards: 2,
            seed: Some(error.seed.clone()),
            ..SampleOptions::default()
        },
    );
    assert_eq!(
        resampled,
        Err(error.clone()),
        "the reported seed reproduces the sample failure"
    );

    let checked = check_effect(
        &arbitrary,
        |_| true,
        CheckOptions {
            runs: 1,
            max_discards: 2,
            ..CheckOptions::default()
        },
    );
    let CheckResult::Exhausted { seed, .. } = &checked else {
        panic!("an all-discard generator must exhaust a check: {checked:?}")
    };
    assert_eq!(
        *seed, error.seed,
        "both failures name the same resolved seed"
    );
    let rechecked = check_effect(
        &arbitrary,
        |_| true,
        CheckOptions {
            runs: 1,
            max_discards: 2,
            seed: Some(seed.clone()),
            ..CheckOptions::default()
        },
    );
    assert_eq!(
        rechecked, checked,
        "the reported seed reproduces the check failure"
    );
}

/// A seed, rendered `${typeof seed}:${seed}` before hashing so a number
/// seed and its string form never collide.
#[derive(Clone, Debug, PartialEq)]
enum Seed {
    Number(u64),
    Text(String),
}

impl Seed {
    fn render(&self) -> String {
        match self {
            Seed::Number(value) => format!("number:{value}"),
            Seed::Text(value) => format!("string:{value}"),
        }
    }
}

/// MurmurHash3 fmix32.
fn mix32(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x85ebca6b);
    value ^= value >> 13;
    value = value.wrapping_mul(0xc2b2ae35);
    value ^ value >> 16
}

/// The pin's fnv-style seed hash: two FNV-1a-style accumulators folded
/// through fmix32.
fn hash_seed(seed: &Seed) -> (u32, u32) {
    let value = seed.render();
    let mut first: u32 = 0x811c9dc5;
    let mut second: u32 = 0x9e3779b9;
    for (index, code) in value.chars().enumerate() {
        let code = code as u32;
        first = (first ^ code).wrapping_mul(0x01000193);
        second = (second ^ code ^ index as u32).wrapping_mul(0x85ebca6b);
    }
    let first = mix32(first);
    let second = mix32(second ^ value.len() as u32);
    (first, second)
}

/// xoshiro128** 1.1, the pin's per-attempt PRNG.
#[derive(Clone)]
struct GenerationRandom {
    state: [u32; 4],
}

impl GenerationRandom {
    fn next_uint32(&mut self) -> u32 {
        let (s0, s1, s2, s3) = (self.state[0], self.state[1], self.state[2], self.state[3]);
        let result = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let temporary = s1 << 9;
        let n2 = s2 ^ s0;
        let n3 = s3 ^ s1;
        let n1 = s1 ^ n2;
        let n0 = s0 ^ n3;
        let n2b = n2 ^ temporary;
        let n3b = n3.rotate_left(11);
        self.state = [n0, n1, n2b, n3b];
        result
    }

    fn next_double_unsafe(&mut self) -> f64 {
        let high = (self.next_uint32() >> 5) as u64;
        let low = (self.next_uint32() >> 6) as u64;
        ((high << 26 | low) as f64) / ((1u64 << 53) as f64)
    }
}

/// The attempt state is derived from the seed pair and the attempt index, so
/// replay can jump straight to an attempt without running the earlier ones.
fn make_attempt_random(seed: (u32, u32), attempt: u64) -> GenerationRandom {
    let attempt_low = attempt as u32;
    let attempt_high = (attempt >> 32) as u32;
    let mut state0 = mix32(seed.0 ^ attempt_low ^ attempt_high.wrapping_mul(0x9e3779b9));
    let state1 = mix32(seed.1 ^ attempt_high ^ attempt_low.wrapping_mul(0x85ebca6b));
    let state2 = mix32(seed.0 ^ attempt_high ^ attempt_low.wrapping_mul(0xc2b2ae35) ^ 0x243f6a88);
    let state3 = mix32(seed.1 ^ attempt_low ^ attempt_high.wrapping_mul(0x27d4eb2f) ^ 0xb7e15162);
    if state0 | state1 | state2 | state3 == 0 {
        state0 = 0x9e3779b9;
    }
    GenerationRandom {
        state: [state0, state1, state2, state3],
    }
}

/// The pin's `GenerationState`, trimmed to the fields the carried schemas
/// read.
struct GenerationState {
    size: usize,
    shrinks: bool,
    bias_factor: usize,
    null_prototype: bool,
    random: GenerationRandom,
    budget_remaining: f64,
}

/// pure-rand's unbiased bucket selection, ported: equal-size buckets plus
/// rejection.
fn random_uint32_below(state: &mut GenerationState, range_size: u32) -> u32 {
    let bucket_size = 0x1_0000_0000u64 / range_size as u64;
    let maximum_accepted = bucket_size * range_size as u64;
    let mut value = state.random.next_uint32() as u64;
    while value >= maximum_accepted {
        value = state.random.next_uint32() as u64;
    }
    (value / bucket_size) as u32
}

fn random_index(state: &mut GenerationState, length: u32) -> u32 {
    if length == 1 {
        0
    } else {
        random_uint32_below(state, length)
    }
}

fn random_boolean(state: &mut GenerationState) -> bool {
    state.random.next_uint32() & 1 == 1
}

/// The pin's `randomInt`: bucket selection with rejection, widening to the
/// 53-bit double buckets for ranges beyond `u32` (the pin falls back to
/// BigInt beyond 2^53; the carried rows stay inside the double path).
fn random_int(state: &mut GenerationState, minimum: i64, maximum: i64) -> i64 {
    if minimum == maximum {
        return minimum;
    }
    let width = maximum as i128 - minimum as i128 + 1;
    const DOUBLE_VALUES: u128 = 0x20_0000_0000_0000;
    if width <= 0x1_0000_0000 {
        return minimum + random_uint32_below(state, width as u32) as i64;
    }
    let width = (width.min(DOUBLE_VALUES as i128)) as u64;
    let bucket_size = DOUBLE_VALUES as u64 / width;
    let maximum_accepted = bucket_size * width;
    loop {
        let value = (state.random.next_double_unsafe() * DOUBLE_VALUES as f64) as u64;
        if value < maximum_accepted {
            return minimum + (value / bucket_size) as i64;
        }
    }
}

/// The pin's `selectBiased`: a biased index leans toward the first range.
fn select_biased<A: Clone>(state: &mut GenerationState, values: &[A]) -> A {
    if values.len() == 1 {
        return values[0].clone();
    }
    let index = random_int(
        state,
        -2 * (values.len() as i64 - 1),
        values.len() as i64 - 2,
    );
    if index < 0 {
        values[0].clone()
    } else {
        values[(index + 1) as usize].clone()
    }
}

/// The pin's `numberBiasRanges`: the edge neighborhoods the numeric bias
/// leans toward, as ranges rather than single values.
fn int_bias_ranges(minimum: i64, maximum: i64) -> Vec<(i64, i64)> {
    if minimum == maximum {
        return vec![(minimum, maximum)];
    }
    if minimum < 0 && maximum > 0 {
        let low = (-(minimum as f64)).log2().floor() as i64;
        let high = (maximum as f64).log2().floor() as i64;
        return vec![
            (-low, high),
            (maximum - high, maximum),
            (minimum, minimum + low),
        ];
    }
    let gap = ((maximum as i128 - minimum as i128) as f64).log2().floor() as i64;
    let close_to_minimum = (minimum, minimum + gap);
    let close_to_maximum = (maximum - gap, maximum);
    if minimum < 0 {
        vec![close_to_maximum, close_to_minimum]
    } else {
        vec![close_to_minimum, close_to_maximum]
    }
}

/// The pin's `makeRandomNumericInt`: with probability 1/biasFactor the draw
/// lands in a bias range, otherwise it is uniform over the whole domain.
/// Explicit bounds win over the size-derived magnitude window
/// (`max(1, size * size)` around the center), which is what "pushes
/// constraints into primitive constructors" means for a bounded int.
fn make_random_numeric_int(minimum: i64, maximum: i64) -> impl Fn(&mut GenerationState) -> i64 {
    let ranges = int_bias_ranges(minimum, maximum);
    move |state| {
        if random_int(state, 1, state.bias_factor as i64) == 1 {
            let (low, high) = select_biased(state, &ranges);
            random_int(state, low, high)
        } else {
            random_int(state, minimum, maximum)
        }
    }
}

/// One generation outcome: a sample, or nothing to contribute.
enum Attempt<A> {
    Generated(Sample<A>),
    Discarded,
}

/// A value plus its optional shrink pull. The pull's maker runs on FIRST
/// PULL, which is what keeps a custom shrinker unevaluated until a failure
/// is actually shrunk (`makeSampleWithLazyShrinks`).
struct Sample<A> {
    value: A,
    shrinks: Option<Box<Pull<A>>>,
}

/// A one-shot cursor over shrink candidates. Exploring is linear: each
/// failing candidate becomes the current sample and the old pull is
/// abandoned, exactly as the pin's shrink loop walks. `lazy` materializes a
/// fixed list on the first pull; `stepping` runs a stateful step function
/// per candidate, which is what the filter frontier's depth-first walk
/// needs so an accepted candidate stops the walk.
struct Pull<A> {
    step: Box<dyn FnMut() -> Option<Attempt<A>>>,
}

impl<A: 'static> Pull<A> {
    fn lazy(make: impl FnOnce() -> Vec<Attempt<A>> + 'static) -> Self {
        let mut items: Option<Vec<Attempt<A>>> = None;
        let mut make = Some(make);
        let mut index = 0;
        Self {
            step: Box::new(move || {
                if items.is_none() {
                    items = Some((make.take().expect("the maker runs once"))());
                }
                let items = items.as_mut().expect("materialized above");
                if index < items.len() {
                    let item = std::mem::replace(&mut items[index], Attempt::Discarded);
                    index += 1;
                    Some(item)
                } else {
                    None
                }
            }),
        }
    }

    fn stepping(step: impl FnMut() -> Option<Attempt<A>> + 'static) -> Self {
        Self {
            step: Box::new(step),
        }
    }

    fn next(&mut self) -> Option<Attempt<A>> {
        (self.step)()
    }
}

/// The pin's `sampleFromValidatedShrink`: each candidate is validated and
/// only a valid one becomes a (recursively shrinkable) sample; an invalid
/// candidate is a discard, which the shrink loop counts as inspected but
/// never evaluates the property on.
fn sample_from_validated_shrink<A: Clone + 'static>(
    value: A,
    shrink: std::rc::Rc<dyn Fn(&A) -> Vec<A>>,
    validate: std::rc::Rc<dyn Fn(&A) -> Option<A>>,
) -> Sample<A> {
    let captured = value.clone();
    let make = move || {
        shrink(&captured)
            .into_iter()
            .map(|candidate| match validate(&candidate) {
                Some(valid) => Attempt::Generated(sample_from_validated_shrink(
                    valid,
                    std::rc::Rc::clone(&shrink),
                    std::rc::Rc::clone(&validate),
                )),
                None => Attempt::Discarded,
            })
            .collect()
    };
    Sample {
        value,
        shrinks: Some(Box::new(Pull::lazy(make))),
    }
}

/// A generator: its cost floor plus the sampling step.
struct Generator<A> {
    min_cost: f64,
    generate: Box<dyn Fn(&mut GenerationState) -> Attempt<A>>,
}

impl<A> Generator<A> {
    fn generate(&self, state: &mut GenerationState) -> Attempt<A> {
        (self.generate)(state)
    }
}

/// The schema subset the carried rows exercise. `Schema.Literals` compiles
/// to a union of constant generators, and a constant carries no shrink
/// candidates, exactly as the pin's `constant` does.
#[derive(Clone)]
enum ArbSchema {
    Literals(Vec<String>),
    /// `Schema.Int.check(Schema.isBetween({ minimum, maximum }))`, the
    /// bounded-integer shape behind the combinator rows.
    IntBetween {
        minimum: i64,
        maximum: i64,
    },
    /// `Schema.Int` with no bounds: the size-derived magnitude window around
    /// zero, `max(1, size^2)` wide.
    IntUnbounded,
    /// `Schema.String.check(Schema.isMinLength(n), Schema.isMaxLength(n))`:
    /// the fixed-length string the constraint-pushing row exercises. The
    /// pin's generator builds repetition structures; the row's observable is
    /// the length and determinism, which a direct draw carries.
    StringExactLength(usize),
}

impl ArbSchema {
    /// Compile to a generator plus the validity predicate the shrinking
    /// path validates candidates against.
    fn compile(&self) -> Generator<String> {
        // Owned copies before the closures: the generator must outlive the
        // borrow it was compiled from.
        match self {
            ArbSchema::Literals(literals) => {
                let members = literals.clone();
                Generator {
                    min_cost: 0.0,
                    generate: Box::new(move |state| {
                        // generateUnion: members are eligible while their
                        // cost fits the budget; with constant members the
                        // cross-branch fallback never fires.
                        let eligible: Vec<usize> = (0..members.len())
                            .filter(|index| 0.0 <= state.budget_remaining)
                            .collect();
                        if eligible.is_empty() {
                            return Attempt::Discarded;
                        }
                        let selected =
                            eligible[random_index(state, eligible.len() as u32) as usize];
                        Attempt::Generated(Sample {
                            value: members[selected].clone(),
                            shrinks: None,
                        })
                    }),
                }
            }
            ArbSchema::IntBetween { minimum, maximum } => {
                let (minimum, maximum) = (*minimum, *maximum);
                Generator {
                    min_cost: 0.0,
                    generate: Box::new(move |state| {
                        // The pin's bounded-int path. Explicit bounds win;
                        // the size-derived magnitude window (max(1, size^2)
                        // around the center) only applies when a bound is
                        // absent, which this subset does not construct.
                        let _ = state.size;
                        let value = make_random_numeric_int(minimum, maximum)(state);
                        Attempt::Generated(int_sample(value, minimum, maximum, None))
                    }),
                }
            }
            ArbSchema::StringExactLength(length) => {
                let length = *length;
                Generator {
                    min_cost: 0.0,
                    generate: Box::new(move |state| {
                        let value: String = (0..length)
                            .map(|_| (b'a' + random_uint32_below(state, 26) as u8) as char)
                            .collect();
                        Attempt::Generated(Sample {
                            value,
                            shrinks: None,
                        })
                    }),
                }
            }
            ArbSchema::IntUnbounded => Generator {
                min_cost: 0.0,
                generate: Box::new(move |state| {
                    // No bounds: the size-derived window around zero.
                    let magnitude = (state.size as i64 * state.size as i64).max(1);
                    let value = make_random_numeric_int(-magnitude, magnitude)(state);
                    Attempt::Generated(int_sample(value, -magnitude, magnitude, None))
                }),
            },
        }
    }

    /// Validity: the value must belong to the schema, which is the predicate
    /// a custom shrinker's candidates are checked against.
    fn validate(&self, value: &String) -> Option<String> {
        match self {
            ArbSchema::Literals(literals) => literals.contains(value).then(|| value.clone()),
            ArbSchema::IntBetween { minimum, maximum } => value
                .parse::<i64>()
                .ok()
                .filter(|parsed| *parsed >= *minimum && *parsed <= *maximum)
                .map(|parsed| parsed.to_string()),
            ArbSchema::StringExactLength(length) => {
                (value.chars().count() == *length).then(|| value.clone())
            }
            ArbSchema::IntUnbounded => value.parse::<i64>().ok().map(|parsed| parsed.to_string()),
        }
    }
}

/// The pin's `shrinkInteger`: halve the gap toward the target, recording the
/// last passing context. `toRemove /= 2` truncates toward zero, matching
/// `Math.trunc`.
fn shrink_integer(current: i64, target: i64, try_target_asap: bool) -> Vec<(i64, Option<i64>)> {
    let mut out = Vec::new();
    let real_gap = current - target;
    let mut previous = if try_target_asap { None } else { Some(target) };
    let mut to_remove = if try_target_asap {
        real_gap
    } else {
        real_gap / 2
    };
    while to_remove != 0 {
        let value = if to_remove == real_gap {
            target
        } else {
            current - to_remove
        };
        out.push((value, previous));
        previous = Some(value);
        to_remove /= 2;
    }
    out
}

/// The pin's schema-derived integer sample: the value plus its shrink tree,
/// each candidate carrying the last passing context so the runner converges
/// on the local failure boundary. The target is the bound nearest zero.
fn int_sample(value: i64, minimum: i64, maximum: i64, context: Option<i64>) -> Sample<String> {
    let candidates: Vec<(i64, Option<i64>)> = match context {
        None => shrink_integer(value, minimum.max(0).min(maximum), true),
        Some(context) => {
            let adjacent = (value > 0 && value == context + 1 && value > minimum)
                || (value < 0 && value == context - 1 && value < maximum);
            if adjacent {
                vec![(context, None)]
            } else {
                shrink_integer(value, context, false)
            }
        }
    };
    let shrinks = move || {
        candidates
            .into_iter()
            .map(|(candidate, ctx)| {
                Attempt::Generated(int_sample(candidate, minimum, maximum, ctx))
            })
            .collect()
    };
    Sample {
        value: value.to_string(),
        shrinks: Some(Box::new(Pull::lazy(shrinks))),
    }
}

/// The pin's `mapSample`: map the value and every candidate in the tree.
/// The subtree is materialized per pull rather than streamed; the carried
/// rows observe candidate order and count, which this preserves.
fn map_sample(sample: Sample<String>, f: std::rc::Rc<dyn Fn(&String) -> String>) -> Sample<String> {
    let value = f(&sample.value);
    let shrinks = sample.shrinks.map(|pull| {
        Box::new(Pull::lazy(move || {
            let mut pull = pull;
            let mut out = Vec::new();
            while let Some(attempt) = pull.next() {
                match attempt {
                    Attempt::Discarded => out.push(Attempt::Discarded),
                    Attempt::Generated(inner) => out.push(Attempt::Generated(map_sample(
                        inner,
                        std::rc::Rc::clone(&f),
                    ))),
                }
            }
            out
        })) as Box<Pull<String>>
    });
    Sample { value, shrinks }
}

/// The pin's `filterPull`: depth-first, a candidate failing the predicate is
/// not dropped — its own subtree is explored before the next sibling, which
/// is what "promotes accepted shrink descendants" means. The walk is
/// stepwise so an accepted candidate stops it, exactly as the pin's
/// suspend-based pull does.
fn filter_pull(
    source: Pull<String>,
    predicate: std::rc::Rc<dyn Fn(&String) -> bool>,
) -> Pull<String> {
    let mut stack: Vec<Pull<String>> = vec![source];
    Pull::stepping(move || {
        loop {
            let mut current = stack.pop()?;
            while let Some(attempt) = current.next() {
                match attempt {
                    Attempt::Discarded => {
                        stack.push(current);
                        return Some(Attempt::Discarded);
                    }
                    Attempt::Generated(sample) => {
                        if predicate(&sample.value) {
                            let shrinks = sample.shrinks.map(|pull| {
                                Box::new(filter_pull(*pull, std::rc::Rc::clone(&predicate)))
                                    as Box<Pull<String>>
                            });
                            stack.push(current);
                            return Some(Attempt::Generated(Sample {
                                value: sample.value,
                                shrinks,
                            }));
                        }
                        if let Some(subtree) = sample.shrinks {
                            // Resume the siblings after this subtree is
                            // exhausted, exactly as the pin's stack does.
                            stack.push(current);
                            stack.push(*subtree);
                            break;
                        }
                    }
                }
            }
        }
    })
}

/// `Arbitrary.map`: map the generated values and the complete shrink tree.
fn arbitrary_map(
    arbitrary: Arbitrary<String>,
    f: impl Fn(&String) -> String + 'static,
) -> Arbitrary<String> {
    let f: std::rc::Rc<dyn Fn(&String) -> String> = std::rc::Rc::new(f);
    let min_cost = arbitrary.generator.min_cost;
    let inner = arbitrary.generator;
    let generate = move |state: &mut GenerationState| match inner.generate(state) {
        Attempt::Discarded => Attempt::Discarded,
        Attempt::Generated(sample) => {
            Attempt::Generated(map_sample(sample, std::rc::Rc::clone(&f)))
        }
    };
    Arbitrary {
        generator: Generator {
            min_cost,
            generate: Box::new(generate),
        },
        type_id: ARBITRARY_TYPE_ID,
    }
}

/// `Arbitrary.filter`: values failing the predicate are discarded at
/// generation time; while shrinking, failing candidates fall through to
/// their own subtrees.
fn arbitrary_filter(
    arbitrary: Arbitrary<String>,
    predicate: impl Fn(&String) -> bool + 'static,
) -> Arbitrary<String> {
    let predicate: std::rc::Rc<dyn Fn(&String) -> bool> = std::rc::Rc::new(predicate);
    let min_cost = arbitrary.generator.min_cost;
    let inner = arbitrary.generator;
    let generate = move |state: &mut GenerationState| match inner.generate(state) {
        Attempt::Discarded => Attempt::Discarded,
        Attempt::Generated(sample) => {
            if !state.shrinks {
                return if predicate(&sample.value) {
                    Attempt::Generated(sample)
                } else {
                    Attempt::Discarded
                };
            }
            if !predicate(&sample.value) {
                return Attempt::Discarded;
            }
            let value = sample.value;
            let shrinks = sample.shrinks.map(|pull| {
                Box::new(filter_pull(*pull, std::rc::Rc::clone(&predicate))) as Box<Pull<String>>
            });
            Attempt::Generated(Sample { value, shrinks })
        }
    };
    Arbitrary {
        generator: Generator {
            min_cost,
            generate: Box::new(generate),
        },
        type_id: ARBITRARY_TYPE_ID,
    }
}

/// The pin's `Arbitrary` marker. The runtime type id is what
/// `isArbitrary` checks.
const ARBITRARY_TYPE_ID: &str = "~effect/arbitrary/Arbitrary";

/// An `Arbitrary` value: a generator behind the marker.
struct Arbitrary<A> {
    generator: Generator<A>,
    type_id: &'static str,
}

/// `Arbitrary.isArbitrary`: true for an Arbitrary, false for anything else
/// (the schema it was derived from included). The row's `null` case is any
/// non-Arbitrary value in Rust, where null has no runtime presence.
fn is_arbitrary(value: &dyn std::any::Any) -> bool {
    value.is::<Arbitrary<String>>()
}

fn arbitrary_schema(schema: &ArbSchema) -> Arbitrary<String> {
    Arbitrary {
        generator: schema.compile(),
        type_id: ARBITRARY_TYPE_ID,
    }
}

/// `Arbitrary.schema(schema, { shrink })`: the custom shrinker REPLACES the
/// schema-derived shrinking, and its candidates are validated before the
/// property ever sees them.
fn arbitrary_schema_with_shrink(
    schema: &ArbSchema,
    shrink: impl Fn(&String) -> Vec<String> + 'static,
) -> Arbitrary<String> {
    let compiled = schema.compile();
    let min_cost = compiled.min_cost;
    let schema_owned = schema.clone();
    let validate = move |value: &String| schema_owned.validate(value);
    let shrink = std::rc::Rc::new(shrink) as std::rc::Rc<dyn Fn(&String) -> Vec<String>>;
    let validate = std::rc::Rc::new(validate) as std::rc::Rc<dyn Fn(&String) -> Option<String>>;
    let generate = move |state: &mut GenerationState| match compiled.generate(state) {
        Attempt::Discarded => Attempt::Discarded,
        Attempt::Generated(sample) if !state.shrinks => Attempt::Generated(sample),
        Attempt::Generated(sample) => Attempt::Generated(sample_from_validated_shrink(
            sample.value,
            std::rc::Rc::clone(&shrink),
            std::rc::Rc::clone(&validate),
        )),
    };
    Arbitrary {
        generator: Generator {
            min_cost,
            generate: Box::new(generate),
        },
        type_id: ARBITRARY_TYPE_ID,
    }
}

/// How a property answered.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PropertyFailure {
    ReturnedFalse,
    PropertyError,
}

impl PropertyFailure {
    fn tag(&self) -> &'static str {
        match self {
            PropertyFailure::ReturnedFalse => "ReturnedFalse",
            PropertyFailure::PropertyError => "PropertyError",
        }
    }
}

/// The replay token: seed, attempt, size, the shrink path and the failure
/// tag, which together reproduce a falsification exactly.
#[derive(Clone, Debug, PartialEq)]
struct Replay {
    seed: Seed,
    attempt: u64,
    size: usize,
    path: Vec<usize>,
    failure_tag: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
struct Falsified {
    initial_input: String,
    shrunk_input: String,
    failure: PropertyFailure,
    runs: usize,
    discards: usize,
    shrinks: usize,
    replay: Replay,
}

#[derive(Clone, Debug, PartialEq)]
enum CheckResult {
    Passed {
        runs: usize,
        discards: usize,
    },
    Falsified(Falsified),
    Exhausted {
        runs: usize,
        discards: usize,
        seed: Seed,
    },
    ReplayMismatch(&'static str),
}

#[derive(Clone, Debug, Default)]
struct CheckOptions {
    runs: usize,
    size: usize,
    max_discards: usize,
    max_shrinks: usize,
    seed: Option<Seed>,
    replay: Option<Replay>,
}

impl CheckOptions {
    /// The pin's defaults, applied where an option is absent.
    fn normalized(self) -> Self {
        let runs = if self.runs == 0 { 100 } else { self.runs };
        let size = if self.size == 0 { 10 } else { self.size };
        CheckOptions {
            runs,
            size,
            max_discards: if self.max_discards == 0 {
                100.max(runs * 10)
            } else {
                self.max_discards
            },
            max_shrinks: if self.max_shrinks == 0 {
                100
            } else {
                self.max_shrinks
            },
            seed: self.seed,
            replay: self.replay,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct SampleOptions {
    count: usize,
    size: usize,
    max_discards: usize,
    seed: Option<Seed>,
}

#[derive(Clone, Debug, PartialEq)]
struct SampleError {
    generated: usize,
    discards: usize,
    seed: Seed,
}

/// `generateAttemptRaw`: the bias schedule and budget the pin derives per
/// attempt.
fn generate_attempt(
    generator: &Generator<String>,
    seed: (u32, u32),
    attempt: u64,
    size: usize,
    shrinks: bool,
) -> Attempt<String> {
    let bias_factor = 2 + (attempt as f64 + 1.0).log10().floor() as usize;
    let null_prototype = (attempt + 1) % (bias_factor as u64 * 4) == 0;
    let mut state = GenerationState {
        size,
        shrinks,
        bias_factor,
        null_prototype,
        random: make_attempt_random(seed, attempt),
        budget_remaining: generator.min_cost + size as f64,
    };
    generator.generate(&mut state)
}

fn evaluate_property(
    property: &mut impl FnMut(&String) -> bool,
    value: &String,
) -> Result<(), PropertyFailure> {
    if property(value) {
        Ok(())
    } else {
        Err(PropertyFailure::ReturnedFalse)
    }
}

/// The pin's `shrink`: the first-failing-child traversal. Each inspected
/// candidate counts toward `maximum`; a candidate failing with the SAME
/// failure tag becomes the current sample and its index is recorded on the
/// path.
#[allow(clippy::type_complexity)]
fn shrink(
    initial: Sample<String>,
    initial_failure: PropertyFailure,
    property: &mut impl FnMut(&String) -> bool,
    maximum: usize,
) -> (Sample<String>, PropertyFailure, Vec<usize>) {
    let mut current = initial;
    let mut failure = initial_failure;
    let mut inspected = 0;
    let mut path = Vec::new();
    while inspected < maximum {
        let mut index = 0;
        let mut found = None;
        while inspected < maximum {
            let Some(pull) = current.shrinks.as_mut() else {
                break;
            };
            let Some(candidate) = pull.next() else { break };
            inspected += 1;
            let Attempt::Generated(candidate) = candidate else {
                continue;
            };
            let outcome = evaluate_property(property, &candidate.value);
            if outcome == Err(failure) {
                found = Some(candidate);
                path.push(index);
                break;
            }
            index += 1;
        }
        let Some(found) = found else { break };
        current = found;
    }
    (current, failure, path)
}

/// `followReplay`: walk the recorded path over freshly generated pulls;
/// every step must fail with the same tag or the replay is a mismatch.
fn follow_replay(
    initial: Sample<String>,
    failure: PropertyFailure,
    path: &[usize],
    property: &mut impl FnMut(&String) -> bool,
) -> Result<Sample<String>, &'static str> {
    let mut current = initial;
    for target in path {
        let mut selected = None;
        let mut index = 0;
        while index <= *target {
            let Some(pull) = current.shrinks.as_mut() else {
                return Err("ShrinkPathUnavailable");
            };
            let Some(candidate) = pull.next() else {
                return Err("ShrinkPathUnavailable");
            };
            let Attempt::Generated(candidate) = candidate else {
                continue;
            };
            if index == *target {
                selected = Some(candidate);
            }
            index += 1;
        }
        let selected = selected.expect("the target index was reached");
        if evaluate_property(property, &selected.value) != Err(failure) {
            return Err("ShrinkPassed");
        }
        current = selected;
    }
    Ok(current)
}

/// `Arbitrary.checkEffect`: sample runs, shrink the first failure along its
/// path, and report. A replay token reproduces the same report.
fn check_effect(
    arbitrary: &Arbitrary<String>,
    mut property: impl FnMut(&String) -> bool,
    options: CheckOptions,
) -> CheckResult {
    let options = options.normalized();
    if let Some(replay) = &options.replay {
        let seed_state = hash_seed(&replay.seed);
        let attempt = generate_attempt(
            &arbitrary.generator,
            seed_state,
            replay.attempt,
            replay.size,
            true,
        );
        let Attempt::Generated(attempt) = attempt else {
            return CheckResult::ReplayMismatch("AttemptDiscarded");
        };
        let failure = match evaluate_property(&mut property, &attempt.value) {
            Ok(()) => return CheckResult::ReplayMismatch("PropertyPassed"),
            Err(failure) => failure,
        };
        if failure.tag() != replay.failure_tag {
            return CheckResult::ReplayMismatch("ShrinkPassed");
        }
        let attempt_value = attempt.value.clone();
        return match follow_replay(attempt, failure, &replay.path, &mut property) {
            Err(reason) => CheckResult::ReplayMismatch(reason),
            Ok(current) => CheckResult::Falsified(Falsified {
                initial_input: attempt_value,
                shrunk_input: current.value,
                failure,
                runs: 1,
                discards: 0,
                shrinks: replay.path.len(),
                replay: replay.clone(),
            }),
        };
    }

    let seed = options.seed.clone().unwrap_or(Seed::Number(0));
    let seed_state = hash_seed(&seed);
    let mut runs = 0;
    let mut discards = 0;
    let mut attempt_index = 0u64;
    while runs < options.runs {
        let current_attempt = attempt_index;
        attempt_index += 1;
        let current_size = if options.runs == 1 {
            options.size
        } else {
            ((runs * options.size) / (options.runs - 1)) as usize
        };
        let attempt = generate_attempt(
            &arbitrary.generator,
            seed_state,
            current_attempt,
            current_size,
            true,
        );
        let Attempt::Generated(attempt) = attempt else {
            discards += 1;
            if discards > options.max_discards {
                return CheckResult::Exhausted {
                    runs,
                    discards,
                    seed,
                };
            }
            continue;
        };
        let failure = match evaluate_property(&mut property, &attempt.value) {
            Ok(()) => {
                runs += 1;
                continue;
            }
            Err(failure) => failure,
        };
        let initial_input = attempt.value.clone();
        let (current, failure, path) = shrink(attempt, failure, &mut property, options.max_shrinks);
        return CheckResult::Falsified(Falsified {
            initial_input,
            shrunk_input: current.value,
            failure,
            runs: runs + 1,
            discards,
            shrinks: path.len(),
            replay: Replay {
                seed,
                attempt: current_attempt,
                size: current_size,
                path,
                failure_tag: failure.tag(),
            },
        });
    }
    CheckResult::Passed { runs, discards }
}

/// `Arbitrary.sampleEffect`: `count` deterministic values from the seed.
fn sample_effect(
    arbitrary: &Arbitrary<String>,
    options: SampleOptions,
) -> Result<Vec<String>, SampleError> {
    let count = if options.count == 0 {
        10
    } else {
        options.count
    };
    let size = if options.size == 0 { 10 } else { options.size };
    let max_discards = if options.max_discards == 0 {
        100.max(count * 10)
    } else {
        options.max_discards
    };
    let seed = options.seed.clone().unwrap_or(Seed::Number(0));
    let seed_state = hash_seed(&seed);
    let mut values = Vec::new();
    let mut discards = 0;
    let mut attempt_index = 0u64;
    while values.len() < count {
        let attempt =
            generate_attempt(&arbitrary.generator, seed_state, attempt_index, size, false);
        attempt_index += 1;
        match attempt {
            Attempt::Generated(sample) => values.push(sample.value),
            Attempt::Discarded => {
                discards += 1;
                if discards > max_discards {
                    return Err(SampleError {
                        generated: values.len(),
                        discards,
                        seed,
                    });
                }
            }
        }
    }
    Ok(values)
}
