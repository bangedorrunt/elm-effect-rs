//! File-backed log across real restarts. Each test uses its own file under the
//! system temp dir, so tests run in parallel without sharing state.

use std::fs;
use std::path::PathBuf;

use elm_effect_journal::durable_log::{AppendError, DurableLog, FileLog, Offset, SCHEMA_VERSION};

fn envelope(producer: &str, epoch: u64, seq: u64, payload: &str) -> String {
    format!(
        "{{\"v\":{SCHEMA_VERSION},\"producer\":{{\"producer_id\":\"{producer}\",\"epoch\":{epoch},\"seq\":{seq}}},\"payload\":{payload}}}"
    )
}

/// A unique path per test run, removed at the end of each test.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("elm-file-log-{name}-{}-{nanos}.jsonl", std::process::id()));
        Scratch(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_file(self.0.with_extension("compacting"));
    }
}

#[test]
fn records_and_offsets_survive_reopen() {
    let scratch = Scratch::new("reopen");
    let first;
    {
        let log = FileLog::open(&scratch.0).unwrap();
        first = log.append_json(&envelope("p", 0, 0, r#"{"a":1}"#)).unwrap();
        log.append_json(&envelope("p", 0, 1, r#"{"b":2}"#)).unwrap();
    }
    let log = FileLog::open(&scratch.0).unwrap();
    let readings = log.read_from(&Offset::Start).unwrap();
    assert_eq!(readings.len(), 2, "both records come back after a restart");
    assert_eq!(readings[0].0, first, "offsets are the same after a restart");
    assert_eq!(readings[0].1, r#"{"a":1}"#);
    assert_eq!(readings[1].1, r#"{"b":2}"#);
    // The next append continues the sequence and mints a later offset.
    let third = log.append_json(&envelope("p", 0, 2, r#"{"c":3}"#)).unwrap();
    assert!(readings[1].0 != third);
}

#[test]
fn fencing_state_survives_reopen() {
    let scratch = Scratch::new("fence");
    {
        let log = FileLog::open(&scratch.0).unwrap();
        log.append_json(&envelope("p", 1, 0, "1")).unwrap();
    }
    let log = FileLog::open(&scratch.0).unwrap();
    assert_eq!(
        log.append_json(&envelope("p", 0, 1, "2")),
        Err(AppendError::StaleEpoch { producer_id: "p".into(), sent: 0, current: 1 }),
        "a restart does not reopen an old epoch"
    );
    assert_eq!(
        log.append_json(&envelope("p", 1, 5, "2")),
        Err(AppendError::SeqGap { got: 5, last: Some(0) }),
        "the sequence resumes where it stopped"
    );
    // The same triple with the same bytes still dedupes across the restart.
    assert!(log.append_json(&envelope("p", 1, 0, "1")).is_ok());
    assert_eq!(log.read_from(&Offset::Start).unwrap().len(), 1);
}

#[test]
fn torn_tail_is_discarded_and_the_retry_is_accepted() {
    let scratch = Scratch::new("torn");
    {
        let log = FileLog::open(&scratch.0).unwrap();
        log.append_json(&envelope("p", 0, 0, "1")).unwrap();
    }
    // Simulate a crash mid-write: a partial line with no newline.
    let mut text = fs::read_to_string(&scratch.0).unwrap();
    text.push_str("{\"offset\":\"000000000000000000");
    fs::write(&scratch.0, text).unwrap();

    let log = FileLog::open(&scratch.0).unwrap();
    assert_eq!(log.read_from(&Offset::Start).unwrap().len(), 1, "the torn line is gone");
    // That append never returned Ok, so its seq is free for the retry.
    assert!(log.append_json(&envelope("p", 0, 1, "2")).is_ok());
}

#[test]
fn corrupt_complete_line_fails_the_open() {
    let scratch = Scratch::new("corrupt");
    fs::write(&scratch.0, "not json at all\n").unwrap();
    let err = FileLog::open(&scratch.0).unwrap_err();
    assert!(matches!(err, AppendError::Storage(ref m) if m.contains("line 1")), "{err:?}");
}

#[test]
fn compaction_rewrites_the_file_and_keeps_offsets() {
    let scratch = Scratch::new("compact");
    let keep_from;
    {
        let log = FileLog::open(&scratch.0).unwrap();
        log.append_json(&envelope("p", 0, 0, "1")).unwrap();
        log.append_json(&envelope("p", 0, 1, "2")).unwrap();
        keep_from = log.append_json(&envelope("p", 0, 2, "3")).unwrap();
        assert_eq!(log.compact_before(&keep_from).unwrap(), 2);
    }
    let log = FileLog::open(&scratch.0).unwrap();
    let readings = log.read_from(&Offset::Start).unwrap();
    assert_eq!(readings.len(), 1, "only the kept record is in the file");
    assert_eq!(readings[0].0, keep_from, "its offset is unchanged by compaction");
    assert_eq!(fs::read_to_string(&scratch.0).unwrap().lines().count(), 2, "checkpoint line plus the kept record");
    // The checkpoint keeps the sequence history, so the next seq is accepted
    // and an old seq is still refused after the records behind it are gone.
    assert_eq!(
        log.append_json(&envelope("p", 0, 1, "5")),
        Err(AppendError::SeqGap { got: 1, last: Some(2) })
    );
    assert!(log.append_json(&envelope("p", 0, 3, "4")).is_ok());
}
