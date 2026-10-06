//! SchemaBinary fence: array-run wire layout for f64/boolean runs.

#[path = "../src/schema_binary.rs"]
mod schema_binary;

use schema_binary::{
    decode_bool_array, decode_number_array, encode_bool_array, encode_number_array,
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
