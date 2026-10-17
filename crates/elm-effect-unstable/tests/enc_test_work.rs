//! Behavior ports for `encoding/*`: Base64, Base64Url, Hex, EncodingError.
//!
//! Modules load via `#[path]` includes (the .15 precedent): no facade touch.

#[path = "../src/encoding.rs"]
mod encoding;
#[path = "../src/ini.rs"]
mod ini;
#[path = "../src/testing.rs"]
mod testing;
#[path = "../src/toml.rs"]
mod toml;

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
fn base64_url_encodes_without_padding() {
    assert_eq!(encoding::base64_url_encode(b"hello?"), "aGVsbG8_");
    assert_eq!(encoding::base64_url_encode(&[251, 255]), "-_8");
}

#[test]
fn base64_url_decodes_padded_and_unpadded() {
    assert_eq!(
        encoding::base64_url_decode("SGVsbG8_").unwrap(),
        vec![72, 101, 108, 108, 111, 63]
    );
    assert_eq!(
        encoding::base64_url_decode("-_8=").unwrap(),
        vec![251, 255]
    );
    assert_eq!(
        encoding::base64_url_decode_string("8J-Riw").unwrap(),
        "\u{1f44b}"
    );
    assert_eq!(
        encoding::base64_url_decode_string("8J-Riw==").unwrap(),
        "\u{1f44b}"
    );
}

#[test]
fn base64_url_rejects_standard_alphabet_and_bad_lengths() {
    let err = encoding::base64_url_decode("+/8=").unwrap_err();
    assert_eq!(err.kind(), "Decode");
    assert_eq!(err.module(), "Base64Url");
    assert_eq!(err.input(), "+/8=");
    assert!(encoding::base64_url_decode("a").is_err());
}

#[test]
fn base64_url_rejects_malformed_padding() {
    for bad in ["SGVsbG8_=", "SGVsbG8_==", "AAAA=", "AAAA==", "AAA=="] {
        assert!(encoding::base64_url_decode(bad).is_err(), "{bad}");
    }
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
fn hex_random_words_are_lowercase_of_exact_length() {
    for (length, expected) in [(0.0, 0), (8.0, 8), (16.0, 16), (32.0, 32)] {
        let value = encoding::hex_random(length);
        assert_eq!(value.len(), expected, "length {length}");
        assert!(
            value
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !(b as char).is_ascii_uppercase()),
            "{value}"
        );
    }
}

#[test]
fn hex_random_rounds_down_to_multiples_of_8() {
    for (length, expected) in [(7.0, 0), (15.9, 8), (23.0, 16)] {
        assert_eq!(encoding::hex_random(length).len(), expected, "length {length}");
    }
}

#[test]
fn hex_random_coerces_u32_without_validation() {
    use std::f64::{INFINITY, NAN};
    for (length, expected) in [
        (NAN, 0),
        (INFINITY, 0),
        (4_294_967_296.0, 0),
        (4_294_967_304.0, 8),
    ] {
        assert_eq!(encoding::hex_random(length).len(), expected, "length {length}");
    }
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

#[test]
fn reactivity_unsubscribe_during_notify_does_not_skip_later_listeners() {
    // Atom.test.ts "subscribe does not skip listeners when unsubscribing
    // during notify": the first listener removes itself while notification is
    // in flight, and the second listener must still fire exactly once.
    //
    // Pinned against the port's real hazard: `set` notifies while holding no
    // lock per callback, so an unwatch from inside a watcher takes effect for
    // the remaining snapshotted ids instead of deadlocking or corrupting the
    // iteration.
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};

    let signal = Arc::new(elm_effect_unstable::reactivity::Signal::new(0));
    let first = Arc::new(AtomicU32::new(0));
    let second = Arc::new(AtomicU32::new(0));
    let first_seen = Arc::clone(&first);
    let second_seen = Arc::clone(&second);

    // The handle lands here after registration, so the watcher can remove
    // itself mid-notify (it cannot capture `signal` while `watch` borrows it).
    let cancel: Arc<Mutex<Option<elm_effect_unstable::reactivity::WatcherId>>> =
        Arc::new(Mutex::new(None));
    let signal_for_watcher = Arc::clone(&signal);
    let cancel_for_watcher = Arc::clone(&cancel);

    let id = signal.watch(move |_value: &i32| {
        first_seen.fetch_add(1, Ordering::SeqCst);
        let handle = *cancel_for_watcher.lock().unwrap();
        if let Some(handle) = handle {
            signal_for_watcher.unwatch(handle);
        }
    });
    *cancel.lock().unwrap() = Some(id);

    signal.watch(move |_value: &i32| {
        second_seen.fetch_add(1, Ordering::SeqCst);
    });

    signal.set(1);
    assert_eq!(
        first.load(Ordering::SeqCst),
        1,
        "the self-removing listener still fired"
    );
    assert_eq!(
        second.load(Ordering::SeqCst),
        1,
        "and the later listener was not skipped"
    );

    // After the self-removal, a later set reaches only the survivor.
    signal.set(2);
    assert_eq!(first.load(Ordering::SeqCst), 1, "the removed listener is gone");
    assert_eq!(second.load(Ordering::SeqCst), 2);
}

// ---------------------------------------------------------------------------
// Ini
// ---------------------------------------------------------------------------

#[test]
fn ini_sections_arrays_scalars() {
    let doc = ini::parse("\nenabled=true\nmissing=null\nname=effect\ntag[]=one\ntag[]=two\n");
    assert_eq!(doc.get("enabled"), Some(&ini::IniValue::Bool(true)));
    assert_eq!(doc.get("missing"), Some(&ini::IniValue::Null));
    assert_eq!(doc.get("name"), Some(&ini::IniValue::Text("effect".into())));
    assert_eq!(
        doc.get("tag"),
        Some(&ini::IniValue::Array(vec![
            ini::IniValue::Text("one".into()),
            ini::IniValue::Text("two".into())
        ]))
    );
}

#[test]
fn ini_quoted_comments_escaped_dots() {
    let doc =
        ini::parse("\nplain=value ; comment\nquoted=\"value # retained\"\n[a\\.b]\nkey=yes\n");
    assert_eq!(doc.get("plain"), Some(&ini::IniValue::Text("value".into())));
    assert_eq!(
        doc.get("quoted"),
        Some(&ini::IniValue::Text("value # retained".into()))
    );
    assert_eq!(
        doc.get("a.b"),
        Some(&ini::IniValue::Section(
            [("key".to_string(), ini::IniValue::Text("yes".into()))].into()
        ))
    );
}

// ---------------------------------------------------------------------------
// Toml
// ---------------------------------------------------------------------------

#[test]
fn toml_tables_dotted_keys_arrays() {
    let doc = toml::parse("\ntitle = \"Effect\"\nports = [8000, 8001]\n").unwrap();
    assert_eq!(doc["title"], toml::TomlValue::Text("Effect".into()));
    assert_eq!(
        doc["ports"],
        toml::TomlValue::Array(vec![
            toml::TomlValue::Integer(8000),
            toml::TomlValue::Integer(8001)
        ])
    );
}

#[test]
fn toml_servers_with_datetimes() {
    let doc =
        toml::parse("\n[[servers]]\nname = \"alpha\"\nstarted = 2026-08-05T01:02:03Z\n").unwrap();
    match &doc["servers"] {
        toml::TomlValue::Array(items) => {
            assert_eq!(items.len(), 1);
            match &items[0] {
                toml::TomlValue::Table(map) => {
                    assert_eq!(
                        map["started"],
                        toml::TomlValue::Datetime("2026-08-05T01:02:03.000Z".into())
                    );
                }
                _ => panic!("not a table"),
            }
        }
        _ => panic!("not an array"),
    }
}

#[test]
fn toml_child_tables_attach_to_last_element() {
    let doc = toml::parse("\n[[servers]]\n[servers.tls]\n\n[[servers]]\n[servers.tls]\n").unwrap();
    match &doc["servers"] {
        toml::TomlValue::Array(items) => {
            assert_eq!(items.len(), 2);
            for item in items {
                match item {
                    toml::TomlValue::Table(map) => assert!(map.contains_key("tls")),
                    _ => panic!("not a table"),
                }
            }
        }
        _ => panic!("not an array"),
    }
}

#[test]
fn toml_multiline_and_numbers() {
    let doc = toml::parse("message = \u{22}\u{22}\u{22}\nhello \\\n  world\u{22}\u{22}\u{22}\nhex = 0xDEAD_BEEF\nfraction = 1_000.5\nlocal = 2026-08-05 01:02:03\n").unwrap();
    assert_eq!(doc["message"], toml::TomlValue::Text("hello world".into()));
    assert_eq!(doc["hex"], toml::TomlValue::Integer(0xdeadbeef));
    assert_eq!(doc["fraction"], toml::TomlValue::Float(1000.5));
    assert_eq!(
        doc["local"],
        toml::TomlValue::Datetime("2026-08-05T01:02:03".into())
    );
}

#[test]
fn toml_rejects_duplicates() {
    assert!(toml::parse("key = 1\nkey = 2\n").is_err());
    assert!(toml::parse("key = 1__000\n").is_err());
}

#[test]
fn testclock_adjust_wakes_in_deadline_order() {
    let mut clock = testing::TestClock::new();
    clock.sleep(1, 30);
    clock.sleep(2, 10);
    clock.sleep(3, 20);
    assert_eq!(clock.pending(), 3);
    assert_eq!(clock.adjust(15), vec![2]);
    assert_eq!(clock.now_millis(), 15);
    assert_eq!(clock.adjust(15), vec![3, 1]);
    assert_eq!(clock.pending(), 0);
}

#[test]
fn testclock_set_time_never_runs_backwards() {
    let mut clock = testing::TestClock::new();
    clock.sleep(7, 50);
    assert_eq!(clock.set_time(100), vec![7]);
    assert_eq!(clock.now_millis(), 100);
    assert_eq!(clock.set_time(40), Vec::<u64>::new());
    assert_eq!(clock.now_millis(), 100);
}

#[test]
fn testconsole_captures_and_clears() {
    let mut console = testing::TestConsole::new();
    assert!(console.is_empty());
    console.log("hello");
    console.error("boom");
    console.debug("trace");
    assert_eq!(console.out_lines(), ["hello"]);
    assert_eq!(console.err_lines(), ["boom"]);
    assert_eq!(console.debug_lines(), ["trace"]);
    assert_eq!(console.len(), 3);
    console.clear();
    assert!(console.is_empty());
}

#[test]
fn testrng_is_seeded_and_bounded() {
    let mut a = testing::TestRng::new(42);
    let mut b = testing::TestRng::new(42);
    for _ in 0..16 {
        assert_eq!(a.next_u64(), b.next_u64());
    }
    let mut rng = testing::TestRng::new(1);
    for _ in 0..64 {
        assert!(rng.next_bounded(10) < 10);
        assert!(rng.next_int_range(5, 5) == 5);
        assert!(rng.next_int_range(-3, 3) >= -3);
    }
    assert_eq!(rng.next_string(8).len(), 8);
    assert_eq!(
        testing::TestRng::new(0).next_u64(),
        testing::TestRng::new(0).next_u64()
    );
}

#[test]
fn test_assert_helpers_hold() {
    let v = testing::assert_roundtrip(7u32, |n| n.to_string(), |s: String| s.parse::<u32>());
    assert_eq!(v, 7);
    assert_eq!(testing::assert_succeeds::<u32, String>(Ok(1)), 1);
    assert_eq!(
        testing::assert_fails::<u32, _>(Err::<u32, String>("nope".into())),
        "nope"
    );
}

#[test]
fn ndjson_roundtrip_skips_blanks() {
    let values = vec![serde_json::json!({"a": 1}), serde_json::json!([true, "x"])];
    let text = encoding::ndjson::encode(&values);
    assert_eq!(text, "{\"a\":1}\n[true,\"x\"]\n");
    assert_eq!(
        encoding::ndjson::decode(&format!("\n{text}\n")).unwrap(),
        values
    );
}

#[test]
fn ndjson_names_the_bad_line() {
    let err = encoding::ndjson::decode("{\"ok\":true}\n{bad}\n").unwrap_err();
    assert!(err.to_string().contains("line 2"), "{err}");
}

#[test]
fn sse_encode_decode_roundtrip() {
    let event = encoding::sse::Event {
        event: "tick".into(),
        data: "one\ntwo".into(),
        id: Some("7".into()),
        retry: Some(1500),
    };
    let text = encoding::sse::encode(&event);
    assert_eq!(
        text,
        "event: tick\ndata: one\ndata: two\nid: 7\nretry: 1500\n\n"
    );
    assert_eq!(encoding::sse::decode(&text), vec![event]);
}

#[test]
fn sse_ignores_comments_and_empty_blocks() {
    let events = encoding::sse::decode(": keep-alive\n\n\ndata: hi\n\n");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "hi");
    assert_eq!(events[0].event, "message");
}

#[test]
fn sse_joins_crlf_split_across_chunks() {
    let mut decoder = encoding::sse::Decoder::new();
    let mut events = decoder.feed("data: first\r").unwrap();
    events.extend(decoder.feed("\ndata: second\n\n").unwrap());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "first\nsecond");
    assert_eq!(events[0].event, "message");
}

#[test]
fn sse_retains_last_event_id() {
    let mut decoder = encoding::sse::Decoder::new();
    let events = decoder.feed("id: 1\ndata: first\n\ndata: second\n\n").unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].id.as_deref(), Some("1"));
    assert_eq!(events[0].data, "first");
    assert_eq!(events[1].id.as_deref(), Some("1"));
    assert_eq!(events[1].data, "second");
}

#[test]
fn sse_strips_leading_bom_once() {
    let mut decoder = encoding::sse::Decoder::new();
    let events = decoder.feed("\u{FEFF}data: ok\n\n").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "ok");
    assert_eq!(events[0].event, "message");
    assert_eq!(events[0].id, None);
}

#[test]
fn sse_parses_events_split_across_chunks() {
    let mut decoder = encoding::sse::Decoder::new();
    let mut events = decoder.feed("id: 1\nevent: up").unwrap();
    events.extend(decoder.feed("date\ndata: hel").unwrap());
    events.extend(decoder.feed("lo\n").unwrap());
    events.extend(decoder.feed("\n").unwrap());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, "update");
    assert_eq!(events[0].data, "hello");
    assert_eq!(events[0].id.as_deref(), Some("1"));
}

#[test]
fn sse_fails_unterminated_line_over_max() {
    use encoding::sse::{Decoder, SseErrorReason};
    let mut decoder = Decoder::with_max_event_size(4);
    let err = decoder.feed("12345").unwrap_err();
    assert_eq!(
        err.reason,
        SseErrorReason::EventTooLarge { max_event_size: 4 }
    );
}

#[test]
fn sse_fails_pending_data_over_max() {
    use encoding::sse::{Decoder, SseErrorReason};
    let mut decoder = Decoder::with_max_event_size(3);
    let err = decoder.feed("data: a\n").unwrap_err();
    assert_eq!(
        err.reason,
        SseErrorReason::EventTooLarge { max_event_size: 3 }
    );
}

#[test]
fn sse_parses_pending_just_under_max() {
    use encoding::sse::Decoder;
    let mut decoder = Decoder::with_max_event_size(10);
    let events = decoder.feed("data: a\ndata: b").unwrap();
    assert!(events.is_empty());
    let events = decoder.feed("\n\n").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "a\nb");
    assert_eq!(events[0].event, "message");
}

#[test]
fn sse_parses_mixed_crlf_and_lf_endings() {
    use encoding::sse::Decoder;
    let mut decoder = Decoder::new();
    let events = decoder.feed("data: first\r\ndata: second\n\n").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "first\nsecond");
    assert_eq!(events[0].event, "message");
}

#[test]
fn sse_roundtrips_empty_data() {
    use encoding::sse::{Event, decode, encode};
    let input = Event {
        event: "message".to_string(),
        data: String::new(),
        id: None,
        retry: None,
    };
    assert_eq!(decode(&encode(&input)), vec![input]);
}

#[test]
fn sse_ignores_non_digit_retry_values() {
    use encoding::sse::Decoder;
    for value in ["123x", "+123", "-123", "1.5"] {
        let mut decoder = Decoder::new();
        let events = decoder.feed(&format!("retry: {value}\n")).unwrap();
        assert!(events.is_empty(), "{value}");
    }
}

#[test]
fn worker_errors_carry_kind_and_message() {
    let e = elm_effect_unstable::workers::WorkerError::new(
        elm_effect_unstable::workers::WorkerErrorReason::spawn("no binary"),
    );
    assert!(e.is_spawn() && !e.is_send() && !e.is_receive());
    assert_eq!(e.message(), "no binary");
    assert_eq!(e.to_string(), "WorkerError(spawn): no binary");
    assert!(elm_effect_unstable::workers::WorkerError::new(
        elm_effect_unstable::workers::WorkerErrorReason::receive("eof"),
    )
    .is_receive());
}

#[test]
fn collector_hands_buffers_out_once() {
    let mut collector = elm_effect_unstable::workers::Collector::new();
    assert!(collector.is_empty());
    let id = collector.offer(vec![1, 2, 3]);
    assert_eq!(collector.len(), 1);
    assert_eq!(collector.take(id), Some(vec![1, 2, 3]));
    assert_eq!(collector.take(id), None);
    assert!(collector.is_empty());
}

#[test]
fn runner_collects_until_close() {
    use elm_effect_unstable::workers::PlatformMessage::{Close, Payload};
    let payloads = elm_effect_unstable::workers::run_runner(&[
        Payload(vec![1]),
        Payload(vec![2]),
        Close,
        Payload(vec![3]),
    ]);
    assert_eq!(payloads, vec![vec![1], vec![2]]);
}

#[test]
fn encoding_error_guard() {
    // EncodingError.test.ts:24 - identifies encoding errors.
    let error = EncodingError::encode("Hex", "", "Invalid input");
    assert_eq!(error.kind(), "Encode");
    assert_eq!(error.module(), "Hex");
    assert_eq!(error.input(), "");
    assert_eq!(error.message(), "Invalid input");
    assert!(error.is_encoding_error());
    assert!(EncodingError::is_encoding_error_tag("EncodingError"));
    assert!(!EncodingError::is_encoding_error_tag("Error"));
}

#[test]
fn ndjson_error_guard() {
    use encoding::{NdjsonError, NdjsonKind};
    // Ndjson.test.ts:35 - identifies Ndjson errors by brand (the type).
    let error = NdjsonError::new(NdjsonKind::Pack, "no JSON representation");
    assert_eq!(error.kind(), "Pack");
    assert!(error.is_ndjson_error());
    assert_eq!(NdjsonError::TYPE_ID, "NdjsonError");
    // Unpack failures name the line through the same type.
    let bad = encoding::ndjson::decode("{bad}\n").unwrap_err();
    assert_eq!(bad.kind(), "Unpack");
    assert!(bad.to_string().contains("line 1"));
}

// ---------------------------------------------------------------------------
// Atom lifecycle (Atom.test.ts :45, :53, :62, :73, :113, :134)
// ---------------------------------------------------------------------------

use std::sync::Arc;

use elm_effect_unstable::reactivity::{map, Atom, Registry};

#[test]
fn atom_get_set_round_trips_through_the_registry() {
    // Atom.test.ts :45 — Atom.make(0) read through a fresh registry, then
    // written and read back.
    let counter = Atom::make(0i64);
    let registry = Registry::make();
    assert_eq!(registry.get(&counter), 0);
    registry.set(&counter, 1);
    assert_eq!(registry.get(&counter), 1);
}

#[test]
fn atom_without_keep_alive_resets_after_the_disposal_tick() {
    // Atom.test.ts :53 — a set value holds until the registry's disposal
    // pass; an unkept atom with no subscribers reverts to its initial value.
    let counter = Atom::make(0i64);
    let registry = Registry::make();
    registry.set(&counter, 1);
    assert_eq!(registry.get(&counter), 1);
    registry.tick();
    assert_eq!(registry.get(&counter), 0);
}

#[test]
fn atom_keep_alive_survives_the_disposal_tick() {
    // Atom.test.ts :62 — keepAlive pins the value across the disposal pass.
    let counter = Atom::make(0i64).keep_alive();
    let registry = Registry::make();
    registry.set(&counter, 1);
    assert_eq!(registry.get(&counter), 1);
    registry.tick();
    assert_eq!(registry.get(&counter), 1);
}

#[test]
fn atom_subscribe_holds_the_node_and_releases_on_cancel() {
    // Atom.test.ts :73 — a subscription notifies with the new value, keeps
    // the node alive across the disposal pass, and after the cancel the
    // next pass reverts the atom to its initial value.
    let counter = Atom::make(0i64);
    let registry = Registry::make();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<i64>::new()));
    let log = std::sync::Arc::clone(&seen);
    let cancel = registry.subscribe(&counter, move |value: i64| {
        log.lock().unwrap().push(value);
    });
    registry.set(&counter, 1);
    assert_eq!(*seen.lock().unwrap(), vec![1]);
    registry.tick();
    assert_eq!(registry.get(&counter), 1, "a subscribed node stays mounted");
    cancel.cancel();
    registry.tick();
    assert_eq!(registry.get(&counter), 0, "after cancel the node reverts");
}

#[test]
fn atom_with_equality_skips_notifications_and_keeps_identity() {
    // Atom.test.ts :113 — an equivalent assignment neither notifies nor
    // replaces the stored value, so the read keeps the same handle; a
    // differing value notifies exactly once.
    let point = Atom::make(Arc::new(Point { x: 0, y: 0 }))
        .with_equality(|a: &Arc<Point>, b: &Arc<Point>| a.x == b.x && a.y == b.y)
        .keep_alive();
    let registry = Registry::make();
    let initial = registry.get_shared(&point);
    let notifications = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = std::sync::Arc::clone(&notifications);
    registry.subscribe(&point, move |_value: Arc<Point>| {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    registry.set(&point, Arc::new(Point { x: 0, y: 0 }));
    assert_eq!(notifications.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(
        Arc::ptr_eq(&initial, &registry.get_shared(&point)),
        "an equivalent set must keep the same value handle"
    );
    registry.set(&point, Arc::new(Point { x: 1, y: 0 }));
    assert_eq!(notifications.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(registry.get(&point).x, 1);
}

#[test]
fn atom_with_equality_skips_invalidation_of_derived_atoms() {
    // Atom.test.ts :134 — a derived atom (Atom.map) rebuilds on read and
    // counts its builds; an equivalent set on the source does not
    // invalidate the derived atom, a differing set rebuilds it exactly once.
    let builds = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = std::sync::Arc::clone(&builds);
    let point = Atom::make(Arc::new(Point { x: 0, y: 0 }))
        .with_equality(|a: &Arc<Point>, b: &Arc<Point>| a.x == b.x && a.y == b.y)
        .keep_alive();
    let x = map(&point, move |p: &Arc<Point>| {
        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        p.x
    });
    let registry = Registry::make();
    let _handle = registry.subscribe(&x, |_value: i64| {});
    assert_eq!(registry.get(&x), 0);
    assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 1);
    registry.set(&point, Arc::new(Point { x: 0, y: 0 }));
    assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 1);
    registry.set(&point, Arc::new(Point { x: 2, y: 0 }));
    assert_eq!(registry.get(&x), 2);
    assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[derive(Clone, PartialEq, Debug)]
struct Point {
    x: i64,
    y: i64,
}
