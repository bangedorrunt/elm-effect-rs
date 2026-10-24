//! Random draws: ranges, the integer forms, shuffle, choice, and seeds.

use elm_effect::{Random, RandomError};

#[test]
fn random_next_and_boolean_match_the_pin() {
    // Random.test.ts:8 — next is in [0, 1).
    let random = Random::with_seed(1);
    for _ in 0..100 {
        let value = random.next_unit();
        assert!((0.0..1.0).contains(&value), "{value} left [0, 1)");
    }

    // :18, :35 — nextBoolean uses a strict greater-than threshold, so exactly
    // 0.5 is false.
    let draws = Random::from_draws([0.5, 0.500001, 0.1]);
    assert_eq!(
        [
            draws.next_boolean(),
            draws.next_boolean(),
            draws.next_boolean()
        ],
        [false, true, false]
    );

    // :25 — the same seed gives the same three booleans.
    let three = |seed: u64| {
        let random = Random::with_seed(seed);
        [
            random.next_boolean(),
            random.next_boolean(),
            random.next_boolean(),
        ]
    };
    assert_eq!(three(7), three(7));
}

#[test]
fn random_int_and_between_match_the_pin() {
    // :52 — nextInt is a safe integer.
    let random = Random::with_seed(3);
    for _ in 0..100 {
        let value = random.next_int();
        assert!(
            (-(2_i64.pow(53) - 1)..=(2_i64.pow(53) - 1)).contains(&value),
            "{value} left the safe range"
        );
    }

    // :63 — a draw that rounds up to the bound steps down instead of
    // returning it.
    let positive = Random::from_draws([1.0 - f64::EPSILON]);
    assert_eq!(positive.next_between(10.0, 20.0), 20.0 - 2f64.powi(-48));

    let adjacent = Random::from_draws([0.75]);
    assert_eq!(adjacent.next_between(1e16, 1e16 + 2.0), 1e16);

    let negative = Random::from_draws([0.75]);
    assert_eq!(negative.next_between(-1e16 - 2.0, -1e16), -1e16 - 2.0);

    let zero = Random::from_draws([0.75]);
    assert_eq!(
        zero.next_between(-f64::from_bits(1), 0.0),
        -f64::from_bits(1)
    );

    // :84, :96 — equal bounds pass through, and an infinite bound is left
    // alone rather than stepped down.
    assert_eq!(Random::from_draws([0.75]).next_between(10.0, 10.0), 10.0);
    assert_eq!(
        Random::from_draws([0.75]).next_between(10.0, f64::INFINITY),
        f64::INFINITY
    );

    // :108, :129 — the seeded draw stays in its half-open range, including
    // when the range is negative.
    let random = Random::with_seed(5);
    for _ in 0..100 {
        assert!((10.0..20.0).contains(&random.next_between(10.0, 20.0)));
        assert!((-10.0..10.0).contains(&random.next_between(-10.0, 10.0)));
    }

    // :117 — the bounds are not rounded.
    assert_eq!(Random::from_draws([0.75]).next_between(10.5, 20.5), 18.0);
}

#[test]
fn random_int_between_match_the_pin() {
    // :139 — the closed form includes both bounds.
    let random = Random::with_seed(9);
    for _ in 0..100 {
        assert!((1..=6).contains(&random.next_int_between(1, 6)));
    }

    // :149 — the half-open form excludes the upper bound.
    for _ in 0..100 {
        assert!((1..6).contains(&random.next_int_between_half_open(1, 6)));
    }

    // :161 — a draw at the top of the range still lands below the bound.
    assert_eq!(
        Random::from_draws([1.0 - f64::EPSILON]).next_int_between_half_open(1, 6),
        5
    );
}

#[test]
fn random_shuffle_and_choice_match_the_pin() {
    // :175 — shuffle hands back a copy holding every value, and leaves the
    // input in place.
    let input = [1, 2, 3, 4, 5];
    let shuffled = Random::with_seed(21).shuffle(&input);
    assert_eq!(input, [1, 2, 3, 4, 5]);
    let mut sorted = shuffled.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, input);

    // :185 — the same seed shuffles the same way.
    assert_eq!(
        Random::with_seed(21).shuffle(&input),
        Random::with_seed(21).shuffle(&input)
    );

    // :197 — the injected draw picks the last element.
    let random = Random::from_draws([0.75]);
    assert_eq!(random.choice(&["a", "b", "c"]), Ok(&"c"));

    // :209 — an empty slice fails with the pin's message.
    let error = Random::with_seed(1).choice::<i32>(&[]).unwrap_err();
    assert_eq!(error, RandomError::NoSuchElement);
    assert_eq!(
        error.message(),
        "Cannot select a random element from an empty array"
    );

    // :217 — the same seed picks the same elements.
    let picks = |seed: u64| {
        let random = Random::with_seed(seed);
        [
            *random.choice(&[1, 2, 3, 4, 5]).unwrap(),
            *random.choice(&[1, 2, 3, 4, 5]).unwrap(),
            *random.choice(&[1, 2, 3, 4, 5]).unwrap(),
        ]
    };
    assert_eq!(picks(33), picks(33));
}

#[test]
fn random_seeds_match_the_pin() {
    // :233, :306 — the same seed produces the same sequence, numeric or not.
    let sequence = |random: &Random| {
        let mut out = vec![random.next_unit().to_bits(), random.next_int() as u64];
        out.extend((0..3).map(|_| random.next_unit().to_bits()));
        out
    };
    assert_eq!(
        sequence(&Random::with_seed(12345)),
        sequence(&Random::with_seed(12345))
    );

    // :247 — different seeds diverge.
    assert_ne!(
        sequence(&Random::with_seed(12345)),
        sequence(&Random::with_seed(67890))
    );

    // :261, :274, :291 — every string seed separates, including one-character
    // seeds, trailing partial words, and astral characters.
    let strings = |seed: &str| {
        let random = Random::with_string_seed(seed);
        (0..5)
            .map(|_| random.next_unit().to_bits())
            .collect::<Vec<_>>()
    };
    let empty = strings("");
    for seed in [
        "a", "b", "1234a", "1234b", "1234ab", "1234ac", "😀", "😁", "🐀",
    ] {
        assert_ne!(
            strings(seed),
            empty,
            "seed {seed:?} collided with the empty seed"
        );
    }
    assert_ne!(strings("a"), strings("b"));
    assert_ne!(strings("1234a"), strings("1234b"));
    assert_ne!(strings("😀"), strings("😁"));
    assert_ne!(strings("😀"), strings("🐀"));
}
