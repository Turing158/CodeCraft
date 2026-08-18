//! Local IPv4 discovery for the LAN web console.
//!
//! The console has to tell the user which URL to open on a phone, so the
//! addresses have to match what other machines on the same subnet can reach.
//! Windows exposes the authoritative list through GetAdaptersAddresses; the
//! portable fallbacks (outbound-route probe and hostname resolution) keep the
//! feature usable when that call is unavailable.

use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs, UdpSocket};

/// Orders candidate addresses so the most likely LAN address comes first.
/// Private ranges beat link-local, and link-local beats anything else.
fn address_rank(address: Ipv4Addr) -> u8 {
    if address.is_private() {
        0
    } else if address.is_link_local() {
        2
    } else {
        1
    }
}

pub(crate) fn is_usable_lan_address(address: Ipv4Addr) -> bool {
    !address.is_loopback()
        && !address.is_unspecified()
        && !address.is_multicast()
        && !address.is_broadcast()
        && !address.is_documentation()
}

/// Sorts and de-duplicates discovered addresses, keeping the preferred address
/// first so the settings panel can highlight a single primary URL.
pub(crate) fn rank_addresses(
    preferred: Option<Ipv4Addr>,
    candidates: impl IntoIterator<Item = Ipv4Addr>,
) -> Vec<Ipv4Addr> {
    let mut addresses: Vec<Ipv4Addr> = Vec::new();
    for address in preferred.into_iter().chain(candidates) {
        if is_usable_lan_address(address) && !addresses.contains(&address) {
            addresses.push(address);
        }
    }
    let preferred = preferred.filter(|address| is_usable_lan_address(*address));
    addresses.sort_by_key(|address| {
        (
            if Some(*address) == preferred { 0 } else { 1 },
            address_rank(*address),
            address.octets(),
        )
    });
    addresses
}

/// Asks the routing table which local address would be used to leave this
/// machine. Connecting a UDP socket only selects a route, it sends no packets.
fn outbound_route_address() -> Option<Ipv4Addr> {
    const PROBE_TARGETS: [&str; 2] = ["10.255.255.255:9", "1.1.1.1:9"];
    for target in PROBE_TARGETS {
        let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
            continue;
        };
        if socket.connect(target).is_err() {
            continue;
        }
        if let Ok(IpAddr::V4(address)) = socket.local_addr().map(|address| address.ip()) {
            if is_usable_lan_address(address) {
                return Some(address);
            }
        }
    }
    None
}

fn hostname() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

/// Resolves the machine's own hostname, which normally yields every bound IPv4.
fn hostname_addresses() -> Vec<Ipv4Addr> {
    let Some(hostname) = hostname() else {
        return Vec::new();
    };
    format!("{hostname}:0")
        .to_socket_addrs()
        .map(|addresses| {
            addresses
                .filter_map(|address| match address.ip() {
                    IpAddr::V4(address) => Some(address),
                    IpAddr::V6(_) => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(windows)]
fn adapter_addresses() -> Vec<Ipv4Addr> {
    use windows::Win32::{
        Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS},
        NetworkManagement::IpHelper::{
            GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
            GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
        },
        NetworkManagement::Ndis::IfOperStatusUp,
        Networking::WinSock::{AF_INET, AF_UNSPEC, SOCKADDR_IN},
    };

    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size: u32 = 16 * 1024;
    let mut buffer: Vec<u8> = Vec::new();

    // The required size is only known after the first call, and it can grow
    // between calls when adapters appear, so retry a bounded number of times.
    for _ in 0..4 {
        buffer.clear();
        buffer.resize(size as usize, 0);
        let result = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_UNSPEC.0),
                flags,
                None,
                Some(buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>()),
                &mut size,
            )
        };
        if result == ERROR_BUFFER_OVERFLOW.0 {
            continue;
        }
        if result != ERROR_SUCCESS.0 {
            return Vec::new();
        }

        let mut addresses = Vec::new();
        let mut adapter = buffer.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        while !adapter.is_null() {
            let entry = unsafe { &*adapter };
            if entry.OperStatus == IfOperStatusUp {
                let mut unicast = entry.FirstUnicastAddress;
                while !unicast.is_null() {
                    let unicast_entry = unsafe { &*unicast };
                    let socket_address = unicast_entry.Address.lpSockaddr;
                    if !socket_address.is_null() && unsafe { (*socket_address).sa_family } == AF_INET
                    {
                        let socket_address = socket_address.cast::<SOCKADDR_IN>();
                        let octets =
                            unsafe { (*socket_address).sin_addr.S_un.S_addr }.to_ne_bytes();
                        addresses.push(Ipv4Addr::from(octets));
                    }
                    unicast = unicast_entry.Next;
                }
            }
            adapter = entry.Next;
        }
        return addresses;
    }

    Vec::new()
}

#[cfg(not(windows))]
fn adapter_addresses() -> Vec<Ipv4Addr> {
    Vec::new()
}

/// Every IPv4 address a phone on the same network could use, best guess first.
pub(crate) fn local_lan_addresses() -> Vec<Ipv4Addr> {
    let preferred = outbound_route_address();
    let mut candidates = adapter_addresses();
    if candidates.is_empty() {
        candidates = hostname_addresses();
    } else {
        candidates.extend(hostname_addresses());
    }
    rank_addresses(preferred, candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_addresses_a_phone_could_never_reach() {
        assert!(!is_usable_lan_address(Ipv4Addr::LOCALHOST));
        assert!(!is_usable_lan_address(Ipv4Addr::UNSPECIFIED));
        assert!(!is_usable_lan_address(Ipv4Addr::BROADCAST));
        assert!(is_usable_lan_address(Ipv4Addr::new(192, 168, 1, 24)));
    }

    #[test]
    fn keeps_the_routed_address_first_and_removes_duplicates() {
        let ranked = rank_addresses(
            Some(Ipv4Addr::new(192, 168, 1, 24)),
            [
                Ipv4Addr::new(169, 254, 8, 1),
                Ipv4Addr::new(10, 0, 0, 5),
                Ipv4Addr::new(192, 168, 1, 24),
                Ipv4Addr::LOCALHOST,
            ],
        );

        assert_eq!(
            ranked,
            vec![
                Ipv4Addr::new(192, 168, 1, 24),
                Ipv4Addr::new(10, 0, 0, 5),
                Ipv4Addr::new(169, 254, 8, 1),
            ]
        );
    }

    #[test]
    fn prefers_private_ranges_when_no_route_is_known() {
        let ranked = rank_addresses(
            None,
            [Ipv4Addr::new(169, 254, 8, 1), Ipv4Addr::new(172, 16, 3, 9)],
        );

        assert_eq!(ranked.first(), Some(&Ipv4Addr::new(172, 16, 3, 9)));
    }
}
