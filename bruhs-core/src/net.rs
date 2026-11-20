//! The one rule for which addresses count as the public internet, shared by
//! every client that must never reach this computer or a private network:
//! media downloads (`rich_responses`) and app calls (`apps`, ruling F7).

use std::net::IpAddr;

/// Whether `ip` is a public, globally routed address. IPv4 refuses private,
/// loopback, link-local, unspecified, multicast, broadcast, documentation,
/// 0/8, 240/4, shared (100.64/10), benchmark (198.18/15) and the IETF and
/// 6to4 relay blocks (192.0.0/24, 192.88.99/24). IPv6 allows only global
/// unicast (`2000::/3`) outside 6to4 (`2002::/16`), the IETF block
/// (`2001::/23`) and documentation (`2001:db8::/32`, `3fff::/20`), so NAT64 and 6to4
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
                && !(parts[0] == 0x3fff && parts[1] < 0x1000)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::public_ip;

    #[test]
    fn documentation_ranges_are_not_public() {
        for ip in ["2001:db8::1", "3fff::1", "3fff:fff:ffff::1", "192.0.2.1"] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "3fff:1000::1",
            "3ffe::1",
            "2606:4700:4700::1111",
            "140.82.112.6",
        ] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }
}
