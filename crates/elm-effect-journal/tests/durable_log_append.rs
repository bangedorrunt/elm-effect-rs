//! Durable append rules: minted offsets, atomic batches, seq discipline.

use elm_effect_journal::durable_log::{AppendError, DurableLog, MemLog, Offset, SCHEMA_VERSION};

fn envelope(producer: &str, epoch: u64, seq: u64, payload: &str) -> String {
    format!(
        "{{\"v\":{SCHEMA_VERSION},\"producer\":{{\"producer_id\":\"{producer}\",\"epoch\":{epoch},\"seq\":{seq}}},\"payload\":{payload}}}"
    )
}

#[test]
fn hand_built_offset_is_rejected() {
    let log = MemLog::new();
    assert!(log.read_from(&Offset::At("forged".into())).is_err());
    assert!(
        log.read_from(&Offset::At("99999999999999999999".into()))
            .is_err()
    );
    assert_eq!(log.read_from(&Offset::Start).unwrap(), vec![]);
    assert_eq!(log.read_from(&Offset::Now).unwrap(), vec![]);
}

#[test]
fn seq_gap_is_rejected() {
    let log = MemLog::new();
    assert_eq!(
        log.append_json(&envelope("p", 0, 5, "1")),
        Err(AppendError::SeqGap { got: 5, last: None })
    );
    assert!(log.append_json(&envelope("p", 0, 0, "1")).is_ok());
    // Idempotent replay of an accepted seq returns its offset, no duplicate.
    let first = log.append_json(&envelope("p", 0, 0, "1")).unwrap();
    let readings = log.read_from(&Offset::Start).unwrap();
    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].0, first);
    assert_eq!(
        log.append_json(&envelope("p", 0, 2, "1")),
        Err(AppendError::SeqGap {
            got: 2,
            last: Some(0)
        })
    );
    assert!(log.append_json(&envelope("p", 0, 1, "2")).is_ok());
}

#[test]
fn partial_identity_is_rejected() {
    let log = MemLog::new();
    assert_eq!(
        log.append_json(r#"{"v":1,"payload":1}"#),
        Err(AppendError::PartialIdentity)
    );
    assert_eq!(
        log.append_json(r#"{"v":1,"producer":{"producer_id":"p"},"payload":1}"#),
        Err(AppendError::PartialIdentity)
    );
    assert_eq!(
        log.append_json(
            r#"{"v":1,"producer":{"producer_id":"p","epoch":0,"seq":null},"payload":1}"#
        ),
        Err(AppendError::PartialIdentity)
    );
}

#[test]
fn append_is_atomic() {
    let log = MemLog::new();
    let first = log.append_json(&envelope("p", 0, 0, r#"{"a":1}"#)).unwrap();
    let second = log.append_json(&envelope("p", 0, 1, r#"{"b":2}"#)).unwrap();
    assert!(matches!(first, Offset::At(_)));
    assert!(matches!(second, Offset::At(_)));
    assert_ne!(first, second);
    // Offsets order lexicographically in append order.
    let (Offset::At(a), Offset::At(b)) = (first.clone(), second.clone()) else {
        panic!("minted")
    };
    assert!(a < b);
    // Reads return whole messages with JSON boundaries intact.
    let readings = log.read_from(&Offset::Start).unwrap();
    assert_eq!(readings.len(), 2);
    assert_eq!(readings[0].1, r#"{"a":1}"#);
    assert_eq!(readings[1].1, r#"{"b":2}"#);
    // Reading past the first returns only the second.
    assert_eq!(log.read_from(&first).unwrap().len(), 1);
    assert_eq!(log.read_from(&Offset::Now).unwrap(), vec![]);
}

#[test]
fn unknown_schema_version_is_typed_error() {
    let log = MemLog::new();
    assert_eq!(
        log.append_json(
            r#"{"v":999,"producer":{"producer_id":"p","epoch":0,"seq":0},"payload":1}"#
        ),
        Err(AppendError::UnknownVersion(999))
    );
    assert!(matches!(
        log.append_json("not json"),
        Err(AppendError::NotAnEnvelope(_))
    ));
}
