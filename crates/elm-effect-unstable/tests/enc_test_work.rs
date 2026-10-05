//! Behavior ports for `encoding/*`: Base64, Base64Url, Hex, EncodingError.
//!
//! Modules load via `#[path]` includes (the .15 precedent): no facade touch.

#[path = "../src/encoding.rs"]
mod encoding;

use encoding::EncodingError;

#[test]
fn base64_encodes_strings_and_bytes() {
    assert_eq!(encoding::base64_encode(b"hello"), "aGVsbG8=");
    assert_eq!(
        encoding::base64_encode(&[72, 101, 108, 108, 111]),
        "SGVsbG8="
    );
}

#[test]
fn base64_decodes_bytes_and_strings() {
    assert_eq!(
        encoding::base64_decode("SGVs\r\nbG8=").unwrap(),
        vec![72, 101, 108, 108, 111]
    );
    assert_eq!(
        encoding::base64_decode_string("8J+Riw==").unwrap(),
        "\u{1f44b}"
    );
}

#[test]
fn base64_rejects_invalid_input() {
    for bad in ["abc", "ab=c", "!!!!"] {
        let err = encoding::base64_decode(bad).unwrap_err();
        assert_eq!(err.kind(), "Decode");
        assert_eq!(err.module(), "Base64");
        assert_eq!(err.input(), bad);
    }
}

#[test]
fn base64_url_roundtrip() {
    assert_eq!(
        encoding::base64_url_decode(&encoding::base64_url_encode(b"\xff\xfe?/=+")).unwrap(),
        vec![255, 254, 63, 47, 61, 43]
    );
    assert!(encoding::base64_decode("ab-c").is_err());
    assert!(encoding::base64_url_decode("ab+c").is_err());
}

#[test]
fn hex_encodes_lowercase() {
    assert_eq!(encoding::hex_encode(b"hello"), "68656c6c6f");
    assert_eq!(
        encoding::hex_encode(&[72, 101, 108, 108, 111]),
        "48656c6c6f"
    );
}

#[test]
fn hex_decodes_mixed_case() {
    assert_eq!(
        encoding::hex_decode("48656C6cF").unwrap_err().module(),
        "Hex"
    );
    assert_eq!(
        encoding::hex_decode("48656C6c6F").unwrap(),
        vec![72, 101, 108, 108, 111]
    );
    assert_eq!(
        encoding::hex_decode_string("f09f918b").unwrap(),
        "\u{1f44b}"
    );
}

#[test]
fn hex_rejects_odd_and_nonhex() {
    let odd = encoding::hex_decode("abc").unwrap_err();
    assert_eq!(
        (odd.kind(), odd.module(), odd.input()),
        ("Decode", "Hex", "abc")
    );
    assert!(encoding::hex_decode("zz").is_err());
}

#[test]
fn hex_random_shape() {
    let value = encoding::hex_random(8);
    assert_eq!(value.len(), 16);
    assert!(
        value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !(b as char).is_ascii_uppercase())
    );
}

#[test]
fn encoding_error_metadata() {
    let error = EncodingError::decode("Base64", "!!!!", "Invalid input");
    assert_eq!(error.kind(), "Decode");
    assert_eq!(error.module(), "Base64");
    assert_eq!(error.input(), "!!!!");
    assert_eq!(error.message(), "Invalid input");
}

#[test]
fn process_args_and_env() {
    let argv = elm_effect_unstable::process::args();
    assert!(!argv.is_empty(), "argv always names the executable");
    assert!(elm_effect_unstable::process::env("ELM_EFFECT_DEFINITELY_ABSENT_XYZ").is_none());
    let missing = elm_effect_unstable::process::require_env("ELM_EFFECT_DEFINITELY_ABSENT_XYZ");
    assert!(missing.is_err());
}

#[test]
fn process_runs_echo() {
    let out = elm_effect_unstable::process::run("echo", &["hi"]).unwrap();
    assert!(out.is_success());
    assert_eq!(out.stdout_text().trim(), "hi");
    let missing = elm_effect_unstable::process::run("elm-effect-definitely-absent-xyz", &[]);
    assert!(missing.is_err());
}

#[test]
fn workers_spawn_and_join() {
    let worker = elm_effect_unstable::workers::spawn(Some("adder"), || 40 + 2);
    assert_eq!(worker.join(), 42);
}

#[test]
fn reactivity_signal_versions_and_watchers() {
    use std::sync::{Arc, Mutex};
    let signal = elm_effect_unstable::reactivity::Signal::new(1);
    assert_eq!(signal.version(), 0);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let probe = seen.clone();
    signal.watch(move |value: &i32| {
        probe.lock().unwrap().push(*value);
    });
    signal.set(2);
    assert_eq!(signal.get(), 2);
    assert_eq!(signal.version(), 1);
    assert_eq!(*seen.lock().unwrap(), vec![2]);
    let doubled = elm_effect_unstable::reactivity::computed(&signal, |n: &i32| n * 2);
    assert_eq!(doubled, 4);
}
