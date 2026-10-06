//! IP network (CIDR) parsing, canonical form, and containment.
//!
//! Ports the portable core of `packages/effect/src/net/IpNetwork.ts`
//! (strict CIDR parsing, canonical text, first/last/count, address and
//! network containment). Schema codecs are not modeled here.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

/// A parsed CIDR network with host bits masked off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IpNetwork {
    network: IpAddr,
    prefix: u8,
}

impl IpNetwork {
    /// Parse strict `address/prefix` CIDR. Fails on malformed input,
    /// out-of-range prefixes, and set host bits (non-network input).
    pub fn parse(text: &str) -> Option<Self> {
        let (addr_text, prefix_text) = text.split_once('/')?;
        let addr: IpAddr = addr_text.parse().ok()?;
        let prefix: u8 = prefix_text.parse().ok()?;
        let max = match addr {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix > max {
            return None;
        }
        let network = mask(addr, prefix);
        if network != addr {
            return None;
        }
        Some(Self { network, prefix })
    }

    /// Checked constructor: finite integer prefix in family range with no
    /// host bits set, echoing the address on any failure (TS `make` /
    /// `fromAddress`, whose failures carry the supplied address as input).
    pub fn make(addr: IpAddr, prefix: f64) -> Result<Self, NetworkError> {
        let max = match addr {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if !prefix.is_finite()
            || prefix.fract() != 0.0
            || prefix < 0.0
            || prefix > f64::from(max)
        {
            return Err(NetworkError::addr(
                addr,
                format!("prefix length must be an integer from 0 through {max}"),
            ));
        }
        let bits = prefix as u8;
        if mask(addr, bits) != addr {
            return Err(NetworkError::addr(addr, "host bits set".to_string()));
        }
        Ok(Self {
            network: addr,
            prefix: bits,
        })
    }

    /// Checked unsafe constructor: like `make`, but panics on invalid
    /// input (TS `makeUnsafe`).
    pub fn make_unsafe(addr: IpAddr, prefix: f64) -> Self {
        Self::make(addr, prefix).expect("invalid network")
    }

    /// The runtime type tag (TS `_tag: "IpNetwork"`).
    pub const TAG: &'static str = "IpNetwork";

    /// Any network counts (TS `isIpNetwork`; the negative halves are
    /// unrepresentable here, and freezing is structural).
    pub fn is_network(&self) -> bool {
        true
    }

    /// True for IPv4 networks (TS `isIpv4Network`).
    pub fn is_ipv4(&self) -> bool {
        matches!(self.network, IpAddr::V4(_))
    }

    /// True for IPv6 networks (TS `isIpv6Network`).
    pub fn is_ipv6(&self) -> bool {
        matches!(self.network, IpAddr::V6(_))
    }

    /// Lenient checked constructor: masks host bits instead of rejecting
    /// them, still failing on out-of-range prefixes (TS `fromAddress`).
    pub fn from_address(addr: IpAddr, prefix: f64) -> Result<Self, NetworkError> {
        let max = match addr {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if !prefix.is_finite()
            || prefix.fract() != 0.0
            || prefix < 0.0
            || prefix > f64::from(max)
        {
            return Err(NetworkError::addr(
                addr,
                format!("prefix length must be an integer from 0 through {max}"),
            ));
        }
        let bits = prefix as u8;
        Ok(Self {
            network: mask(addr, bits),
            prefix: bits,
        })
    }

    /// Lenient unsafe constructor: masks host bits instead of rejecting
    /// them, still panicking on out-of-range prefixes (TS
    /// `fromAddressUnsafe`).
    pub fn from_address_unsafe(addr: IpAddr, prefix: f64) -> Self {
        Self::from_address(addr, prefix).expect("invalid network")
    }

    /// Checked unsafe parse: panics on invalid input (TS
    /// `fromStringUnsafe`).
    pub fn from_string_unsafe(text: &str) -> Self {
        Self::parse_reported(text).expect("invalid network")
    }

    /// Strict parse with error-input echo (TS `fromString`): an unparsable
    /// address echoes the offending dotted quad (or whole address text); a
    /// malformed prefix echoes the full input once the address parsed;
    /// out-of-range prefixes and host bits echo the parsed address.
    pub fn parse_reported(text: &str) -> Result<Self, NetworkError> {
        let (addr_text, prefix_text) = text
            .split_once('/')
            .ok_or_else(|| NetworkError::text(text, "missing prefix".to_string()))?;
        let addr: IpAddr = addr_text.parse().map_err(|_| {
            NetworkError::text(
                offending_address(addr_text),
                "invalid address".to_string(),
            )
        })?;
        if prefix_text.starts_with('+') {
            return Err(NetworkError::text(text, "invalid prefix".to_string()));
        }
        let prefix: f64 = prefix_text
            .parse()
            .map_err(|_| NetworkError::text(text, "invalid prefix".to_string()))?;
        Self::make(addr, prefix)
    }

    /// The masked network address.
    pub fn network_address(&self) -> IpAddr {
        self.network
    }

    /// The prefix length.
    pub fn prefix(&self) -> u8 {
        self.prefix
    }

    /// Canonical `address/prefix` text.
    pub fn render(&self) -> String {
        format!("{}/{}", self.network, self.prefix)
    }

    /// First address in the network.
    pub fn first(&self) -> IpAddr {
        self.network
    }

    /// Last address in the network (broadcast for IPv4).
    pub fn last(&self) -> IpAddr {
        broadcast(self.network, self.prefix)
    }

    /// Exact number of addresses ( panics-free: saturates at `u128::MAX`).
    pub fn address_count(&self) -> u128 {
        let bits = match self.network {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        let host_bits = bits - self.prefix as u32;
        if host_bits >= 128 {
            u128::MAX
        } else {
            1u128 << host_bits
        }
    }

    /// True when `addr` (same family) lies inside this network.
    pub fn contains_addr(&self, addr: IpAddr) -> bool {
        same_family(addr, self.network) && mask(addr, self.prefix) == self.network
    }

    /// True when every address of `other` lies inside `self`.
    pub fn contains_network(&self, other: &Self) -> bool {
        self.prefix <= other.prefix && self.contains_addr(other.network)
    }

    /// True when the two networks share at least one address: either base
    /// lies inside the other. CIDR blocks are nested-or-disjoint, so this
    /// is exact; cross-family pairs never overlap.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.contains_addr(other.network) || other.contains_addr(self.network)
    }
}

/// Which input a network constructor rejected: the parsed address or the
/// raw text, mirroring TS `failure(...).input`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkErrorInput {
    Addr(IpAddr),
    Text(String),
    /// A constructed address value (TS failures carrying `input`).
    Address(NetAddress),
}

/// A rejected network construction, carrying the offending input.
/// `tag`/`message` mirror TS `NetAddressError` failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkError {
    pub input: NetworkErrorInput,
    pub tag: &'static str,
    pub message: String,
    /// Underlying failure text, when the constructor reports one.
    pub cause: Option<String>,
}

impl NetworkError {
    fn addr(addr: IpAddr, message: impl Into<String>) -> Self {
        Self {
            input: NetworkErrorInput::Addr(addr),
            tag: "NetAddressError",
            message: message.into(),
            cause: None,
        }
    }

    fn address(address: NetAddress, message: impl Into<String>) -> Self {
        Self {
            input: NetworkErrorInput::Address(address),
            tag: "NetAddressError",
            message: message.into(),
            cause: None,
        }
    }

    fn caused(mut self, cause: impl Into<String>) -> Self {
        self.cause = Some(cause.into());
        self
    }

    fn text(text: &str, message: impl Into<String>) -> Self {
        Self {
            input: NetworkErrorInput::Text(text.to_string()),
            tag: "NetAddressError",
            message: message.into(),
            cause: None,
        }
    }
}

/// The dotted quad that failed inside an address text: the tail after the
/// last colon when it carries dots (embedded IPv4), else the whole text.
fn offending_address(addr_text: &str) -> &str {
    match addr_text.rsplit_once(':') {
        Some((_, tail)) if tail.contains('.') => tail,
        _ => addr_text,
    }
}

fn same_family(a: IpAddr, b: IpAddr) -> bool {
    matches!(
        (a, b),
        (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_))
    )
}

fn mask(addr: IpAddr, prefix: u8) -> IpAddr {
    match addr {
        IpAddr::V4(v4) => {
            let bits = u32::from(v4);
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            IpAddr::V4((bits & mask).into())
        }
        IpAddr::V6(v6) => {
            let bits = u128::from(v6);
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            IpAddr::V6((bits & mask).into())
        }
    }
}

fn broadcast(network: IpAddr, prefix: u8) -> IpAddr {
    match network {
        IpAddr::V4(v4) => {
            let bits = u32::from(v4);
            let host = if prefix == 0 {
                u32::MAX
            } else {
                u32::MAX >> prefix
            };
            IpAddr::V4((bits | host).into())
        }
        IpAddr::V6(v6) => {
            let bits = u128::from(v6);
            let host = if prefix == 0 {
                u128::MAX
            } else {
                u128::MAX >> prefix
            };
            IpAddr::V6((bits | host).into())
        }
    }
}

/// An IP interface: address plus prefix, host bits preserved.
///
/// Ports the portable core of `net/IpInterface.ts`: unlike [`IpNetwork`],
/// construction keeps host bits; [`Self::network`] derives the containing
/// network. Bare addresses default to full-width prefixes (/32, /128).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IpInterface {
    address: IpAddr,
    prefix: u8,
}

impl IpInterface {
    /// Parse `address/prefix`, keeping host bits. Fails on malformed
    /// input or out-of-range prefixes.
    pub fn parse(text: &str) -> Option<Self> {
        let (addr_text, prefix_text) = text.split_once('/')?;
        let address: IpAddr = addr_text.parse().ok()?;
        let prefix: u8 = prefix_text.parse().ok()?;
        let max = match address {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix > max {
            return None;
        }
        Some(Self { address, prefix })
    }

    /// A bare address with a full-width prefix.
    pub fn from_addr(address: IpAddr) -> Self {
        let prefix = match address {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        Self { address, prefix }
    }

    /// The address, host bits intact.
    pub fn address(&self) -> IpAddr {
        self.address
    }

    /// The prefix length.
    pub fn prefix(&self) -> u8 {
        self.prefix
    }

    /// Canonical `address/prefix` text.
    pub fn render(&self) -> String {
        format!("{}/{}", self.address, self.prefix)
    }

    /// The containing network (host bits masked off).
    pub fn network(&self) -> IpNetwork {
        IpNetwork {
            network: mask(self.address, self.prefix),
            prefix: self.prefix,
        }
    }
}

impl IpNetwork {
    /// Build from an interface's address and prefix, masking host bits
    /// (TS `fromInterface`).
    pub fn from_interface(interface: &IpInterface) -> Self {
        interface.network()
    }
}

/// Address class for bit classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpClass {
    /// `0.0.0.0` / `::`.
    Unspecified,
    /// `127.0.0.0/8` / `::1`.
    Loopback,
    /// RFC 1918 / ULA.
    Private,
    /// Link-local (`169.254/16`, `fe80::/10`).
    LinkLocal,
    /// Multicast.
    Multicast,
    /// Limited broadcast `255.255.255.255`.
    Broadcast,
    /// Anything else (no reachability claim).
    Global,
}

/// A strictly parsed IP address with classification.
///
/// Ports the portable core of `NetAddress.ts` IP handling: strict dotted
/// decimal (no whitespace, no empty parts, no leading zeros, 0-255),
/// full IPv6 via the standard parser, canonical text, octets, widths,
/// and bit classification. Sockets, zones, MAC, and schemas are out of
/// scope here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StrictIp {
    addr: IpAddr,
}

impl StrictIp {
    /// Parse strictly. IPv4 requires exactly four dot-separated decimal
    /// parts without leading zeros; IPv6 uses standard notation.
    pub fn parse(text: &str) -> Option<Self> {
        if text.is_empty() || text.bytes().any(|b| b.is_ascii_whitespace()) {
            return None;
        }
        if text.contains('.') && !text.contains(':') {
            let parts: Vec<&str> = text.split('.').collect();
            if parts.len() != 4 {
                return None;
            }
            let mut octets = [0u8; 4];
            for (i, part) in parts.iter().enumerate() {
                if part.is_empty()
                    || part.len() > 3
                    || !part.bytes().all(|b| b.is_ascii_digit())
                    || (part.len() > 1 && part.starts_with('0'))
                {
                    return None;
                }
                octets[i] = part.parse().ok()?;
            }
            return Some(Self {
                addr: IpAddr::V4(octets.into()),
            });
        }
        let addr: IpAddr = text.parse().ok()?;
        Some(Self { addr })
    }

    /// The parsed address value.
    pub fn addr(&self) -> IpAddr {
        self.addr
    }

    /// The address width in bits (32 or 128).
    pub fn width(&self) -> u8 {
        match self.addr {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        }
    }

    /// Raw octets in network order.
    pub fn octets(&self) -> Vec<u8> {
        match self.addr {
            IpAddr::V4(v4) => v4.octets().to_vec(),
            IpAddr::V6(v6) => v6.octets().to_vec(),
        }
    }

    /// Canonical text (compressed IPv6, dotted IPv4).
    pub fn render(&self) -> String {
        self.addr.to_string()
    }

    /// Bit classification (no reachability claim).
    pub fn classify(&self) -> IpClass {
        match self.addr {
            IpAddr::V4(v4) => {
                let o = v4.octets();
                if o == [0, 0, 0, 0] {
                    IpClass::Unspecified
                } else if o[0] == 127 {
                    IpClass::Loopback
                } else if o[0] == 10
                    || (o[0] == 172 && (16..32).contains(&o[1]))
                    || (o[0] == 192 && o[1] == 168)
                {
                    IpClass::Private
                } else if o[0] == 169 && o[1] == 254 {
                    IpClass::LinkLocal
                } else if (224..240).contains(&o[0]) {
                    IpClass::Multicast
                } else if o == [255, 255, 255, 255] {
                    IpClass::Broadcast
                } else {
                    IpClass::Global
                }
            }
            IpAddr::V6(v6) => {
                if v6.is_unspecified() {
                    IpClass::Unspecified
                } else if v6.is_loopback() {
                    IpClass::Loopback
                } else if (v6.segments()[0] & 0xfe00) == 0xfc00 {
                    IpClass::Private
                } else if (v6.segments()[0] & 0xffc0) == 0xfe80 {
                    IpClass::LinkLocal
                } else if v6.is_multicast() {
                    IpClass::Multicast
                } else {
                    IpClass::Global
                }
            }
        }
    }
}

// Numeric addresses

/// A bare IP or Unix-socket address value: the runtime representation
/// shared by the `NetAddress` constructors (hostnames and URLs arrive as
/// later variants). Debug prints canonical text (see below).
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum NetAddress {
    V4(Ipv4Addr),
    V6(Ipv6Addr),
    /// A Unix socket path, kept opaque: never encoded or normalized.
    UnixPath(String),
    /// An IP socket address: bare IP plus port (TS `InetAddress`).
    Inet(SocketAddr),
}

impl NetAddress {
    /// `0.0.0.0` (TS `ipv4Unspecified`).
    pub const IPV4_UNSPECIFIED: Self = Self::V4(Ipv4Addr::UNSPECIFIED);
    /// `::` (TS `ipv6Unspecified`).
    pub const IPV6_UNSPECIFIED: Self = Self::V6(Ipv6Addr::UNSPECIFIED);

    /// Wrap a standard address (shared by Inet classification).
    fn from_std_ip(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(v4) => Self::V4(v4),
            IpAddr::V6(v6) => Self::V6(v6),
        }
    }

    /// Build from eight 16-bit segments (TS `ipv6FromSegments`).
    pub fn ipv6_from_segments(segments: [u16; 8]) -> Self {
        Self::V6(Ipv6Addr::from(segments))
    }

    /// Build from four octets (TS `ipv4FromBytesUnsafe`; any other length
    /// cannot name an IPv4 address and panics).
    pub fn ipv4_from_bytes(bytes: &[u8]) -> Self {
        assert_eq!(bytes.len(), 4, "IPv4 needs exactly 4 octets");
        Self::V4(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]))
    }

    /// Build from sixteen octets in network order (TS
    /// `ipv6FromBytesUnsafe`; any other length panics).
    pub fn ipv6_from_bytes(bytes: &[u8]) -> Self {
        assert_eq!(bytes.len(), 16, "IPv6 needs exactly 16 octets");
        let mut segments = [0u16; 8];
        for (i, pair) in bytes.chunks_exact(2).enumerate() {
            segments[i] = u16::from_be_bytes([pair[0], pair[1]]);
        }
        Self::V6(Ipv6Addr::from(segments))
    }

    /// Strict dotted-quad parse, rejecting non-IPv4 input (TS
    /// `ipv4FromString`).
    pub fn ipv4_from_string(text: &str) -> Result<Self, NetworkError> {
        match StrictIp::parse(text) {
            Some(strict) => match strict.addr() {
                IpAddr::V4(v4) => Ok(Self::V4(v4)),
                IpAddr::V6(_) => Err(NetworkError::text(text, "not an IPv4 address")),
            },
            None => Err(NetworkError::text(text, "invalid address")),
        }
    }

    /// Checked IP parse dispatching on family: dotted text needs exactly
    /// four decimal octets (message verbatim from TS `ipFromString`),
    /// colon text parses as IPv6.
    pub fn ip_from_string(text: &str) -> Result<Self, NetworkError> {
        if text.contains(':') {
            return Self::ipv6_from_string(text);
        }
        if text.split('.').count() != 4 {
            return Err(NetworkError::text(
                text,
                "expected exactly four decimal octets",
            ));
        }
        Self::ipv4_from_string(text)
    }

    /// Strict IP parse of either family (TS `ipv6FromString`).
    pub fn ipv6_from_string(text: &str) -> Result<Self, NetworkError> {
        match StrictIp::parse(text) {
            Some(strict) => match strict.addr() {
                IpAddr::V6(v6) => Ok(Self::V6(v6)),
                IpAddr::V4(_) => Err(NetworkError::text(
                    offending_address(text),
                    "not an IPv6 address",
                )),
            },
            None => Err(NetworkError::text(
                offending_address(text),
                "invalid address",
            )),
        }
    }

    /// Canonical text: compressed IPv6, dotted IPv4 (TS `formatIp`).
    pub fn format_ip(&self) -> String {
        match self {
            Self::V4(v4) => v4.to_string(),
            Self::V6(v6) => v6.to_string(),
            // Opaque passthrough: Unix paths have no IP text.
            Self::UnixPath(path) => path.clone(),
            // Socket text: the canonical form of an Inet value.
            Self::Inet(socket) => socket.to_string(),
        }
    }

    /// Map an IPv4 address into `::ffff:0:0/96`, passing other values
    /// through (TS `toIpv4Mapped`).
    pub fn to_ipv4_mapped(&self) -> Self {
        match self {
            Self::V4(v4) => {
                let o = v4.octets();
                Self::V6(Ipv6Addr::new(
                    0,
                    0,
                    0,
                    0,
                    0,
                    0xffff,
                    u16::from_be_bytes([o[0], o[1]]),
                    u16::from_be_bytes([o[2], o[3]]),
                ))
            }
            Self::V6(_) => self.clone(),
            Self::UnixPath(_) => self.clone(),
            Self::Inet(_) => self.clone(),
        }
    }

    /// True for `::ffff:a.b.c.d` forms (TS `isIpv4Mapped`).
    pub fn is_ipv4_mapped(&self) -> bool {
        match self {
            Self::V4(_) => false,
            Self::UnixPath(_) => false,
            Self::Inet(_) => false,
            Self::V6(v6) => {
                let s = v6.segments();
                s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0xffff
            }
        }
    }

    /// The IPv4 address inside a mapped `::ffff:a.b.c.d` value, if mapped.
    fn mapped_ipv4(v6: &Ipv6Addr) -> Option<Ipv4Addr> {
        let s = v6.segments();
        if s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0xffff {
            Some(Ipv4Addr::new(
                (s[6] >> 8) as u8,
                (s[6] & 0xff) as u8,
                (s[7] >> 8) as u8,
                (s[7] & 0xff) as u8,
            ))
        } else {
            None
        }
    }

    /// Recover the IPv4 address from a mapped value (TS `fromIpv4Mapped`).
    pub fn from_ipv4_mapped(&self) -> Option<Self> {
        match self {
            Self::V4(_) => None,
            Self::UnixPath(_) => None,
            Self::Inet(socket) => match socket.ip() {
                IpAddr::V6(v6) => Self::mapped_ipv4(&v6)
                    .map(|v4| Self::Inet(SocketAddr::new(IpAddr::V4(v4), socket.port()))),
                _ => None,
            },
            Self::V6(v6) => Self::mapped_ipv4(v6).map(Self::V4),
        }
    }

    /// Canonical form: mapped values collapse to IPv4, everything else
    /// passes through (TS `toCanonical`).
    pub fn to_canonical(&self) -> Self {
        self.from_ipv4_mapped().unwrap_or_else(|| self.clone())
    }

    /// Build a Unix socket address from a path, kept verbatim (TS
    /// `unixPathAddress`).
    pub fn unix_path_address(path: impl Into<String>) -> Self {
        Self::UnixPath(path.into())
    }

    /// Build a socket address from strict `ip:port` text (TS
    /// `inetAddressFromStringUnsafe`; panics on hostnames or bad ports:
    /// `SocketAddr` parsing never resolves names).
    pub fn inet_address_from_string_unsafe(text: &str) -> Self {
        Self::Inet(text.parse::<SocketAddr>().expect("invalid socket address"))
    }

    /// Build an IPv4 socket address from parts (TS `inetAddressV4`).
    pub fn inet_address_v4(address: &NetAddress, port: u32) -> Result<Self, NetworkError> {
        Self::socket_address_from_parts(address, port)
    }

    /// Build an IPv6 socket address with scope id (TS `inetAddressV6`).
    /// Scope zero is the default; only IPv6 carries one.
    pub fn inet_address_v6(address: &NetAddress, port: u32, scope_id: u32) -> Result<Self, NetworkError> {
        if port > 65535 {
            return Err(NetworkError::text("out-of-range port", "invalid port"));
        }
        match address {
            Self::V6(v6) => Ok(Self::Inet(SocketAddr::V6(SocketAddrV6::new(
                *v6,
                port as u16,
                0,
                scope_id,
            )))),
            _ => Err(NetworkError::text("non-IPv6 socket address", "invalid address")),
        }
    }

    /// Build from an address plus port: bare IPs gain the port, Unix and
    /// nested values fail (TS `socketAddressFromInput` struct form).
    pub fn socket_address_from_parts(
        address: &NetAddress,
        port: u32,
    ) -> Result<Self, NetworkError> {
        let ip = match address {
            Self::V4(v4) => IpAddr::V4(*v4),
            Self::V6(v6) => IpAddr::V6(*v6),
            Self::UnixPath(_) | Self::Inet(_) => {
                return Err(NetworkError::text(
                    "non-IP socket address",
                    "invalid address",
                ));
            }
        };
        if port > 65535 {
            return Err(NetworkError::addr(ip, "invalid port"));
        }
        Ok(Self::Inet(SocketAddr::new(ip, port as u16)))
    }

    /// Build from `host:port` / `[IPv6]:port` text, rejecting hostnames (TS
    /// `socketAddressFromString` string form). Structural failures keep the
    /// full text; a bad host keeps the offending address; a bad port keeps
    /// the parsed address.
    pub fn socket_address_from_input(text: &str) -> Result<Self, NetworkError> {
        let invalid = |message: &str| NetworkError::text(text, message);
        let (host, port_text, bracketed, scope_id) = if let Some(rest) = text.strip_prefix('[') {
            let end = rest
                .find(']')
                .ok_or_else(|| invalid("expected [IPv6]:port"))?;
            let after = &rest[end + 1..];
            let port = after
                .strip_prefix(':')
                .filter(|port| !port.contains(']'))
                .ok_or_else(|| invalid("expected [IPv6]:port"))?;
            let (host, scope_id) = match rest[..end].split_once('%') {
                None => (rest[..end].to_owned(), 0u32),
                Some((bare, zone)) => {
                    if zone.is_empty() || !zone.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(invalid("scope identifier must be decimal"));
                    }
                    let scope_id: u32 = zone.parse().map_err(|_| {
                        invalid("scope identifier must be an unsigned 32-bit integer")
                    })?;
                    (bare.to_owned(), scope_id)
                }
            };
            (host, port, true, scope_id)
        } else {
            match text.rfind(':') {
                None => return Err(invalid("expected host:port or [IPv6]:port")),
                Some(separator) => {
                    if text[..separator].contains(':') {
                        return Err(invalid("IPv6 addresses must be bracketed"));
                    }
                    (
                        text[..separator].to_owned(),
                        &text[separator + 1..],
                        false,
                        0,
                    )
                }
            }
        };
        if port_text.is_empty()
            || !port_text.bytes().all(|b| b.is_ascii_digit())
            || (port_text.len() > 1 && port_text.starts_with('0'))
        {
            return Err(invalid("port must be an unpadded decimal integer"));
        }
        // Unrepresentable ports fail the range check below like TS Number().
        let port = port_text.parse::<u64>().unwrap_or(u64::MAX);
        let ip = match StrictIp::parse(&host) {
            Some(strict) => strict.addr(),
            None => {
                return Err(NetworkError::text(
                    offending_address(&host),
                    "invalid address",
                ));
            }
        };
        if bracketed != matches!(ip, IpAddr::V6(_)) {
            return Err(invalid("only IPv6 addresses use brackets"));
        }
        if port > 65535 {
            return Err(NetworkError::addr(ip, "invalid port"));
        }
        match ip {
            IpAddr::V4(v4) => Ok(Self::Inet(SocketAddr::new(IpAddr::V4(v4), port as u16))),
            IpAddr::V6(v6) => Ok(Self::Inet(SocketAddr::V6(SocketAddrV6::new(
                v6,
                port as u16,
                0,
                scope_id,
            )))),
        }
    }

    /// Throwing twin of [`Self::socket_address_from_input`] (TS
    /// `socketAddressFromInputUnsafe`).
    pub fn socket_address_from_input_unsafe(text: &str) -> Self {
        Self::socket_address_from_input(text).expect("invalid socket address")
    }

    /// The socket path for Unix addresses (TS `path`).
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::UnixPath(path) => Some(path),
            Self::V4(_) | Self::V6(_) | Self::Inet(_) => None,
        }
    }

    /// `unix://` plus the verbatim path: no encoding, no normalization
    /// (TS `formatUnixPath`).
    pub fn format_unix_path(&self) -> Option<String> {
        self.path().map(|path| format!("unix://{path}"))
    }

    /// The socket address text: Unix paths verbatim, bare IPs without a
    /// port (TS `formatSocketAddress`; ports arrive with socket addresses).
    pub fn format_socket_address(&self) -> String {
        match self {
            Self::UnixPath(path) => path.clone(),
            Self::V4(_) | Self::V6(_) => self.format_ip(),
            Self::Inet(socket) => socket.to_string(),
        }
    }

    /// URL text: `unix://` paths verbatim under any scheme, IPs as
    /// `scheme://host` with brackets for IPv6 and no trailing slash (TS
    /// `formatUrl`/`formatUrlUnsafe` for the portless forms).
    pub fn format_url(&self, scheme: &str) -> String {
        match self {
            Self::UnixPath(path) => format!("unix://{path}"),
            Self::V4(v4) => format!("{scheme}://{v4}"),
            Self::V6(v6) => format!("{scheme}://[{v6}]"),
            // Portless URL forms arrive with socket addresses; the socket
            // text is the only faithful rendering available here.
            Self::Inet(socket) => format!("{scheme}://{socket}"),
        }
    }

    /// Normalized URL value (TS `toUrl`): lowercased scheme, default
    /// ports omitted, trailing slash. Scoped IPv6, bad schemes, and
    /// file URLs with ports fail with the address as input.
    pub fn to_url(&self, scheme: &str) -> Result<String, NetworkError> {
        if !Self::valid_scheme(scheme) {
            return Err(
                NetworkError::address(self.clone(), "failed to construct URL")
                    .caused("TypeError: invalid scheme"),
            );
        }
        let scheme = scheme.to_ascii_lowercase();
        let (host, port) = match self {
            Self::V4(v4) => (v4.to_string(), None),
            Self::V6(v6) => (format!("[{v6}]"), None),
            Self::UnixPath(_) => {
                return Err(
                    NetworkError::address(self.clone(), "failed to construct URL")
                        .caused("TypeError: unsupported address"),
                );
            }
            Self::Inet(socket) => {
                if let std::net::SocketAddr::V6(v6) = socket {
                    if v6.scope_id() != 0 {
                        return Err(NetworkError::address(
                            self.clone(),
                            "scoped IPv6 addresses are not supported",
                        ));
                    }
                }
                let host = match socket.ip() {
                    std::net::IpAddr::V4(v4) => v4.to_string(),
                    std::net::IpAddr::V6(v6) => format!("[{v6}]"),
                };
                (host, Some(socket.port()))
            }
        };
        let default = matches!(
            (scheme.as_str(), port),
            (_, None)
                | ("ftp", Some(21))
                | ("http" | "ws", Some(80))
                | ("https" | "wss", Some(443))
        );
        if scheme == "file" && port.is_some() {
            return Err(
                NetworkError::address(self.clone(), "failed to construct URL")
                    .caused("TypeError: file URLs take no port"),
            );
        }
        Ok(match port {
            Some(port) if !default => format!("{scheme}://{host}:{port}/"),
            _ => format!("{scheme}://{host}/"),
        })
    }

    /// `true` for URL scheme syntax (`ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`).
    fn valid_scheme(scheme: &str) -> bool {
        let mut chars = scheme.chars();
        match chars.next() {
            Some(first) if first.is_ascii_alphabetic() => (),
            _ => return false,
        }
        scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    }

    /// Display URL without trailing slash, defaulting an omitted scheme
    /// to http (TS `formatUrl`). Returns `None` for Unix paths.
    pub fn format_url_value(&self, scheme: Option<&str>) -> Option<String> {
        let scheme = scheme.unwrap_or("http").to_ascii_lowercase();
        let default = matches!(
            (scheme.as_str(), self.port()),
            (_, None)
                | ("ftp", Some(21))
                | ("http" | "ws", Some(80))
                | ("https" | "wss", Some(443))
        );
        let host = match self {
            Self::V4(v4) => v4.to_string(),
            Self::V6(v6) => format!("[{v6}]"),
            Self::UnixPath(_) => return None,
            Self::Inet(socket) => match socket.ip() {
                std::net::IpAddr::V4(v4) => v4.to_string(),
                std::net::IpAddr::V6(v6) => format!("[{v6}]"),
            },
        };
        Some(match self.port() {
            Some(port) if !default => format!("{scheme}://{host}:{port}"),
            _ => format!("{scheme}://{host}"),
        })
    }

    /// Port for socket addresses, if present.
    pub fn port(&self) -> Option<u16> {
        match self {
            Self::Inet(socket) => Some(socket.port()),
            _ => None,
        }
    }
}

impl std::fmt::Display for NetAddress {
    /// Canonical text per variant: bare IPs, socket addrs, verbatim Unix
    /// paths (TS `toJSON` strings).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::V4(_) | Self::V6(_) => write!(f, "{}", self.format_ip()),
            Self::UnixPath(path) => write!(f, "{path}"),
            Self::Inet(socket) => write!(f, "{socket}"),
        }
    }
}

impl std::fmt::Debug for NetAddress {
    /// Canonical text, no private bytes (TS `inspect` parity; mirrors the
    /// `Redacted` convention of hiding internals in `Debug`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl NetAddress {
    /// True for `0.0.0.0` / `::` (TS `isUnspecified`).
    pub fn is_unspecified(&self) -> bool {
        match self {
            Self::V4(v4) => u32::from_be_bytes(v4.octets()) == 0,
            Self::V6(v6) => v6.segments() == [0, 0, 0, 0, 0, 0, 0, 0],
            Self::UnixPath(_) => false,
            Self::Inet(socket) => Self::from_std_ip(socket.ip()).is_unspecified(),
        }
    }

    /// True for IPv4 `127.0.0.0/8` or IPv6 `::1` (TS `isLoopback`).
    pub fn is_loopback(&self) -> bool {
        match self {
            Self::V4(v4) => (u32::from_be_bytes(v4.octets()) >> 24) == 127,
            Self::V6(v6) => v6.segments() == [0, 0, 0, 0, 0, 0, 0, 1],
            Self::UnixPath(_) => false,
            Self::Inet(socket) => Self::from_std_ip(socket.ip()).is_loopback(),
        }
    }

    /// True for IPv4 `224.0.0.0/4` (top nibble `0xe`, so `240.0.0.0/4`
    /// reserved space is excluded) or IPv6 `ff00::/8` (TS `isMulticast`).
    pub fn is_multicast(&self) -> bool {
        match self {
            Self::V4(v4) => (u32::from_be_bytes(v4.octets()) >> 28) == 0xe,
            Self::V6(v6) => (v6.segments()[0] >> 8) == 0xff,
            Self::UnixPath(_) => false,
            Self::Inet(socket) => Self::from_std_ip(socket.ip()).is_multicast(),
        }
    }

    /// True for the IPv4 limited broadcast `255.255.255.255` (TS
    /// `isBroadcast`; directed broadcast needs a prefix and is not
    /// classified here).
    pub fn is_broadcast(&self) -> bool {
        match self {
            Self::V4(v4) => u32::from_be_bytes(v4.octets()) == 0xffff_ffff,
            Self::V6(_) => false,
            Self::UnixPath(_) => false,
            Self::Inet(socket) => Self::from_std_ip(socket.ip()).is_broadcast(),
        }
    }

    /// True when syntactically unicast: not multicast, not unspecified,
    /// and (for IPv4) not broadcast (TS `isUnicast`). Says nothing about
    /// reachability.
    pub fn is_unicast(&self) -> bool {
        !self.is_multicast() && !self.is_unspecified() && !self.is_broadcast()
    }

    /// True for IPv4 `169.254.0.0/16` or IPv6 `fe80::/10` (TS
    /// `isLinkLocal`).
    pub fn is_link_local(&self) -> bool {
        match self {
            Self::V4(v4) => (u32::from_be_bytes(v4.octets()) >> 16) == 0xa9fe,
            Self::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
            Self::UnixPath(_) => false,
            Self::Inet(socket) => Self::from_std_ip(socket.ip()).is_link_local(),
        }
    }

    /// True for RFC 1918 ranges `10/8`, `172.16/12`, `192.168/16` (TS
    /// `isPrivate`; IPv4-only).
    pub fn is_private(&self) -> bool {
        match self {
            Self::V4(v4) => {
                let value = u32::from_be_bytes(v4.octets());
                (value >> 24) == 10 || ((value >> 16) & 0xfff0) == 0xac10 || (value >> 16) == 0xc0a8
            }
            Self::V6(_) => false,
            Self::UnixPath(_) => false,
            Self::Inet(socket) => Self::from_std_ip(socket.ip()).is_private(),
        }
    }

    /// True for IPv6 unique-local `fc00::/7` (TS `isUniqueLocal`).
    pub fn is_unique_local(&self) -> bool {
        match self {
            Self::V4(_) => false,
            Self::V6(v6) => ((v6.segments()[0] >> 8) & 0xfe) == 0xfc,
            Self::UnixPath(_) => false,
            Self::Inet(socket) => Self::from_std_ip(socket.ip()).is_unique_local(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MacAddress([u8; 6]);

impl MacAddress {
    /// Parse `xx:xx:xx:xx:xx:xx` with exactly two hex digits per group
    /// (TS `macAddressFromString`).
    pub fn parse(text: &str) -> Option<Self> {
        let parts: Vec<&str> = text.split(':').collect();
        if parts.len() != 6 {
            return None;
        }
        let mut octets = [0u8; 6];
        for (i, part) in parts.iter().enumerate() {
            if part.len() != 2 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            octets[i] = u8::from_str_radix(part, 16).ok()?;
        }
        Some(Self(octets))
    }

    /// Parse, keeping the rejected text on failure.
    pub fn parse_reported(text: &str) -> Result<Self, NetworkError> {
        Self::parse(text).ok_or_else(|| NetworkError::text(text, "invalid MAC address"))
    }

    /// The six octets in transmission order.
    pub fn octets(&self) -> [u8; 6] {
        self.0
    }

    /// Build from six octets (always valid: `u8` carries the range).
    pub fn from_octets(octets: [u8; 6]) -> Self {
        Self(octets)
    }

    /// Build from decimal octet values, rejecting out-of-range entries.
    pub fn from_numbers(octets: [i64; 6]) -> Option<Self> {
        if !octets.iter().all(|o| (0..=255).contains(o)) {
            return None;
        }
        Some(Self(octets.map(|o| o as u8)))
    }

    /// True when the group bit (first-octet LSB) marks multicast (TS
    /// `isMulticast` / `isMacMulticast`).
    pub fn is_multicast(&self) -> bool {
        self.0[0] & 1 == 1
    }

    /// MAC-scoped twin of [`Self::is_multicast`].
    pub fn is_mac_multicast(&self) -> bool {
        self.is_multicast()
    }

    /// True when unicast (TS `isUnicast` / `isMacUnicast`).
    pub fn is_unicast(&self) -> bool {
        !self.is_multicast()
    }

    /// MAC-scoped twin of [`Self::is_unicast`].
    pub fn is_mac_unicast(&self) -> bool {
        self.is_unicast()
    }

    /// True for the all-ones address (TS `isBroadcast` / `isMacBroadcast`).
    pub fn is_broadcast(&self) -> bool {
        self.0 == [0xff; 6]
    }

    /// MAC-scoped twin of [`Self::is_broadcast`].
    pub fn is_mac_broadcast(&self) -> bool {
        self.is_broadcast()
    }

    /// True when the administration bit (first-octet second LSB) marks
    /// local administration (TS `isMacLocallyAdministered`).
    pub fn is_mac_locally_administered(&self) -> bool {
        self.0[0] & 2 == 2
    }

    /// True when universally administered (TS
    /// `isMacUniversallyAdministered`).
    pub fn is_mac_universally_administered(&self) -> bool {
        !self.is_mac_locally_administered()
    }
}

impl std::fmt::Display for MacAddress {
    /// Lowercase colon-separated hex (TS canonical MAC text).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let o = self.0;
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            o[0], o[1], o[2], o[3], o[4], o[5]
        )
    }
}

// URL forms

/// A URL built from a bare IP address: scheme plus host, no port, no
/// path beyond `/` (TS `toUrl` over bare IPs; hostnames, ports, and
/// paths arrive with URL parsing).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NetUrl {
    scheme: String,
    host: String,
}

impl NetUrl {
    /// Build from a bare IP address with an explicit scheme (TS `toUrl`).
    /// Non-IP values have no URL form here.
    pub fn from_ip(address: &NetAddress, scheme: &str) -> Option<Self> {
        let host = match address {
            NetAddress::V4(v4) => v4.to_string(),
            NetAddress::V6(v6) => format!("[{v6}]"),
            NetAddress::UnixPath(_) | NetAddress::Inet(_) => return None,
        };
        Some(Self {
            scheme: scheme.to_string(),
            host,
        })
    }

    /// Canonical `scheme://host/` text (TS `href`).
    pub fn href(&self) -> String {
        format!("{}://{}/", self.scheme, self.host)
    }

    /// The explicit port text: always empty for bare-IP URLs (TS `port`).
    pub fn port(&self) -> &str {
        ""
    }

    /// The URL scheme.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }
}

impl IpInterface {
    /// Parse, panicking on invalid input. The throwing counterpart of
    /// [`Self::parse`] for trusted literals.
    pub fn must_parse(text: &str) -> Self {
        Self::parse(text).expect("valid interface")
    }

    /// The runtime type tag (TS `_tag: "IpInterface"`).
    pub const TAG: &'static str = "IpInterface";

    /// Checked unsafe constructor: range-checked prefix, host bits kept
    /// (interfaces carry an address, not a network). Panics on invalid
    /// input (TS `makeUnsafe`).
    pub fn make_unsafe(address: IpAddr, prefix: f64) -> Self {
        let max = match address {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        assert!(
            prefix.is_finite()
                && prefix.fract() == 0.0
                && prefix >= 0.0
                && prefix <= f64::from(max),
            "invalid prefix"
        );
        Self {
            address,
            prefix: prefix as u8,
        }
    }

    /// Any interface counts (TS `isIpInterface`; the negative halves are
    /// unrepresentable here).
    pub fn is_interface(&self) -> bool {
        true
    }

    /// True for IPv4 interfaces (TS `isIpv4Interface`).
    pub fn is_ipv4(&self) -> bool {
        matches!(self.address, IpAddr::V4(_))
    }

    /// True for IPv6 interfaces (TS `isIpv6Interface`).
    pub fn is_ipv6(&self) -> bool {
        matches!(self.address, IpAddr::V6(_))
    }
}
