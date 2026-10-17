//! Crash recovery at each append boundary. A flaky log fails after a fixed
//! budget of appends, which stands in for a crash. One test function runs
//! every scenario in order, because the provider counter is process-global.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect::{Effect, ManagedRuntime};
use elm_effect_elm::{
    Boot, Cmd, DispatchPlan, Durability, Failure, Interpret, InterpretError, Loop, Producer,
    Program, ProviderCapability, RecoverError, SnapshotError, Update,
};
use elm_effect_journal::durable_log::{AppendError, DurableLog, FileLog, MemLog, Offset};
use serde::{Deserialize, Serialize};

static PROVIDER_RUNS: AtomicUsize = AtomicUsize::new(0);

/// What the provider offers for a command. Plain has no retry and no query,
/// so an uncertain outcome must be refused.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum Mode {
    Plain,
    Keyed,
    Queryable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Msg {
    Add(i64),
    AddKeyed(i64),
    AddQueryable(i64),
    Ran,
    Failed(Failure<InterpretError>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Persist(i64, Mode);

impl Cmd for Persist {
    fn durability(&self) -> Durability {
        Durability::Durable {
            key: "count".to_string(),
        }
    }

    fn capability(&self) -> ProviderCapability {
        match self.1 {
            Mode::Plain => ProviderCapability {
                supports_idempotency_key: false,
                supports_reconcile_query: false,
            },
            Mode::Keyed => ProviderCapability {
                supports_idempotency_key: true,
                supports_reconcile_query: false,
            },
            Mode::Queryable => ProviderCapability {
                supports_idempotency_key: false,
                supports_reconcile_query: true,
            },
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Model {
    count: i64,
    provider_runs: usize,
}

struct Counter;

impl Program for Counter {
    type Model = Model;
    type Msg = Msg;
    type Cmd = Persist;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<Model, Persist> {
        Update::quiet(Model::default())
    }

    fn update(model: &Model, msg: &Msg) -> Update<Model, Persist> {
        let add = |n: &i64, mode: Mode| {
            let count = model.count + n;
            Update::with_commands(
                Model {
                    count,
                    ..model.clone()
                },
                vec![Persist(count, mode)],
            )
        };
        match msg {
            Msg::Add(n) => add(n, Mode::Plain),
            Msg::AddKeyed(n) => add(n, Mode::Keyed),
            Msg::AddQueryable(n) => add(n, Mode::Queryable),
            Msg::Ran => Update::quiet(Model {
                provider_runs: model.provider_runs + 1,
                ..model.clone()
            }),
            Msg::Failed(_) => Update::quiet(model.clone()),
        }
    }

    fn failure(failure: &Failure<InterpretError>) -> Msg {
        Msg::Failed(failure.clone())
    }
}

struct Run;

impl Interpret<Counter> for Run {
    fn interpret(_cmd: &Persist) -> Effect<Msg, InterpretError> {
        Effect::sync(|| {
            PROVIDER_RUNS.fetch_add(1, Ordering::SeqCst);
            Msg::Ran
        })
    }

    /// Asking a queryable provider is not a dispatch, so it does not count as
    /// a provider run.
    fn reconcile(cmd: &Persist) -> Option<Effect<Msg, InterpretError>> {
        match cmd.1 {
            Mode::Queryable => Some(Effect::sync(|| Msg::Ran)),
            _ => None,
        }
    }
}

/// Appends succeed until `budget` runs out, then every append fails. A failed
/// append is the stand-in for a crash at that boundary, and it never touches
/// the inner log.
struct Flaky {
    inner: Arc<dyn DurableLog>,
    budget: AtomicUsize,
}

impl Flaky {
    fn with_budget(inner: Arc<dyn DurableLog>, budget: usize) -> Arc<Self> {
        Arc::new(Self {
            inner,
            budget: AtomicUsize::new(budget),
        })
    }

    fn set_budget(&self, budget: usize) {
        self.budget.store(budget, Ordering::SeqCst);
    }
}

impl DurableLog for Flaky {
    fn append_json(&self, envelope: &str) -> Result<Offset, AppendError> {
        let spent = self
            .budget
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |b| b.checked_sub(1));
        if spent.is_err() {
            return Err(AppendError::NotAnEnvelope("injected: log down".to_string()));
        }
        self.inner.append_json(envelope)
    }

    fn read_from(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError> {
        self.inner.read_from(from)
    }

    fn compact_before(&self, before: &Offset) -> Result<usize, AppendError> {
        self.inner.compact_before(before)
    }
}

fn recover(log: Arc<dyn DurableLog>) -> Result<Loop<Counter, Run>, RecoverError> {
    Loop::<Counter, Run>::recover(
        &Boot::default(),
        ManagedRuntime::new(),
        log,
        Producer::new("p"),
    )
}

fn boot(log: Arc<dyn DurableLog>) -> Loop<Counter, Run> {
    Loop::<Counter, Run>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        log,
        Producer::new("p"),
    )
    .unwrap()
}

fn runs() -> usize {
    PROVIDER_RUNS.load(Ordering::SeqCst)
}

/// A unique file path per run, removed when dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_file(self.0.with_extension("compacting"));
    }
}

#[test]
fn crash_at_each_boundary_recovers_exactly_once() {
    // Scenario 1: the message reached the log, but the intent did not. The
    // command never started, so recovery runs it exactly once.
    let inner: Arc<dyn DurableLog> = Arc::new(MemLog::new());
    let flaky = Flaky::with_budget(Arc::clone(&inner), usize::MAX);
    let mut app = boot(Arc::clone(&flaky) as Arc<dyn DurableLog>);
    app.send(Msg::Add(3)).unwrap();
    // Budget 1: the message append succeeds, the intent append fails, and the
    // outcome append fails too, so the send reports the error.
    flaky.set_budget(1);
    assert!(
        app.send(Msg::Add(4)).is_err(),
        "send must report the refused append"
    );
    drop(app);

    let before = runs();
    let rebuilt = recover(Arc::clone(&inner)).unwrap();
    assert_eq!(
        rebuilt.model().count,
        7,
        "the refused-intent message is folded from the log"
    );
    assert_eq!(
        runs(),
        before + 1,
        "the pending command runs exactly once on recovery"
    );
    assert!(
        rebuilt.in_flight().is_empty(),
        "a never-started command is pending, not in flight"
    );

    let before = runs();
    let again = recover(Arc::clone(&inner)).unwrap();
    assert_eq!(again.model(), rebuilt.model());
    assert_eq!(runs(), before, "a second recovery finds nothing pending");

    // Scenario 2: the intent and the provider run, but the outcome is lost.
    // The plain provider has no retry and no query, so the planner refuses.
    let inner: Arc<dyn DurableLog> = Arc::new(MemLog::new());
    let flaky = Flaky::with_budget(Arc::clone(&inner), usize::MAX);
    let mut app = boot(Arc::clone(&flaky) as Arc<dyn DurableLog>);
    app.send(Msg::Add(3)).unwrap();
    flaky.set_budget(2);
    let before = runs();
    assert!(app.send(Msg::Add(4)).is_err());
    assert_eq!(runs(), before + 1, "the provider ran once before the crash");
    drop(app);

    let before = runs();
    let mut rebuilt = recover(Arc::clone(&inner)).unwrap();
    assert_eq!(
        runs(),
        before,
        "an in-flight command must not run during recovery"
    );
    assert_eq!(
        rebuilt.in_flight().len(),
        1,
        "the lost outcome is reported in flight"
    );
    assert_eq!(rebuilt.in_flight()[0].key, "count");
    assert_eq!(rebuilt.in_flight()[0].origin.index, 0);
    assert!(matches!(
        rebuilt.in_flight()[0].plan,
        DispatchPlan::RefuseBlindRetry { .. }
    ));
    assert_eq!(
        rebuilt.reconcile().unwrap(),
        0,
        "a plain provider cannot be reconciled"
    );
    assert_eq!(rebuilt.in_flight().len(), 1, "the command stays in flight");
    assert_eq!(runs(), before, "refusal dispatches nothing");
    assert_eq!(rebuilt.snapshot(), Err(SnapshotError::InFlight(1)));

    // Scenario 3: the same loss, but the provider accepts an idempotency key.
    // Reconcile re-dispatches under the same key as attempt 2.
    let inner: Arc<dyn DurableLog> = Arc::new(MemLog::new());
    let flaky = Flaky::with_budget(Arc::clone(&inner), usize::MAX);
    let mut app = boot(Arc::clone(&flaky) as Arc<dyn DurableLog>);
    app.send(Msg::Add(3)).unwrap();
    flaky.set_budget(2);
    assert!(app.send(Msg::AddKeyed(4)).is_err());
    drop(app);

    let mut rebuilt = recover(Arc::clone(&inner)).unwrap();
    assert!(
        matches!(
            rebuilt.in_flight()[0].plan,
            DispatchPlan::RetrySameKey { attempt: 2 }
        ),
        "a keyed provider is retried under the same key: {:?}",
        rebuilt.in_flight()[0].plan
    );
    let before = runs();
    assert_eq!(
        rebuilt.reconcile().unwrap(),
        1,
        "the retry produces the outcome"
    );
    assert_eq!(runs(), before + 1, "exactly one re-dispatch");
    assert!(rebuilt.in_flight().is_empty());
    assert_eq!(rebuilt.model().count, 7);
    assert_eq!(
        rebuilt.model().provider_runs,
        2,
        "the first run's lost outcome stays lost; the retry's is folded"
    );
    let before = runs();
    let settled = recover(Arc::clone(&inner)).unwrap();
    assert!(
        settled.in_flight().is_empty(),
        "the retry's outcome resolved the command after a restart"
    );
    assert_eq!(runs(), before);

    // Scenario 4: a queryable provider is asked first. The query answers
    // without dispatching, so the provider run count does not move.
    let inner: Arc<dyn DurableLog> = Arc::new(MemLog::new());
    let flaky = Flaky::with_budget(Arc::clone(&inner), usize::MAX);
    let mut app = boot(Arc::clone(&flaky) as Arc<dyn DurableLog>);
    app.send(Msg::Add(3)).unwrap();
    flaky.set_budget(2);
    assert!(app.send(Msg::AddQueryable(4)).is_err());
    drop(app);

    let mut rebuilt = recover(Arc::clone(&inner)).unwrap();
    assert!(matches!(
        rebuilt.in_flight()[0].plan,
        DispatchPlan::ReconcileFirst { .. }
    ));
    let before = runs();
    assert_eq!(rebuilt.reconcile().unwrap(), 1);
    assert_eq!(runs(), before, "a query never dispatches the provider");
    assert_eq!(
        rebuilt.model().provider_runs,
        2,
        "the query's answer is folded as the outcome"
    );
    assert!(rebuilt.in_flight().is_empty());
}

#[test]
fn a_crash_on_the_file_log_recovers_after_a_real_restart() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let scratch = Scratch(
        std::env::temp_dir().join(format!("elm-crash-{}-{nanos}.jsonl", std::process::id())),
    );

    let file = FileLog::open(&scratch.0).unwrap();
    let flaky = Flaky::with_budget(Arc::new(file), usize::MAX);
    let mut app = boot(Arc::clone(&flaky) as Arc<dyn DurableLog>);
    app.send(Msg::Add(3)).unwrap();
    flaky.set_budget(1);
    assert!(app.send(Msg::Add(4)).is_err());
    drop(app);
    drop(flaky);

    // A fresh process: the file is the only thing that survives.
    let reopened: Arc<dyn DurableLog> = Arc::new(FileLog::open(&scratch.0).unwrap());
    let before = runs();
    let rebuilt = recover(reopened).unwrap();
    assert_eq!(
        rebuilt.model().count,
        7,
        "the message appended before the crash is on disk"
    );
    assert_eq!(
        runs(),
        before + 1,
        "the pending command runs once after the restart"
    );
}
