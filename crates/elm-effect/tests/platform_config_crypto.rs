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
