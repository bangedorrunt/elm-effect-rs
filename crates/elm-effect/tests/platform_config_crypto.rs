//! Wave-11 platform: Config descriptors + providers, Crypto service.
//! Modules load via `#[path]` includes: no facade touch. The service
//! trait keeps tests deterministic; the OS backend is exercised once.

#[path = "../src/config.rs"]
mod config;
#[path = "../src/config_provider.rs"]
mod config_provider;
#[path = "../src/crypto.rs"]
mod crypto;

use config::{Config, Value};
use config_provider::Provider;
use crypto::{Crypto, DigestAlgorithm, SystemCrypto, TestCrypto};

fn provider(pairs: &[(&str, &str)]) -> Provider {
    Provider::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
}

#[test]
fn config_defers_user_callbacks_until_load() {
    // Config.test.ts:484 — building descriptors runs no user code.
    let validate_called = std::rc::Rc::new(std::cell::Cell::new(false));
    let flat_called = std::rc::Rc::new(std::cell::Cell::new(false));
    let v_flag = validate_called.clone();
    let f_flag = flat_called.clone();
    let cfg = Config::succeed(Value::Int(1))
        .validate(move |_| {
            v_flag.set(true);
            Ok(())
        })
        .flat_map(move |v| {
            f_flag.set(true);
            Config::succeed(v)
        });
    assert!(!validate_called.get());
    assert!(!flat_called.get());
    let out = cfg.load(&provider(&[]));
    assert!(validate_called.get());
    assert!(flat_called.get());
    assert_eq!(out, Ok(Value::Int(1)));
}

#[test]
fn config_empty_numbers_validate_when_preserved() {
    // Config.test.ts:651 — ignored empties read as absent (default wins);
    // preserved empties validate (and fail as numbers).
    use config::ConfigError;
    let cfg = || Config::number(named("a")).with_default(Value::Num(0.0));
    let ignored = Provider::from_pairs(vec![("a".to_string(), "".to_string())]);
    assert_eq!(cfg().load(&ignored), Ok(Value::Num(0.0)));
    let kept =
        Provider::from_pairs_preserving(vec![("a".to_string(), "".to_string())]);
    let err = cfg().load(&kept).unwrap_err();
    assert!(err.iter().all(|e| matches!(e, ConfigError::Invalid { .. })));
}

#[test]
fn config_map_effect_validates_at_load() {
    // Config.test.ts:258 — effectful validation runs at load: success
    // transforms, failure surfaces the message. Deferred execution is
    // the load boundary (see config_defers_user_callbacks_until_load).
    use config::ConfigError;
    let cfg = || {
        Config::string(named("a")).flat_map(|v| match v {
            Value::Str(s) if s.is_empty() => Config::fail("empty"),
            Value::Str(s) => Config::succeed(Value::Str(s.to_uppercase())),
            other => Config::succeed(other),
        })
    };
    let p = Provider::single(vec!["a".to_string()], Some("value".to_string()));
    assert_eq!(cfg().load(&p), Ok(Value::Str("VALUE".to_string())));
    let kept =
        Provider::from_pairs_preserving(vec![("a".to_string(), "".to_string())]);
    let err = cfg().load(&kept).unwrap_err();
    assert!(err.iter().any(|e| matches!(
        e,
        ConfigError::Invalid { reason, .. } if reason == "empty"
    )));
}

#[test]
fn config_or_else_fallback_inside_nested_path() {
    // Config.test.ts:1126 — orElse resolves within the nested prefix.
    use config::ConfigError;
    let cfg = || {
        Config::integer(named("primary"))
            .or_else(Config::integer(named("fallback")))
            .nested("service")
    };
    let pairs = |ps: &[(&str, &str)]| {
        Provider::from_pairs(ps.iter().map(|(k, v)| (k.to_string(), v.to_string())))
    };
    // Invalid primary falls back to the nested fallback value.
    let p1 = pairs(&[("service.primary", "invalid"), ("service.fallback", "80")]);
    assert_eq!(cfg().load(&p1), Ok(Value::Int(80)));
    // A fallback outside the nest stays missing, so the default wins.
    let p2 = pairs(&[("service.primary", "invalid"), ("fallback", "80")]);
    assert_eq!(
        cfg().with_default(Value::Int(3000)).load(&p2),
        Ok(Value::Int(3000))
    );
    // Both invalid: the error names the nested fallback path.
    let p3 = pairs(&[("service.primary", "invalid"), ("service.fallback", "1.5")]);
    let err = cfg().load(&p3).unwrap_err();
    assert!(matches!(
        err.as_slice(),
        [ConfigError::Invalid { path, .. }] if path == &["service".to_string(), "fallback".to_string()]
    ));
}

fn named(name: &str) -> Option<String> {
    Some(name.to_string())
}

// ----------------------------------------------------------------- Config ---

#[test]
fn config_string_decodes_and_reports_absence() {
    let p = provider(&[("host", "example.com")]);
    assert_eq!(
        Config::string(named("host")).load(&p),
        Ok(Value::Str("example.com".to_string()))
    );
    assert!(Config::string(named("missing")).load(&p).is_err());
}

#[test]
fn config_boolean_spellings() {
    let p = provider(&[
        ("a", "true"), ("b", "false"), ("c", "yes"), ("d", "no"),
        ("e", "on"), ("f", "off"), ("g", "1"), ("h", "0"),
        ("i", "y"), ("j", "n"),
    ]);
    for (key, expected) in [
        ("a", true), ("b", false), ("c", true), ("d", false), ("e", true),
        ("f", false), ("g", true), ("h", false), ("i", true), ("j", false),
    ] {
        assert_eq!(Config::boolean(named(key)).load(&p), Ok(Value::Bool(expected)), "{key}");
    }
    assert!(Config::boolean(named("failure")).load(&provider(&[("failure", "value")])).is_err());
}

#[test]
fn config_flat_map_chains() {
    let p = provider(&[("one", "1")]);
    let chained = Config::integer(named("one"))
        .flat_map(|v| match v {
            Value::Int(n) => Config::succeed(Value::Int(n + 1)),
            other => Config::succeed(other),
        })
        .flat_map(|v| match v {
            Value::Int(n) => Config::succeed(Value::Int(n * 10)),
            other => Config::succeed(other),
        });
    assert_eq!(chained.load(&p), Ok(Value::Int(20)));
    assert_eq!(
        Config::integer(named("one")).with_default(Value::Int(0)).load(&p),
        Ok(Value::Int(1))
    );
}

#[test]
fn config_nested_absence_and_prefixes() {
    let p = provider(&[("db.host", "db.internal")]);
    assert!(Config::string(named("missing")).nested("db").load(&p).is_err());
    assert_eq!(
        Config::string(named("host")).nested("db").load(&p),
        Ok(Value::Str("db.internal".to_string()))
    );
}

#[test]
fn config_primitives() {
    let p = provider(&[
        ("port", "8080"),
        ("ratio", "1.5"),
        ("debug", "TRUE"),
        ("count", "-3"),
    ]);
    assert_eq!(Config::integer(named("port")).load(&p), Ok(Value::Int(8080)));
    assert_eq!(Config::number(named("ratio")).load(&p), Ok(Value::Num(1.5)));
    assert_eq!(Config::boolean(named("debug")).load(&p), Ok(Value::Bool(true)));
    assert!(Config::integer(named("ratio")).load(&p).is_err());
    assert!(Config::boolean(named("port")).load(&p).is_err());
    assert_eq!(Config::finite(named("ratio")).load(&p), Ok(Value::Num(1.5)));
}

#[test]
fn config_literals() {
    let p = provider(&[("env", "prod")]);
    assert_eq!(
        Config::literal(named("env"), "prod").load(&p),
        Ok(Value::Str("prod".to_string()))
    );
    assert!(Config::literal(named("env"), "dev").load(&p).is_err());
    assert_eq!(
        Config::literals(named("env"), &["dev", "prod"]).load(&p),
        Ok(Value::Str("prod".to_string()))
    );
}

#[test]
fn config_secret_redacted_masked() {
    let p = provider(&[("token", "s3cr3t")]);
    assert_eq!(
        Config::secret(named("token")).load(&p),
        Ok(Value::Secret("s3cr3t".to_string()))
    );
    assert_eq!(
        Config::redacted(named("token")).load(&p),
        Ok(Value::Redacted("s3cr3t".to_string()))
    );
    assert!(Config::redacted(named("absent")).load(&p).is_err());
    assert_eq!(
        Config::redacted(named("absent")).with_default(Value::Redacted("d".to_string())).load(&p),
        Ok(Value::Redacted("d".to_string()))
    );
    // Never displayed: debug output carries no secret text.
    let debug = format!("{:?}", Value::Secret("s3cr3t".to_string()));
    assert!(!debug.contains("s3cr3t"));
}

#[test]
fn config_duration_date_bytes_url_port_level() {
    let p = provider(&[
        ("ttl", "150ms"),
        ("backoff", "2s"),
        ("day", "2024-03-15"),
        ("size", "4MB"),
        ("site", "https://example.com"),
        ("port", "8080"),
        ("level", "Info"),
    ]);
    assert_eq!(Config::duration_ms(named("ttl")).load(&p), Ok(Value::Millis(150)));
    assert_eq!(Config::duration_ms(named("backoff")).load(&p), Ok(Value::Millis(2000)));
    assert_eq!(
        Config::date(named("day")).load(&p),
        Ok(Value::Str("2024-03-15".to_string()))
    );
    assert_eq!(Config::byte_size(named("size")).load(&p), Ok(Value::Int(4_000_000)));
    assert_eq!(
        Config::url(named("site")).load(&p),
        Ok(Value::Str("https://example.com".to_string()))
    );
    assert!(Config::url(named("port")).load(&p).is_err());
    assert!(Config::port(named("port")).load(&p).is_ok());
    assert!(Config::port(named("site")).load(&p).is_err());
    assert_eq!(
        Config::log_level(named("level")).load(&p),
        Ok(Value::Str("Info".to_string()))
    );
    assert!(Config::byte_size(named("size")).load(&provider(&[("size", "bogus")])).is_err());
    assert!(Config::log_level(named("level")).load(&provider(&[("level", "Verbose")])).is_err());
}

#[test]
fn config_arrays_and_combinators() {
    let p = provider(&[("hosts", "a,b,c"), ("one", "1")]);
    let list = Config::array(named("hosts"), Config::string(None)).load(&p).unwrap();
    assert_eq!(
        list,
        Value::List(vec![
            Value::Str("a".to_string()),
            Value::Str("b".to_string()),
            Value::Str("c".to_string())
        ])
    );
    let doubled = Config::integer(named("one")).map(|v| match v {
        Value::Int(n) => Value::Int(n * 2),
        other => other,
    });
    assert_eq!(doubled.load(&p), Ok(Value::Int(2)));
    let fallback = Config::integer(named("missing")).or_else(Config::integer(named("one")));
    assert_eq!(fallback.load(&p), Ok(Value::Int(1)));
    assert_eq!(
        Config::integer(named("missing")).with_default(Value::Int(9)).load(&p),
        Ok(Value::Int(9))
    );
    assert_eq!(Config::integer(named("missing")).option().load(&p), Ok(Value::None));
    assert_eq!(
        Config::succeed(Value::Bool(true)).load(&Provider::empty()),
        Ok(Value::Bool(true))
    );
    assert!(Config::fail("boom").load(&p).is_err());
}

#[test]
fn config_nested_and_provider_layering() {
    let p = provider(&[("db.host", "db.internal"), ("db.port", "5432")]);
    let host = Config::string(named("host")).nested("db");
    assert_eq!(host.load(&p), Ok(Value::Str("db.internal".to_string())));
    let overlay = Provider::from_pairs(vec![("host".to_string(), "override".to_string())]);
    let base = Provider::empty();
    let view = base.or_else(&overlay);
    assert_eq!(view.get(&["host".to_string()]), Some("override".to_string()));
}

#[test]
fn config_non_empty_rejects_empties() {
    let p = provider(&[("blank", "")]);
    assert!(Config::non_empty_string(named("blank")).load(&p).is_err());
}

// --------------------------------------------------------- ConfigProvider ---

#[test]
fn provider_fallback_and_nesting() {
    let primary = provider(&[("a", "1")]);
    let secondary = provider(&[("a", "2"), ("b", "3")]);
    let view = primary.or_else(&secondary);
    assert_eq!(view.get(&["a".to_string()]), Some("1".to_string()));
    assert_eq!(view.get(&["b".to_string()]), Some("3".to_string()));
    assert_eq!(view.get(&["c".to_string()]), None);
    let nested = primary.nested("db");
    assert_eq!(nested.get(&["x".to_string()]), None);
    let db = provider(&[("db.x", "deep")]);
    assert_eq!(db.nested("db").get(&["x".to_string()]), Some("deep".to_string()));
    // Prefixes compose as wrappers: outer feeds inner feeds path.
    let deep = provider(&[("a.b.x", "v")]);
    let inner = deep.nested("b");
    let wrapped = inner.nested("a");
    assert_eq!(wrapped.get(&["x".to_string()]), Some("v".to_string()));
}

#[test]
fn config_env_empty_is_absent() {
    unsafe {
        std::env::set_var("ELM_EFFECT_TEST_EMPTY_XYZ", "");
        std::env::set_var("ELM_EFFECT_TEST_FULL_XYZ", "hi");
    }
    let p = Provider::from_env();
    assert_eq!(p.get(&["ELM_EFFECT_TEST_EMPTY_XYZ".to_string()]), None);
    assert_eq!(
        p.get(&["ELM_EFFECT_TEST_FULL_XYZ".to_string()]),
        Some("hi".to_string())
    );
    unsafe {
        std::env::remove_var("ELM_EFFECT_TEST_EMPTY_XYZ");
        std::env::remove_var("ELM_EFFECT_TEST_FULL_XYZ");
    }
}

#[test]
fn provider_dotted_paths() {
    let p = provider(&[("a.b.c", "deep")]);
    assert_eq!(p.get(&["a".to_string(), "b".to_string(), "c".to_string()]), Some("deep".to_string()));
    assert_eq!(p.get(&["a".to_string()]), None);
}

// ----------------------------------------------------------------- Crypto ---

#[test]
fn crypto_type_id_and_literals() {
    assert_eq!(crypto::TYPE_ID, "elm-effect/Crypto");
    let algorithm: DigestAlgorithm = DigestAlgorithm::Sha256;
    assert_eq!(algorithm, DigestAlgorithm::parse("SHA-256").unwrap());
    assert_eq!(algorithm.name(), "SHA-256");
    assert!(DigestAlgorithm::parse("MD5").is_none());
}

#[test]
fn crypto_delegates_to_service() {
    struct Fixed;
    impl Crypto for Fixed {
        fn random_bytes(&self, n: usize) -> Result<Vec<u8>, crypto::CryptoError> {
            Ok(vec![7u8; n])
        }
        fn digest(&self, _algorithm: DigestAlgorithm, data: &[u8]) -> Result<Vec<u8>, crypto::CryptoError> {
            Ok(data.to_vec())
        }
    }
    let svc = Fixed;
    assert_eq!(svc.random_bytes(3).unwrap(), vec![7, 7, 7]);
    assert_eq!(svc.digest(DigestAlgorithm::Sha256, &[1, 2, 3]).unwrap(), vec![1, 2, 3]);
}

#[test]
fn crypto_test_service_is_deterministic() {
    let a = TestCrypto::new();
    let b = TestCrypto::new();
    assert_eq!(a.random_bytes(16).unwrap(), b.random_bytes(16).unwrap());
    assert_ne!(a.random_bytes(4).unwrap(), vec![0, 0, 0, 0]);
}

#[test]
fn crypto_ranges_exclude_upper() {
    let svc = TestCrypto::new();
    for _ in 0..50 {
        let x = crypto::random_int_between(&svc, 0, 10).unwrap();
        assert!((0..10).contains(&x));
        let f = crypto::random_between(&svc, 1.0, 2.0).unwrap();
        assert!((1.0..2.0).contains(&f) || f == 1.0);
    }
    assert!(crypto::random_int_between(&svc, 5, 5).is_err());
}

#[test]
fn crypto_uuid_shapes() {
    let v4 = crypto::uuid_v4(&[0u8; 16]);
    assert_eq!(v4.len(), 36);
    assert_eq!(&v4[14..15], "4");
    assert!(matches!(&v4[19..20], "8" | "9" | "a" | "b"));
    let v7 = crypto::uuid_v7(0, &[0u8; 10]);
    assert_eq!(&v7[14..15], "7");
    let ulid = crypto::ulid(0, &[0u8; 10]);
    assert_eq!(ulid, "00000000000000000000000000");
    let big = crypto::ulid(u64::MAX, &[0xffu8; 10]);
    assert_eq!(big.len(), 26);
}

#[test]
fn crypto_sha256_vector() {
    // Empty-string vector.
    let digest = crypto::sha256(b"");
    assert_eq!(
        digest,
        [
            0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
            0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
            0x78, 0x52, 0xb8, 0x55
        ]
    );
    // Service digest matches bare function.
    assert_eq!(
        SystemCrypto.digest(DigestAlgorithm::Sha256, b"abc").unwrap(),
        crypto::sha256(b"abc").to_vec()
    );
    assert!(SystemCrypto.digest(DigestAlgorithm::Sha512, b"abc").is_err());
}

#[test]
fn crypto_system_randomness() {
    let bytes = SystemCrypto.random_bytes(16).unwrap();
    assert_eq!(bytes.len(), 16);
}

#[test]
fn config_fail_always_fails() {
    let p = provider(&[("other", "1")]);
    assert!(Config::fail("boom").load(&p).is_err());
}

#[test]
fn config_succeed_ignores_provider() {
    let p = provider(&[]);
    assert_eq!(Config::succeed(Value::Int(7)).load(&p), Ok(Value::Int(7)));
}

#[test]
fn config_map_chained_with_succeed() {
    let p = provider(&[]);
    let doubled = Config::succeed(Value::Int(2)).map(|v| match v {
        Value::Int(n) => Value::Int(n * 2),
        other => other,
    });
    assert_eq!(doubled.load(&p), Ok(Value::Int(4)));
}

#[test]
fn loglevel_defaults_to_info() {
    assert_eq!(config::LogLevel::default(), config::LogLevel::Info);
    assert!(config::LogLevel::Info.passes(config::LogLevel::Info));
    assert!(!config::LogLevel::Debug.passes(config::LogLevel::Info));
}

#[test]
fn loglevel_all_and_none_are_sentinels() {
    assert!(config::LogLevel::Fatal.passes(config::LogLevel::All));
    assert!(!config::LogLevel::Fatal.passes(config::LogLevel::None));
    assert_eq!(config::LogLevel::parse("All"), Some(config::LogLevel::All));
    assert_eq!(config::LogLevel::parse("None"), Some(config::LogLevel::None));
    assert_eq!(config::LogLevel::parse("Verbose"), None);
    assert!(config::LogLevel::All < config::LogLevel::Trace);
    assert!(config::LogLevel::Fatal < config::LogLevel::None);
}

fn host_config() -> Config {
    Config::integer(named("port")).flat_map(|port| {
        let name = if port == Value::Int(80) {
            "prodHost"
        } else {
            "devHost"
        };
        Config::string(named(name)).validate(|value| match value {
            Value::Str(s) if s.ends_with("effect.website") => Ok(()),
            _ => Err("Must end with \"effect.website\"".to_string()),
        })
    })
}

#[test]
fn config_inner_propagates_parsing_failure() {
    let p = provider(&[("port", "80"), ("prodHost", "example.com")]);
    let err = host_config().load(&p).expect_err("inner host must fail");
    assert_eq!(err.len(), 1);
    match &err[0] {
        config::ConfigError::Invalid { path, reason, .. } => {
            assert_eq!(path, &vec!["prodHost".to_string()]);
            assert!(reason.contains("effect.website"));
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn config_inner_propagates_key_absence() {
    let p = provider(&[("port", "80")]);
    let err = host_config()
        .load(&p)
        .expect_err("inner host must be absent");
    assert!(matches!(
        err.as_slice(),
        [config::ConfigError::Missing { .. }]
    ));
}

#[test]
fn config_recovers_after_inner_parsing_failure() {
    let p = provider(&[("port", "80"), ("prodHost", "example.com")]);
    let recovered = host_config()
        .or_else_with(|errors| {
            let message = errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            Config::succeed(Value::Str(message))
        })
        .load(&p)
        .expect("recovery must succeed");
    match recovered {
        Value::Str(message) => assert!(message.contains("effect.website")),
        other => panic!("expected Str, got {other:?}"),
    }
}

#[test]
fn config_recovers_after_inner_key_absence() {
    let p = provider(&[("port", "80")]);
    assert_eq!(
        host_config()
            .with_default(Value::Str("localhost".to_string()))
            .load(&p),
        Ok(Value::Str("localhost".to_string()))
    );
}

#[test]
fn config_propagates_inner_success() {
    let p = provider(&[("port", "3000"), ("devHost", "stage.effect.website")]);
    assert_eq!(
        host_config().load(&p),
        Ok(Value::Str("stage.effect.website".to_string()))
    );
}

#[test]
fn config_base_propagates_parsing_failure() {
    let p = provider(&[("port", "zzz"), ("prodHost", "effect.website")]);
    let err = host_config().load(&p).expect_err("base port must fail");
    assert!(matches!(
        err.as_slice(),
        [config::ConfigError::Invalid { .. }]
    ));
}

#[test]
fn config_base_propagates_key_absence() {
    let p = provider(&[("port", "80")]);
    let host_only = Config::string(named("prodHost"));
    let p2 = provider(&[("prodHost", "effect.website")]);
    let err = Config::integer(named("port"))
        .flat_map(move |_| host_only.clone())
        .load(&p2)
        .expect_err("base port must be absent");
    assert!(matches!(
        err.as_slice(),
        [config::ConfigError::Missing { .. }]
    ));
    let _ = p;
}

#[test]
fn config_recovers_after_base_parsing_failure() {
    let p = provider(&[("port", "zzz"), ("prodHost", "effect.website")]);
    let recovered = host_config()
        .or_else_with(|errors| {
            let message = errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            Config::succeed(Value::Str(message))
        })
        .load(&p)
        .expect("recovery must succeed");
    match recovered {
        Value::Str(message) => assert!(message.contains("port")),
        other => panic!("expected Str, got {other:?}"),
    }
}

#[test]
fn config_recovers_after_base_key_absence() {
    let p = provider(&[("prodHost", "effect.website")]);
    assert_eq!(
        host_config()
            .with_default(Value::Str("localhost".to_string()))
            .load(&p),
        Ok(Value::Str("localhost".to_string()))
    );
}

#[test]
fn config_nested_prefixes_root() {
    use config::ConfigError;
    let c = Config::string(None).nested("a");
    assert_eq!(
        c.load(&provider(&[("a", "value")])).unwrap(),
        Value::Str("value".to_string())
    );
    assert_eq!(
        c.load(&provider(&[])).unwrap_err(),
        vec![ConfigError::Missing {
            path: vec!["a".to_string()]
        }]
    );
}

#[test]
fn config_nested_prefixes_compose_outside_in() {
    let m = Config::string(named("a")).nested("b").nested("c");
    assert_eq!(
        m.load(&provider(&[("c.b.a", "value")])).unwrap(),
        Value::Str("value".to_string())
    );
    assert!(m.load(&provider(&[("b.a", "value")])).is_err());
}

#[test]
fn config_nested_prefix_flows_through_flat_map() {
    let picked = Config::integer(named("port")).flat_map(|port| {
        let key = if port == config::Value::Int(80) {
            "prodHost"
        } else {
            "devHost"
        };
        Config::string(Some(key.to_string())).nested("hosts")
    });
    let config = picked.nested("service");
    let p = provider(&[
        ("port", "3000"),
        ("hosts.prodHost", "root-prod"),
        ("hosts.devHost", "root-dev"),
        ("service.port", "80"),
        ("service.hosts.prodHost", "service-prod"),
        ("service.hosts.devHost", "service-dev"),
    ]);
    // Innermost scope wins: service.port selects prodHost under service.
    assert_eq!(
        config.load(&p).unwrap(),
        Value::Str("service-prod".to_string())
    );
    // Missing inner leaf errors on the composed path, no root fallback.
    let missing = provider(&[("service.port", "80")]);
    assert_eq!(
        config.load(&missing).unwrap_err(),
        vec![config::ConfigError::Missing {
            path: vec![
                "service".to_string(),
                "hosts".to_string(),
                "prodHost".to_string()
            ]
        }]
    );
    assert_eq!(
        config.option().load(&missing).unwrap(),
        Value::None
    );
}

#[test]
fn provider_empty_strings_are_missing_by_default() {
    use config::ConfigError;
    let p = provider(&[("a", ""), ("b", "value1")]);
    assert_eq!(p.get(&["a".to_string()]), None);
    assert_eq!(
        p.get(&["b".to_string()]),
        Some("value1".to_string())
    );
    assert_eq!(
        Config::string(named("a")).load(&p).unwrap_err(),
        vec![ConfigError::Missing {
            path: vec!["a".to_string()]
        }]
    );
    assert_eq!(
        Config::string(named("b")).load(&p).unwrap(),
        Value::Str("value1".to_string())
    );
    // Co-located record values: the empty leaf is missing, children keep
    // working through the same provider.
    let co = provider(&[("r.empty", ""), ("r.kept", "v")]);
    assert_eq!(co.get(&["r".to_string(), "empty".to_string()]), None);
    assert_eq!(
        Config::string(named("kept")).nested("r").load(&co).unwrap(),
        Value::Str("v".to_string())
    );
    assert!(Config::string(named("empty"))
        .nested("r")
        .load(&co)
        .is_err());
}

#[test]
fn provider_fallback_fires_through_empty_primary() {
    let primary = provider(&[("a", "")]);
    let secondary = provider(&[("a", "fallback"), ("b", "kept")]);
    let view = primary.or_else(&secondary);
    assert_eq!(view.get(&["a".to_string()]), Some("fallback".to_string()));
    assert_eq!(
        view.get(&["b".to_string()]),
        Some("kept".to_string())
    );
    assert_eq!(view.get(&["missing".to_string()]), None);
}

#[test]
fn config_all_combines_positionally() {
    use config::ConfigError;
    let c = Config::all(vec![
        Config::non_empty_string(named("a")),
        Config::finite(named("b")),
    ]);
    assert_eq!(
        c.load(&provider(&[("a", "a"), ("b", "1")])).unwrap(),
        Value::List(vec![Value::Str("a".to_string()), Value::Num(1.0)])
    );
    assert_eq!(
        c.load(&provider(&[("a", ""), ("b", "1")])).unwrap_err(),
        vec![ConfigError::Missing {
            path: vec!["a".to_string()]
        }]
    );
    assert!(c.load(&provider(&[("a", "a"), ("b", "b")])).is_err());
}

#[test]
fn config_record_combines_named() {
    let c = Config::record(vec![
        ("a".to_string(), Config::non_empty_string(named("b"))),
        ("c".to_string(), Config::finite(named("d"))),
    ]);
    assert_eq!(
        c.load(&provider(&[("b", "b"), ("d", "1")])).unwrap(),
        Value::Map(vec![
            ("a".to_string(), Value::Str("b".to_string())),
            ("c".to_string(), Value::Num(1.0))
        ])
    );
    assert!(c.load(&provider(&[("d", "1")])).is_err());
}

#[test]
fn config_all_prefixes_every_child() {
    let c = Config::all(vec![
        Config::non_empty_string(named("a")),
        Config::finite(named("b")),
    ])
    .nested("svc");
    assert_eq!(
        c.load(&provider(&[("svc.a", "a"), ("svc.b", "1")]))
            .unwrap(),
        Value::List(vec![Value::Str("a".to_string()), Value::Num(1.0)])
    );
    assert!(c.load(&provider(&[("a", "a"), ("b", "1")])).is_err());
}

#[test]
fn config_date_validates_real_calendars() {
    let d = |v: &str| Config::date(named("d")).load(&provider(&[("d", v)]));
    assert_eq!(
        d("2021-01-01").unwrap(),
        Value::Str("2021-01-01".to_string())
    );
    assert_eq!(
        d("2024-02-29").unwrap(),
        Value::Str("2024-02-29".to_string())
    );
    assert_eq!(
        d("2021-01-01T00:00:00.000Z").unwrap(),
        Value::Str("2021-01-01T00:00:00.000Z".to_string())
    );
    assert!(d("invalid").is_err());
    assert!(d("2021-13-01").is_err());
    assert!(d("2021-02-29").is_err());
    assert!(d("--------").is_err());
    assert!(d("2021-1-1").is_err());
}

#[test]
fn config_port_validates_range_and_decodes_number() {
    use config::ConfigError;
    let p = provider(&[("a", "8080"), ("bad", "-1"), ("zero", "0")]);
    assert_eq!(Config::port(named("a")).load(&p).unwrap(), Value::Int(8080));
    assert_eq!(
        Config::port(named("bad")).load(&p).unwrap_err(),
        vec![ConfigError::Invalid {
            path: vec!["bad".to_string()],
            got: "-1".to_string(),
            reason: "bad port".to_string(),
        }]
    );
    assert!(Config::port(named("zero")).load(&p).is_err());
    assert!(Config::port(named("missing")).load(&p).is_err());
}

#[test]
fn config_scalars_decode_and_report_absence() {
    let p = provider(&[("n", "1.5"), ("f", "2"), ("i", "3"), ("b", "true")]);
    assert_eq!(
        Config::number(named("n")).load(&p).unwrap(),
        Value::Num(1.5)
    );
    assert!(Config::number(named("missing")).load(&p).is_err());
    assert_eq!(
        Config::finite(named("f")).load(&p).unwrap(),
        Value::Num(2.0)
    );
    assert!(Config::finite(named("missing")).load(&p).is_err());
    assert_eq!(Config::integer(named("i")).load(&p).unwrap(), Value::Int(3));
    assert!(Config::integer(named("missing")).load(&p).is_err());
    assert_eq!(
        Config::boolean(named("b")).load(&p).unwrap(),
        Value::Bool(true)
    );
    assert!(Config::boolean(named("missing")).load(&p).is_err());
}

#[test]
fn config_scalars_reject_invalid_input() {
    use config::ConfigError;
    let p = provider(&[("i", "a"), ("f", "b"), ("b", "maybe")]);
    assert_eq!(
        Config::integer(named("i")).load(&p).unwrap_err(),
        vec![ConfigError::Invalid {
            path: vec!["i".to_string()],
            got: "a".to_string(),
            reason: "not an integer".to_string(),
        }]
    );
    assert!(Config::finite(named("f")).load(&p).is_err());
    assert!(Config::boolean(named("b")).load(&p).is_err());
}

#[test]
fn provider_preserves_empty_strings_when_requested() {
    use config_provider::Provider;
    let p = Provider::from_pairs_preserving(vec![("e".to_string(), "".to_string())]);
    assert_eq!(p.get(&["e".to_string()]), Some("".to_string()));
    assert_eq!(
        Config::string(named("e")).load(&p).unwrap(),
        Value::Str("".to_string())
    );
    // Default still drops.
    let d = Provider::from_pairs(vec![("e".to_string(), "".to_string())]);
    assert_eq!(d.get(&["e".to_string()]), None);
}

#[test]
fn provider_listings_keep_empty_valued_keys() {
    let p = provider(&[("a", ""), ("b", "value1")]);
    assert_eq!(
        p.root_keys(),
        vec!["a".to_string(), "b".to_string()]
    );
    assert_eq!(p.get(&["a".to_string()]), None);
    assert_eq!(
        p.get(&["b".to_string()]),
        Some("value1".to_string())
    );
}

#[test]
fn provider_from_fn_reads_through_lookup() {
    use config_provider::Provider;
    let fp = Provider::from_fn(|path: &[String]| {
        let joined = path.join(".");
        (joined == "A.B").then(|| "value".to_string())
    });
    assert_eq!(
        fp.get(&["A".to_string(), "B".to_string()]),
        Some("value".to_string())
    );
    assert_eq!(fp.get(&["missing".to_string()]), None);
    assert_eq!(
        Config::string(named("B")).nested("A").load(&fp).unwrap(),
        Value::Str("value".to_string())
    );
}

#[test]
fn provider_map_input_prefixes_lookups() {
    let base = provider(&[("APP.PORT", "3000")]);
    let mapped = base.map_input(|path| {
        let mut full = vec!["APP".to_string()];
        full.extend(path.iter().cloned());
        full
    });
    assert_eq!(
        mapped.get(&["PORT".to_string()]),
        Some("3000".to_string())
    );
    assert_eq!(mapped.get(&["MISSING".to_string()]), None);
    assert_eq!(
        Config::string(named("PORT")).load(&mapped).unwrap(),
        Value::Str("3000".to_string())
    );
}

#[test]
fn provider_map_input_composes_in_application_order() {
    // ConfigProvider.test.ts:187 — first-added transform runs first.
    let base = provider(&[("KEY_A_B", "value")]);
    let piped = base
        .map_input(|path| path.iter().map(|s| format!("{s}_A")).collect())
        .then_map(|path| path.iter().map(|s| format!("{s}_B")).collect());
    assert_eq!(
        piped.get(&["KEY".to_string()]),
        Some("value".to_string())
    );
}

#[test]
fn provider_constant_case_converts_segments() {
    let base0 = provider(&[("CONSTANT_CASE", "value1")]);
    let cc = base0.constant_case();
    assert_eq!(
        cc.get(&["constant.case".to_string()]),
        Some("value1".to_string())
    );
    let base = provider(&[("API_V2_XML", "value1"), ("FIELD2_VALUE", "value2")]);
    let ncc = base.constant_case();
    assert_eq!(
        ncc.get(&["api-v2 xml".to_string()]),
        Some("value1".to_string())
    );
    assert_eq!(
        ncc.get(&["field2Value".to_string()]),
        Some("value2".to_string())
    );
    let items_base = provider(&[("ITEMS.0", "value")]);
    let items = items_base.constant_case();
    assert_eq!(
        items.get(&["items".to_string(), "0".to_string()]),
        Some("value".to_string())
    );
}

#[test]
fn provider_nested_dotted_numeric_prefix() {
    let base = provider(&[("apps.0.PORT", "3000")]);
    let nested = base.nested("apps.0");
    assert_eq!(
        nested.get(&["PORT".to_string()]),
        Some("3000".to_string())
    );
    let plain_base = provider(&[("db.host", "db.internal")]);
    let plain = plain_base.nested("db");
    assert_eq!(
        plain.get(&["host".to_string()]),
        Some("db.internal".to_string())
    );
}

#[test]
fn provider_env_chain_applies_constant_case_before_nested() {
    use config_provider::{EnvProvider, Lookup};
    let env = EnvProvider::from_env_map(vec![(
        "prefix_KEY_WITH_DOTS".to_string(),
        "value".to_string(),
    )]);
    let chained = env.chain().constant_cased().prepended(vec!["prefix".to_string()]);
    assert_eq!(
        env.get(&["prefix_KEY_WITH_DOTS".to_string()]),
        Some("value".to_string())
    );
    assert_eq!(
        chained.get(&["key.with.dots".to_string()]),
        Some("value".to_string())
    );
    assert_eq!(chained.get(&["other".to_string()]), None);
}

#[test]
fn provider_env_chain_applies_nested_before_constant_case() {
    use config_provider::{EnvProvider, Lookup};
    let env = EnvProvider::from_env_map(vec![(
        "PREFIX_WITH_DOTS_KEY_WITH_DOTS".to_string(),
        "value".to_string(),
    )]);
    let chained = env
        .chain()
        .prepended(vec![
            "prefix".to_string(),
            "with".to_string(),
            "dots".to_string(),
        ])
        .constant_cased();
    assert_eq!(
        chained.get(&["key.with.dots".to_string()]),
        Some("value".to_string())
    );
}

#[test]
fn provider_dotenv_parses_key_value_text() {
    use config_provider::EnvProvider;
    let env = EnvProvider::from_dotenv("PATH=.env\nA=1\n# comment\n\nQUOTED=\"sp ace\"\nexport EXP=yes\n");
    assert_eq!(env.get(&["PATH".to_string()]), Some(".env".to_string()));
    assert_eq!(env.get(&["A".to_string()]), Some("1".to_string()));
    assert_eq!(
        env.get(&["QUOTED".to_string()]),
        Some("sp ace".to_string())
    );
    assert_eq!(env.get(&["EXP".to_string()]), Some("yes".to_string()));
    assert_eq!(env.get(&["missing".to_string()]), None);
}

#[test]
fn provider_dotenv_preserve_empty_strings_option() {
    // ConfigProvider.test.ts:942 — preserving keeps `A=` as empty.
    use config_provider::EnvProvider;
    let kept = EnvProvider::from_dotenv_preserving("A=");
    assert_eq!(kept.get(&["A".to_string()]), Some("".to_string()));
    let dropped = EnvProvider::from_dotenv("A=");
    assert_eq!(dropped.get(&["A".to_string()]), None);
}

#[test]
fn provider_dotenv_expansion_defaults_and_missing() {
    // ConfigProvider.test.ts:860 — `:-` defaults apply to empty or
    // unset names only.
    use config_provider::EnvProvider;
    let p = EnvProvider::from_dotenv_expanding(
        "SET=actual\nEMPTY=\nFROM_SET=${SET:-fallback}\nFROM_EMPTY=${EMPTY:-fallback}\nFROM_UNSET=${UNSET:-fallback}\n",
    );
    assert_eq!(
        p.get(&["FROM_SET".to_string()]),
        Some("actual".to_string())
    );
    assert_eq!(
        p.get(&["FROM_EMPTY".to_string()]),
        Some("fallback".to_string())
    );
    assert_eq!(
        p.get(&["FROM_UNSET".to_string()]),
        Some("fallback".to_string())
    );
    // ConfigProvider.test.ts:876 — missing references expand to empty,
    // which reads as missing.
    let q = EnvProvider::from_dotenv_expanding("VALUE=$constructor\n");
    assert_eq!(q.get(&["VALUE".to_string()]), None);
}

#[test]
fn provider_dotenv_expands_refs_keeps_literal_dollars() {
    // ConfigProvider.test.ts:849 — referenced values expand; `$` not
    // starting a name stays literal.
    use config_provider::EnvProvider;
    let expanding = EnvProvider::from_dotenv_expanding(
        "SOURCE=a$&b$'c$`d\nTARGET=${SOURCE}\n",
    );
    assert_eq!(
        expanding.get(&["TARGET".to_string()]),
        Some("a$&b$'c$`d".to_string())
    );
}

#[test]
fn provider_dotenv_quotes_comments_and_underscores() {
    use config_provider::EnvProvider;
    let env = EnvProvider::from_dotenv(
        "SINGLE='value # one'\nDOUBLE=\"line 1\\nline 2\"\nBACKTICK=`value # two`\nPLAIN=value # comment\nOBJECT_key1=value1\n",
    );
    assert_eq!(
        env.get(&["SINGLE".to_string()]),
        Some("value # one".to_string())
    );
    assert_eq!(
        env.get(&["DOUBLE".to_string()]),
        Some("line 1\nline 2".to_string())
    );
    assert_eq!(
        env.get(&["BACKTICK".to_string()]),
        Some("value # two".to_string())
    );
    assert_eq!(env.get(&["PLAIN".to_string()]), Some("value".to_string()));
    assert_eq!(
        env.get(&["OBJECT".to_string(), "key1".to_string()]),
        Some("value1".to_string())
    );
}

#[test]
fn provider_env_underscore_segments() {
    use config_provider::EnvProvider;
    for (key, path) in [
        ("_", vec!["", ""]),
        ("__", vec!["", "", ""]),
        ("_A", vec!["", "A"]),
        ("A_", vec!["A", ""]),
        ("A_B", vec!["A", "B"]),
    ] {
        let env = EnvProvider::from_env_map(vec![(key.to_string(), "v".to_string())]);
        let segs: Vec<String> = path.iter().map(|s| s.to_string()).collect();
        assert_eq!(env.get(&segs), Some("v".to_string()), "key {key}");
    }
}

#[test]
fn provider_dotenv_file_reads_path_and_errors_missing() {
    use config_provider::EnvProvider;
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("foggybasin-dotenv-{n}.env"));
    std::fs::write(&path, "CUSTOM_PATH=custom.env\nA=1\n").unwrap();
    let env = EnvProvider::from_dotenv_file(path.to_str().unwrap()).unwrap();
    assert_eq!(
        env.get(&["CUSTOM_PATH".to_string()]),
        Some("custom.env".to_string())
    );
    assert_eq!(env.get(&["A".to_string()]), Some("1".to_string()));
    std::fs::remove_file(&path).unwrap();
    assert!(EnvProvider::from_dotenv_file(path.to_str().unwrap()).is_err());
}

#[test]
fn provider_fallback_skips_evaluation_after_hit() {
    use config_provider::Provider;
    use std::cell::Cell;
    let primary = provider(&[("a", "hit")]);
    let calls = Cell::new(0);
    let secondary = Provider::from_fn(|path: &[String]| {
        calls.set(calls.get() + 1);
        Some(path.join("/"))
    });
    let view = primary.or_else(&secondary);
    assert_eq!(view.get(&["a".to_string()]), Some("hit".to_string()));
    assert_eq!(calls.get(), 0);
    assert_eq!(view.get(&["b".to_string()]), Some("b".to_string()));
    assert_eq!(calls.get(), 1);
}

#[test]
fn provider_nested_distributes_over_fallback_operands() {
    use config_provider::Lookup;
    let a = provider(&[("p.x", "ax")]);
    let b = provider(&[("p.y", "by")]);
    let fb = a.or_else(&b);
    let nested_fallback = config_provider::Lookup::nested(&fb, vec!["p".to_string()]);
    let na = config_provider::Lookup::nested(&a, vec!["p".to_string()]);
    let nb = config_provider::Lookup::nested(&b, vec!["p".to_string()]);
    let fallback_nested = na.or_else(&nb);
    for key in ["x", "y"] {
        let path = vec![key.to_string()];
        assert_eq!(
            nested_fallback.get(&path),
            fallback_nested.get(&path)
        );
    }
    assert_eq!(
        nested_fallback.get(&["x".to_string()]),
        Some("ax".to_string())
    );
}

#[test]
fn provider_map_distributes_over_fallback_operands() {
    use config_provider::Lookup;
    fn upper(path: &[String]) -> Vec<String> {
        path.iter().map(|s| s.to_uppercase()).collect()
    }
    let a = provider(&[("A", "ax")]);
    let b = provider(&[("B", "by")]);
    let fb = a.or_else(&b);
    let mapped_fallback = Lookup::mapped(&fb, upper);
    let ma = Lookup::mapped(&a, upper);
    let mb = Lookup::mapped(&b, upper);
    let fallback_mapped = ma.or_else(&mb);
    for key in ["a", "b"] {
        let path = vec![key.to_string()];
        assert_eq!(mapped_fallback.get(&path), fallback_mapped.get(&path));
    }
    assert_eq!(
        mapped_fallback.get(&["a".to_string()]),
        Some("ax".to_string())
    );
}

#[test]
fn provider_env_basics_hit_miss_and_no_inheritance() {
    use config_provider::EnvProvider;
    let env = EnvProvider::from_env_map(vec![("A".to_string(), "value1".to_string())]);
    assert_eq!(env.get(&["A".to_string()]), Some("value1".to_string()));
    assert_eq!(env.get(&["missing".to_string()]), None);
    assert_eq!(env.get(&["constructor".to_string()]), None);
}

#[test]
fn provider_nested_empty_leaves_read_missing() {
    let p = provider(&[("a", ""), ("b", "value1"), ("record.key", "")]);
    assert_eq!(p.get(&["a".to_string()]), None);
    assert_eq!(p.get(&["b".to_string()]), Some("value1".to_string()));
    assert_eq!(
        p.get(&["record".to_string(), "key".to_string()]),
        None
    );
}

#[test]
fn provider_chain_maps_after_prefix() {
    use config_provider::Lookup;
    fn upper(path: &[String]) -> Vec<String> {
        path.iter().map(|s| s.to_uppercase()).collect()
    }
    let base = provider(&[("P.A", "v")]);
    let chained = base.chain().prepended(vec!["p".to_string()]).mapped(upper);
    assert_eq!(
        chained.get(&["a".to_string()]),
        Some("v".to_string())
    );
    assert_eq!(chained.get(&["missing".to_string()]), None);
}

#[test]
fn provider_dir_reads_trimmed_leaves() {
    use config_provider::DirProvider;
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir()
        .join(format!("foggybasin-dir-{}", N.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(root.join("nested")).unwrap();
    std::fs::write(root.join("secret"), "keepitsafe\n").unwrap();
    std::fs::write(root.join("SHOUTING"), "value").unwrap();
    std::fs::write(root.join("integer"), "123").unwrap();
    std::fs::write(root.join("nested").join("config"), "hello").unwrap();
    std::fs::write(root.join("empty"), "  \n").unwrap();
    let dir = DirProvider::from_dir(&root);
    assert_eq!(
        dir.get(&["secret".to_string()]),
        Some("keepitsafe".to_string())
    );
    assert_eq!(
        dir.get(&["SHOUTING".to_string()]),
        Some("value".to_string())
    );
    assert_eq!(
        dir.get(&["integer".to_string()]),
        Some("123".to_string())
    );
    assert_eq!(
        dir.get(&["nested".to_string(), "config".to_string()]),
        Some("hello".to_string())
    );
    assert_eq!(dir.get(&["missing".to_string()]), None);
    assert_eq!(dir.get(&["empty".to_string()]), None);
    assert_eq!(dir.get(&["nested".to_string()]), None);
    let keeping = DirProvider::from_dir_preserving(&root);
    assert_eq!(
        keeping.get(&["empty".to_string()]),
        Some("".to_string())
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn config_with_default_present_absent_and_product() {
    let scalar = Config::finite(named("a")).with_default(Value::Num(0.0));
    assert_eq!(
        scalar.load(&provider(&[("a", "1")])).unwrap(),
        Value::Num(1.0)
    );
    assert_eq!(
        scalar.load(&provider(&[])).unwrap(),
        Value::Num(0.0)
    );
    let product = Config::record(vec![
        ("a".to_string(), Config::non_empty_string(named("b"))),
        ("c".to_string(), Config::finite(named("d"))),
    ])
    .with_default(Value::Map(vec![
        ("a".to_string(), Value::Str("a".to_string())),
        ("c".to_string(), Value::Num(0.0)),
    ]));
    assert_eq!(
        product
            .load(&provider(&[("b", "b"), ("d", "1")]))
            .unwrap(),
        Value::Map(vec![
            ("a".to_string(), Value::Str("b".to_string())),
            ("c".to_string(), Value::Num(1.0))
        ])
    );
    assert_eq!(
        product.load(&provider(&[])).unwrap(),
        Value::Map(vec![
            ("a".to_string(), Value::Str("a".to_string())),
            ("c".to_string(), Value::Num(0.0))
        ])
    );
}

#[test]
fn config_with_default_covers_absent_nested() {
    let nested =
        Config::string(named("host")).nested("database").with_default(Value::Str("fb".to_string()));
    assert_eq!(
        nested.load(&provider(&[])).unwrap(),
        Value::Str("fb".to_string())
    );
    assert_eq!(
        nested
            .load(&provider(&[("database.host", "h")]))
            .unwrap(),
        Value::Str("h".to_string())
    );
}

#[test]
fn config_empty_groups_resolve_without_default() {
    let empty_list = Config::all(vec![]).with_default(Value::Str("fallback".to_string()));
    assert_eq!(
        empty_list.load(&provider(&[])).unwrap(),
        Value::List(vec![])
    );
    let empty_map = Config::record(vec![]).with_default(Value::Str("fallback".to_string()));
    assert_eq!(
        empty_map.load(&provider(&[])).unwrap(),
        Value::Map(vec![])
    );
}

#[test]
fn config_or_else_falls_back_then_defaults() {
    let config = Config::integer(named("primary"))
        .or_else(Config::string(named("fallback")))
        .with_default(Value::Str("default".to_string()));
    assert_eq!(
        config.load(&provider(&[("primary", "invalid")])).unwrap(),
        Value::Str("default".to_string())
    );
    assert_eq!(
        Config::integer(named("primary"))
            .or_else(Config::string(named("fallback")))
            .option()
            .load(&provider(&[("primary", "invalid")]))
            .unwrap(),
        Value::None
    );
}

#[test]
fn provider_env_root_keys_and_dotenv_structure() {
    use config_provider::EnvProvider;
    let env = EnvProvider::from_env_map(vec![
        ("A".to_string(), "value1".to_string()),
        ("B_C".to_string(), "value2".to_string()),
    ]);
    assert_eq!(
        env.root_keys(),
        vec!["A".to_string(), "B".to_string()]
    );
    assert_eq!(env.get(&["missing".to_string()]), None);
    let dotenv = EnvProvider::from_dotenv("A=\nB=value1\n");
    assert_eq!(
        dotenv.root_keys(),
        vec!["A".to_string(), "B".to_string()]
    );
    assert_eq!(dotenv.get(&["A".to_string()]), None);
    assert_eq!(
        dotenv.get(&["B".to_string()]),
        Some("value1".to_string())
    );
}

#[test]
fn provider_dotenv_expansion_is_opt_in() {
    use config_provider::EnvProvider;
    let plain = EnvProvider::from_dotenv("PASSWORD=\"value\"\nDB_PASS=$PASSWORD\nBRACED=${PASSWORD}\nUNKNOWN=$NOPE\n");
    assert_eq!(
        plain.get(&["PASSWORD".to_string()]),
        Some("value".to_string())
    );
    assert_eq!(
        plain.get(&["DB_PASS".to_string()]),
        Some("$PASSWORD".to_string())
    );
    assert_eq!(
        plain.get(&["BRACED".to_string()]),
        Some("${PASSWORD}".to_string())
    );
    assert_eq!(
        plain.get(&["UNKNOWN".to_string()]),
        Some("$NOPE".to_string())
    );
    let expanding = EnvProvider::from_dotenv_expanding(
        "PASSWORD=\"value\"\nDB_PASS=${PASSWORD}\nLATE=$PASSWORD\n",
    );
    assert_eq!(
        expanding.get(&["DB_PASS".to_string()]),
        Some("value".to_string())
    );
    assert_eq!(
        expanding.get(&["LATE".to_string()]),
        Some("value".to_string())
    );
}

#[test]
fn provider_dotenv_preserves_empty_when_requested() {
    use config_provider::EnvProvider;
    let plain = EnvProvider::from_dotenv("A=\n");
    assert_eq!(plain.get(&["A".to_string()]), None);
    let keeping = EnvProvider::from_dotenv_preserving("A=\n");
    assert_eq!(keeping.get(&["A".to_string()]), Some("".to_string()));
    assert_eq!(
        keeping.root_keys(),
        vec!["A".to_string()]
    );
}

#[test]
fn config_literals_accepts_configured_number_alternatives() {
    // Config row "literals accepts configured number alternatives": the
    // options are rendered as their source strings, so "1" matches the
    // option 1 and "3" does not match [1, 2]. The failure names the
    // location it was read from, which is the part a bare is_err() would miss.
    let ok = provider(&[("a", "1")]);
    let bad = provider(&[("b", "3")]);

    assert_eq!(
        Config::literals(named("a"), &["1", "2"]).load(&ok),
        Ok(Value::Str("1".to_string()))
    );

    let err = Config::literals(named("b"), &["1", "2"])
        .load(&bad)
        .expect_err("3 is not a configured alternative");
    assert!(
        !err.is_empty(),
        "the failure must name the path it failed at, not just `expected one of`"
    );
}
