//! `Request` at the pin: a request names work a resolver performs, and
//! `request` submits one to a batch.
//!
//! Submissions to the same resolver (and the same `grouped` key) join one
//! batch: the first entry opens it, later entries join until the window
//! closes, then the resolver runs once for the whole batch and answers each
//! entry through [`Entry::complete`]. The pin closes a batch at the next
//! `yieldNow`; this port has one thread per fiber and no tick to hang that on,
//! so a batch stays open for [`BATCH_WINDOW`] of wall-clock time instead.
//!
//! A caller waits for its own entry, not for the batch: an interrupted caller
//! withdraws its entry (unless another caller coalesced onto it through the
//! cache) and the rest of the batch still runs.

use std::collections::HashMap;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use crate::service::Services;
use crate::{Cause, Defect, Effect, Exit};

/// How long a batch stays open for more entries after its first one.
///
/// ponytail: a fixed wall-clock window stands in for the pin's `yieldNow`.
/// A caller that needs a batch to close now can wait this long; a scheduler
/// tick on the fiber runtime would replace it.
pub const BATCH_WINDOW: Duration = Duration::from_millis(25);

/// How often a waiting request re-checks for its answer and for interruption.
/// A condvar notify wakes it sooner; the poll only bounds interrupt latency.
const POLL: Duration = Duration::from_millis(1);

// Request

/// A request: a value that names work, plus the types of its answer and its
/// failure.
///
/// `Clone` because a resolver receives copies of what was submitted; `Hash`
/// and `Eq` where a resolver caches by request value.
pub trait Request: Clone + Send + Sync + 'static {
    /// What the resolver answers for this request.
    type Success: Clone + Send + 'static;
    /// How the resolver fails this request.
    type Error: Clone + Send + 'static;
    /// The tag a [`TaggedResolver`] dispatches on.
    fn tag(&self) -> &'static str;
}

// Entry

type Hook<R> = Box<dyn FnOnce(&Exit<<R as Request>::Success, <R as Request>::Error>) + Send>;

struct EntryState<R: Request> {
    exit: Mutex<Option<Exit<R::Success, R::Error>>>,
    ready: Condvar,
    /// Set when the waiting caller withdrew this entry.
    cancelled: AtomicBool,
    /// Set when a second caller coalesced onto this entry: withdrawing one
    /// caller must not withdraw an answer the other still waits for.
    shared: AtomicBool,
    on_complete: Mutex<Vec<Hook<R>>>,
}

/// One submitted request, as the resolver sees it.
///
/// Cloning an entry shares its answer: every clone sees the same completion.
pub struct Entry<R: Request> {
    /// The request itself.
    pub request: R,
    state: Arc<EntryState<R>>,
}

impl<R: Request> Clone for Entry<R> {
    fn clone(&self) -> Self {
        Self {
            request: self.request.clone(),
            state: Arc::clone(&self.state),
        }
    }
}

impl<R: Request> Entry<R> {
    /// Wrap a request for submission.
    pub fn new(request: R) -> Self {
        Self {
            request,
            state: Arc::new(EntryState {
                exit: Mutex::new(None),
                ready: Condvar::new(),
                cancelled: AtomicBool::new(false),
                shared: AtomicBool::new(false),
                on_complete: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Answer this entry. The first answer wins; a later one is ignored, which
    /// is what makes a cancellation racing a resolver harmless.
    pub fn complete(&self, exit: Exit<R::Success, R::Error>) {
        let hooks = {
            let mut slot = self.state.exit.lock().unwrap();
            if slot.is_some() {
                return;
            }
            *slot = Some(exit);
            self.state.ready.notify_all();
            std::mem::take(&mut *self.state.on_complete.lock().unwrap())
        };
        let exit = self
            .state
            .exit
            .lock()
            .unwrap()
            .clone()
            .expect("the exit was just stored");
        for hook in hooks {
            hook(&exit);
        }
    }

    /// The pin's name for the same contract: answer from inside a resolver
    /// body. Kept as an alias so a carried test reads like its source.
    pub fn complete_unsafe(&self, exit: Exit<R::Success, R::Error>) {
        self.complete(exit);
    }

    /// True once this entry has an answer.
    pub fn is_done(&self) -> bool {
        self.state.exit.lock().unwrap().is_some()
    }

    /// This entry's answer, if it has one.
    pub fn exit(&self) -> Option<Exit<R::Success, R::Error>> {
        self.state.exit.lock().unwrap().clone()
    }

    /// Run `hook` with this entry's answer when it arrives. Hooks run in
    /// registration order, outside the entry's locks, so a hook may complete
    /// another entry (the cache does exactly that).
    pub fn register_on_complete(&self, hook: Hook<R>) {
        let mut hooks = self.state.on_complete.lock().unwrap();
        // An answer already in hand runs the hook now: the pin patches
        // `completeUnsafe`, which also takes effect for later completions.
        if let Some(exit) = self.state.exit.lock().unwrap().clone() {
            drop(hooks);
            hook(&exit);
            return;
        }
        hooks.push(Box::new(hook));
    }

    fn mark_shared(&self) {
        self.state.shared.store(true, Ordering::SeqCst);
    }

    /// Withdraw this entry. A shared entry stays: another caller still waits
    /// for it, and the batch has to answer it once.
    fn cancel(&self) {
        if self.state.shared.load(Ordering::SeqCst) {
            return;
        }
        self.state.cancelled.store(true, Ordering::SeqCst);
        self.complete(Exit::Failure(Cause::interrupt(None)));
    }

    fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::SeqCst)
    }

    /// Wait for this entry's answer, honouring interruption. A masked region
    /// keeps waiting: the pin's `uninterruptible` request does not let go
    /// until the batch answers.
    fn wait(&self) -> Result<R::Success, Cause<R::Error>> {
        loop {
            if let Some(exit) = self.state.exit.lock().unwrap().clone() {
                return match exit {
                    Exit::Success(value) => Ok(value),
                    Exit::Failure(cause) => Err(cause),
                };
            }
            // Without the fiber runtime a run cannot be interrupted, so there
            // is no flag to poll and the wait is a plain condvar wait.
            #[cfg(feature = "tokio")]
            {
                let fiber = crate::fiber::current();
                if let Some(fiber) = &fiber
                    && fiber.is_interrupted()
                    && !fiber.masked_now()
                {
                    self.cancel();
                    return Err(Cause::interrupt(Some(fiber.id)));
                }
            }
            let slot = self.state.exit.lock().unwrap();
            if slot.is_none() {
                let _ = self.state.ready.wait_timeout(slot, POLL).unwrap();
            }
        }
    }
}

// Resolver

type RunFn<R> = Arc<dyn Fn(Vec<Entry<R>>) -> Effect<(), <R as Request>::Error> + Send + Sync>;
type PreCheck<R> = Arc<dyn Fn(&Entry<R>) -> bool + Send + Sync>;
type DelayFn = Arc<dyn Fn() -> Effect<(), std::convert::Infallible> + Send + Sync>;
/// A resolver for `R`: one call per batch, answering every entry it was given.
///
/// Cloning shares both the batch queue and the resolver body: clones are the
/// same resolver, so submissions through them join one batch.
pub struct Resolver<R: Request> {
    inner: Arc<Inner<R>>,
}

impl<R: Request> Clone for Resolver<R> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

struct Inner<R: Request> {
    run: RunFn<R>,
    delay: Option<DelayFn>,
    /// Which batch a request belongs to (`grouped`'s key, hashed).
    key_of: Arc<dyn Fn(&R) -> u64 + Send + Sync>,
    /// Whether a batch of this many entries should keep collecting
    /// (`batchN`'s ceiling).
    collect_while: Arc<dyn Fn(usize) -> bool + Send + Sync>,
    /// Answers an entry before it reaches a batch. `true` submits the entry
    /// (the cache opened a row for it); `false` means it was answered already
    /// or coalesced onto another caller's entry.
    pre_check: Option<PreCheck<R>>,
    groups: Mutex<HashMap<u64, Arc<Group<R>>>>,
}

struct Group<R: Request> {
    state: Mutex<GroupState<R>>,
    ready: Condvar,
}

struct GroupState<R: Request> {
    pending: Vec<Entry<R>>,
    closed: bool,
    spawned: bool,
}

impl<R: Request> Default for Group<R> {
    fn default() -> Self {
        Self {
            state: Mutex::new(GroupState {
                pending: Vec::new(),
                closed: false,
                spawned: false,
            }),
            ready: Condvar::new(),
        }
    }
}

impl<R: Request> Resolver<R> {
    /// Build from a batch body: one effect per batch, free to answer entries
    /// in any order and at any time before it returns.
    pub fn make<F>(run: F) -> Self
    where
        F: Fn(Vec<Entry<R>>) -> Effect<(), R::Error> + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(Inner {
                run: Arc::new(run),
                delay: None,
                key_of: Arc::new(|_| 0),
                collect_while: Arc::new(|_| true),
                pre_check: None,
                groups: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Build from a plain function that answers every entry synchronously.
    pub fn make_sync<F>(run: F) -> Self
    where
        F: Fn(&[Entry<R>]) + Send + Sync + 'static,
    {
        let run = Arc::new(run);
        Self::make(move |entries| {
            let run = Arc::clone(&run);
            Effect::sync(move || run(&entries))
        })
    }

    /// `RequestResolver.fromEffect` at the pin: one effect per entry, each
    /// answered with its own exit. Entries run in batch order.
    pub fn from_effect<F>(f: F) -> Self
    where
        F: Fn(&Entry<R>) -> Effect<R::Success, R::Error> + Send + Sync + 'static,
    {
        let f = Arc::new(f);
        Self::make(move |entries| {
            let mut chain: Effect<(), std::convert::Infallible> = Effect::sync(|| ());
            for entry in entries {
                let f = Arc::clone(&f);
                chain = chain.flat_map(move |()| {
                    let entry = entry.clone();
                    f(&entry).exit().map(move |exit| entry.complete(exit))
                });
            }
            chain.map_err(|never| -> R::Error { match never {} })
        })
    }

    /// `RequestResolver.batchN` at the pin: close the batch once it holds `n`
    /// entries, so the remainder runs in a later batch. The pin's
    /// `Count.normalizeNonEmpty` floors the value and lifts anything below one
    /// (including NaN and negatives) to one.
    pub fn batch_n(self, n: f64) -> Self {
        let size = if n.is_finite() {
            (n.floor() as i64).max(1) as usize
        } else {
            1
        };
        self.with(|inner| Inner {
            collect_while: Arc::new(move |count| count < size),
            ..inner
        })
    }

    /// `RequestResolver.grouped` at the pin: entries with the same key run in
    /// their own batch. Keys are hashed for the batch table.
    ///
    /// ponytail: hash-only keys, so two distinct keys that collide share a
    /// batch; a `HashMap<Box<dyn Any>>` key would remove the ceiling.
    pub fn grouped<K, F>(self, key: F) -> Self
    where
        K: Hash + Send + 'static,
        F: Fn(&R) -> K + Send + Sync + 'static,
    {
        self.with(|inner| Inner {
            key_of: Arc::new(move |request| hash_key(&key(request))),
            ..inner
        })
    }

    /// `RequestResolver.setDelayEffect` at the pin: run `delay` once per batch
    /// before the resolver sees the batch.
    pub fn set_delay_effect<F>(self, delay: F) -> Self
    where
        F: Fn() -> Effect<(), std::convert::Infallible> + Send + Sync + 'static,
    {
        self.with(|inner| Inner {
            delay: Some(Arc::new(delay)),
            ..inner
        })
    }

    /// `RequestResolver.race` at the pin: send the batch to both resolvers and
    /// answer with the first success. The losing run is interrupted and
    /// released before the batch returns.
    pub fn race(self, that: Resolver<R>) -> Self {
        let left = Arc::clone(&self.inner.run);
        let right = Arc::clone(&that.inner.run);
        self.with(|inner| Inner {
            run: Arc::new(move |entries| left(entries.clone()).race(right(entries))),
            ..inner
        })
    }

    /// `RequestResolver.around` at the pin: run `before`, then the wrapped
    /// resolver, then `after`, which runs even when the batch failed.
    pub fn around<B, A2>(self, before: B, after: A2) -> Self
    where
        B: Fn(&[Entry<R>]) -> Effect<(), R::Error> + Send + Sync + 'static,
        A2: Fn(&[Entry<R>]) -> Effect<(), R::Error> + Send + Sync + 'static,
    {
        let run = Arc::clone(&self.inner.run);
        let before = Arc::new(before);
        let after = Arc::new(after);
        self.with(|inner| Inner {
            run: Arc::new(move |entries| {
                let run = Arc::clone(&run);
                let before = Arc::clone(&before);
                let after = Arc::clone(&after);
                before(&entries).flat_map(move |()| {
                    let after = Arc::clone(&after);
                    let entries = entries.clone();
                    run(entries.clone())
                        .exit()
                        .map_err(|never| -> R::Error { match never {} })
                        .flat_map(move |exit| after(&entries).flat_map(move |()| rethrow(exit)))
                })
            }),
            ..inner
        })
    }

    /// `RequestResolver.withCache` at the pin: answer repeated equal requests
    /// from memory, coalesce concurrent ones onto the first entry, and keep at
    /// most `capacity` answers.
    ///
    /// Failures without interruptions are cached like successes; a result that
    /// carries an interruption drops its row instead.
    pub fn with_cache(self, capacity: usize) -> Self
    where
        R: Hash + Eq,
    {
        let cache: Arc<Mutex<Cache<R>>> = Arc::new(Mutex::new(Cache {
            capacity,
            order: 0,
            rows: HashMap::new(),
        }));
        let pre = Arc::clone(&cache);
        self.with(|inner| Inner {
            pre_check: Some(Arc::new(move |entry: &Entry<R>| admit(&pre, entry))),
            ..inner
        })
    }

    /// A combinator builds a new resolver, as the pin's `makeWith({...self})`
    /// does. A fresh group table is that new resolver's batch identity: the
    /// wrapped resolver keeps its own queue.
    fn with<F>(self, change: F) -> Self
    where
        F: FnOnce(Inner<R>) -> Inner<R>,
    {
        let fresh = Inner {
            run: Arc::clone(&self.inner.run),
            delay: self.inner.delay.clone(),
            key_of: Arc::clone(&self.inner.key_of),
            collect_while: Arc::clone(&self.inner.collect_while),
            pre_check: self.inner.pre_check.clone(),
            groups: Mutex::new(HashMap::new()),
        };
        Self {
            inner: Arc::new(change(fresh)),
        }
    }

    /// Submit `entry`, first giving the cache a chance to answer it.
    fn admit(&self, entry: Entry<R>, services: Services) {
        if let Some(pre) = &self.inner.pre_check
            && !pre(&entry)
        {
            return;
        }
        self.enqueue(entry, services);
    }

    fn enqueue(&self, entry: Entry<R>, services: Services) {
        let inner = &self.inner;
        let key = (inner.key_of)(&entry.request);
        // The map lock covers the push and the close decision. Without it two
        // submitters can both find one open group and push, so `batchN(1)`
        // would sometimes run two entries in one call.
        let (group, spawned) = {
            let mut groups = inner.groups.lock().unwrap();
            let group = match groups.get(&key) {
                Some(group) if !group.state.lock().unwrap().closed => Arc::clone(group),
                _ => {
                    let group = Arc::new(Group::<R>::default());
                    groups.insert(key, Arc::clone(&group));
                    group
                }
            };
            let mut state = group.state.lock().unwrap();
            state.pending.push(entry);
            if !(inner.collect_while)(state.pending.len()) {
                state.closed = true;
                group.ready.notify_all();
            }
            let spawned = state.spawned;
            state.spawned = true;
            drop(state);
            (group, spawned)
        };
        if !spawned {
            spawn_batch(Arc::clone(inner), group, key, services);
        }
    }
}

fn hash_key<K: Hash>(key: &K) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

// Batching

/// Run the batch this group collected: wait out the window, run the delay,
/// then hand every surviving entry to the resolver.
fn run_group<R: Request>(inner: Arc<Inner<R>>, group: Arc<Group<R>>, key: u64, services: Services) {
    {
        let mut state = group.state.lock().unwrap();
        let deadline = Instant::now() + BATCH_WINDOW;
        while !state.closed {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let (guard, _) = group.ready.wait_timeout(state, deadline - now).unwrap();
            state = guard;
        }
    }
    // The delay runs while the batch is still collecting, which is the pin's
    // `delay` half of the collection loop: a request that arrives during it
    // joins this batch rather than opening the next one.
    if let Some(delay) = &inner.delay
        && let Err(cause) = delay().run_sync_with(&services)
    {
        let pending = {
            let mut state = group.state.lock().unwrap();
            state.closed = true;
            std::mem::take(&mut state.pending)
        };
        // An `Infallible` delay can only fail by defect or interrupt.
        answer_unfinished(&pending, &cause.map(|never| match never {}));
        return;
    }
    let entries = {
        let mut state = group.state.lock().unwrap();
        state.closed = true;
        std::mem::take(&mut state.pending)
    };
    {
        let mut groups = inner.groups.lock().unwrap();
        let current = groups
            .get(&key)
            .is_some_and(|current| Arc::ptr_eq(current, &group));
        if current {
            groups.remove(&key);
        }
    }
    if entries.is_empty() {
        return;
    }
    let live: Vec<Entry<R>> = entries
        .into_iter()
        .filter(|entry| !entry.is_cancelled())
        .collect();
    if live.is_empty() {
        return;
    }
    match (inner.run)(live.clone()).run_sync_with(&services) {
        Ok(()) => {
            // The pin's documented gotcha: an entry the resolver never
            // completed fails, rather than hanging its caller forever.
            let unanswered = Cause::die(Defect::new(
                "a request resolver finished without answering one of its entries",
            ));
            answer_unfinished(&live, &unanswered);
        }
        Err(cause) => answer_unfinished(&live, &cause),
    }
}

/// Re-raise a batch's exit after its `after` hook ran. The hook's own failure
/// only stands when the batch had none, which is `acquireUseRelease`'s order.
fn rethrow<E: Send + 'static>(exit: Exit<(), E>) -> Effect<(), E> {
    match exit {
        Exit::Success(()) => Effect::sync(|| ()),
        Exit::Failure(cause) => Effect::fail_cause(cause),
    }
}

fn answer_unfinished<R: Request>(entries: &[Entry<R>], cause: &Cause<R::Error>) {
    for entry in entries {
        if !entry.is_done() {
            entry.complete(Exit::Failure(cause.clone()));
        }
    }
}

/// Run the batch on a fiber of its own. It must outlive the caller that
/// started it: an interrupted caller withdraws its entry, and the rest of the
/// batch still has to run.
#[cfg(feature = "tokio")]
fn spawn_batch<R: Request>(
    inner: Arc<Inner<R>>,
    group: Arc<Group<R>>,
    key: u64,
    services: Services,
) {
    static BATCH: std::sync::OnceLock<crate::Runtime> = std::sync::OnceLock::new();
    let runtime = BATCH.get_or_init(crate::Runtime::new);
    // Immediate: the batch has no handle to join, so a deferred start latch
    // would never be fired and the batch would never run.
    runtime.fork_detach_with(
        move |ctx| {
            let mut maps = (*services).clone();
            maps.replace_tag(
                <crate::fiber::FiberTag as crate::ServiceTag>::ID,
                Arc::new(ctx.fiber()),
            );
            let batch_services: Services = Arc::new(maps);
            run_group(inner, group, key, batch_services);
            Exit::<(), std::convert::Infallible>::succeed(())
        },
        crate::ForkOpts::immediate(),
    );
}

/// Without the fiber runtime there is no fiber to give the batch, so it runs
/// on an OS thread with the caller's services.
#[cfg(not(feature = "tokio"))]
fn spawn_batch<R: Request>(
    inner: Arc<Inner<R>>,
    group: Arc<Group<R>>,
    key: u64,
    services: Services,
) {
    std::thread::spawn(move || run_group(inner, group, key, services));
}

// Cache

struct Cache<R: Request> {
    capacity: usize,
    order: u64,
    rows: HashMap<R, Row<R>>,
}

struct Row<R: Request> {
    entry: Entry<R>,
    exit: Option<Exit<R::Success, R::Error>>,
    order: u64,
}

/// `withCache`'s admission check, the pin's `preCheck`: answer from the cache,
/// attach to an in-flight row, or open a row for this entry and let it batch.
fn admit<R: Request + Hash + Eq>(cache: &Arc<Mutex<Cache<R>>>, entry: &Entry<R>) -> bool {
    let mut state = cache.lock().unwrap();
    match state.rows.get(&entry.request) {
        Some(row) if row.exit.is_some() => {
            let exit = row.exit.clone().expect("checked above");
            state.order += 1;
            let touched = state.order;
            if let Some(row) = state.rows.get_mut(&entry.request) {
                row.order = touched;
            }
            drop(state);
            entry.complete(exit);
            false
        }
        Some(row) => {
            // A second caller for an in-flight request: it waits on the row's
            // entry, and cancelling either caller keeps that entry alive.
            let shared = row.entry.clone();
            shared.mark_shared();
            let waiter = entry.clone();
            shared.register_on_complete(Box::new(move |exit: &Exit<R::Success, R::Error>| {
                waiter.complete((*exit).clone())
            }));
            false
        }
        None => {
            let key = entry.request.clone();
            state.order += 1;
            let order = state.order;
            state.rows.insert(
                key.clone(),
                Row {
                    entry: entry.clone(),
                    exit: None,
                    order,
                },
            );
            drop(state);
            let weak: Weak<EntryState<R>> = Arc::downgrade(&entry.state);
            let cache = Arc::clone(cache);
            entry.register_on_complete(Box::new(move |exit| {
                let Some(ours) = weak.upgrade() else {
                    return;
                };
                let mut state = cache.lock().unwrap();
                let held = state
                    .rows
                    .get(&key)
                    .is_some_and(|row| Arc::ptr_eq(&row.entry.state, &ours));
                if !held {
                    // A later request replaced (or evicted) this row; its
                    // answer belongs to the newer row, not this one.
                    return;
                }
                let interrupted = match exit {
                    Exit::Failure(cause) => cause.has_interrupts(),
                    Exit::Success(_) => false,
                };
                if interrupted {
                    state.rows.remove(&key);
                    return;
                }
                if let Some(row) = state.rows.get_mut(&key) {
                    row.exit = Some(exit.clone());
                }
                while state.rows.len() > state.capacity {
                    let oldest = state
                        .rows
                        .iter()
                        .min_by_key(|(_, row)| row.order)
                        .map(|(key, _)| key.clone());
                    match oldest {
                        Some(key) => {
                            state.rows.remove(&key);
                        }
                        None => break,
                    }
                }
            }));
            true
        }
    }
}

// Tagged resolvers

/// The handler a [`TaggedResolver`] runs for one tag: the requests carrying
/// that tag, in submission order, answered with one value each.
pub type Handler<R> = Box<
    dyn Fn(&[R]) -> Result<Vec<<R as Request>::Success>, Cause<<R as Request>::Error>>
        + Send
        + Sync,
>;

/// `RequestResolver.fromEffectTagged` at the pin: a resolver built from one
/// handler per tag.
///
/// Each handler receives only the requests carrying its own tag, in submission
/// order, and its answer is reassembled into entry order. A missing handler is
/// a defect, not a silent skip: the port's fail-closed answer to a tag nobody
/// claimed.
pub struct TaggedResolver<R: Request> {
    handlers: Vec<(&'static str, Handler<R>)>,
}

impl<R: Request> Default for TaggedResolver<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Request> TaggedResolver<R> {
    /// A resolver with no handlers yet.
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    /// Claim one tag.
    pub fn on<F>(mut self, tag: &'static str, handler: F) -> Self
    where
        F: Fn(&[R]) -> Result<Vec<R::Success>, Cause<R::Error>> + Send + Sync + 'static,
    {
        self.handlers.push((tag, Box::new(handler)));
        self
    }

    /// The batching resolver this handler table describes.
    pub fn into_resolver(self) -> Resolver<R> {
        let handlers = self.handlers;
        Resolver::make(move |entries| {
            let outcome = dispatch(&handlers, &entries);
            Effect::sync(move || match outcome {
                Ok(()) => (),
                Err(cause) => {
                    // Every entry the handler table could not answer shares
                    // the failure; the batch's own completion covers the rest.
                    for entry in &entries {
                        if !entry.is_done() {
                            entry.complete(Exit::Failure(cause.clone()));
                        }
                    }
                }
            })
        })
    }
}

impl<R: Request> From<TaggedResolver<R>> for Resolver<R> {
    fn from(tagged: TaggedResolver<R>) -> Self {
        tagged.into_resolver()
    }
}

fn dispatch<R: Request>(
    handlers: &[(&'static str, Handler<R>)],
    entries: &[Entry<R>],
) -> Result<(), Cause<R::Error>> {
    let mut answers: Vec<Option<R::Success>> = entries.iter().map(|_| None).collect();
    let mut tags: Vec<&'static str> = Vec::new();
    for entry in entries {
        let tag = entry.request.tag();
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    for tag in tags {
        let Some((_, handler)) = handlers.iter().find(|(claimed, _)| *claimed == tag) else {
            return Err(Cause::die(Defect::new(format!(
                "no handler for request tag: {tag}"
            ))));
        };
        let indexes: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.request.tag() == tag)
            .map(|(index, _)| index)
            .collect();
        let batch: Vec<R> = indexes
            .iter()
            .map(|index| entries[*index].request.clone())
            .collect();
        let values = handler(&batch)?;
        if values.len() != indexes.len() {
            return Err(Cause::die(Defect::new(format!(
                "resolver for `{tag}` answered {} requests, was given {}",
                values.len(),
                indexes.len()
            ))));
        }
        for (index, value) in indexes.into_iter().zip(values) {
            answers[index] = Some(value);
        }
    }
    for (entry, answer) in entries.iter().zip(answers) {
        entry.complete(Exit::Success(answer.expect("every entry answered")));
    }
    Ok(())
}

// Submission

/// The step `request` builds: admit one entry, then wait for its own answer.
struct RequestStep<R: Request> {
    resolver: Resolver<R>,
    entry: Entry<R>,
}

impl<R: Request> crate::effect::ErasedStep<R::Success, R::Error> for RequestStep<R> {
    fn run_erased(self: Box<Self>, scope: &Services) -> Result<R::Success, Cause<R::Error>> {
        let Self { resolver, entry } = *self;
        resolver.admit(entry.clone(), Arc::clone(scope));
        entry.wait()
    }
}

/// `Effect.request` at the pin: submit `request` to `resolver` and answer with
/// its result.
pub fn request<R: Request>(resolver: &Resolver<R>, request: R) -> Effect<R::Success, R::Error> {
    Effect {
        step: crate::effect::Step::Suspend(Box::new(RequestStep {
            resolver: resolver.clone(),
            entry: Entry::new(request),
        })),
        services: crate::service::empty_services(),
        marker: PhantomData,
    }
}
