//! Durable stream closure: monotonic, idempotent, atomic with the final
//! append, durable across a FileLog restart, and readable at EOF.
//!
//! These are the `DurableLog` half of L16's close/gone contract; `stream.rs`
//! owns the lifecycle model itself.

use std::fs;
use std::path::PathBuf;

use elm_effect_journal::durable_log::{
    AppendError, DurableLog, FileLog, MemLog, Offset, SCHEMA_VERSION,
};
use elm_effect_journal::stream::Status;

fn envelope(producer: &str, epoch: u64, seq: u64, payload: &str) -> String {
    format!(
        "{{\"v\":{SCHEMA_VERSION},\"producer\":{{\"producer_id\":\"{producer}\",\"epoch\":{epoch},\"seq\":{seq}}},\"payload\":{payload}}}"
    )
}

/// A unique path per test run, removed at the end.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "elm-log-close-{name}-{}-{nanos}.jsonl",
            std::process::id()
        ));
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
fn close_is_monotonic_and_idempotent() {
    let log = MemLog::new();
    assert_eq!(log.status(), Status::UpToDate);
    log.append_json(&envelope("p", 0, 0, "1")).unwrap();
    log.close().unwrap();
    assert_eq!(log.status(), Status::Closed);
    assert!(log.is_closed());
    // A second close succeeds and changes nothing.
    log.close().unwrap();
    assert_eq!(log.status(), Status::Closed);
    assert_eq!(
        log.append_json(&envelope("p", 0, 1, "2")),
        Err(AppendError::Closed),
        "a closed log accepts no further appends"
    );
}

#[test]
fn bytes_before_close_stay_readable() {
    let log = MemLog::new();
    let first = log.append_json(&envelope("p", 0, 0, "1")).unwrap();
    let second = log.append_json(&envelope("p", 0, 1, "2")).unwrap();
    log.close().unwrap();
    let readings = log.read_from(&Offset::Start).unwrap();
    assert_eq!(readings.len(), 2);
    assert_eq!(readings[0].0, first);
    assert_eq!(readings[1].0, second);
    assert_eq!(log.read_from(&second).unwrap(), vec![]);
}

#[test]
fn append_and_close_commits_both_halves_at_once() {
    let log = MemLog::new();
    let offset = log
        .append_json_and_close(&envelope("p", 0, 0, r#"{"final":true}"#))
        .unwrap();
    assert_eq!(log.status(), Status::Closed);
    let readings = log.read_from(&Offset::Start).unwrap();
    assert_eq!(readings.len(), 1, "the final record is not lost");
    assert_eq!(readings[0].0, offset);
    assert_eq!(readings[0].1, r#"{"final":true}"#);
    assert_eq!(
        log.append_json(&envelope("p", 0, 1, "2")),
        Err(AppendError::Closed)
    );
}

#[test]
fn retrying_the_closing_append_dedupes_on_a_closed_log() {
    let log = MemLog::new();
    let closing = envelope("p", 0, 0, "1");
    let offset = log.append_json_and_close(&closing).unwrap();
    let again = log.append_json_and_close(&closing).unwrap();
    assert_eq!(again, offset, "the closing triple replies its offset");
    assert_eq!(log.read_from(&Offset::Start).unwrap().len(), 1);
}

#[test]
fn an_earlier_triple_is_not_deduped_after_close() {
    let log = MemLog::new();
    let first = envelope("p", 0, 0, "1");
    log.append_json(&first).unwrap();
    log.append_json_and_close(&envelope("p", 0, 1, "2"))
        .unwrap();
    // Replaying a non-closing append on a closed stream is a closed refusal,
    // not a successful dedupe: the stream ended after it.
    assert_eq!(log.append_json(&first), Err(AppendError::Closed));
}

#[test]
fn file_close_marker_survives_reopen() {
    let scratch = Scratch::new("marker");
    let offset;
    {
        let log = FileLog::open(&scratch.0).unwrap();
        offset = log.append_json(&envelope("p", 0, 0, "1")).unwrap();
        log.close().unwrap();
    }
    let log = FileLog::open(&scratch.0).unwrap();
    assert_eq!(log.status(), Status::Closed, "closure is durable");
    assert_eq!(
        log.append_json(&envelope("p", 0, 1, "2")),
        Err(AppendError::Closed)
    );
    let readings = log.read_from(&Offset::Start).unwrap();
    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].0, offset);
}

#[test]
fn file_sealed_line_survives_reopen() {
    let scratch = Scratch::new("sealed");
    let closing = envelope("p", 0, 0, "1");
    let offset;
    {
        let log = FileLog::open(&scratch.0).unwrap();
        offset = log.append_json_and_close(&closing).unwrap();
    }
    let log = FileLog::open(&scratch.0).unwrap();
    assert_eq!(log.status(), Status::Closed);
    assert_eq!(log.read_from(&Offset::Start).unwrap().len(), 1);
    // The reopening log knows which triple closed it, so the retry still
    // dedupes instead of appending or refusing.
    assert_eq!(log.append_json_and_close(&closing).unwrap(), offset);
    assert_eq!(fs::read_to_string(&scratch.0).unwrap().lines().count(), 1);
}

#[test]
fn file_compaction_after_close_keeps_closure_and_reads() {
    let scratch = Scratch::new("compact");
    let keep;
    {
        let log = FileLog::open(&scratch.0).unwrap();
        log.append_json(&envelope("p", 0, 0, "1")).unwrap();
        log.append_json(&envelope("p", 0, 1, "2")).unwrap();
        keep = log.append_json(&envelope("p", 0, 2, "3")).unwrap();
        log.close().unwrap();
        assert_eq!(log.compact_before(&keep).unwrap(), 2);
    }
    let log = FileLog::open(&scratch.0).unwrap();
    assert_eq!(
        log.status(),
        Status::Closed,
        "the checkpoint carries the closed flag"
    );
    let readings = log.read_from(&Offset::Start).unwrap();
    assert_eq!(readings.len(), 1);
    assert_eq!(readings[0].0, keep);
    assert_eq!(
        log.append_json(&envelope("p", 0, 3, "4")),
        Err(AppendError::Closed)
    );
}

#[test]
fn a_record_after_a_close_marker_is_corruption() {
    let scratch = Scratch::new("late");
    {
        let log = FileLog::open(&scratch.0).unwrap();
        log.append_json(&envelope("p", 0, 0, "1")).unwrap();
        log.close().unwrap();
    }
    // Simulate a writer that ignored the close and appended anyway.
    let mut text = fs::read_to_string(&scratch.0).unwrap();
    let late = serde_json::json!({
        "offset": "00000000000000000001",
        "envelope": envelope("p", 0, 1, "2"),
    });
    text.push_str(&format!("{late}\n"));
    fs::write(&scratch.0, text).unwrap();

    let err = FileLog::open(&scratch.0).unwrap_err();
    assert!(
        matches!(err, AppendError::Storage(ref m) if m.contains("record after close")),
        "{err:?}"
    );
}

#[test]
fn close_only_file_is_atomic_on_reopen() {
    // A close marker is one line: once it is there, a reopen sees EOF; if the
    // write never completed, the torn line is cut and the log stays open.
    let scratch = Scratch::new("torn-close");
    fs::write(&scratch.0, "{\"closed\": tru").unwrap();
    let log = FileLog::open(&scratch.0).unwrap();
    assert_eq!(
        log.status(),
        Status::UpToDate,
        "a torn close line never returned Ok"
    );
    assert!(
        log.append_json(&envelope("p", 0, 0, "1")).is_ok(),
        "the stream stays writable after the torn close"
    );
}
