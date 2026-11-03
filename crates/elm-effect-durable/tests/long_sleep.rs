//! Long-horizon sleep proof (§7): a 30-day park is an absolute `until_ms`
//! with only the reported wait capped. This bead proves it; it changes no `src/`.

use std::cell::RefCell;
use std::rc::Rc;

use elm_effect::{ClockImpl, Duration, TestClock};
use elm_effect_durable::signal::SignalRow;
use elm_effect_durable::{
    DurableEngine, EngineMsg, EngineStorage, MemStorage, OurEngine, ParkOutcome, ParkedRow,
    RunPhase, RunRow, StartOutcome, StorageError,
};
use elm_effect_journal::store::JournalStore;

/// Thirty days in ms: the Temporal `workflow.sleep("30 days")` shape.
const THIRTY_DAYS_MS: u64 = 30 * 24 * 60 * 60 * 1_000;

fn started<S: EngineStorage, J: JournalStore>(engine: &mut OurEngine<S, J>, key: &str) {
    match engine.start(key, 0).expect("control start") {
        StartOutcome::Started { .. } => {}
        StartOutcome::Already { .. } => panic!("fresh key must start: {key}"),
    }
}

/// Store with shared state, so the proof can restart an engine on the
/// exact storage the previous engine wrote.
struct SharedStore {
    inner: Rc<RefCell<MemStorage>>,
}

impl EngineStorage for SharedStore {
    fn get_run(&self, key: &str) -> Result<Option<RunRow>, StorageError> {
        self.inner.borrow().get_run(key)
    }

    fn put_run(&mut self, row: RunRow) -> Result<(), StorageError> {
        self.inner.borrow_mut().put_run(row)
    }

    fn list_parked(&self) -> Result<Vec<ParkedRow>, StorageError> {
        self.inner.borrow().list_parked()
    }

    fn upsert_park(&mut self, row: ParkedRow) -> Result<bool, StorageError> {
        self.inner.borrow_mut().upsert_park(row)
    }

    fn clear_parks(&mut self, key: &str) -> Result<(), StorageError> {
        self.inner.borrow_mut().clear_parks(key)
    }

    fn put_signal(
        &mut self,
        key: &str,
        signal_id: &str,
        payload: &str,
    ) -> Result<bool, StorageError> {
        self.inner.borrow_mut().put_signal(key, signal_id, payload)
    }

    fn pending_signals(&self, key: &str) -> Result<Vec<SignalRow>, StorageError> {
        self.inner.borrow().pending_signals(key)
    }

    fn take_signals(&mut self, key: &str) -> Result<Vec<SignalRow>, StorageError> {
        self.inner.borrow_mut().take_signals(key)
    }

    fn clear_signals(&mut self, key: &str) -> Result<(), StorageError> {
        self.inner.borrow_mut().clear_signals(key)
    }

    fn get_digest(&self, key: &str) -> Result<Option<String>, StorageError> {
        self.inner.borrow().get_digest(key)
    }

    fn set_digest(&mut self, key: &str, digest: &str) -> Result<(), StorageError> {
        self.inner.borrow_mut().set_digest(key, digest)
    }
}

#[test]
fn thirty_day_park_stores_the_absolute_deadline_with_capped_wait() {
    let clock = TestClock::new();
    clock.set_time(1_000);
    let mut engine = OurEngine::new();
    started(&mut engine, "long");
    let until_ms = clock.now_millis() + THIRTY_DAYS_MS;
    match engine
        .park_until("long", until_ms, 3_600, clock.now_millis(), None)
        .expect("park")
    {
        ParkOutcome::Wait {
            until_ms: got,
            wait_ms,
        } => {
            assert_eq!(got, until_ms, "until_ms stays absolute, never shortened");
            assert_eq!(wait_ms, 3_600_000, "only the reported wait is capped");
        }
        other => panic!("30-day park must Wait: {other:?}"),
    }
    assert_eq!(
        engine.query("long").expect("query").until_ms,
        Some(until_ms)
    );
}

#[test]
fn drain_fires_only_at_the_deadline() {
    let clock = TestClock::new();
    clock.set_time(0);
    let mut engine = OurEngine::new();
    started(&mut engine, "deadline");
    let until_ms = THIRTY_DAYS_MS;
    assert!(matches!(
        engine
            .park_until("deadline", until_ms, 3_600, 0, None)
            .expect("park"),
        ParkOutcome::Wait { .. }
    ));
    clock.adjust(Duration::from_millis(until_ms - 1));
    assert!(
        engine
            .drain_due(clock.now_millis())
            .expect("drain")
            .is_empty(),
        "one ms early drains nothing"
    );
    clock.adjust(Duration::from_millis(1));
    let wakes = engine.drain_due(clock.now_millis()).expect("drain");
    assert_eq!(wakes.len(), 1, "the deadline wakes exactly once");
    let EngineMsg::Woke {
        key, overdue_ms, ..
    } = &wakes[0];
    assert_eq!(key, "deadline");
    assert_eq!(*overdue_ms, 0);
    assert_eq!(
        engine.query("deadline").expect("query").phase,
        RunPhase::Running
    );
}

#[test]
fn long_park_survives_engine_restart() {
    let shared = Rc::new(RefCell::new(MemStorage::new()));
    let mut engine = OurEngine::with_storage(SharedStore {
        inner: shared.clone(),
    });
    started(&mut engine, "survive");
    assert!(matches!(
        engine
            .park_until("survive", THIRTY_DAYS_MS, 3_600, 0, None)
            .expect("park"),
        ParkOutcome::Wait { .. }
    ));
    drop(engine);
    let mut recovered = OurEngine::recover(SharedStore { inner: shared });
    let wakes = recovered.drain_due(THIRTY_DAYS_MS).expect("drain");
    assert_eq!(wakes.len(), 1, "the park survives the restart");
    let EngineMsg::Woke { overdue_ms, .. } = &wakes[0];
    assert_eq!(*overdue_ms, 0);
}

#[test]
fn over_ceiling_still_refuses_a_long_sleep() {
    let mut engine = OurEngine::new();
    started(&mut engine, "ceiling");
    match engine
        .park_until("ceiling", THIRTY_DAYS_MS, 200_000, 0, Some(86_400))
        .expect("park")
    {
        ParkOutcome::RefusedOverCeiling {
            asked_secs,
            ceiling_secs,
        } => {
            assert_eq!(asked_secs, 200_000);
            assert_eq!(ceiling_secs, 86_400);
        }
        other => panic!("above-ceiling park must refuse: {other:?}"),
    }
    assert_eq!(
        engine.query("ceiling").expect("query").phase,
        RunPhase::Running,
        "a refused park leaves the run Running"
    );
}

#[test]
fn reparking_a_long_sleep_keeps_one_park() {
    let mut engine = OurEngine::new();
    started(&mut engine, "repark-long");
    assert!(matches!(
        engine
            .park_until("repark-long", THIRTY_DAYS_MS, 3_600, 0, None)
            .expect("park"),
        ParkOutcome::Wait { .. }
    ));
    let later = THIRTY_DAYS_MS + 1_000;
    match engine
        .park_until("repark-long", later, 3_600, 0, None)
        .expect("repark")
    {
        ParkOutcome::Wait { until_ms, .. } => assert_eq!(until_ms, later),
        other => panic!("re-park must Wait: {other:?}"),
    }
    let parks = engine.parked().expect("parked");
    assert_eq!(parks.len(), 1, "re-parking keeps one park");
    assert_eq!(parks[0].until_ms, later);
}
