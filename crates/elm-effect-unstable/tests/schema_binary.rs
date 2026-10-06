//! SchemaBinary fence: array-run wire layout for f64/boolean runs.

#[path = "../src/schema_binary.rs"]
mod schema_binary;

use schema_binary::{
    decode_bool_array, decode_number_array, decode_number_scalar, decode_string_array,
    encode_bool_array, encode_number_array, encode_number_scalar, encode_string_array,
};

#[test]
fn binary_arrays_pack_fixed_elements() {
    // SchemaBinary.test.ts:148 — length, envelope, count, mode byte, f64s.
    let values = [1.5, f64::NAN, -0.0, f64::INFINITY];
    let bytes = encode_number_array(&values);
    assert_eq!(bytes.len(), 36);
    assert_eq!(&bytes[..4], &[0x23, 0x20, 0x04, 0x00]);
    let back = decode_number_array(&bytes).expect("decodes");
    assert_eq!(back[0], 1.5);
    assert!(back[1].is_nan());
    assert!(back[2].is_sign_negative() && back[2] == 0.0);
    assert_eq!(back[3], f64::INFINITY);
    // Same row — boolean run: 6 bytes, bit-exact payload, roundtrip.
    let bools = encode_bool_array(&[true, false, true]);
    assert_eq!(bools, vec![0x05, 0x20, 0x03, 0x01, 0x00, 0x01]);
    assert_eq!(
        decode_bool_array(&bools).expect("decodes"),
        vec![true, false, true]
    );
}

#[test]
fn binary_number_runs_pick_compact_modes() {
    // Integer-valued runs take the varint mode (differentials vs the TS codec).
    assert_eq!(
        encode_number_array(&[0.0, 1.0, 2.0, -2.0, 300.0]),
        vec![0x09, 0x20, 0x05, 0x01, 0x00, 0x02, 0x04, 0x05, 0xd8, 0x04]
    );
    // Short decimals take the decimal mode; negatives keep their sign bit.
    assert_eq!(
        encode_number_array(&[0.5, 2.5]),
        vec![0x07, 0x20, 0x02, 0x02, 0xa1, 0x01, 0xa1, 0x06]
    );
    assert_eq!(
        encode_number_array(&[-0.5, -1.5]),
        vec![0x07, 0x20, 0x02, 0x02, 0xb1, 0x01, 0xf1, 0x03]
    );
    // Big magnitudes kill the decimal path and fall back to raw f64.
    assert_eq!(
        decode_number_array(&encode_number_array(&[2f64.powi(42), 1.5])).expect("decodes"),
        vec![2f64.powi(42), 1.5]
    );
    // Empty runs keep their mode byte (numbers) or omit it (bools).
    assert_eq!(encode_number_array(&[]), vec![0x03, 0x20, 0x00, 0x01]);
    assert_eq!(encode_bool_array(&[]), vec![0x02, 0x20, 0x00]);
    assert_eq!(
        decode_number_array(&[0x03, 0x20, 0x00, 0x01]).expect("decodes"),
        Vec::<f64>::new()
    );
}

#[test]
fn binary_scalars_pack_varints_by_width() {
    // SchemaBinary.test.ts:324 — integral scalars as sign-magnitude varints.
    assert_eq!(encode_number_scalar(0.0), vec![2, 0x20, 0]);
    assert_eq!(encode_number_scalar(-0.0), vec![2, 0x20, 1]);
    assert_eq!(encode_number_scalar(1.0), vec![2, 0x20, 2]);
    assert_eq!(encode_number_scalar(-1.0), vec![2, 0x20, 3]);
    for v in [0.0, -0.0, 1.0, -1.0, 42.0, -42.0, 127.0, -128.0] {
        let back = decode_number_scalar(&encode_number_scalar(v)).expect("decodes");
        assert_eq!(back.to_bits(), v.to_bits());
    }
    // :334 — every varint width to the seven-byte cap, both signs; f64 past it.
    for width in 1..=7u32 {
        let last = (1u64 << (7 * width - 1)) - 1;
        for sign in [1.0, -1.0] {
            let inside = sign * last as f64;
            assert_eq!(encode_number_scalar(inside).len(), 2 + width as usize);
            assert_eq!(
                decode_number_scalar(&encode_number_scalar(inside))
                    .expect("decodes")
                    .to_bits(),
                inside.to_bits()
            );
            let outside = sign * (last as f64 + 1.0);
            let expected = if width == 7 { 10 } else { 3 + width as usize };
            assert_eq!(encode_number_scalar(outside).len(), expected);
            assert_eq!(
                decode_number_scalar(&encode_number_scalar(outside))
                    .expect("decodes")
                    .to_bits(),
                outside.to_bits()
            );
        }
    }
}

#[test]
fn binary_scalars_pack_decimals_and_f64_fallback() {
    // :352 — short decimals as mantissa varint plus scale byte.
    assert_eq!(encode_number_scalar(1.5), vec![3, 0x20, 30, 1]);
    assert_eq!(encode_number_scalar(-0.5), vec![3, 0x20, 11, 1]);
    for v in [0.1, 1.3, 4.7, 98.5, -123.456] {
        let back = decode_number_scalar(&encode_number_scalar(v)).expect("decodes");
        assert_eq!(back.to_bits(), v.to_bits());
    }
    // :369 — neither capped-varint nor decimal stays raw f64.
    for v in [
        f64::EPSILON,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        9_007_199_254_740_991.0,
        -9_007_199_254_740_991.0,
        f64::MAX,
        std::f64::consts::PI,
        1e-9,
        2f64.powi(48),
        -(2f64.powi(48)),
    ] {
        let bytes = encode_number_scalar(v);
        assert_eq!(bytes.len(), 10);
        assert_eq!(
            decode_number_scalar(&bytes).expect("decodes").to_bits(),
            v.to_bits()
        );
    }
}

#[test]
fn binary_string_runs_are_length_prefixed() {
    // SchemaBinary.test.ts:164 - variable-size elements carry a length prefix.
    assert_eq!(
        encode_string_array(&["a", "bb", ""]),
        vec![0x08, 0x20, 0x03, 0x01, b'a', 0x02, b'b', b'b', 0x00]
    );
    assert_eq!(
        encode_string_array(&["hello"]),
        vec![0x08, 0x20, 0x01, 0x05, b'h', b'e', b'l', b'l', b'o']
    );
    // Multi-byte UTF-8 is measured in bytes, not chars.
    let unicode = encode_string_array(&["héllo"]);
    assert_eq!(unicode[3], 6);
    // :175 - zero-width elements round-trip.
    assert_eq!(
        decode_string_array(&encode_string_array(&["", ""])).expect("decodes"),
        vec!["".to_owned(), "".to_owned()]
    );
    assert_eq!(
        decode_string_array(&encode_string_array(&["a", "bb", ""])).expect("decodes"),
        vec!["a".to_owned(), "bb".to_owned(), "".to_owned()]
    );
    assert_eq!(
        decode_string_array(&unicode).expect("decodes"),
        vec!["héllo".to_owned()]
    );
}
