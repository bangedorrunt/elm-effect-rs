//! `durable_log_append` fence: minted offsets, seq rules, atomic append.
//! The log module loads via `#[path]` (the .15 precedent): no facade touch.

#[path = "../src/durable_log.rs"]
mod durable_log;

use durable_log::{
    DurableLog, InProcessLog, JsonMessage, LogError, OFFSET_START, OFFSET_TAIL, ProducerId,
    ReadBound, SCHEMA_VERSION,
};

fn producer(id: &str, epoch: u64, seq: u64) -> ProducerId {
    ProducerId {
        producer_id: id.to_owned(),
        epoch,
        seq,
    }
}

fn msg(body: &str) -> JsonMessage {
    JsonMessage {
        schema_version: SCHEMA_VERSION,
        body: body.to_owned(),
    }
}

#[test]
fn hand_built_offset_is_rejected() {
    let mut log = InProcessLog::new();
    let first = log.append(producer("p", 0, 1), msg("{}")).expect("append");
    assert!(first.as_str() != OFFSET_START && first.as_str() != OFFSET_TAIL);
    // A hand-built token is rejected anywhere a real offset is required.
    let fake = durable_log::Offset::from_unminted_for_test("00000000000000009999");
    assert_eq!(
        log.read(&ReadBound::After(fake.clone()), 10),
        Err(LogError::HandBuiltOffset(fake.as_str().to_owned()))
    );
    // The sentinels are never real offsets either.
    for sentinel in [OFFSET_START, OFFSET_TAIL] {
        let fake = durable_log::Offset::from_unminted_for_test(sentinel);
        assert!(matches!(
            log.read(&ReadBound::After(fake), 10),
            Err(LogError::HandBuiltOffset(_))
        ));
    }
    // Minted offsets order lexicographically in append order.
    let second = log.append(producer("p", 0, 2), msg("{}")).expect("append");
    assert!(first < second);
}

#[test]
fn seq_gap_is_rejected() {
    let mut log = InProcessLog::new();
    assert!(matches!(
        log.append(producer("p", 0, 2), msg("{}")),
        Err(LogError::SeqGap {
            expected: 1,
            offered: 2,
            ..
        })
    ));
    // Same triple appends once: idempotent replay returns the first offset.
    let first = log
        .append(producer("p", 0, 1), msg("{\"a\":1}"))
        .expect("append");
    let replay = log
        .append(producer("p", 0, 1), msg("{\"a\":1}"))
        .expect("replay");
    assert_eq!(first, replay);
    assert_eq!(log.latest(), Some(first));
    // Independent epochs track their own sequences.
    log.append(producer("p", 1, 1), msg("{}"))
        .expect("new epoch starts at 1");
}

#[test]
fn partial_identity_is_rejected() {
    let mut log = InProcessLog::new();
    assert_eq!(
        log.append(producer("", 0, 1), msg("{}")),
        Err(LogError::PartialIdentity)
    );
    assert!(log.latest().is_none());
}

#[test]
fn append_is_atomic() {
    let mut log = InProcessLog::new();
    for i in 1..=5u64 {
        log.append(producer("p", 0, i), msg(&format!("{{\"i\":{i}}}")))
            .expect("append");
    }
    // A read never returns a partial message: every body parses whole.
    let records = log.read(&ReadBound::Start, 100).expect("read");
    assert_eq!(records.len(), 5);
    for (n, record) in records.iter().enumerate() {
        assert_eq!(record.message.body, format!("{{\"i\":{}}}", n + 1));
    }
    // Bounded reads page in order without overlap.
    let page1 = log.read(&ReadBound::Start, 2).expect("page");
    let after = ReadBound::After(page1[1].offset.clone());
    let page2 = log.read(&after, 10).expect("page");
    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 3);
    assert_eq!(page2[0].message.body, "{\"i\":3}");
    // Tail reads nothing.
    assert_eq!(log.read(&ReadBound::Tail, 10).expect("tail"), vec![]);
}

#[test]
fn unknown_schema_version_is_typed_error() {
    let mut log = InProcessLog::new();
    assert_eq!(
        log.append(
            producer("p", 0, 1),
            JsonMessage {
                schema_version: 999,
                body: "{}".to_owned()
            }
        ),
        Err(LogError::UnknownSchemaVersion(999))
    );
    assert!(log.latest().is_none());
}
