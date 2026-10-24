//! Hashing laws: equal values hash alike, and structurally different values
//! do not borrow another value's hash by rendering it to a string.

use elm_effect::{Hash, hash_f64, hash_str};

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
fn hash_null_and_booleans_are_not_their_names() {
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
}

#[test]
fn hash_zero_and_negative_zero_alike() {
    // :143 - 0 and -0 are the same number for Effect 4's hash.
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
