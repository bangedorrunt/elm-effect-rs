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

#[test]
fn checked_constructors_retain_rejected_address() {
    use net::{IpNetwork, NetworkErrorInput};
    let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
    for bad in [
        IpNetwork::make(loopback, 33.0),
        IpNetwork::make(loopback, 8.0),
    ] {
        let err = bad.unwrap_err();
        assert_eq!(err.input, NetworkErrorInput::Addr(loopback));
    }
}

#[test]
fn parse_errors_echo_offending_input() {
    use net::{IpNetwork, NetworkErrorInput};
    let err = IpNetwork::parse_reported("1.2.3.4/+24").unwrap_err();
    assert_eq!(
        err.input,
        NetworkErrorInput::Text("1.2.3.4/+24".to_string())
    );
    let err = IpNetwork::parse_reported("010.2.3.4/24").unwrap_err();
    assert_eq!(
        err.input,
        NetworkErrorInput::Text("010.2.3.4".to_string())
    );
    let err = IpNetwork::parse_reported("1.2.3.4/33").unwrap_err();
    assert_eq!(
        err.input,
        NetworkErrorInput::Addr("1.2.3.4".parse().unwrap())
    );
    let err = IpNetwork::parse_reported("::ffff:192.000.2.1/128").unwrap_err();
    assert_eq!(
        err.input,
        NetworkErrorInput::Text("192.000.2.1".to_string())
    );
}

#[test]
fn checked_constructors_reject_bad_prefixes_and_host_bits() {
    use net::{IpNetwork, NetworkErrorInput};
    let v4: std::net::IpAddr = "10.1.2.3".parse().unwrap();
    let v6: std::net::IpAddr = "2001:db8::1".parse().unwrap();
    for prefix in [1.5, f64::NAN, f64::INFINITY, 33.0] {
        assert!(IpNetwork::make(v4, prefix).is_err(), "prefix {prefix}");
    }
    assert!(IpNetwork::make(v6, 129.0).is_err());
    let err = IpNetwork::make(v4, -1.0).unwrap_err();
    assert_eq!(err.tag, "NetAddressError");
    assert_eq!(
        err.message,
        "prefix length must be an integer from 0 through 32"
    );
    let err = IpNetwork::parse_reported("10.1.2.3/8").unwrap_err();
    assert_eq!(err.input, NetworkErrorInput::Addr(v4));
    assert!(IpNetwork::make("10.0.0.0".parse().unwrap(), 8.0).is_ok());
}
