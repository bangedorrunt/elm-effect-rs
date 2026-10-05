//! IpNetwork numeric identity and overlap across text forms.

#[path = "../src/net.rs"]
mod net;

use net::IpNetwork;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn network(text: &str) -> IpNetwork {
    IpNetwork::parse(text).expect("valid CIDR in test")
}

fn hash_of(network: &IpNetwork) -> u64 {
    let mut hasher = DefaultHasher::new();
    network.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn networks_equal_across_text_forms_with_matching_hashes() {
    let first = network("2001:0DB8:0:0:0:0:0:0/32");
    let second = network("2001:db8::/32");
    assert_eq!(first, second);
    assert_eq!(hash_of(&first), hash_of(&second));
    assert_ne!(first, network("2001:db8::/33"));
}

#[test]
fn networks_contain_and_overlap() {
    let parent = network("2001:db8::/63");
    let child = network("2001:db8:0:1:8000::/65");
    let sibling = network("2001:db8:0:1::/65");
    let disjoint = network("2001:db8:0:2::/63");
    assert!(parent.contains_network(&parent));
    assert!(parent.contains_network(&child));
    assert!(!child.contains_network(&parent));
    assert!(parent.overlaps(&child));
    assert!(child.overlaps(&parent));
    assert!(!child.overlaps(&sibling));
    assert!(!parent.overlaps(&disjoint));
    assert!(!parent.overlaps(&network("0.0.0.0/0")));
}
