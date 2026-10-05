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
