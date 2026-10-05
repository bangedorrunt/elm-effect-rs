//! Cluster rest: sharding config, runner storage, resource map.

#[path = "../src/cluster.rs"]
#[allow(dead_code)]
mod cluster;

use cluster::{MemoryRunnerStorage, ResourceMap, RunnerHealth, ShardingConfig};

#[test]
fn sharding_config_defaults_and_validation() {
    let defaults = ShardingConfig::defaults().validated().unwrap();
    assert_eq!(defaults.runner_residency_secs, 300);
    assert_eq!(defaults.storage_batch, 100);
    assert_eq!(defaults.listen_address(), None);

    let bad_residency = ShardingConfig {
        runner_residency_secs: 0,
        ..ShardingConfig::defaults()
    };
    assert!(bad_residency.validated().is_err());

    let bad_batch = ShardingConfig {
        storage_batch: 0,
        ..ShardingConfig::defaults()
    };
    assert!(bad_batch.validated().is_err());
}

#[test]
fn sharding_config_listen_address_needs_host() {
    let no_host = ShardingConfig {
        listen_host: None,
        listen_port: Some(8080),
        ..ShardingConfig::defaults()
    };
    assert_eq!(no_host.listen_address(), None);

    let with_host = ShardingConfig {
        listen_host: Some("0.0.0.0".to_string()),
        listen_port: Some(8080),
        ..ShardingConfig::defaults()
    };
    assert_eq!(with_host.listen_address(), Some("0.0.0.0:8080".to_string()));
}

#[test]
fn runner_storage_tracks_health_in_memory() {
    let mut s = MemoryRunnerStorage::new();
    assert_eq!(s.health("r1"), None);
    s.register("r1");
    assert_eq!(s.health("r1"), Some(RunnerHealth::Healthy));
}

#[test]
fn runner_storage_acquires_one_shot_iterables() {
    let mut s = MemoryRunnerStorage::new();
    s.acquire_all(vec!["a".to_string(), "b".to_string()].into_iter());
    assert_eq!(s.health("a"), Some(RunnerHealth::Healthy));
    assert_eq!(s.health("b"), Some(RunnerHealth::Healthy));
}

#[test]
fn resource_map_closes_failed_lookup_scopes() {
    let mut m: ResourceMap<String> = ResourceMap::new();
    let out: Result<&String, &str> = m.get_or_load("k", "scope-1", || Err("boom"));
    assert!(out.is_err());
    assert_eq!(m.closed_scopes, vec!["scope-1".to_string()]);

    let ok: Result<&String, &str> = m.get_or_load("k", "scope-2", || Ok("v".to_string()));
    assert_eq!(ok.unwrap(), "v");
    assert_eq!(m.closed_scopes.len(), 1);
}
