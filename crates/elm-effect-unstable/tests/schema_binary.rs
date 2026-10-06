//! SchemaBinary fence: array-run wire layout for f64/boolean runs.

#[path = "../src/schema_binary.rs"]
mod schema_binary;

use schema_binary::{
    decode_bool_array, decode_int_scalar, decode_nested_number_array, decode_number_array,
    decode_number_scalar, decode_string_array, encode_bool_array, encode_nested_number_array,
    encode_number_array, encode_number_scalar, encode_string_array,
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

#[test]
fn binary_int_scalars_reject_unsafe_varints() {
    // SchemaBinary.test.ts:517 - magnitudes past 2^53-1 fail, not round.
    let unsafe_bytes = [9u8, 0x20, 0x82, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x20];
    let err = decode_int_scalar(&unsafe_bytes).expect_err("rejects");
    assert!(err.message().contains("integer"));
    // The boundary itself still decodes bit-exactly.
    let max_safe = [9u8, 0x20, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x1f];
    assert_eq!(
        decode_int_scalar(&max_safe).expect("decodes"),
        9_007_199_254_740_991.0
    );
    assert_eq!(decode_int_scalar(&[2, 0x20, 4]).expect("decodes"), 2.0);
    let neg_zero = decode_int_scalar(&[2, 0x20, 1]).expect("decodes");
    assert!(neg_zero.is_sign_negative() && neg_zero == 0.0);
}

#[test]
fn binary_number_runs_pack_behind_one_mode_byte() {
    // SchemaBinary.test.ts:410 - uniform runs share a single mode byte.
    let integers = [0.0, -0.0, 1.0, -1.0, 1000.0, -1000.0];
    assert_eq!(encode_number_array(&integers).len(), 12);
    let back = decode_number_array(&encode_number_array(&integers)).expect("decodes");
    for (got, want) in back.iter().zip(integers.iter()) {
        assert_eq!(got.to_bits(), want.to_bits());
    }
    // One short decimal moves the run to packed decimal mode.
    let mixed = [1.0, 2.5];
    assert_eq!(encode_number_array(&mixed).len(), 7);
    assert_eq!(
        decode_number_array(&encode_number_array(&mixed)).expect("decodes"),
        vec![1.0, 2.5]
    );
    let decimals = [-0.0, 0.25, 1e6, -12.75];
    let back = decode_number_array(&encode_number_array(&decimals)).expect("decodes");
    for (got, want) in back.iter().zip(decimals.iter()) {
        assert_eq!(got.to_bits(), want.to_bits());
    }
    // One value outside both modes moves the whole run to f64.
    let wide = [1.0, std::f64::consts::PI];
    assert_eq!(encode_number_array(&wide).len(), 20);
    let back = decode_number_array(&encode_number_array(&wide)).expect("decodes");
    assert_eq!(back, vec![1.0, std::f64::consts::PI]);
    // SchemaBinary.test.ts:437 needs runtime non-numbers, which cannot
    // inhabit &[f64]; the type system rejects them at compile time.
}

#[test]
fn binary_nested_number_arrays_roundtrip() {
    // SchemaBinary.test.ts:164 - variable-size elements and nested arrays.
    assert_eq!(
        decode_string_array(&encode_string_array(&["ab", "", "cd\u{e9}"])).expect("decodes"),
        vec!["ab".to_owned(), "".to_owned(), "cd\u{e9}".to_owned()]
    );
    let nested = vec![vec![1.0, 2.0], vec![], vec![3.0]];
    assert_eq!(
        decode_nested_number_array(&encode_nested_number_array(&nested)).expect("decodes"),
        nested
    );
}
