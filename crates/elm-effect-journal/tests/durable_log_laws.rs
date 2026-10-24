//! Trait-level L16 laws, run through `&dyn DurableLog` (ADR-0002 0002:644:
//! `stale_epoch_is_fenced` must hold on the trait, not only on `EpochLog`).
//! Negative controls prove each law discriminates: a log without the epoch
//! fence fails `stale_epoch_is_fenced`, and a log without dedupe fails
//! `same_triple_appends_once`.

use std::collections::HashMap;
use std::sync::Mutex;

use elm_effect_journal::durable_log::{AppendError, DurableLog, MemLog, Offset};
use elm_effect_journal::laws;
use elm_effect_journal::stream::Status;

#[test]
fn stale_epoch_is_fenced_on_the_trait() {
    let log = MemLog::new();
    laws::stale_epoch_is_fenced(&log).unwrap();
}

#[test]
fn same_triple_appends_once_on_the_trait() {
    laws::same_triple_appends_once(&MemLog::new()).unwrap();
}

#[test]
fn seq_gap_is_rejected_on_the_trait() {
    laws::seq_gap_is_rejected(&MemLog::new()).unwrap();
}

#[test]
fn new_epoch_starts_at_zero_on_the_trait() {
    laws::new_epoch_starts_at_zero(&MemLog::new()).unwrap();
}

#[test]
fn paging_reaches_every_record_on_the_trait() {
    laws::paging_reaches_every_record(&MemLog::new()).unwrap();
}

#[test]
fn every_law_holds_on_one_shared_memlog() {
    let log = MemLog::new();
    for (name, outcome) in laws::all(&log) {
        assert!(outcome.is_ok(), "{name}: {outcome:?}");
    }
}

#[test]
fn closing_laws_hold_on_fresh_logs() {
    // Each closure law closes its log, so each gets its own: sharing one
    // would make the second law's first append hit the closed wall.
    for (name, law) in laws::closing() {
        let outcome = law(&MemLog::new());
        assert!(outcome.is_ok(), "{name}: {outcome:?}");
    }
}

#[test]
fn stale_epoch_error_names_producer_and_epochs() {
    let log = MemLog::new();
    log.append_json(&laws::envelope("z", 3, 0, "1")).unwrap();
    assert_eq!(
        log.append_json(&laws::envelope("z", 2, 0, "1")),
        Err(AppendError::StaleEpoch {
            producer: "z".into(),
            sent: 2,
            current: 3
        })
    );
    // A different producer is unaffected by z's epoch.
    assert!(log.append_json(&laws::envelope("y", 0, 0, "1")).is_ok());
    assert_eq!(log.read_from(&Offset::Start).unwrap().len(), 2);
}

/// Negative control: the pre-fix shape. One independent MemLog per epoch,
/// so seq rules hold within an epoch but a lower epoch is never fenced.
#[derive(Default)]
struct UnfencedLog {
    per_epoch: Mutex<HashMap<u64, MemLog>>,
}

impl DurableLog for UnfencedLog {
    fn append_json(&self, envelope: &str) -> Result<Offset, AppendError> {
        let v: serde_json::Value = serde_json::from_str(envelope).unwrap();
        let epoch = v["producer"]["epoch"].as_u64().unwrap();
        let mut map = self.per_epoch.lock().unwrap();
        // Re-key the envelope to epoch 0 inside the per-epoch log so each
        // epoch's sequence starts fresh (the MemLog itself now fences).
        let mut v0 = v.clone();
        v0["producer"]["epoch"] = serde_json::json!(0);
        map.entry(epoch).or_default().append_json(&v0.to_string())
    }
    fn read_from(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError> {
        let map = self.per_epoch.lock().unwrap();
        let mut out = Vec::new();
        for log in map.values() {
            out.extend(log.read_from(from)?);
        }
        Ok(out)
    }
    fn compact_before(&self, _before: &Offset) -> Result<usize, AppendError> {
        Ok(0)
    }
    fn close(&self) -> Result<(), AppendError> {
        let map = self.per_epoch.lock().unwrap();
        for log in map.values() {
            log.close()?;
        }
        Ok(())
    }
    fn append_json_and_close(&self, envelope: &str) -> Result<Offset, AppendError> {
        let offset = self.append_json(envelope)?;
        self.close()?;
        Ok(offset)
    }
    fn status(&self) -> Status {
        let map = self.per_epoch.lock().unwrap();
        if map.is_empty() || map.values().any(|log| !log.is_closed()) {
            Status::UpToDate
        } else {
            Status::Closed
        }
    }
}

/// Negative control: appends every call, no dedupe.
#[derive(Default)]
struct NoDedupeLog {
    rows: Mutex<Vec<String>>,
    closed: Mutex<bool>,
}

impl DurableLog for NoDedupeLog {
    fn append_json(&self, envelope: &str) -> Result<Offset, AppendError> {
        if *self.closed.lock().unwrap() {
            return Err(AppendError::Closed);
        }
        let mut rows = self.rows.lock().unwrap();
        rows.push(envelope.to_string());
        Ok(Offset::At(format!("{:020}", rows.len() - 1)))
    }
    fn read_from(&self, _from: &Offset) -> Result<Vec<(Offset, String)>, AppendError> {
        let rows = self.rows.lock().unwrap();
        Ok(rows
            .iter()
            .enumerate()
            .map(|(i, r)| (Offset::At(format!("{i:020}")), r.clone()))
            .collect())
    }
    fn compact_before(&self, _before: &Offset) -> Result<usize, AppendError> {
        Ok(0)
    }
    fn close(&self) -> Result<(), AppendError> {
        *self.closed.lock().unwrap() = true;
        Ok(())
    }
    fn append_json_and_close(&self, envelope: &str) -> Result<Offset, AppendError> {
        let offset = self.append_json(envelope)?;
        self.close()?;
        Ok(offset)
    }
    fn status(&self) -> Status {
        if *self.closed.lock().unwrap() {
            Status::Closed
        } else {
            Status::UpToDate
        }
    }
}

#[test]
fn negative_control_unfenced_log_fails_the_fence_law() {
    let log = UnfencedLog::default();
    let outcome = laws::stale_epoch_is_fenced(&log);
    assert!(
        outcome.is_err(),
        "an unfenced log must fail stale_epoch_is_fenced"
    );
    // The law is specific: the same log still dedupes within an epoch.
    laws::same_triple_appends_once(&UnfencedLog::default()).unwrap();
}

#[test]
fn negative_control_no_dedupe_log_fails_the_once_law() {
    let outcome = laws::same_triple_appends_once(&NoDedupeLog::default());
    assert!(
        outcome.is_err(),
        "a log without dedupe must fail same_triple_appends_once"
    );
}
