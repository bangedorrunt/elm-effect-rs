//! STM-lite: serializable transactions over one global lock.
//!
//! Every operation runs inside [`atomically`]: a global mutex serializes
//! transactions, so multi-cell blocks are atomic and isolated. Per-cell
//! mutexes nest inside (global first, always), so no lock order inverts.
//! This is coarse but correct; optimistic concurrency, blocking `retry`
//! and cross-thread fairness are residuals, not silent gaps.

use std::cell::Cell;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::sync::{Mutex, MutexGuard};

static GLOBAL: Mutex<()> = Mutex::new(());

thread_local! {
    static IN_TX: Cell<bool> = const { Cell::new(false) };
}

/// Run `body` as one atomic, isolated transaction.
pub fn atomically<R>(body: impl FnOnce() -> R) -> R {
    if IN_TX.with(|flag| flag.get()) {
        return body();
    }
    let _guard: MutexGuard<'_, ()> = GLOBAL.lock().unwrap();
    IN_TX.with(|flag| flag.set(true));
    let result = body();
    IN_TX.with(|flag| flag.set(false));
    result
}

fn tx_cell<T>(cell: &Mutex<T>, f: impl FnOnce(&mut T)) {
    atomically(|| f(&mut cell.lock().unwrap()));
}

fn tx_read<T: Clone>(cell: &Mutex<T>) -> T {
    atomically(|| cell.lock().unwrap().clone())
}

/// A transactional cell: every op is atomic; [`atomically`] composes.
pub struct TxRef<T> {
    cell: Mutex<T>,
}

impl<T> TxRef<T> {
    /// A cell holding `initial`.
    pub fn new(initial: T) -> Self {
        Self {
            cell: Mutex::new(initial),
        }
    }

    /// The current value.
    pub fn read(&self) -> T
    where
        T: Clone,
    {
        tx_read(&self.cell)
    }

    /// Replace the value.
    pub fn write(&self, value: T) {
        tx_cell(&self.cell, |slot| *slot = value);
    }

    /// Run `f` on the value, returning its result.
    pub fn modify<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        atomically(|| f(&mut self.cell.lock().unwrap()))
    }
}

/// A transactional one-shot gate: first completion wins.
pub struct TxDeferred<T> {
    cell: Mutex<Option<T>>,
}

impl<T> TxDeferred<T> {
    /// An empty gate.
    pub fn new() -> Self {
        Self {
            cell: Mutex::new(None),
        }
    }

    /// Complete once; `false` when already completed.
    pub fn complete(&self, value: T) -> bool {
        atomically(|| match *self.cell.lock().unwrap() {
            Some(_) => false,
            ref mut slot => {
                *slot = Some(value);
                true
            }
        })
    }

    /// The value without blocking (`None` until completed).
    pub fn poll(&self) -> Option<T>
    where
        T: Clone,
    {
        tx_read(&self.cell)
    }
}

impl<T> Default for TxDeferred<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// What a full bounded queue does on `offer`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum QueueStrategy {
    /// Reject the value (`false`).
    BackPressure,
    /// Keep the queue, drop the value (`true`).
    Dropping,
    /// Evict the oldest, keep the value (`true`).
    Sliding,
}

/// A transactional FIFO queue, optionally bounded.
pub struct TxQueue<T> {
    inner: Mutex<VecDeque<T>>,
    capacity: Option<usize>,
    strategy: QueueStrategy,
    closed: Mutex<bool>,
}

impl<T> TxQueue<T> {
    /// A queue holding at most `capacity` items; `offer` fails when full.
    pub fn bounded(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: Some(capacity),
            strategy: QueueStrategy::BackPressure,
            closed: Mutex::new(false),
        }
    }

    /// A queue that never fills.
    pub fn unbounded() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: None,
            strategy: QueueStrategy::BackPressure,
            closed: Mutex::new(false),
        }
    }

    /// A bounded queue that drops new values when full (`offer` stays `true`).
    pub fn dropping(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: Some(capacity),
            strategy: QueueStrategy::Dropping,
            closed: Mutex::new(false),
        }
    }

    /// A bounded queue that evicts the oldest value when full.
    pub fn sliding(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity: Some(capacity),
            strategy: QueueStrategy::Sliding,
            closed: Mutex::new(false),
        }
    }

    /// Push back; `false` when a bounded queue is full or closed.
    pub fn offer(&self, value: T) -> bool {
        if *self.closed.lock().unwrap() {
            return false;
        }
        atomically(|| {
            let mut queue = self.inner.lock().unwrap();
            match (self.capacity, self.strategy) {
                (Some(limit), QueueStrategy::BackPressure) if queue.len() >= limit => false,
                (Some(limit), QueueStrategy::Dropping) if queue.len() >= limit => true,
                (Some(limit), QueueStrategy::Sliding) if queue.len() >= limit => {
                    queue.pop_front();
                    queue.push_back(value);
                    true
                }
                _ => {
                    queue.push_back(value);
                    true
                }
            }
        })
    }

    /// Pop front without blocking (`None` when empty).
    pub fn take(&self) -> Option<T> {
        atomically(|| self.inner.lock().unwrap().pop_front())
    }

    /// The current depth.
    pub fn size(&self) -> usize {
        atomically(|| self.inner.lock().unwrap().len())
    }

    /// True when no items wait.
    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    /// Pop front without blocking (`None` when empty): the `poll` shape.
    pub fn poll(&self) -> Option<T> {
        self.take()
    }

    /// Stop accepting offers; queued items still drain, then `take`/`poll`
    /// settle on `None`.
    pub fn close(&self) {
        *self.closed.lock().unwrap() = true;
    }
}

/// A transactional counting semaphore (non-blocking end only).
pub struct TxSemaphore {
    permits: Mutex<usize>,
}

impl TxSemaphore {
    /// A semaphore with `permits` available.
    pub fn new(permits: usize) -> Self {
        Self {
            permits: Mutex::new(permits),
        }
    }

    /// Take one permit without blocking (`false` when empty).
    pub fn try_acquire(&self) -> bool {
        atomically(|| {
            let mut permits = self.permits.lock().unwrap();
            if *permits == 0 {
                false
            } else {
                *permits -= 1;
                true
            }
        })
    }

    /// Return one permit.
    pub fn release(&self) {
        tx_cell(&self.permits, |permits| *permits += 1);
    }

    /// Permits currently available.
    pub fn available(&self) -> usize {
        atomically(|| *self.permits.lock().unwrap())
    }
}

/// A transactional reentrant mutex: the holding thread relocks freely.
pub struct TxReentrantLock {
    inner: Mutex<ReentrantState>,
}

struct ReentrantState {
    owner: Option<std::thread::ThreadId>,
    count: usize,
}

impl TxReentrantLock {
    /// An unlocked lock.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ReentrantState {
                owner: None,
                count: 0,
            }),
        }
    }

    /// Lock (reentrant for the holder; other threads wait cooperatively).
    pub fn lock(&self) {
        let me = std::thread::current().id();
        loop {
            let claimed = atomically(|| {
                let mut state = self.inner.lock().unwrap();
                if state.owner == Some(me) || state.owner.is_none() {
                    state.owner = Some(me);
                    state.count += 1;
                    true
                } else {
                    false
                }
            });
            if claimed {
                return;
            }
            // The global lock is free here: holders can always complete.
            std::thread::yield_now();
        }
    }

    /// Unlock one level.
    pub fn unlock(&self) {
        tx_cell(&self.inner, |state| {
            state.count -= 1;
            if state.count == 0 {
                state.owner = None;
            }
        });
    }

    /// The current hold count.
    pub fn holds(&self) -> usize {
        atomically(|| self.inner.lock().unwrap().count)
    }
}

impl Default for TxReentrantLock {
    fn default() -> Self {
        Self::new()
    }
}

/// A transactional growable chunk (push/get/concat).
pub struct TxChunk<T> {
    inner: Mutex<Vec<T>>,
}

impl<T> TxChunk<T> {
    /// An empty chunk.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }

    /// Append one element.
    pub fn push(&self, value: T) {
        tx_cell(&self.inner, |items| items.push(value));
    }

    /// Append every element of `values`.
    pub fn extend(&self, values: Vec<T>) {
        tx_cell(&self.inner, |items| items.extend(values));
    }

    /// The element at `index`, if any.
    pub fn get(&self, index: usize) -> Option<T>
    where
        T: Clone,
    {
        atomically(|| self.inner.lock().unwrap().get(index).cloned())
    }

    /// The element count.
    pub fn len(&self) -> usize {
        atomically(|| self.inner.lock().unwrap().len())
    }

    /// Whether no elements are stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T> FromIterator<T> for TxChunk<T> {
    /// Build from values (`fromIterable`).
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let chunk = Self::new();
        chunk.extend(iter.into_iter().collect());
        chunk
    }
}

impl<T> TxChunk<T> {
    /// Replace the whole contents (`set`).
    pub fn set_all(&self, values: Vec<T>) {
        atomically(|| {
            *self.inner.lock().unwrap() = values;
        });
    }

    /// Insert at the front (`prepend`).
    pub fn prepend(&self, value: T) {
        atomically(|| {
            self.inner.lock().unwrap().insert(0, value);
        });
    }
}

impl<T> Default for TxChunk<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A transactional hash map.
pub struct TxHashMap<K, V> {
    inner: Mutex<HashMap<K, V>>,
}

impl<K: Eq + Hash, V> FromIterator<(K, V)> for TxHashMap<K, V> {
    /// Build from pairs; later pairs win (`fromIterable`).
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let map = Self::new();
        for (key, value) in iter {
            map.insert(key, value);
        }
        map
    }
}

impl<K: Eq + Hash, V> TxHashMap<K, V> {
    /// An empty map.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// True when `key` maps to anything (`has`).
    pub fn contains(&self, key: &K) -> bool {
        atomically(|| self.inner.lock().unwrap().contains_key(key))
    }

    /// Insert, returning the displaced value if any.
    pub fn insert(&self, key: K, value: V) -> Option<V> {
        atomically(|| self.inner.lock().unwrap().insert(key, value))
    }

    /// The value for `key`, if any.
    pub fn get(&self, key: &K) -> Option<V>
    where
        V: Clone,
    {
        atomically(|| self.inner.lock().unwrap().get(key).cloned())
    }

    /// Remove `key`, returning its value if any.
    pub fn remove(&self, key: &K) -> Option<V> {
        atomically(|| self.inner.lock().unwrap().remove(key))
    }

    /// The entry count.
    pub fn len(&self) -> usize {
        atomically(|| self.inner.lock().unwrap().len())
    }

    /// Whether no entries are stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<K: Eq + Hash, V> Default for TxHashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

/// A transactional hash set.
pub struct TxHashSet<T> {
    inner: Mutex<HashSet<T>>,
}

impl<T: Eq + Hash> FromIterator<T> for TxHashSet<T> {
    /// Build from values; duplicates collapse (`make`/`fromIterable`).
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let set = Self::new();
        for value in iter {
            set.insert(value);
        }
        set
    }
}

impl<T: Eq + Hash> TxHashSet<T> {
    /// An empty set.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashSet::new()),
        }
    }

    /// Insert; `false` when already present.
    pub fn insert(&self, value: T) -> bool {
        atomically(|| self.inner.lock().unwrap().insert(value))
    }

    /// Whether `value` is present.
    pub fn contains(&self, value: &T) -> bool {
        atomically(|| self.inner.lock().unwrap().contains(value))
    }

    /// The element count.
    pub fn len(&self) -> usize {
        atomically(|| self.inner.lock().unwrap().len())
    }

    /// Whether no elements are stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Eq + Hash> Default for TxHashSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A transactional max-heap priority queue.
pub struct TxPriorityQueue<T> {
    inner: Mutex<BinaryHeap<T>>,
}

impl<T: Ord> TxPriorityQueue<T> {
    /// An empty queue.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(BinaryHeap::new()),
        }
    }

    /// Push one element.
    pub fn push(&self, value: T) {
        tx_cell(&self.inner, |heap| heap.push(value));
    }

    /// Pop the maximum, if any.
    pub fn pop(&self) -> Option<T> {
        atomically(|| self.inner.lock().unwrap().pop())
    }

    /// The element count.
    pub fn len(&self) -> usize {
        atomically(|| self.inner.lock().unwrap().len())
    }

    /// Whether no elements are stored.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Ord> Default for TxPriorityQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// A transactional broadcast bus: every published value is retained for
/// draining subscribers. Bounded: publishing past capacity drops oldest.
pub struct TxPubSub<T> {
    inner: Mutex<VecDeque<T>>,
    capacity: usize,
}

impl<T> TxPubSub<T> {
    /// A bus retaining at most `capacity` values.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            capacity,
        }
    }

    /// Publish one value, dropping oldest past capacity.
    pub fn publish(&self, value: T) {
        tx_cell(&self.inner, |buffer| {
            if buffer.len() >= self.capacity {
                buffer.pop_front();
            }
            buffer.push_back(value);
        });
    }

    /// Take every retained value in order.
    pub fn drain(&self) -> Vec<T> {
        atomically(|| self.inner.lock().unwrap().drain(..).collect())
    }
}

/// A transactional cell remembering every assigned value: subscribers
/// replay the full history including the initial value.
pub struct TxSubscriptionRef<T> {
    current: Mutex<T>,
    history: Mutex<Vec<T>>,
}

impl<T: Clone> TxSubscriptionRef<T> {
    /// A cell holding `initial` with one history entry.
    pub fn new(initial: T) -> Self {
        Self {
            current: Mutex::new(initial.clone()),
            history: Mutex::new(vec![initial]),
        }
    }

    /// The current value.
    pub fn get(&self) -> T {
        tx_read(&self.current)
    }

    /// Assign and record.
    pub fn set(&self, value: T) {
        atomically(|| {
            *self.current.lock().unwrap() = value.clone();
            self.history.lock().unwrap().push(value);
        });
    }

    /// Every assigned value, oldest first.
    pub fn changes(&self) -> Vec<T> {
        atomically(|| self.history.lock().unwrap().clone())
    }
}
