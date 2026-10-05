//! Cluster core, part 1: membership and message storage as data.
//!
//! No transport here: [`Membership`] refcounts entity keys, and
//! [`MessageStorage`] is the address-indexed request/reply journal behind
//! `clearAddress` / `resetRequests` / `resetAddresses` /
//! `unprocessedMessages`. RPC payloads, reply-handler parking, and runners
//! belong to later parts.

use std::collections::{HashMap, HashSet};

// Addresses

/// Names an entity type (`"Counter"`) plus one entity (`"42"`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EntityAddress {
    /// The entity type.
    pub entity_type: String,
    /// The entity id.
    pub entity_id: String,
}

impl EntityAddress {
    /// Build from parts.
    pub fn new(entity_type: impl Into<String>, entity_id: impl Into<String>) -> Self {
        Self {
            entity_type: entity_type.into(),
            entity_id: entity_id.into(),
        }
    }
}

/// One machine in the cluster.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MachineId(pub String);

// Membership

/// Refcounted entity keys: acquiring the same key twice needs two releases
/// before membership returns to baseline. Shared (`&self`) state: a
/// membership registry is read and written through one handle.
#[derive(Debug, Default)]
pub struct Membership {
    counts: std::cell::RefCell<HashMap<String, usize>>,
}

impl Membership {
    /// Empty membership.
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire a key (refcounts overlapping acquires).
    pub fn acquire(&self, key: impl Into<String>) {
        *self.counts.borrow_mut().entry(key.into()).or_default() += 1;
    }

    /// Release a key. Unknown keys are ignored.
    pub fn release(&self, key: &str) {
        let mut counts = self.counts.borrow_mut();
        if let Some(count) = counts.get_mut(key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                counts.remove(key);
            }
        }
    }

    /// Run `f`, holding the key for its duration: the key is restored
    /// afterwards either way.
    pub fn around_entity_type<R>(&self, key: &str, f: impl FnOnce() -> R) -> R {
        self.acquire(key);
        let out = f();
        self.release(key);
        out
    }

    /// Live keys.
    pub fn keys(&self) -> HashSet<String> {
        self.counts.borrow().keys().cloned().collect()
    }

    /// True when nothing is held.
    pub fn is_empty(&self) -> bool {
        self.counts.borrow().is_empty()
    }
}

// Message storage

/// A persisted request: payload plus claim/completion state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRequest {
    /// Request id.
    pub id: String,
    /// Owning address.
    pub address: EntityAddress,
    /// Claimed by a runner, if any.
    pub claimed_by: Option<String>,
    /// Finished processing.
    pub complete: bool,
}

/// The journal: requests by id, a primary-key index by address, queued
/// interrupts by address.
#[derive(Debug, Default)]
pub struct MessageStorage {
    requests: HashMap<String, StoredRequest>,
    by_address: HashMap<EntityAddress, Vec<String>>,
    interrupts: HashMap<EntityAddress, Vec<String>>,
}

/// Empty filters and empty address lists fail closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmptyFilter;

impl MessageStorage {
    /// Empty storage.
    pub fn new() -> Self {
        Self::default()
    }

    /// Save a request. Duplicates (same id) are rejected.
    pub fn save(&mut self, request: StoredRequest) -> Result<(), String> {
        if self.requests.contains_key(&request.id) {
            return Err(format!("duplicate request {}", request.id));
        }
        self.by_address
            .entry(request.address.clone())
            .or_default()
            .push(request.id.clone());
        self.requests.insert(request.id.clone(), request);
        Ok(())
    }

    /// Mark a request complete.
    pub fn complete(&mut self, id: &str) {
        if let Some(request) = self.requests.get_mut(id) {
            request.complete = true;
        }
    }

    /// Queue an interrupt for an address.
    pub fn interrupt(&mut self, address: EntityAddress, reason: impl Into<String>) {
        self.interrupts
            .entry(address)
            .or_default()
            .push(reason.into());
    }

    /// Pending interrupts for an address.
    pub fn interrupts_for(&self, address: &EntityAddress) -> Vec<String> {
        self.interrupts.get(address).cloned().unwrap_or_default()
    }

    /// Unprocessed (unclaimed, incomplete) requests, optionally filtered by
    /// address and capped by limit. An empty address filter fails closed.
    pub fn unprocessed_messages(
        &self,
        addresses: &[EntityAddress],
        limit: Option<usize>,
    ) -> Result<Vec<StoredRequest>, EmptyFilter> {
        if addresses.is_empty() {
            return Err(EmptyFilter);
        }
        let wanted: HashSet<&EntityAddress> = addresses.iter().collect();
        let mut out: Vec<StoredRequest> = self
            .requests
            .values()
            .filter(|r| wanted.contains(&r.address) && r.claimed_by.is_none() && !r.complete)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        if let Some(limit) = limit {
            out.truncate(limit);
        }
        Ok(out)
    }

    /// Release claims for the given request ids. An empty id list leaves
    /// existing claims untouched. Replies and completion survive.
    pub fn reset_requests(&mut self, ids: &[&str]) {
        for id in ids {
            if let Some(request) = self.requests.get_mut(*id) {
                request.claimed_by = None;
            }
        }
    }

    /// Make claimed messages eligible again for these addresses. An empty
    /// address list fails closed.
    pub fn reset_addresses(&mut self, addresses: &[EntityAddress]) -> Result<(), EmptyFilter> {
        if addresses.is_empty() {
            return Err(EmptyFilter);
        }
        for address in addresses {
            if let Some(ids) = self.by_address.get(address) {
                for id in ids.clone() {
                    if let Some(request) = self.requests.get_mut(&id) {
                        request.claimed_by = None;
                    }
                }
            }
        }
        Ok(())
    }

    /// Clear an address: drops its requests, its primary-key index, and its
    /// queued interrupts.
    pub fn clear_address(&mut self, address: &EntityAddress) {
        if let Some(ids) = self.by_address.remove(address) {
            for id in ids {
                self.requests.remove(&id);
            }
        }
        self.interrupts.remove(address);
    }

    /// Claim a request for a runner (test seam).
    pub fn claim(&mut self, id: &str, runner: impl Into<String>) {
        if let Some(request) = self.requests.get_mut(id) {
            request.claimed_by = Some(runner.into());
        }
    }

    /// Live request ids for an address, if the index still holds it.
    pub fn indexed(&self, address: &EntityAddress) -> Option<Vec<String>> {
        self.by_address.get(address).cloned()
    }
}

/// A stored reply: an acknowledgeable chunk or a terminal exit.
///
/// Ports the reply half of `MessageStorage` in
/// `packages/effect/src/cluster/MessageStorage.ts`: `repliesFor` returns
/// un-acknowledged chunks plus exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredReply {
    Chunk { payload: Vec<u8>, acked: bool },
    Exit { payload: Vec<u8> },
}

/// Why a parked reply waiter was released without its reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaiterFault {
    /// Shard moved: the waiter can re-route the wait elsewhere
    /// (`EntityNotAssignedToRunner`).
    NotAssigned,
    /// Runner shutting down: the reply will never arrive here
    /// (cluster-abandon annotation).
    Abandoned,
}

/// Sync port of the `MessageStorage` reply-handler surface: stored
/// replies per request plus parked waiters per shard. Fibers become
/// explicit poll/unregister calls; the fault semantics match
/// `unregisterShardReplyHandlers` (rebalance fails waiters, shutdown
/// with `interrupt` abandons them).
#[derive(Debug, Default)]
pub struct ReplyRegistry {
    replies: HashMap<String, Vec<StoredReply>>,
    parked: HashMap<String, String>,
}

impl ReplyRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a reply for a request.
    pub fn store(&mut self, request_id: impl Into<String>, reply: StoredReply) {
        self.replies
            .entry(request_id.into())
            .or_default()
            .push(reply);
    }

    /// Un-acknowledged chunks plus exits for a request, in store order.
    pub fn replies_for(&self, request_id: &str) -> Vec<StoredReply> {
        self.replies
            .get(request_id)
            .map(|replies| {
                replies
                    .iter()
                    .filter(|reply| match reply {
                        StoredReply::Chunk { acked, .. } => !acked,
                        StoredReply::Exit { .. } => true,
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Mark every chunk for a request acknowledged; true when any was.
    pub fn acknowledge(&mut self, request_id: &str) -> bool {
        let mut acked = false;
        if let Some(replies) = self.replies.get_mut(request_id) {
            for reply in replies {
                if let StoredReply::Chunk { acked: flag, .. } = reply
                    && !*flag
                {
                    *flag = true;
                    acked = true;
                }
            }
        }
        acked
    }

    /// Park a waiter for a request on a shard (locally sent message).
    pub fn register_waiter(&mut self, request_id: impl Into<String>, shard: impl Into<String>) {
        self.parked.insert(request_id.into(), shard.into());
    }

    /// True while the waiter is still parked.
    pub fn is_parked(&self, request_id: &str) -> bool {
        self.parked.contains_key(request_id)
    }

    /// Release a parked waiter whose reply arrived; the request id when
    /// a reply is stored for it, `None` while still waiting.
    pub fn poll_waiter(&mut self, request_id: &str) -> Option<String> {
        if !self.parked.contains_key(request_id) {
            return None;
        }
        if self.replies_for(request_id).is_empty() {
            return None;
        }
        self.parked.remove(request_id);
        Some(request_id.to_string())
    }

    /// Drop a single waiter; true when one was parked.
    pub fn unregister_waiter(&mut self, request_id: &str) -> bool {
        self.parked.remove(request_id).is_some()
    }

    /// Release every waiter parked on a shard. Rebalance fails them with
    /// `NotAssigned`; shutdown (`interrupt`) abandons them instead.
    /// Returns the released request ids with their faults.
    pub fn unregister_shard(&mut self, shard: &str, interrupt: bool) -> Vec<(String, WaiterFault)> {
        let fault = if interrupt {
            WaiterFault::Abandoned
        } else {
            WaiterFault::NotAssigned
        };
        let ids: Vec<String> = self
            .parked
            .iter()
            .filter(|(_, s)| s.as_str() == shard)
            .map(|(id, _)| id.clone())
            .collect();
        ids.into_iter()
            .map(|id| {
                self.parked.remove(&id);
                (id, fault.clone())
            })
            .collect()
    }
}

// Sharding config

/// How long runners and storage reads linger, and where to listen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardingConfig {
    /// Runner residency, in seconds.
    pub runner_residency_secs: u64,
    /// Storage read batch cap.
    pub storage_batch: usize,
    /// Optional listen host: absent means no listen address.
    pub listen_host: Option<String>,
    /// Optional listen port.
    pub listen_port: Option<u16>,
}

/// Config validation failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadConfig(pub String);

impl ShardingConfig {
    /// Defaults: 300s residency, batches of 100.
    pub fn defaults() -> Self {
        Self {
            runner_residency_secs: 300,
            storage_batch: 100,
            listen_host: None,
            listen_port: None,
        }
    }

    /// Validate: residency and batch limits must be positive.
    pub fn validated(self) -> Result<Self, BadConfig> {
        if self.runner_residency_secs == 0 {
            return Err(BadConfig("runner residency must be positive".to_string()));
        }
        if self.storage_batch == 0 {
            return Err(BadConfig("storage batch limit must be positive".to_string()));
        }
        Ok(self)
    }

    /// The listen address, if a host is configured.
    pub fn listen_address(&self) -> Option<String> {
        let host = self.listen_host.as_ref()?;
        Some(match self.listen_port {
            Some(port) => format!("{host}:{port}"),
            None => host.clone(),
        })
    }
}

// Runner storage

/// Health of one runner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunnerHealth {
    /// Serving.
    Healthy,
    /// Draining.
    Draining,
}

/// In-memory runner registry: health plus one-shot acquisition.
#[derive(Debug, Default)]
pub struct MemoryRunnerStorage {
    health: HashMap<String, RunnerHealth>,
}

impl MemoryRunnerStorage {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a runner as healthy.
    pub fn register(&mut self, runner: impl Into<String>) {
        self.health.insert(runner.into(), RunnerHealth::Healthy);
    }

    /// Current health, if known.
    pub fn health(&self, runner: &str) -> Option<RunnerHealth> {
        self.health.get(runner).cloned()
    }

    /// Acquire every runner from a one-shot iterable.
    pub fn acquire_all(&mut self, runners: impl IntoIterator<Item = String>) {
        for runner in runners {
            self.register(runner);
        }
    }
}

// Resource map

/// A lookup cache that closes the scope when a lookup fails: failures never
/// leave half-built entries behind.
#[derive(Debug, Default)]
pub struct ResourceMap<T> {
    entries: HashMap<String, T>,
    /// Scopes closed after failed lookups (test seam).
    pub closed_scopes: Vec<String>,
}

impl<T> ResourceMap<T> {
    /// Empty map.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            closed_scopes: Vec::new(),
        }
    }

    /// Get the cached value, or load it. A failed load closes the lookup
    /// scope and stores nothing.
    pub fn get_or_load<E>(
        &mut self,
        key: &str,
        scope: impl Into<String>,
        load: impl FnOnce() -> Result<T, E>,
    ) -> Result<&T, E> {
        if !self.entries.contains_key(key) {
            match load() {
                Ok(value) => {
                    self.entries.insert(key.to_string(), value);
                }
                Err(e) => {
                    self.closed_scopes.push(scope.into());
                    return Err(e);
                }
            }
        }
        Ok(&self.entries[key])
    }
}

/// Address of a runner: host plus port.
///
/// Ports `packages/effect/src/cluster/RunnerAddress.ts` (`make`):
/// parses and renders `host:port` pairs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunnerAddress {
    /// Hostname or IP.
    pub host: String,
    /// Port number.
    pub port: u16,
}

impl RunnerAddress {
    /// Build from parts.
    pub fn make(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    /// Parse `host:port`; fails on missing or non-numeric ports.
    pub fn parse(text: &str) -> Option<Self> {
        let (host, port) = text.rsplit_once(':')?;
        if host.is_empty() {
            return None;
        }
        Some(Self {
            host: host.to_string(),
            port: port.parse().ok()?,
        })
    }

    /// Render as `host:port`.
    pub fn render(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Cluster failure modes.
///
/// Ports `packages/effect/src/cluster/ClusterError.ts`: one variant per
/// tagged error class, each carrying a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClusterError {
    /// A message arrived while another was still processing.
    AlreadyProcessingMessage(String),
    /// The entity is not assigned to this runner.
    EntityNotAssignedToRunner(String),
    /// The entity mailbox is full.
    MailboxFull(String),
    /// A message failed to deserialize.
    MalformedMessage(String),
    /// The persistence backend failed.
    PersistenceError(String),
    /// No runner is registered for the address.
    RunnerNotRegistered(String),
    /// The runner is unreachable.
    RunnerUnavailable(String),
}

impl ClusterError {
    /// The failure message.
    pub fn message(&self) -> &str {
        match self {
            Self::AlreadyProcessingMessage(m)
            | Self::EntityNotAssignedToRunner(m)
            | Self::MailboxFull(m)
            | Self::MalformedMessage(m)
            | Self::PersistenceError(m)
            | Self::RunnerNotRegistered(m)
            | Self::RunnerUnavailable(m) => m,
        }
    }

    /// The upstream error tag.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::AlreadyProcessingMessage(_) => "AlreadyProcessingMessage",
            Self::EntityNotAssignedToRunner(_) => "EntityNotAssignedToRunner",
            Self::MailboxFull(_) => "MailboxFull",
            Self::MalformedMessage(_) => "MalformedMessage",
            Self::PersistenceError(_) => "PersistenceError",
            Self::RunnerNotRegistered(_) => "RunnerNotRegistered",
            Self::RunnerUnavailable(_) => "RunnerUnavailable",
        }
    }
}

impl core::fmt::Display for ClusterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.tag(), self.message())
    }
}
