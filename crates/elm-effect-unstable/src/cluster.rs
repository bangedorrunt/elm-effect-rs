//! Cluster core, part 1: membership and message storage as data.
//!
//! No transport here: [`Membership`] refcounts entity keys, and
//! [`MessageStorage`] is the address-indexed request/reply journal behind
//! `clearAddress` / `resetRequests` / `resetAddresses` /
//! `unprocessedMessages`. RPC payloads, reply-handler parking, and runners
//! belong to later parts.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
    /// Encode failed: the persisted `MalformedMessage` defect that reply
    /// handlers observe instead of the unencodable payload.
    Defect { reason: String },
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

    /// Store an encoded reply: the encoded bytes on success, the
    /// persisted `MalformedMessage` defect on encode failure.
    ///
    /// One request's failure never touches another's replies, which is
    /// what `a reply that fails to serialize fails only its own request`
    /// asserts upstream.
    pub fn store_encoded(
        &mut self,
        request_id: impl Into<String>,
        encode: impl FnOnce() -> Result<Vec<u8>, String>,
    ) -> StoredReply {
        let reply = match encode() {
            Ok(payload) => StoredReply::Exit { payload },
            Err(reason) => StoredReply::Defect {
                reason: format!("MalformedMessage: {reason}"),
            },
        };
        self.store(request_id, reply.clone());
        reply
    }

    /// Store an encoded chunk, so a stream reply ends on its defect.
    pub fn store_encoded_chunk(
        &mut self,
        request_id: impl Into<String>,
        encode: impl FnOnce() -> Result<Vec<u8>, String>,
    ) -> StoredReply {
        let reply = match encode() {
            Ok(payload) => StoredReply::Chunk { payload, acked: false },
            Err(reason) => StoredReply::Defect {
                reason: format!("MalformedMessage: {reason}"),
            },
        };
        self.store(request_id, reply.clone());
        reply
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
                        StoredReply::Exit { .. } | StoredReply::Defect { .. } => true,
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

    /// Client endpoint for a runner protocol: `scheme://host:port` plus
    /// the layer path, adding a missing leading slash.
    ///
    /// Ports the `HttpRunner`/`SocketRunner` client layers in
    /// `packages/effect/src/cluster/HttpRunner.ts`: the path is appended
    /// verbatim once slash-led (`/runner` stays `/runner`). The HTTP RPC
    /// protocol adds its own trailing call slash after this.
    pub fn endpoint(&self, scheme: &str, path: &str) -> String {
        let host = if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        format!("{scheme}://{host}:{}{path}", self.port)
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

/// One entity's activation scope: finalizers run when the scope closes.
///
/// Ports the per-entity `Scope` that `Entity.makeTestClient` provides to
/// an entity layer, in its sync form: a scope per activation, closed with
/// the client.
pub struct EntityScope {
    entity_id: String,
    closed: AtomicBool,
    finalizers: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
}

impl EntityScope {
    /// A fresh open scope for one entity.
    pub fn new(entity_id: impl Into<String>) -> Self {
        Self {
            entity_id: entity_id.into(),
            closed: AtomicBool::new(false),
            finalizers: Mutex::new(Vec::new()),
        }
    }

    /// The entity this scope belongs to.
    pub fn entity_id(&self) -> &str {
        &self.entity_id
    }

    /// Register a finalizer; ignored once closed.
    pub fn add_finalizer(&self, f: impl FnOnce() + Send + 'static) {
        let mut finalizers = self.finalizers.lock().unwrap();
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        finalizers.push(Box::new(f));
    }

    /// True once [`Self::close`] has run.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Close the scope, running finalizers newest first. Idempotent.
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        loop {
            let next = self.finalizers.lock().unwrap().pop();
            match next {
                Some(run) => run(),
                None => break,
            }
        }
    }
}

impl Drop for EntityScope {
    fn drop(&mut self) {
        self.close();
    }
}

/// The client side of [`EntityScope`]: one scope per activated entity,
/// all closed together with the client.
///
/// Ports `Entity.makeTestClient`, which hands each activation its own
/// scope and closes them with the client.
pub struct EntityClients {
    scopes: Mutex<Vec<Arc<EntityScope>>>,
}

impl Default for EntityClients {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityClients {
    /// A client with no activations yet.
    pub fn new() -> Self {
        Self {
            scopes: Mutex::new(Vec::new()),
        }
    }

    /// Activate an entity, returning its own scope.
    pub fn activate(&self, entity_id: impl Into<String>) -> Arc<EntityScope> {
        let scope = Arc::new(EntityScope::new(entity_id));
        self.scopes.lock().unwrap().push(Arc::clone(&scope));
        Arc::clone(&scope)
    }

    /// Live activation scopes, oldest first.
    pub fn activations(&self) -> Vec<Arc<EntityScope>> {
        self.scopes.lock().unwrap().clone()
    }

    /// Close every scope, newest first.
    pub fn close(&self) {
        loop {
            let next = self.scopes.lock().unwrap().pop();
            match next {
                Some(scope) => scope.close(),
                None => break,
            }
        }
    }
}

impl Drop for EntityClients {
    fn drop(&mut self) {
        self.close();
    }
}

// ------------------------------------------------------------- Entity.ts ---

/// What one request's handler did (the pin's exit shapes).
#[derive(Clone, Debug, PartialEq)]
pub enum HandlerOutcome {
    /// Succeeded with a value.
    Success(i64),
    /// Failed with a typed error: request-local by definition.
    TypedFailure(String),
    /// Died with a defect: fatal unless the entity disables fatal defects.
    Defect(String),
}

/// What the client observed for one request.
#[derive(Clone, Debug, PartialEq)]
pub enum ReplyOutcome {
    /// Succeeded with a value.
    Success(i64),
    /// The typed failure the handler reported.
    TypedFailure(String),
    /// The defect that ended the request, which is the entity's original
    /// defect once it is poisoned.
    Defect(String),
}

/// One entity's server: its own services plus the fatal-defect state the
/// pin keeps per entity address.
///
/// `disable_fatal_defects` mirrors the pin's option: `Some(true)` isolates
/// defects per request, while `Some(false)` and `None` let a defect poison
/// the entity, so every later request answers with that same original
/// defect.
pub struct EntityServer {
    entity_id: String,
    disable_fatal_defects: bool,
    services: HashMap<String, String>,
    fatal: Option<String>,
}

impl EntityServer {
    /// A server for one entity, with the pin's `disableFatalDefects`.
    pub fn new(entity_id: impl Into<String>, disable_fatal_defects: Option<bool>) -> Self {
        Self {
            entity_id: entity_id.into(),
            disable_fatal_defects: disable_fatal_defects.unwrap_or(false),
            services: HashMap::new(),
            fatal: None,
        }
    }

    /// This entity's id.
    pub fn entity_id(&self) -> &str {
        &self.entity_id
    }

    /// Provide a service to the server itself. Only services provided here
    /// are visible to handlers; a client's acquisition-time context never
    /// travels, which is the `makeTestClient` context-bleed regression.
    pub fn provide(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.services.insert(key.into(), value.into());
    }

    /// The server's own view of a service.
    pub fn service(&self, key: &str) -> Option<&str> {
        self.services.get(key).map(String::as_str)
    }

    /// The defect that poisoned this entity, if any.
    pub fn fatal_defect(&self) -> Option<&str> {
        self.fatal.as_deref()
    }

    /// Run one request.
    pub fn run(&mut self, outcome: HandlerOutcome) -> ReplyOutcome {
        if let Some(fatal) = &self.fatal {
            // A poisoned entity answers every later request with the
            // original defect, whatever its own handler would have done.
            return ReplyOutcome::Defect(fatal.clone());
        }
        match outcome {
            HandlerOutcome::Success(value) => ReplyOutcome::Success(value),
            HandlerOutcome::TypedFailure(error) => ReplyOutcome::TypedFailure(error),
            HandlerOutcome::Defect(defect) => {
                if !self.disable_fatal_defects {
                    self.fatal = Some(defect.clone());
                }
                ReplyOutcome::Defect(defect)
            }
        }
    }
}

/// The client side of `Entity.makeTestClient`: a context captured at
/// acquisition that the entity server never observes.
#[derive(Clone, Debug)]
pub struct EntityClient {
    entity_id: String,
    services: HashMap<String, String>,
}

impl EntityClient {
    /// Acquire the client for one entity.
    pub fn acquire(entity_id: impl Into<String>) -> Self {
        Self {
            entity_id: entity_id.into(),
            services: HashMap::new(),
        }
    }

    /// Add a service to the acquiring fiber's context.
    pub fn with_service(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.services.insert(key.into(), value.into());
        self
    }

    /// The client's own view of a service.
    pub fn service(&self, key: &str) -> Option<&str> {
        self.services.get(key).map(String::as_str)
    }

    /// The entity this client talks to.
    pub fn entity_id(&self) -> &str {
        &self.entity_id
    }
}

/// The dequeue half of [`ReplyRegistry::succeed_queue`]: values offered
/// until `end`, which is also the stream's end.
#[derive(Clone, Debug, Default)]
pub struct ReplyQueue {
    offered: Vec<i64>,
    ended: bool,
}

impl ReplyQueue {
    /// An open, empty dequeue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer one value.
    pub fn offer(&mut self, value: i64) {
        self.offered.push(value);
    }

    /// End the dequeue: the stream completes after what was offered.
    pub fn end(&mut self) {
        self.ended = true;
    }

    /// `true` once [`Self::end`] ran.
    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// Values offered so far.
    pub fn offered(&self) -> &[i64] {
        &self.offered
    }
}

impl ReplyRegistry {
    /// Answer a streaming RPC with a finite stream (`replier.succeed(req,
    /// Stream.make(...))`): one chunk per element, then the exit.
    pub fn succeed_stream(&mut self, request_id: &str, values: &[i64]) {
        for value in values {
            self.store(
                request_id,
                StoredReply::Chunk {
                    payload: value.to_le_bytes().to_vec(),
                    acked: false,
                },
            );
        }
        self.store(
            request_id,
            StoredReply::Exit {
                payload: Vec::new(),
            },
        );
    }

    /// Answer a streaming RPC with a dequeue: the values offered so far,
    /// and the exit only once the dequeue ended. An un-ended dequeue stays
    /// open, which is what the pin's client keeps waiting on.
    pub fn succeed_queue(&mut self, request_id: &str, queue: &ReplyQueue) {
        for value in queue.offered() {
            self.store(
                request_id,
                StoredReply::Chunk {
                    payload: value.to_le_bytes().to_vec(),
                    acked: false,
                },
            );
        }
        if queue.is_ended() {
            self.store(
                request_id,
                StoredReply::Exit {
                    payload: Vec::new(),
                },
            );
        }
    }
}
