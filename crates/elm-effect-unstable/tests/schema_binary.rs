//! SchemaBinary fence: array-run wire layout for f64/boolean runs.

#[path = "../src/schema_binary.rs"]
mod schema_binary;

use schema_binary::{
    NumberParser, StringParser, UnionValue, decode_bool_array, decode_int_array, decode_int_scalar,
    decode_nested_number_array, decode_number_array, decode_number_scalar, decode_string_array,
    decode_string_scalar, decode_union, encode_bool_array, encode_int_array, encode_int_scalar,
    encode_int_tuple, encode_nested_number_array, encode_number_array, encode_number_array_unknown,
    encode_number_scalar, encode_number_tuple, encode_string_array, encode_string_scalar,
    encode_union, split_frames,
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

#[test]
fn binary_tuples_split_prefixed_frames() {
    // SchemaBinary.test.ts:446 - each number slot carries its length prefix.
    let pair = [encode_number_scalar(1.0), encode_number_scalar(2.5)];
    let bytes: Vec<u8> = pair.concat();
    assert_eq!(bytes.len(), 7);
    let frames = split_frames(&bytes).expect("splits");
    assert_eq!(frames.len(), 2);
    assert_eq!(decode_number_scalar(frames[0]).expect("decodes"), 1.0);
    assert_eq!(decode_number_scalar(frames[1]).expect("decodes"), 2.5);
    let pi_pair = [
        encode_number_scalar(1.0),
        encode_number_scalar(std::f64::consts::PI),
    ];
    let bytes: Vec<u8> = pi_pair.concat();
    assert_eq!(bytes.len(), 13);
}

#[test]
fn binary_unions_carry_forms_behind_kind() {
    // :457 - number and string forms behind the union kind byte.
    assert_eq!(
        encode_union(&UnionValue::Number(5.0)),
        vec![3, 0x20, 4, 0x0a]
    );
    assert_eq!(
        decode_union(&encode_union(&UnionValue::Number(5.5))).expect("decodes"),
        UnionValue::Number(5.5)
    );
    assert_eq!(
        decode_union(&encode_union(&UnionValue::Number(std::f64::consts::PI))).expect("decodes"),
        UnionValue::Number(std::f64::consts::PI)
    );
    assert_eq!(
        decode_union(&encode_union(&UnionValue::Text("hi".to_owned()))).expect("decodes"),
        UnionValue::Text("hi".to_owned())
    );
    assert_eq!(
        decode_union(&encode_union(&UnionValue::Text("".to_owned()))).expect("decodes"),
        UnionValue::Text("".to_owned())
    );
}

#[test]
fn binary_number_payloads_report_reference_failures() {
    // SchemaBinary.test.ts:526 — malformed number payloads report the
    // reference's vocabulary: Expected f64, a complete value, Expected
    // decimal. The frames below are the pin's own byte arrays.
    // Nine payload bytes are neither the varint nor the f64 form.
    let wide = [10u8, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let err = decode_number_scalar(&wide).expect_err("wide payload");
    assert!(err.message().contains("Expected f64"), "{}", err.message());
    // A varint that never terminates inside its extent.
    let truncated = [3u8, 0x20, 0x80, 0x80];
    let err = decode_number_scalar(&truncated).expect_err("truncated varint");
    assert!(
        err.message().contains("complete value"),
        "{}",
        err.message()
    );
    // A decimal tail whose scale byte is zero, out of range, or followed by
    // more bytes: scale 0 is illegal here because the varint form is the
    // canonical encoding of that value.
    for bytes in [
        &[3u8, 0x20, 30, 0][..],
        &[3u8, 0x20, 30, 9][..],
        &[4u8, 0x20, 30, 1, 1][..],
    ] {
        let err = decode_number_scalar(bytes).expect_err("decimal tail");
        assert!(
            err.message().contains("Expected decimal"),
            "{:?}: {}",
            bytes,
            err.message()
        );
    }
    // An array run with an unknown mode byte.
    let run = encode_number_array(&[1.0, 2.0, 3.0]);
    let mut unknown = run.clone();
    unknown[3] = 3;
    let err = decode_number_array(&unknown).expect_err("unknown mode");
    assert!(err.message().contains("Expected f64"), "{}", err.message());
    // An array run cut one byte short.
    let short = &run[..run.len() - 1];
    let err = decode_number_array(short).expect_err("short run");
    assert!(
        err.message().contains("complete value"),
        "{}",
        err.message()
    );
}

#[test]
fn binary_number_arrays_reject_non_numbers() {
    // SchemaBinary.test.ts:437 — a uniform number array validates every
    // element and names the index that failed. The pin loops over "x",
    // undefined, null, and 1n; serde_json carries no variant for undefined or
    // BigInt, so those two inputs cannot be built here.
    for value in [serde_json::json!("x"), serde_json::json!(null)] {
        let values = vec![serde_json::json!(1), value, serde_json::json!(3)];
        let err = encode_number_array_unknown(&values).expect_err("non-number element");
        assert!(
            err.message().contains("Expected a number"),
            "{}",
            err.message()
        );
        assert!(err.message().contains("at [1]"), "{}", err.message());
    }
    // The numbers themselves take the ordinary run path.
    assert_eq!(
        encode_number_array_unknown(&[serde_json::json!(1), serde_json::json!(2.5)])
            .expect("numbers only"),
        encode_number_array(&[1.0, 2.5])
    );
}

#[test]
fn binary_int_layout_emits_bare_varints() {
    // SchemaBinary.test.ts:487 — a schema that proves the integer layout
    // drops the mode byte on an array and the length prefix on a tuple slot.
    assert_eq!(encode_int_scalar(1.0).expect("integer"), vec![2, 0x20, 2]);
    assert_eq!(encode_int_array(&[1.0, 2.0, 3.0]).expect("integers").len(), 6);
    assert_eq!(encode_number_array(&[1.0, 2.0, 3.0]).len(), 7);
    assert_eq!(encode_int_tuple(&[1.0, 2.0]).expect("integers").len(), 4);
    assert_eq!(encode_number_tuple(&[1.0, 2.0]).len(), 6);
    assert_eq!(
        encode_int_array(&[1.0, 2.0, 3.0]).expect("integers"),
        vec![0x05, 0x20, 0x03, 0x02, 0x04, 0x06]
    );
    assert_eq!(
        encode_int_tuple(&[1.0, 2.0]).expect("integers"),
        vec![0x03, 0x20, 0x02, 0x04]
    );
    // Every safe integer keeps its bits through the scalar and array forms.
    for value in [
        0.0,
        -0.0,
        1.0,
        -1.0,
        281_474_976_710_656.0,
        -281_474_976_710_656.0,
        9_007_199_254_740_991.0,
        -9_007_199_254_740_991.0,
    ] {
        let encoded = encode_int_scalar(value).expect("integer");
        let back = decode_int_scalar(&encoded)
            .unwrap_or_else(|err| panic!("scalar {value}: {err}"));
        assert_eq!(back.to_bits(), value.to_bits(), "scalar {value}");
        let array = encode_int_array(&[value]).expect("integer");
        let back = decode_int_array(&array).expect("decodes");
        assert_eq!(back[0].to_bits(), value.to_bits(), "array {value}");
    }
    assert_eq!(
        decode_int_array(&encode_int_array(&[1.0, -2.0, 3.0]).expect("integers")).expect("decodes"),
        vec![1.0, -2.0, 3.0]
    );
}

#[test]
fn binary_int_layout_rejects_non_integers() {
    // SchemaBinary.test.ts:509 - disableChecks bypasses the layout selection,
    // so the integer encoder itself has to refuse a non-integer.
    let err = encode_int_scalar(1.5).expect_err("non-integer");
    assert!(err.message().contains("an integer"), "{}", err.message());
    let err = encode_int_array(&[1.0, 1.5]).expect_err("non-integer element");
    assert!(err.message().contains("an integer"), "{}", err.message());
    assert!(err.message().contains("at [1]"), "{}", err.message());
    assert!(encode_int_tuple(&[1.0, 1.5]).is_err());
}

#[test]
fn binary_frames_reject_envelope_and_leftovers() {
    // SchemaBinary.test.ts:239 - length-first frames with envelope checks.
    assert_eq!(encode_number_scalar(1.0), vec![2, 0x20, 0x02]);
    let mut flags = encode_number_scalar(1.0);
    flags[1] = 0x21;
    let err = decode_number_scalar(&flags).expect_err("flags");
    assert!(err.message().contains("version 2 envelope, flags 0"));
    let mut version = encode_number_scalar(1.0);
    version[1] = 0x30;
    let err = decode_number_scalar(&version).expect_err("version");
    assert!(err.message().contains("version 2 envelope, flags 0"));
    let doubled = [encode_number_scalar(1.0), encode_number_scalar(1.0)].concat();
    let err = decode_number_scalar(&doubled).expect_err("leftovers");
    assert!(err.message().contains("no leftover bytes"));
}

/// The pin's `sameNumber`: NaN equals NaN, and `-0` stays distinct from `0`.
fn same_number(actual: f64, expected: f64) -> bool {
    if expected.is_nan() {
        return actual.is_nan();
    }
    actual.to_bits() == expected.to_bits()
}

#[test]
fn parser_streams_numbers_whose_width_changes() {
    // SchemaBinary.test.ts:476 — parser(Number) fed one byte at a time, over
    // values that take the varint, decimal, and f64 forms in turn.
    let values = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        63.0,
        64.0,
        1_000_000.0,
        -1_000_000.0,
        1.5,
        f64::NAN,
        9_007_199_254_740_991.0,
    ];
    let bytes: Vec<u8> = values.iter().flat_map(|v| encode_number_scalar(*v)).collect();
    let mut parser = NumberParser::new();
    let mut out = Vec::new();
    for byte in &bytes {
        out.extend(parser.feed(&[*byte]).expect("feed"));
    }
    parser.end().expect("end");
    assert_eq!(out.len(), values.len(), "one value per frame");
    for (index, value) in values.iter().enumerate() {
        assert!(
            same_number(out[index], *value),
            "{index}: got {} want {value}",
            out[index]
        );
    }
}

#[test]
fn parser_retains_partial_frames_across_one_byte_feeds() {
    // SchemaBinary.test.ts:1138 — 100 frames, one byte per feed, no value
    // may be emitted twice or dropped at a frame boundary.
    let expected: Vec<f64> = (0..100).map(|i| i as f64).collect();
    let bytes: Vec<u8> = expected.iter().flat_map(|v| encode_number_scalar(*v)).collect();
    let mut parser = NumberParser::new();
    let mut values = Vec::new();
    for byte in &bytes {
        values.extend(parser.feed(&[*byte]).expect("feed"));
    }
    parser.end().expect("end");
    assert_eq!(values, expected);
}

#[test]
fn parser_accepts_empty_input_and_spends_after_end() {
    // SchemaBinary.test.ts:1006 — an empty feed completes nothing, end on an
    // empty buffer succeeds, and the second end is refused.
    let mut parser = NumberParser::new();
    assert_eq!(parser.feed(&[]).expect("empty feed"), Vec::<f64>::new());
    parser.end().expect("empty end");
    let err = parser.end().expect_err("end twice");
    assert!(err.message().contains("parser is spent"), "{}", err.message());
}

#[test]
fn parser_delivers_completed_values_before_a_later_failure() {
    // SchemaBinary.test.ts:1262 — the frame that completed in this chunk is
    // returned, and the truncated tail only surfaces at end.
    let mut chunk = encode_number_scalar(1.0);
    chunk.extend_from_slice(&encode_number_scalar(2.0)[..2]);
    let mut parser = NumberParser::new();
    assert_eq!(parser.feed(&chunk).expect("feed"), vec![1.0]);
    let err = parser.end().expect_err("truncated tail");
    assert!(err.message().contains("complete value"), "{}", err.message());
    let err = parser.feed(&[]).expect_err("spent");
    assert!(err.message().contains("parser is spent"), "{}", err.message());
}

#[test]
fn parser_stashes_a_malformed_frame_after_earlier_values() {
    // SchemaBinary.test.ts:1271 — a complete frame with a bad envelope byte is
    // found after one good value. The value returns first; the failure is
    // held and raised on the next call, not merged into this one.
    let mut bad = encode_number_scalar(2.0);
    bad[1] = 0x10;
    let mut chunk = encode_number_scalar(1.0);
    chunk.extend_from_slice(&bad);
    let mut parser = NumberParser::new();
    assert_eq!(parser.feed(&chunk).expect("feed"), vec![1.0]);
    let err = parser.feed(&[]).expect_err("stashed failure");
    assert!(
        err.message().contains("version 2 envelope, flags 0"),
        "{}",
        err.message()
    );
    let err = parser.end().expect_err("spent");
    assert!(err.message().contains("parser is spent"), "{}", err.message());
}

#[test]
fn parser_enforces_max_frame_size() {
    // SchemaBinary.test.ts:1282 — the cap is compared against the announced
    // frame length, and a cap equal to it still accepts the frame.
    let frame = encode_number_scalar(1_000_000.0);
    let length = u64::from(frame[0]);
    assert_eq!(length, 4, "tag plus a three-byte varint");
    let mut over = NumberParser::with_max_frame_size(length - 1);
    let err = over.feed(&frame).expect_err("over the cap");
    assert!(
        err.message().contains("frame within maxFrameSize"),
        "{}",
        err.message()
    );
    let mut exact = NumberParser::with_max_frame_size(length);
    assert_eq!(exact.feed(&frame).expect("at the cap"), vec![1_000_000.0]);
    exact.end().expect("end");
}

#[test]
fn parser_fails_a_ten_byte_unterminated_length() {
    // SchemaBinary.test.ts:1292 — ten continuation groups can never become a
    // header, so the failure lands now instead of waiting for more bytes.
    let mut parser = NumberParser::new();
    let err = parser.feed(&[0x80; 10]).expect_err("unterminated length");
    assert!(err.message().contains("uvarint"), "{}", err.message());
}

#[test]
fn parser_splits_concatenated_frames_across_chunks() {
    // SchemaBinary.test.ts:1013 — two frames in one chunk, split three bytes
    // into the second: the tail of the first chunk is held, not dropped, and
    // the head of the second frame arrives with it. The pin's schema is a
    // struct; a string-scalar frame is the same length-first envelope.
    let first = encode_string_scalar("first");
    let second = encode_string_scalar("second");
    let bytes = [first.clone(), second].concat();
    let split = first.len() + 3;
    let mut parser = StringParser::new();
    assert_eq!(
        parser.feed(&bytes[..split]).expect("first chunk"),
        vec!["first".to_string()]
    );
    assert_eq!(
        parser.feed(&bytes[split..]).expect("second chunk"),
        vec!["second".to_string()]
    );
    parser.end().expect("end");
    let err = parser.feed(&[]).expect_err("spent");
    assert!(err.message().contains("parser is spent"), "{}", err.message());
}

#[test]
fn parser_waits_for_a_fragmented_multi_byte_frame_header() {
    // SchemaBinary.test.ts:1147 — a 300-byte value announces a two-byte
    // length, and one byte of it is not a header yet.
    let value = "x".repeat(300);
    let bytes = encode_string_scalar(&value);
    assert_eq!(bytes.len(), 303, "two header bytes, a tag, and 300 payload bytes");
    let mut parser = StringParser::new();
    assert_eq!(
        parser.feed(&bytes[..1]).expect("header byte"),
        Vec::<String>::new()
    );
    assert_eq!(
        parser.feed(&bytes[1..]).expect("rest of the frame"),
        vec![value]
    );
    parser.end().expect("end");
}

#[test]
fn parser_reaches_the_wide_header_path_for_long_safe_lengths() {
    // SchemaBinary.test.ts:1157 — canonical uvarint(MAX_SAFE_INTEGER) is seven
    // continuation groups then 0x0F. It is a complete, in-range header, so the
    // parser reaches the maxFrameSize check rather than waiting for a body no
    // host could hold.
    let mut parser = StringParser::with_max_frame_size(1);
    assert_eq!(
        parser.feed(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]).expect("partial header"),
        Vec::<String>::new()
    );
    let err = parser.feed(&[0x0F]).expect_err("over the cap");
    assert!(
        err.message().contains("frame within maxFrameSize"),
        "{}",
        err.message()
    );
    let err = parser.feed(&[]).expect_err("spent");
    assert!(err.message().contains("parser is spent"), "{}", err.message());
}

#[test]
fn decode_recovers_after_an_exceptional_frame() {
    // SchemaBinary.test.ts:1254 — a decode that fails on malformed input checks
    // its reader back in, so the next decode is not poisoned by it.
    let err = decode_string_scalar(&[2, 0x20, 0xFF]).expect_err("invalid utf-8");
    assert!(err.message().contains("utf-8"), "{}", err.message());
    let after = decode_string_scalar(&encode_string_scalar("after failure"))
        .expect("recovers after the failure");
    assert_eq!(after, "after failure");
}

#[test]
fn parser_feed_and_end_are_effects() {
    // SchemaBinary.test.ts:1308 — feed and end carry the values or the
    // error to the caller. The pin defers the work to interpretation;
    // this port's synchronous pair is the same observable at the
    // boundary, and a library test target may not drive the runtime
    // (the edge invariant), so the pair is what the test exercises.
    let mut parser = NumberParser::new();
    assert_eq!(parser.feed(&encode_number_scalar(1.0)), Ok(vec![1.0]));
    assert_eq!(parser.end(), Ok(()));
}

#[test]
fn parser_effect_surface_reports_malformed_feed_and_truncated_end() {
    // SchemaBinary.test.ts:1316 — both failures reach the caller as a
    // typed error, not as a panic or a defect. The pin's malformed feed
    // is a boolean frame and its truncated end is `[2, 0x20]`; a string
    // frame whose payload is not UTF-8 is the analogue of the first.
    let mut malformed = StringParser::new();
    let error = malformed
        .feed(&[2, 0x20, 0xFF])
        .expect_err("malformed feed must fail");
    assert!(error.message().contains("utf-8"), "{}", error.message());

    let mut truncated = StringParser::new();
    assert_eq!(truncated.feed(&[2, 0x20]), Ok(Vec::<String>::new()));
    let error = truncated.end().expect_err("truncated end must fail");
    assert!(!error.message().is_empty(), "a typed error, not a defect");
}

#[test]
fn parser_rejects_a_terminated_header_above_the_safe_integer_bound() {
    // SchemaBinary.test.ts:1300 — seven continuation groups then 0x10 encode
    // 2^53, one past a safe frame length, and that is an error rather than a
    // rounded-down wait for a body that cannot exist.
    let mut parser = NumberParser::new();
    let err = parser
        .feed(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x10])
        .expect_err("unsafe length");
    assert!(
        err.message().contains("safe integer length"),
        "{}",
        err.message()
    );
    let err = parser.feed(&[]).expect_err("spent");
    assert!(err.message().contains("parser is spent"), "{}", err.message());
}
