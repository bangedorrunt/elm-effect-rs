//! `Hash.test.ts` rows, ported against [`elm_effect::Hash`] and the hash
//! helpers. Equal values hash alike; structurally different values do not
//! borrow another value's hash by rendering it to a string.

use elm_effect::{Hash, Tagged, hash_f64, hash_str, tagged};

#[test]
fn hash_keeps_array_order_and_length() {
    // :122 - a vector's hash folds in order, so a permutation differs, and
    // the empty vector is not the one-element vector.
    assert_ne!(vec![1, 2].hash_value(), vec![2, 1].hash_value());
    assert_ne!(vec![1].hash_value(), vec![1, 2].hash_value());
    assert_ne!(Vec::<i32>::new().hash_value(), vec![1].hash_value());
    assert_eq!(vec![1, 2].hash_value(), vec![1, 2].hash_value());
}

#[test]
fn hash_null_undefined_booleans_and_symbols_are_not_their_names() {
    // :127 - the rendered name is not the value. Each special value gets its
    // own mix, so it cannot collide with the string that spells it.
    assert_ne!(
        Option::<i32>::None.hash_value(),
        "None".to_string().hash_value()
    );
    assert_ne!(true.hash_value(), "true".to_string().hash_value());
    assert_ne!(false.hash_value(), "false".to_string().hash_value());
    assert_ne!(
        Option::<i32>::Some(1).hash_value(),
        "Some".to_string().hash_value()
    );
    // `Tagged` is the crate's symbol-like carrier: a tag is not a string
    // value, and its hash mixes the tag instead of returning the name's hash.
    let symbol = tagged("Symbol.for(\"x\")", 1);
    assert_ne!(
        symbol.hash_value(),
        "Symbol.for(\"x\")".to_string().hash_value()
    );
    assert_eq!(
        symbol.hash_value(),
        tagged("Symbol.for(\"x\")", 1).hash_value()
    );
    assert_ne!(
        symbol.hash_value(),
        Tagged {
            tag: "Symbol.for(\"x\")",
            value: 2
        }
        .hash_value()
    );
}

#[test]
fn hash_zero_and_negative_zero_alike() {
    // :143 - 0 and -0 are the same number for the pin's hash.
    assert_eq!(hash_f64(0.0), hash_f64(-0.0));
    assert_eq!(0.0f64.hash_value(), (-0.0f64).hash_value());
    assert_eq!(0i32.hash_value(), 0i32.hash_value());
}

#[test]
fn hash_every_nan_alike() {
    // :147 - every NaN payload and sign is canonicalized to one hash, which
    // is what lets `Equal` treat all NaNs as one value.
    let nan = f64::NAN;
    assert_eq!(hash_f64(nan), hash_f64(-nan));
    assert_eq!(nan.hash_value(), (-nan).hash_value());
    assert_eq!(
        nan.hash_value(),
        f64::from_bits(0x7ff8_0000_0000_0001).hash_value()
    );
    assert_ne!(nan.hash_value(), hash_f64(1.0));
    assert_eq!(hash_str("abc"), hash_str("abc"));
}
