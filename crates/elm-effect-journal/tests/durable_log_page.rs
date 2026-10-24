//! Catch-up pages: bounded slices, a resume cursor that works on an empty
//! tail, and the two end signals (`up_to_date` vs `closed`).

use std::fs;
use std::path::PathBuf;

use elm_effect_journal::durable_log::{
    AppendError, DurableLog, FileLog, MemLog, Offset, Page, SCHEMA_VERSION,
};

fn envelope(producer: &str, epoch: u64, seq: u64, payload: &str) -> String {
    format!(
        "{{\"v\":{SCHEMA_VERSION},\"producer\":{{\"producer_id\":\"{producer}\",\"epoch\":{epoch},\"seq\":{seq}}},\"payload\":{payload}}}"
    )
}

fn at(n: u64) -> Offset {
    Offset::At(format!("{n:020}"))
}

fn append(log: &dyn DurableLog, from: u64, to: u64) {
    for seq in from..to {
        log.append_json(&envelope("p", 0, seq, &seq.to_string()))
            .unwrap();
    }
}

fn payloads(page: &Page) -> Vec<String> {
    page.records.iter().map(|(_, body)| body.clone()).collect()
}

#[test]
fn page_walks_the_log_in_bounded_slices() {
    let log = MemLog::new();
    append(&log, 0, 5);
    let first = log.read_page(&Offset::Start, 2).unwrap();
    assert_eq!(payloads(&first), vec!["0", "1"]);
    assert_eq!(first.next, at(1), "resume after the last returned record");
    assert!(!first.up_to_date);
    assert!(!first.closed);

    let second = log.read_page(&first.next, 2).unwrap();
    assert_eq!(payloads(&second), vec!["2", "3"]);
    assert_eq!(second.next, at(3));
    assert!(!second.up_to_date);

    let third = log.read_page(&second.next, 2).unwrap();
    assert_eq!(payloads(&third), vec!["4"]);
    assert_eq!(third.next, at(4));
    assert!(third.up_to_date);
    assert!(!third.closed);

    // The tail cursor is readable and yields an empty, caught-up page.
    let tail = log.read_page(&third.next, 2).unwrap();
    assert!(tail.records.is_empty());
    assert_eq!(tail.next, at(4));
    assert!(tail.up_to_date);
}

#[test]
fn an_empty_page_returns_a_tail_cursor() {
    let log = MemLog::new();
    let empty = log.read_page(&Offset::Start, 10).unwrap();
    assert!(empty.records.is_empty());
    assert_eq!(empty.next, Offset::Start);
    assert!(empty.up_to_date);
    assert!(!empty.closed);

    append(&log, 0, 1);
    let page = log.read_page(&empty.next, 10).unwrap();
    assert_eq!(
        payloads(&page),
        vec!["0"],
        "the cursor from an empty page sees the append"
    );
    assert_eq!(page.next, at(0));
    assert!(page.up_to_date);
}

#[test]
fn now_resolves_to_a_concrete_tail() {
    let log = MemLog::new();
    append(&log, 0, 2);
    let page = log.read_page(&Offset::Now, 10).unwrap();
    assert!(page.records.is_empty());
    assert!(page.up_to_date);
    assert_eq!(
        page.next,
        at(1),
        "Now resolves to the last existing offset, not the skipped tail"
    );

    append(&log, 2, 3);
    let later = log.read_page(&page.next, 10).unwrap();
    assert_eq!(payloads(&later), vec!["2"]);
    assert_eq!(later.next, at(2));
}

#[test]
fn a_closed_page_reports_eof_only_at_the_tail() {
    let log = MemLog::new();
    append(&log, 0, 3);
    log.close().unwrap();

    let first = log.read_page(&Offset::Start, 2).unwrap();
    assert_eq!(payloads(&first), vec!["0", "1"]);
    assert!(!first.up_to_date, "a partial page is not caught up");
    assert!(
        !first.closed,
        "EOF belongs on the page that reaches the tail"
    );

    let last = log.read_page(&first.next, 2).unwrap();
    assert_eq!(payloads(&last), vec!["2"]);
    assert!(last.up_to_date);
    assert!(last.closed);
    assert_eq!(last.next, at(2));

    let eof = log.read_page(&last.next, 2).unwrap();
    assert!(eof.records.is_empty());
    assert!(eof.up_to_date);
    assert!(
        eof.closed,
        "an empty page at a closed tail is the EOF signal"
    );
}

#[test]
fn limit_zero_still_returns_one_record() {
    let log = MemLog::new();
    append(&log, 0, 2);
    let page = log.read_page(&Offset::Start, 0).unwrap();
    assert_eq!(
        payloads(&page),
        vec!["0"],
        "a page must always make progress"
    );
    assert_eq!(page.next, at(0));
    assert!(!page.up_to_date);
}

#[test]
fn compaction_does_not_strand_a_cursor() {
    let log = MemLog::new();
    append(&log, 0, 5);
    assert_eq!(log.compact_before(&at(3)).unwrap(), 3);

    let page = log.read_page(&at(0), 10).unwrap();
    assert_eq!(payloads(&page), vec!["3", "4"]);
    assert_eq!(page.next, at(4));
    assert!(page.up_to_date);
}

#[test]
fn a_cursor_beyond_the_tail_is_rejected() {
    let log = MemLog::new();
    append(&log, 0, 1);
    assert_eq!(
        log.read_page(&at(1), 1).unwrap_err(),
        AppendError::UnknownOffset,
        "the tail is not a record cursor"
    );
    assert_eq!(
        log.read_page(&at(9), 1).unwrap_err(),
        AppendError::UnknownOffset
    );
    assert_eq!(
        log.read_page(&Offset::At("forged".into()), 1).unwrap_err(),
        AppendError::UnknownOffset
    );
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "elm-file-log-page-{name}-{}-{nanos}.jsonl",
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
fn a_file_log_pages_the_same_way_after_reopen() {
    let scratch = Scratch::new("page");
    {
        let log = FileLog::open(&scratch.0).unwrap();
        append(&log, 0, 4);
    }
    let log = FileLog::open(&scratch.0).unwrap();

    let first = log.read_page(&Offset::Start, 3).unwrap();
    assert_eq!(payloads(&first), vec!["0", "1", "2"]);
    assert_eq!(first.next, at(2));
    assert!(!first.up_to_date);

    let second = log.read_page(&first.next, 3).unwrap();
    assert_eq!(payloads(&second), vec!["3"]);
    assert_eq!(second.next, at(3));
    assert!(second.up_to_date);

    let tail = log.read_page(&second.next, 3).unwrap();
    assert!(tail.records.is_empty());
    assert_eq!(tail.next, at(3));
    assert!(tail.up_to_date);
}
