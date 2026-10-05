//! IP network (CIDR) parsing, canonical form, and containment.
//!
//! Ports the portable core of `packages/effect/src/net/IpNetwork.ts`
//! (strict CIDR parsing, canonical text, first/last/count, address and
//! network containment). Schema codecs are not modeled here.

use std::net::IpAddr;

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

    /// Lenient unsafe constructor: masks host bits instead of rejecting
    /// them, still panicking on out-of-range prefixes (TS
    /// `fromAddressUnsafe`).
    pub fn from_address_unsafe(addr: IpAddr, prefix: f64) -> Self {
        let max = match addr {
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
        let bits = prefix as u8;
        Self {
            network: mask(addr, bits),
            prefix: bits,
        }
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
}

/// A rejected network construction, carrying the offending input.
/// `tag`/`message` mirror TS `NetAddressError` failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkError {
    pub input: NetworkErrorInput,
    pub tag: &'static str,
    pub message: String,
}

impl NetworkError {
    fn addr(addr: IpAddr, message: String) -> Self {
        Self {
            input: NetworkErrorInput::Addr(addr),
            tag: "NetAddressError",
            message,
        }
    }

    fn text(text: &str, message: String) -> Self {
        Self {
            input: NetworkErrorInput::Text(text.to_string()),
            tag: "NetAddressError",
            message,
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

impl IpInterface {
    /// Parse, panicking on invalid input. The throwing counterpart of
    /// [`Self::parse`] for trusted literals.
    pub fn must_parse(text: &str) -> Self {
        Self::parse(text).expect("valid interface")
    }
}
