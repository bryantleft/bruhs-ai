//! The one rule for which addresses count as the public internet, shared by
//! every client that must never reach this computer or a private network:
//! media downloads (`rich_responses`) and app calls (`apps`, ruling F7).

use std::net::IpAddr;

/// Whether `ip` is a public, globally routed address. IPv4 refuses private,
/// loopback, link-local, unspecified, multicast, broadcast, documentation,
/// 0/8, 240/4, shared (100.64/10), benchmark (198.18/15) and the IETF and
/// 6to4 relay blocks (192.0.0/24, 192.88.99/24). IPv6 allows only global
/// unicast (`2000::/3`) outside 6to4 (`2002::/16`), the IETF block
/// (`2001::/23`) and documentation (`2001:db8::/32`), so NAT64 and 6to4
/// forms that wrap a private IPv4 address never pass; a mapped IPv4 address
/// is judged as IPv4.
pub(crate) fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (b == 18 || b == 19))
                && !(a == 192 && (b == 0 || b == 88))
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(mapped));
            }
            let parts = ip.segments();
            parts[0] & 0xe000 == 0x2000
                && parts[0] != 0x2002
                && !(parts[0] == 0x2001 && (parts[1] < 0x200 || parts[1] == 0xdb8))
        }
    }
}
