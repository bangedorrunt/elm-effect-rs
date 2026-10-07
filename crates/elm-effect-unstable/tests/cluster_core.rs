//! Cluster core: membership refcounts and the message-storage journal.

#[path = "../src/cluster.rs"]
mod cluster;

use cluster::{EntityAddress, MachineId, Membership, MessageStorage, StoredRequest};

fn addr(t: &str, id: &str) -> EntityAddress {
    EntityAddress::new(t, id)
}

fn req(id: &str, address: EntityAddress) -> StoredRequest {
    StoredRequest {
        id: id.to_string(),
        address,
        claimed_by: None,
        complete: false,
    }
}

#[test]
fn addresses_and_machines_compare_by_value() {
    assert_eq!(addr("Counter", "1"), addr("Counter", "1"));
    assert_ne!(addr("Counter", "1"), addr("Counter", "2"));
    assert_eq!(MachineId("m1".to_string()), MachineId("m1".to_string()));
}

#[test]
fn membership_returns_to_baseline() {
    let m = Membership::new();
    m.acquire("counter/1");
    m.acquire("counter/1");
    m.release("counter/1");
    assert!(!m.is_empty());
    m.release("counter/1");
    assert!(m.is_empty());
}

#[test]
fn around_entity_type_restores_the_key() {
    let m = Membership::new();
    m.around_entity_type("queue/a", || {
        assert!(!m.is_empty());
    });
    assert!(m.is_empty());
}

#[test]
fn save_detects_duplicates() {
    let mut s = MessageStorage::new();
    s.save(req("r1", addr("T", "1"))).unwrap();
    assert!(s.save(req("r1", addr("T", "1"))).is_err());
}

#[test]
fn unprocessed_filters_complete_claimed_and_capped() {
    let mut s = MessageStorage::new();
    s.save(req("r1", addr("T", "1"))).unwrap();
    s.save(req("r2", addr("T", "1"))).unwrap();
    s.save(req("r3", addr("T", "2"))).unwrap();
    s.complete("r2");
    s.claim("r3", "runner-a");

    let all = s
        .unprocessed_messages(&[addr("T", "1"), addr("T", "2")], None)
        .unwrap();
    assert_eq!(
        all.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["r1"]
    );

    let capped = s
        .unprocessed_messages(&[addr("T", "1"), addr("T", "2")], Some(0))
        .unwrap();
    assert!(capped.is_empty());

    assert!(s.unprocessed_messages(&[], None).is_err());
}

#[test]
fn reset_requests_releases_only_selected() {
    let mut s = MessageStorage::new();
    let a = addr("T", "1");
    s.save(req("r1", a.clone())).unwrap();
    s.save(req("r2", a.clone())).unwrap();
    s.claim("r1", "runner-a");
    s.claim("r2", "runner-a");

    s.reset_requests(&[]);
    assert!(
        s.unprocessed_messages(&[a.clone()], None)
            .unwrap()
            .is_empty()
    );

    s.reset_requests(&["r1"]);
    let pending = s.unprocessed_messages(&[a.clone()], None).unwrap();
    assert_eq!(
        pending.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["r1"]
    );
}

#[test]
fn reset_addresses_makes_claimed_eligible() {
    let mut s = MessageStorage::new();
    let a = addr("T", "1");
    s.save(req("r1", a.clone())).unwrap();
    s.claim("r1", "runner-a");
    assert!(s.reset_addresses(&[]).is_err());
    s.reset_addresses(&[a.clone()]).unwrap();
    assert_eq!(s.unprocessed_messages(&[a], None).unwrap().len(), 1);
}

#[test]
fn clear_address_drops_index_and_interrupts() {
    let mut s = MessageStorage::new();
    let a = addr("T", "1");
    s.save(req("r1", a.clone())).unwrap();
    s.interrupt(a.clone(), "rebalance");
    assert!(s.indexed(&a).is_some());

    s.clear_address(&a);
    assert!(s.indexed(&a).is_none());
    assert!(s.interrupts_for(&a).is_empty());
    assert!(s.unprocessed_messages(&[a], None).unwrap().is_empty());
}

#[test]
fn runner_address_roundtrip() {
    let addr = cluster::RunnerAddress::make("runner-1", 8080);
    assert_eq!(addr.render(), "runner-1:8080");
    assert_eq!(cluster::RunnerAddress::parse("runner-1:8080"), Some(addr));
    assert_eq!(cluster::RunnerAddress::parse("no-port"), None);
    assert_eq!(cluster::RunnerAddress::parse(":8080"), None);
    assert_eq!(cluster::RunnerAddress::parse("host:abc"), None);
}

#[test]
fn cluster_errors_carry_tag_and_message() {
    let e = cluster::ClusterError::MailboxFull("box at 100".into());
    assert_eq!(e.tag(), "MailboxFull");
    assert_eq!(e.message(), "box at 100");
    assert_eq!(e.to_string(), "MailboxFull: box at 100");
    assert_eq!(
        cluster::ClusterError::RunnerUnavailable("down".into()).tag(),
        "RunnerUnavailable"
    );
}
