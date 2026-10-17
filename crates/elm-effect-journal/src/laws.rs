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

/// Every law, by name, with its outcome.
pub fn all(log: &dyn DurableLog) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("same_triple_appends_once", same_triple_appends_once(log)),
        ("seq_gap_is_rejected", seq_gap_is_rejected(log)),
        ("stale_epoch_is_fenced", stale_epoch_is_fenced(log)),
        ("new_epoch_starts_at_zero", new_epoch_starts_at_zero(log)),
    ]
}
