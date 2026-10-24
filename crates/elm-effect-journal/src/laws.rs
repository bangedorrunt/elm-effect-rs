//! Trait-level laws for [`DurableLog`] (ADR-0002 L16).
//!
//! Each law drives a log only through `&dyn DurableLog`, so a sibling's own
//! implementation (toron's store-backed log, for one) runs exactly the
//! checks [`MemLog`](crate::durable_log::MemLog) runs. A law returns
//! `Err(reason)` instead of panicking, so a negative control can assert that
//! a broken log FAILS a law rather than merely that a good one passes.
//!
//! Every law uses its own producer id, so the laws can share one log. Pass a
//! log that has never seen the `laws/` producer namespace.

use crate::durable_log::{AppendError, DurableLog, Offset, SCHEMA_VERSION};
use crate::stream::Status;

/// The JSON envelope every `DurableLog` accepts: schema version, the
/// all-or-none producer triple, and an opaque payload.
pub fn envelope(producer: &str, epoch: u64, seq: u64, payload: &str) -> String {
    format!(
        "{{\"v\":{SCHEMA_VERSION},\"producer\":{{\"producer_id\":\"{producer}\",\"epoch\":{epoch},\"seq\":{seq}}},\"payload\":{payload}}}"
    )
}

fn count(log: &dyn DurableLog) -> Result<usize, String> {
    log.read_from(&Offset::Start)
        .map(|v| v.len())
        .map_err(|e| format!("read_from(Start) failed: {e}"))
}

fn ok(log: &dyn DurableLog, p: &str, epoch: u64, seq: u64) -> Result<Offset, String> {
    log.append_json(&envelope(p, epoch, seq, "null"))
        .map_err(|e| format!("append ({p},{epoch},{seq}) should succeed, got {e:?}"))
}

/// Same `(producer, epoch, seq)` appends once and replies the same offset.
pub fn same_triple_appends_once(log: &dyn DurableLog) -> Result<(), String> {
    let p = "laws/once";
    let before = count(log)?;
    let first = ok(log, p, 0, 0)?;
    let again = ok(log, p, 0, 0)?;
    if first != again {
        return Err(format!(
            "replay returned {again:?}, first append minted {first:?}"
        ));
    }
    let after = count(log)?;
    if after != before + 1 {
        return Err(format!(
            "expected exactly one new row, log grew by {}",
            after - before
        ));
    }
    Ok(())
}

/// A sequence that is neither `last + 1` nor already accepted is a gap.
pub fn seq_gap_is_rejected(log: &dyn DurableLog) -> Result<(), String> {
    let p = "laws/gap";
    ok(log, p, 0, 0)?;
    match log.append_json(&envelope(p, 0, 2, "null")) {
        Err(AppendError::SeqGap {
            got: 2,
            last: Some(0),
        }) => Ok(()),
        other => Err(format!(
            "seq 2 after 0 must be SeqGap{{got:2,last:Some(0)}}, got {other:?}"
        )),
    }
}

/// A lower epoch than the producer's current one is fenced with
/// [`AppendError::StaleEpoch`] and writes nothing, including a replay of a
/// triple the log accepted before the epoch moved.
pub fn stale_epoch_is_fenced(log: &dyn DurableLog) -> Result<(), String> {
    let p = "laws/fence";
    ok(log, p, 0, 0)?;
    ok(log, p, 1, 0)?;
    let before = count(log)?;
    let fresh = log.append_json(&envelope(p, 0, 1, "null"));
    match &fresh {
        Err(AppendError::StaleEpoch {
            producer,
            sent: 0,
            current: 1,
        }) if producer == p => {}
        other => {
            return Err(format!(
                "epoch 0 after epoch 1 must be StaleEpoch{{sent:0,current:1}}, got {other:?}"
            ));
        }
    }
    let replay = log.append_json(&envelope(p, 0, 0, "null"));
    if !matches!(replay, Err(AppendError::StaleEpoch { .. })) {
        return Err(format!(
            "replay of an old-epoch triple must be fenced, got {replay:?}"
        ));
    }
    let after = count(log)?;
    if after != before {
        return Err(format!("a fenced append wrote {} row(s)", after - before));
    }
    Ok(())
}

/// A new, higher epoch starts its sequence at 0; the old epoch's sequence
/// does not carry over.
pub fn new_epoch_starts_at_zero(log: &dyn DurableLog) -> Result<(), String> {
    let p = "laws/zero";
    ok(log, p, 0, 0)?;
    ok(log, p, 0, 1)?;
    ok(log, p, 1, 0)?;
    match log.append_json(&envelope(p, 2, 5, "null")) {
        Err(AppendError::SeqGap { got: 5, last: None }) => Ok(()),
        other => Err(format!(
            "epoch 2 must start at seq 0 (seq 5 is a gap), got {other:?}"
        )),
    }
}

/// A bare close is monotonic and idempotent: the status turns `Closed` and
/// stays there, a second close succeeds, a later append is refused, and the
/// bytes written before the close stay readable. Closes the log.
pub fn close_contract(log: &dyn DurableLog) -> Result<(), String> {
    let p = "laws/close";
    let first = ok(log, p, 0, 0)?;
    log.close()
        .map_err(|e| format!("close should succeed, got {e:?}"))?;
    log.close()
        .map_err(|e| format!("a second close should be idempotent, got {e:?}"))?;
    if log.status() != Status::Closed || !log.is_closed() {
        return Err(format!(
            "after close status must be Closed, got {:?}",
            log.status()
        ));
    }
    match log.append_json(&envelope(p, 0, 1, "null")) {
        Err(AppendError::Closed) => {}
        other => return Err(format!("append after close must be Closed, got {other:?}")),
    }
    let readings = log
        .read_from(&Offset::Start)
        .map_err(|e| format!("closed log must stay readable, got {e:?}"))?;
    if readings.len() != 1 || readings[0].0 != first {
        return Err(format!(
            "closed log read lost its record: expected one at {first:?}, got {readings:?}"
        ));
    }
    Ok(())
}

/// Append-and-close commits both halves once: the record is readable, the
/// status is `Closed`, a later append is refused, and a retry of the closing
/// triple returns the same offset without a duplicate. Closes the log.
pub fn append_and_close_contract(log: &dyn DurableLog) -> Result<(), String> {
    let p = "laws/seal";
    let closing = envelope(p, 0, 0, "null");
    let offset = log
        .append_json_and_close(&closing)
        .map_err(|e| format!("append-and-close should succeed, got {e:?}"))?;
    if log.status() != Status::Closed {
        return Err(format!(
            "append-and-close must close the stream, status {:?}",
            log.status()
        ));
    }
    let readings = log
        .read_from(&Offset::Start)
        .map_err(|e| format!("closed log must stay readable, got {e:?}"))?;
    if readings.len() != 1 || readings[0].0 != offset {
        return Err(format!(
            "the closing record must be readable at {offset:?}, got {readings:?}"
        ));
    }
    match log.append_json(&envelope(p, 0, 1, "null")) {
        Err(AppendError::Closed) => {}
        other => return Err(format!("append after close must be Closed, got {other:?}")),
    }
    // The exact closing triple is the one dedupe exception on a closed log.
    let retried = log
        .append_json_and_close(&closing)
        .map_err(|e| format!("retrying the closing append must dedupe, got {e:?}"))?;
    if retried != offset {
        return Err(format!(
            "retry returned {retried:?}, first close minted {offset:?}"
        ));
    }
    if log.read_from(&Offset::Start).map(|r| r.len()) != Ok(1) {
        return Err("the closing append was duplicated by its retry".to_string());
    }
    Ok(())
}

/// Pages walk every record once, in order, within the requested bound, and
/// the cursor left at the tail sees the next append. A live reader depends on
/// that last part: a tail cursor that repeats or jumps would either spin or
/// skip the record it is waiting for.
pub fn paging_reaches_every_record(log: &dyn DurableLog) -> Result<(), String> {
    let p = "laws/page";
    let before = count(log)?;
    for seq in 0..5 {
        ok(log, p, 0, seq)?;
    }
    let mut cursor = Offset::Start;
    let mut seen = 0;
    loop {
        let page = log
            .read_page(&cursor, 2)
            .map_err(|e| format!("read_page failed: {e}"))?;
        if page.records.len() > 2 {
            return Err(format!(
                "page returned {} records for limit 2",
                page.records.len()
            ));
        }
        seen += page.records.len();
        cursor = page.next;
        if page.up_to_date {
            break;
        }
        if page.records.is_empty() {
            return Err("a partial page returned no records".to_string());
        }
    }
    if seen != before + 5 {
        return Err(format!("paged {seen} records, expected {}", before + 5));
    }
    ok(log, p, 0, 5)?;
    let page = log
        .read_page(&cursor, 2)
        .map_err(|e| format!("read_page at the tail failed: {e}"))?;
    if page.records.len() != 1 {
        return Err(format!(
            "the tail cursor missed the next append: got {} record(s)",
            page.records.len()
        ));
    }
    Ok(())
}

/// Every non-closing law, by name, with its outcome.
pub fn all(log: &dyn DurableLog) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("same_triple_appends_once", same_triple_appends_once(log)),
        ("seq_gap_is_rejected", seq_gap_is_rejected(log)),
        ("stale_epoch_is_fenced", stale_epoch_is_fenced(log)),
        ("new_epoch_starts_at_zero", new_epoch_starts_at_zero(log)),
        (
            "paging_reaches_every_record",
            paging_reaches_every_record(log),
        ),
    ]
}

/// One closure law as a function, so a caller can hand it a fresh log.
pub type ClosingLaw = fn(&dyn DurableLog) -> Result<(), String>;

/// The two closure laws, by name, as functions. Each one closes its log, so
/// they do not belong in [`all`]: a caller runs every law in [`all`] on one
/// log, then runs each of these on its own fresh log.
pub fn closing() -> Vec<(&'static str, ClosingLaw)> {
    vec![
        ("close_contract", close_contract),
        ("append_and_close_contract", append_and_close_contract),
    ]
}
