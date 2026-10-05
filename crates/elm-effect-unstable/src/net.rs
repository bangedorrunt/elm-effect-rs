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
