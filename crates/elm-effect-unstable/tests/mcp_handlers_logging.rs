//! LoggingTest carry rows: setLevel routing (in-process; HTTP/streaming
//! isolation rows excused — need the full server harness).
use elm_effect_unstable::mcp_handlers::{LogLevel, Logger};

#[test]
fn accepts_every_level_and_rejects_unknown() {
    let mut logger = Logger::new();
    for level in ["debug", "info", "notice", "warning", "error", "critical", "alert", "emergency"] {
        logger.set_level_name(level).unwrap();
        assert_eq!(logger.level(), level.parse::<LogLevel>().unwrap());
    }
    assert!(logger.set_level_name("verbose").is_err());
}

#[test]
fn sends_at_or_above_threshold_and_suppresses_below() {
    let mut logger = Logger::new();
    logger.set_level_name("warning").unwrap();
    assert!(logger.log("info", Some("test"), serde_json::json!({})).is_none());
    let note = logger.log("error", Some("test"), serde_json::json!({"n": 1})).unwrap();
    assert_eq!(note.method, "notifications/message");
    assert!(note.id.is_none());
    assert_eq!(note.params["level"], serde_json::json!("error"));
    assert_eq!(note.params["logger"], serde_json::json!("test"));
    assert_eq!(note.params["data"], serde_json::json!({"n": 1}));
}

#[test]
fn allows_arbitrary_json_data_and_rejects_bad_levels() {
    let mut logger = Logger::new();
    logger.set_level_name("debug").unwrap();
    let data = serde_json::json!({"list": [1, "two", null], "nested": {"k": true}});
    let note = logger.log("debug", None, data.clone()).unwrap();
    assert_eq!(note.params["data"], data);
    assert!(logger.log("nope", None, serde_json::json!(null)).is_none());
}

#[test]
fn level_ordering_matches_severity() {
    assert!("debug".parse::<LogLevel>().unwrap() < "emergency".parse::<LogLevel>().unwrap());
    assert!("info".parse::<LogLevel>().unwrap() < "warning".parse::<LogLevel>().unwrap());
}

#[test]
fn updates_minimum_for_subsequent_operations() {
    let mut logger = Logger::new();
    logger.set_level_name("error").unwrap();
    assert!(logger.log("warning", None, serde_json::json!(1)).is_none());
    logger.set_level_name("warning").unwrap();
    assert!(logger.log("warning", None, serde_json::json!(1)).is_some());
}
