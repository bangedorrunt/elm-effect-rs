//! A transient append error must not disable compaction for good.
//!
//! `drain` halts the loop when an append fails, and `snapshot` refuses while
//! halted, so a request whose append was refused could never be covered. The
//! halt is cleared by the next drain that finishes, which is the only evidence
//! the loop has that the log is usable again.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use elm_effect::{Effect, ManagedRuntime};
use elm_effect_elm::{
    Boot, Cmd, Durability, Failure, Interpret, InterpretError, Loop, Producer, Program,
    SnapshotError, Update,
};
use elm_effect_journal::durable_log::Status;
use elm_effect_journal::durable_log::{AppendError, DurableLog, MemLog, Offset, Page};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Msg {
    Add(i64),
    Failure(Failure<InterpretError>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct NoCmd;

impl Cmd for NoCmd {
    fn durability(&self) -> Durability {
        Durability::Ephemeral
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Model {
    count: i64,
}

struct Counter;

impl Program for Counter {
    type Model = Model;
    type Msg = Msg;
    type Cmd = NoCmd;
    type Err = InterpretError;

    fn init(_boot: &Boot) -> Update<Model, NoCmd> {
        Update::quiet(Model::default())
    }

    fn update(model: &Model, msg: &Msg) -> Update<Model, NoCmd> {
        match msg {
            Msg::Add(n) => Update::quiet(Model {
                count: model.count + n,
            }),
            Msg::Failure(_) => Update::quiet(model.clone()),
        }
    }

    fn failure(failure: &Failure<InterpretError>) -> Msg {
        Msg::Failure(failure.clone())
    }
}

struct Nothing;

impl Interpret<Counter> for Nothing {
    fn interpret(_cmd: &NoCmd) -> Effect<Msg, InterpretError> {
        // No command is ever produced by this program; the arm only satisfies
        // the trait.
        Effect::sync(|| Msg::Add(0))
    }
}

/// Fails every append once its budget is spent, then delegates.
struct Flaky {
    inner: Arc<dyn DurableLog>,
    budget: AtomicUsize,
}

impl Flaky {
    fn new(inner: Arc<dyn DurableLog>) -> Self {
        Self {
            inner,
            budget: AtomicUsize::new(usize::MAX),
        }
    }

    fn set_budget(&self, budget: usize) {
        self.budget.store(budget, Ordering::SeqCst);
    }
}

impl DurableLog for Flaky {
    fn append_json(&self, envelope: &str) -> Result<Offset, AppendError> {
        if self
            .budget
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |b| b.checked_sub(1))
            .is_err()
        {
            return Err(AppendError::Unavailable("injected".to_string()));
        }
        self.inner.append_json(envelope)
    }

    fn read_from(&self, from: &Offset) -> Result<Vec<(Offset, String)>, AppendError> {
        self.inner.read_from(from)
    }

    fn read_page(&self, from: &Offset, limit: usize) -> Result<Page, AppendError> {
        self.inner.read_page(from, limit)
    }

    fn compact_before(&self, before: &Offset) -> Result<usize, AppendError> {
        self.inner.compact_before(before)
    }

    fn close(&self) -> Result<(), AppendError> {
        self.inner.close()
    }

    fn append_json_and_close(&self, envelope: &str) -> Result<Offset, AppendError> {
        self.inner.append_json_and_close(envelope)
    }

    fn status(&self) -> Status {
        self.inner.status()
    }
}

#[test]
fn a_finished_drain_clears_the_halt_so_snapshot_works_again() {
    let flaky = Arc::new(Flaky::new(Arc::new(MemLog::new())));
    let as_log: Arc<dyn DurableLog> = Arc::clone(&flaky) as Arc<dyn DurableLog>;
    let mut app = Loop::<Counter, Nothing>::boot(
        &Boot::default(),
        ManagedRuntime::new(),
        as_log,
        Producer::new("p"),
    )
    .unwrap();
    app.send(Msg::Add(1)).unwrap();

    flaky.set_budget(0);
    assert!(app.send(Msg::Add(1)).is_err(), "the append is refused");
    assert_eq!(app.model().count, 1, "the refused message never folded");
    assert_eq!(
        app.snapshot(),
        Err(SnapshotError::Halted),
        "while the log is down, a snapshot would cover unrun work"
    );

    flaky.set_budget(usize::MAX);
    app.send(Msg::Add(2)).unwrap();
    assert_eq!(app.model().count, 3);
    assert!(
        app.snapshot().is_ok(),
        "one transient append error must not disable compaction forever"
    );
}
