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

use crate::EffectState;
use crate::durable_log::{AppendError, DurableLog, Offset, SCHEMA_VERSION, Status};
use crate::store::{Disposition, JournalStore};

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

// JournalStore laws: one `&dyn JournalStore` drives them all, so a
// sibling's own store (SQLite, for one) runs exactly the checks
// [`InMemoryStore`](crate::store::InMemoryStore) runs. Each law uses its
// own `laws/` key, so the laws can share one store.

/// Recording the same intent key twice returns the same row.
pub fn intent_is_idempotent(store: &dyn JournalStore) -> Result<(), String> {
    let first = store
        .record_intent("laws/intent", 3)
        .map_err(|e| format!("record_intent failed: {e}"))?;
    let second = store
        .record_intent("laws/intent", 3)
        .map_err(|e| format!("second record_intent failed: {e}"))?;
    if first != second {
        return Err(format!(
            "same key recorded twice diverged: {first:?} vs {second:?}"
        ));
    }
    Ok(())
}

/// A live lease under another owner cannot be stolen; reclaim after expiry
/// succeeds and moves the row to the new owner.
pub fn claim_lease_is_fenced(store: &dyn JournalStore) -> Result<(), String> {
    store
        .record_intent("laws/claim", 3)
        .map_err(|e| format!("record_intent failed: {e}"))?;
    let row = store
        .claim("laws/claim", "worker-a", 9_999, 0, 0)
        .map_err(|e| format!("first claim failed: {e}"))?
        .ok_or("first claim should succeed")?;
    if store
        .claim("laws/claim", "worker-b", 10_000, 0, row.generation)
        .map_err(|e| format!("steal attempt failed: {e}"))?
        .is_some()
    {
        return Err("stealing a live lease must return None".to_string());
    }
    let row = store
        .claim("laws/claim", "worker-b", 20_000, 10_000, row.generation)
        .map_err(|e| format!("reclaim after expiry failed: {e}"))?
        .ok_or("reclaim after expiry should succeed")?;
    if row.owner.as_deref() != Some("worker-b") {
        return Err(format!(
            "reclaim must move the row to worker-b, got {:?}",
            row.owner
        ));
    }
    Ok(())
}

/// A stale generation settles nothing; the current generation settles along
/// a legal edge.
pub fn settle_generation_is_fenced(store: &dyn JournalStore) -> Result<(), String> {
    store
        .record_intent("laws/settle", 3)
        .map_err(|e| format!("record_intent failed: {e}"))?;
    let row = store
        .claim("laws/settle", "worker-a", 9_999, 0, 0)
        .map_err(|e| format!("claim failed: {e}"))?
        .ok_or("claim should succeed")?;
    if store
        .settle("laws/settle", 0, EffectState::InFlight)
        .map_err(|e| format!("stale settle failed: {e}"))?
        .is_some()
    {
        return Err("a stale generation must settle nothing".to_string());
    }
    let settled = store
        .settle("laws/settle", row.generation, EffectState::InFlight)
        .map_err(|e| format!("settle failed: {e}"))?
        .ok_or("the current generation should settle")?;
    if settled.state != EffectState::InFlight {
        return Err(format!(
            "settle must move Prepared to InFlight, got {:?}",
            settled.state
        ));
    }
    Ok(())
}

/// Attempts past `max_attempts` dead-letter: `Failed` plus
/// [`TerminalFailed`](Disposition::TerminalFailed).
pub fn attempts_dead_letter(store: &dyn JournalStore) -> Result<(), String> {
    store
        .record_intent("laws/dead-letter", 1)
        .map_err(|e| format!("record_intent failed: {e}"))?;
    let row = store
        .claim("laws/dead-letter", "worker-a", 9_999, 0, 0)
        .map_err(|e| format!("claim failed: {e}"))?
        .ok_or("claim should succeed")?;
    let generation = row.generation;
    let row = store
        .attempt("laws/dead-letter", generation)
        .map_err(|e| format!("first attempt failed: {e}"))?
        .ok_or("the first attempt should note")?;
    if row.attempts != 1 || row.state != EffectState::Prepared {
        return Err(format!(
            "one attempt of one allowed must stay Prepared, got {:?}/{:?}",
            row.attempts, row.state
        ));
    }
    let row = store
        .attempt("laws/dead-letter", generation)
        .map_err(|e| format!("exhausting attempt failed: {e}"))?
        .ok_or("the exhausting attempt should note")?;
    if row.state != EffectState::Failed || row.disposition != Some(Disposition::TerminalFailed) {
        return Err(format!(
            "exhaustion must dead-letter (Failed + TerminalFailed), got {:?}/{:?}",
            row.state, row.disposition
        ));
    }
    Ok(())
}

/// A live lease renews without moving the fence; the deadline moves, the
/// owner and generation stay.
fn heartbeat_renews_the_lease(store: &dyn JournalStore) -> Result<(), String> {
    store
        .record_intent("laws/renew", 3)
        .map_err(|e| format!("record_intent failed: {e}"))?;
    let row = store
        .claim("laws/renew", "worker-a", 9_999, 0, 0)
        .map_err(|e| format!("claim failed: {e}"))?
        .ok_or("claim should succeed")?;
    // A claim leaves the row Prepared; heartbeats need a live lease holder.
    let row = store
        .settle("laws/renew", row.generation, EffectState::InFlight)
        .map_err(|e| format!("settle to InFlight failed: {e}"))?
        .ok_or("settle to InFlight should succeed")?;
    let renewed = store
        .renew_lease("laws/renew", "worker-a", 20_000, 5_000, row.generation)
        .map_err(|e| format!("renew_lease failed: {e}"))?
        .ok_or("a live holder must renew")?;
    if renewed.generation != row.generation {
        return Err(format!(
            "a renew must not move the fence: {} vs {}",
            renewed.generation, row.generation
        ));
    }
    if renewed.lease_deadline != Some(20_000) {
        return Err(format!(
            "a renew must move the deadline, got {:?}",
            renewed.lease_deadline
        ));
    }
    if renewed.owner.as_deref() != Some("worker-a") {
        return Err(format!(
            "a renew must keep the owner, got {:?}",
            renewed.owner
        ));
    }
    Ok(())
}

/// Every fence the renew refuses: stale generation, foreign owner, lapsed
/// deadline, terminal row, non-running row, absent key. Each writes nothing.
fn heartbeat_refusals_renew_nothing(store: &dyn JournalStore) -> Result<(), String> {
    let refused = |key: &str, owner: &str, deadline: i64, now: i64, fence: u64| {
        let before = store
            .get(key)
            .map_err(|e| format!("get({key}) failed: {e}"))?;
        let out = store
            .renew_lease(key, owner, deadline, now, fence)
            .map_err(|e| format!("renew_lease({key}) failed: {e}"))?;
        if out.is_some() {
            return Err(format!("renew_lease({key}) must refuse"));
        }
        let after = store
            .get(key)
            .map_err(|e| format!("get({key}) failed: {e}"))?;
        if before != after {
            return Err(format!("a refused renew must write nothing ({key})"));
        }
        Ok(())
    };
    // Absent key.
    refused("laws/renew-nope", "nobody", 9_999, 0, 0)?;
    // Live row for the fence cases below.
    store
        .record_intent("laws/renew-fence", 3)
        .map_err(|e| format!("record_intent failed: {e}"))?;
    let row = store
        .claim("laws/renew-fence", "worker-a", 9_999, 0, 0)
        .map_err(|e| format!("claim failed: {e}"))?
        .ok_or("claim should succeed")?;
    let row = store
        .settle("laws/renew-fence", row.generation, EffectState::InFlight)
        .map_err(|e| format!("settle to InFlight failed: {e}"))?
        .ok_or("settle to InFlight should succeed")?;
    // Zombie generation.
    refused("laws/renew-fence", "worker-a", 20_000, 0, 0)?;
    // Foreign owner.
    refused("laws/renew-fence", "worker-b", 20_000, 0, row.generation)?;
    // Lapsed deadline (deadline == now is already expired).
    refused(
        "laws/renew-fence",
        "worker-a",
        20_000,
        9_999,
        row.generation,
    )?;
    // Terminal row.
    store
        .settle("laws/renew-fence", row.generation, EffectState::Settled)
        .map_err(|e| format!("settle failed: {e}"))?;
    refused("laws/renew-fence", "worker-a", 20_000, 0, row.generation)?;
    // Prepared (non-running) row.
    store
        .record_intent("laws/renew-prepared", 3)
        .map_err(|e| format!("record_intent failed: {e}"))?;
    refused("laws/renew-prepared", "worker-a", 20_000, 0, 0)?;
    Ok(())
}

/// Lease-renewal laws, by name, with outcomes.
/// [`journal_store_all`] keeps its identical case list.
pub fn journal_lease_laws(store: &dyn JournalStore) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        (
            "heartbeat_renews_the_lease",
            heartbeat_renews_the_lease(store),
        ),
        (
            "heartbeat_refusals_renew_nothing",
            heartbeat_refusals_renew_nothing(store),
        ),
    ]
}

/// Every [`JournalStore`] law, by name, with its outcome.
pub fn journal_store_all(store: &dyn JournalStore) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        ("intent_is_idempotent", intent_is_idempotent(store)),
        ("claim_lease_is_fenced", claim_lease_is_fenced(store)),
        (
            "settle_generation_is_fenced",
            settle_generation_is_fenced(store),
        ),
        ("attempts_dead_letter", attempts_dead_letter(store)),
    ]
}
