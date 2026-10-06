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

#[test]
fn unsafe_string_forms_serialize_canonically() {
    for text in ["192.0.2.0/24", "2001:db8::/32"] {
        let net = IpNetwork::from_string_unsafe(text);
        assert_eq!(net.render(), text);
    }
}

#[test]
fn unsafe_wrappers_mask_or_throw() {
    use net::IpNetwork;
    let masked = IpNetwork::from_address_unsafe("10.1.2.3".parse().unwrap(), 8.0);
    assert_eq!(masked.render(), "10.0.0.0/8");
    let valid = IpNetwork::from_string_unsafe("10.0.0.0/8");
    assert_eq!(valid.render(), "10.0.0.0/8");
    assert!(std::panic::catch_unwind(|| {
        IpNetwork::make_unsafe("10.1.2.3".parse().unwrap(), 8.0)
    })
    .is_err());
    assert!(std::panic::catch_unwind(|| {
        IpNetwork::from_string_unsafe("10.1.2.3/8")
    })
    .is_err());
    assert!(std::panic::catch_unwind(|| {
        IpNetwork::from_address_unsafe("10.1.2.3".parse().unwrap(), 33.0)
    })
    .is_err());
}

#[test]
fn from_address_truncates_host_bits() {
    use net::IpNetwork;
    for (addr, prefix, expected) in [
        ("10.1.2.3", 8.0, "10.0.0.0/8"),
        ("192.0.2.255", 25.0, "192.0.2.128/25"),
        ("2001:db8:0:1:ffff::1", 63.0, "2001:db8::/63"),
        ("2001:db8:0:1:ffff::1", 65.0, "2001:db8:0:1:8000::/65"),
    ] {
        let net = IpNetwork::from_address(addr.parse().unwrap(), prefix).unwrap();
        assert_eq!(net.render(), expected, "{addr}/{prefix}");
    }
}

#[test]
fn interfaces_share_one_tagged_representation() {
    use net::IpInterface;
    let v4 = IpInterface::make_unsafe("0.0.0.0".parse().unwrap(), 0.0);
    let v6 = IpInterface::make_unsafe("::".parse().unwrap(), 0.0);
    assert_eq!(IpInterface::TAG, "IpInterface");
    assert!(v4.is_ipv4());
    assert!(v6.is_ipv6());
    assert!(!v4.is_ipv6());
    assert!(!v6.is_ipv4());
    assert!(v4.is_interface());
    assert!(v6.is_interface());
}

fn as_address(addr: std::net::IpAddr) -> net::NetAddress {
    match addr {
        std::net::IpAddr::V4(v4) => net::NetAddress::V4(v4),
        std::net::IpAddr::V6(v6) => net::NetAddress::V6(v6),
    }
}

#[test]
fn derived_addresses_revalidate_refinements() {
    use net::{IpInterface, IpNetwork, NetAddress};
    let multicast = NetAddress::ipv4_from_bytes(&[224, 0, 0, 1]);
    assert!(multicast.is_multicast());
    let addr = match multicast {
        NetAddress::V4(v4) => std::net::IpAddr::V4(v4),
        NetAddress::V6(v6) => std::net::IpAddr::V6(v6),
        // This test builds an IPv4 multicast value; no other arm runs.
        _ => unreachable!("multicast fixture is IPv4"),
    };
    for derived in [
        IpNetwork::from_address_unsafe(addr, 0.0),
        IpNetwork::from_interface(&IpInterface::make_unsafe(addr, 0.0)),
    ] {
        assert_eq!(derived.network_address().to_string(), "0.0.0.0");
        assert!(!as_address(derived.network_address()).is_multicast());
        let last = derived.last();
        assert_eq!(last.to_string(), "255.255.255.255");
        assert!(!as_address(last).is_multicast());
    }
}

fn ip(text: &str) -> net::NetAddress {
    net::NetAddress::ipv6_from_string(text)
        .or_else(|_| net::NetAddress::ipv4_from_string(text))
        .expect("valid IP in test")
}

#[test]
fn net_addresses_classify_boundaries() {
    for text in [
        "1.2.3.4",
        "127.0.0.1",
        "169.254.1.1",
        "240.0.0.1",
        "::1",
        "fe80::1",
    ] {
        assert!(ip(text).is_unicast(), "{text}");
    }
    for text in ["0.0.0.0", "224.0.0.0", "255.255.255.255", "::", "ff00::"] {
        assert!(!ip(text).is_unicast(), "{text}");
    }
    for text in ["127.0.0.0", "127.255.255.255", "::1"] {
        assert!(ip(text).is_loopback(), "{text}");
    }
    for text in ["128.0.0.0", "::2", "::ffff:127.0.0.1"] {
        assert!(!ip(text).is_loopback(), "{text}");
    }
    assert!(!ip("::ffff:224.0.0.1").is_multicast());
    for text in ["169.254.0.0", "169.254.255.255", "fe80::", "febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff"]
    {
        assert!(ip(text).is_link_local(), "{text}");
    }
    for text in ["169.255.0.0", "fec0::"] {
        assert!(!ip(text).is_link_local(), "{text}");
    }
    for text in [
        "10.0.0.0",
        "10.255.255.255",
        "172.16.0.0",
        "172.31.255.255",
        "192.168.0.0",
    ] {
        assert!(ip(text).is_private(), "{text}");
    }
    for text in [
        "9.255.255.255",
        "172.15.255.255",
        "172.32.0.0",
        "192.169.0.0",
        "100.64.0.1",
    ] {
        assert!(!ip(text).is_private(), "{text}");
    }
    assert!(ip("fc00::").is_unique_local());
    assert!(ip("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff").is_unique_local());
    assert!(!ip("fe00::").is_unique_local());
}

#[test]
fn networks_share_one_tagged_family_identity() {
    use net::IpNetwork;
    let v4 = IpNetwork::make_unsafe("0.0.0.0".parse().unwrap(), 0.0);
    let v6 = IpNetwork::make_unsafe("::".parse().unwrap(), 0.0);
    assert_eq!(IpNetwork::TAG, "IpNetwork");
    assert!(v4.is_ipv4());
    assert!(v6.is_ipv6());
    assert!(!v4.is_ipv6());
    assert!(!v6.is_ipv4());
    assert!(v4.is_network());
    assert!(v6.is_network());
    assert_ne!(v4, v6);
    assert_eq!(v4.render(), "0.0.0.0/0");
    assert_eq!(v6.render(), "::/0");
}

fn net_hash(address: &net::NetAddress) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    address.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn net_addresses_roundtrip_numeric_forms() {
    use net::NetAddress;
    for segments in [
        [0, 0, 0, 0, 0, 0, 0, 0],
        [0x2001, 0xdb8, 0, 1, 2, 3, 4, 5],
        [0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff, 0xffff],
    ] {
        let value = NetAddress::ipv6_from_segments(segments);
        let encoded = value.format_ip();
        assert_eq!(NetAddress::ipv6_from_string(&encoded).unwrap(), value);
    }
}

#[test]
fn net_addresses_share_numeric_equality_and_hash() {
    use net::NetAddress;
    let expanded = NetAddress::ipv6_from_string("2001:db8:0:0:0:0:0:1").unwrap();
    let compressed = NetAddress::ipv6_from_string("2001:db8::1").unwrap();
    assert_eq!(expanded, compressed);
    assert_eq!(net_hash(&expanded), net_hash(&compressed));
}

#[test]
fn net_addresses_convert_mapped_ipv4() {
    use net::NetAddress;
    let ipv4 = NetAddress::ipv4_from_string("192.0.2.128").unwrap();
    let mapped = ipv4.to_ipv4_mapped();
    assert!(mapped.is_ipv4_mapped());
    assert_eq!(mapped.format_ip(), "::ffff:192.0.2.128");
    assert_eq!(mapped.from_ipv4_mapped().unwrap(), ipv4);
    assert!(NetAddress::ipv6_from_string("::1").unwrap().from_ipv4_mapped().is_none());
    assert_eq!(mapped.to_canonical(), ipv4);
    assert_eq!(ipv4.to_canonical(), ipv4);
}

#[test]
fn byte_constructor_inputs_are_copied() {
    use net::NetAddress;
    // Node `Buffer` shares `Uint8Array` semantics: one test covers both
    // constructor rows (487 Uint8Array, 505 Buffer).
    let mut v4bytes = vec![127u8, 0, 0, 1];
    let mut v6bytes = vec![0u8; 16];
    v6bytes[15] = 1;
    let ipv4 = NetAddress::ipv4_from_bytes(&v4bytes);
    let ipv6 = NetAddress::ipv6_from_bytes(&v6bytes);
    let (hash4, hash6) = (net_hash(&ipv4), net_hash(&ipv6));
    v4bytes[0] = 1;
    v6bytes[15] = 2;
    assert_eq!(ipv4.format_ip(), "127.0.0.1");
    assert_eq!(ipv6.format_ip(), "::1");
    assert_eq!(net_hash(&ipv4), hash4);
    assert_eq!(net_hash(&ipv6), hash6);
}

#[test]
fn unix_paths_format_verbatim() {
    use net::NetAddress;
    for (path, expected) in [
        ("/tmp/server.sock", "unix:///tmp/server.sock"),
        ("./run/../server.sock", "unix://./run/../server.sock"),
        ("/tmp/socket?#%20", "unix:///tmp/socket?#%20"),
        ("/tmp/socket with spaces.sock", "unix:///tmp/socket with spaces.sock"),
        ("", "unix://"),
    ] {
        let address = NetAddress::unix_path_address(path);
        assert_eq!(address.format_unix_path().as_deref(), Some(expected));
        assert_eq!(address.format_url("http"), expected);
        assert_eq!(address.format_url("https"), expected);
    }
}

#[test]
fn unix_paths_stay_opaque() {
    use net::NetAddress;
    let address = NetAddress::unix_path_address("./run/../server.sock");
    assert_eq!(address.path(), Some("./run/../server.sock"));
    assert_eq!(address.format_socket_address(), "./run/../server.sock");
    assert_eq!(
        address,
        NetAddress::unix_path_address("./run/../server.sock")
    );
}

#[test]
fn socket_addresses_parse_format_and_reject() {
    use net::NetAddress;
    let inet = NetAddress::inet_address_from_string_unsafe("127.0.0.1:8080");
    assert_eq!(inet.format_socket_address(), "127.0.0.1:8080");
    assert_eq!(
        NetAddress::socket_address_from_input("127.0.0.1:3000")
            .unwrap()
            .format_socket_address(),
        "127.0.0.1:3000"
    );
    assert_eq!(
        NetAddress::socket_address_from_input("[::1]:3000")
            .unwrap()
            .format_socket_address(),
        "[::1]:3000"
    );
    let loopback = NetAddress::ipv4_from_string("127.0.0.1").unwrap();
    let composed = NetAddress::socket_address_from_parts(&loopback, 8080).unwrap();
    assert_eq!(composed.format_socket_address(), "127.0.0.1:8080");
    assert!(NetAddress::socket_address_from_parts(&loopback, 65536).is_err());
    assert!(NetAddress::socket_address_from_input("localhost:8080").is_err());
    assert!(std::panic::catch_unwind(|| {
        NetAddress::socket_address_from_input_unsafe("localhost:8080")
    })
    .is_err());
}
